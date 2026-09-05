mod support;

use narrata_nodes::{SaveArchive, Session, parse_json};
use serde_json::json;
use support::{TestResult, click, compile_error, product, source};

#[test]
fn calls_bind_parameters_keep_locals_isolated_and_return_to_caller() -> TestResult {
    let mut s = Session::new(product(source())?)?;
    let first = click(&mut s, "visit")?;
    assert_eq!(first.frames.len(), 2);
    assert_eq!(first.instance, 2);
    assert_eq!(first.paragraphs, vec!["阿岚，本次局部计数 1，累计 1。"]);
    let returned = click(&mut s, "continue")?;
    assert_eq!(returned.node.node, "start");
    assert_eq!(returned.frames.len(), 1);
    let second = click(&mut s, "visit")?;
    assert_eq!(second.instance, 3);
    assert_eq!(second.paragraphs, vec!["阿岚，本次局部计数 1，累计 2。"]);
    Ok(())
}

#[test]
fn disabled_hidden_and_stale_actions_cannot_change_state() -> TestResult {
    let mut s = Session::new(product(source())?)?;
    let before = s.save()?;
    let root = s.cursor()?.to_owned();
    let view = s.view()?;
    assert!(view.actions.iter().any(|a| a.id == "locked" && !a.enabled));
    assert!(!view.actions.iter().any(|a| a.id == "hidden"));
    for action in ["locked", "hidden", "missing"] {
        assert!(s.select(&root, action).is_err());
        assert_eq!(s.save()?, before);
    }
    click(&mut s, "visit")?;
    let after = s.save()?;
    assert_eq!(
        s.select(&root, "finish")
            .err()
            .ok_or("expected stale error")?
            .code,
        "stale_input"
    );
    assert_eq!(s.save()?, after);
    Ok(())
}

#[test]
fn arithmetic_failure_and_automatic_cycles_abort_the_entire_action() -> TestResult {
    let mut raw = source();
    raw["product"]["shared"]["visits"] = json!(i64::MAX);
    let mut s = Session::new(product(raw)?)?;
    let before = s.save()?;
    assert_eq!(
        click(&mut s, "visit")
            .err()
            .ok_or("expected overflow")?
            .to_string()
            .split_whitespace()
            .next(),
        Some("overflow")
    );
    assert_eq!(s.save()?, before);
    let mut raw = source();
    raw["packages"]["camp"]["graphs"]["visit"]["nodes"]["increment"]["data"]["next"] =
        json!("increment");
    let mut s = Session::new(product(raw)?)?;
    let before = s.save()?;
    assert!(click(&mut s, "visit").is_err());
    assert_eq!(s.save()?, before);
    Ok(())
}

#[test]
fn save_restores_full_snapshots_all_branches_and_exact_instance() -> TestResult {
    let p = product(source())?;
    let mut s = Session::new(p.clone())?;
    let root = s.cursor()?.to_owned();
    let child = click(&mut s, "visit")?.cursor;
    s.checkout(&root)?;
    assert!(click(&mut s, "finish")?.finished);
    s.checkout(&child)?;
    let saved = s.save()?;
    let archive: SaveArchive = parse_json(&saved)?;
    assert_eq!(archive.commits.len(), 3);
    assert!(
        archive
            .commits
            .iter()
            .all(|c| c.snapshot.get("shared").is_some())
    );
    assert!(
        archive
            .commits
            .iter()
            .all(|c| c.snapshot.get("scene").is_none())
    );
    let restored = Session::restore(p, &saved)?;
    assert_eq!(restored.view()?, s.view()?);
    assert_eq!(restored.save()?, saved);
    Ok(())
}

#[test]
fn untrusted_saves_reject_tampering_unknown_parents_and_duplicate_history() -> TestResult {
    let p = product(source())?;
    let mut s = Session::new(p.clone())?;
    click(&mut s, "visit")?;
    let archive: SaveArchive = parse_json(&s.save()?)?;
    let mut bad = archive.clone();
    bad.commits[1].snapshot["shared"]["visits"] = json!(400);
    assert!(Session::restore(p.clone(), &serde_json::to_string(&bad)?).is_err());
    let mut bad = archive.clone();
    bad.commits[1].parent = Some("unknown".into());
    assert!(Session::restore(p.clone(), &serde_json::to_string(&bad)?).is_err());
    let mut bad = archive.clone();
    bad.commits.push(bad.commits[1].clone());
    assert!(Session::restore(p.clone(), &serde_json::to_string(&bad)?).is_err());
    let mut changed = source();
    changed["product"]["shared"]["visits"] = json!(1);
    assert!(Session::restore(product(changed)?, &serde_json::to_string(&archive)?).is_err());
    Ok(())
}

#[test]
fn repeat_replay_is_deterministic_and_reuses_existing_branch() -> TestResult {
    let p = product(source())?;
    let mut a = Session::new(p.clone())?;
    let mut b = Session::new(p.clone())?;
    for _ in 0..15 {
        assert_eq!(click(&mut a, "visit")?, click(&mut b, "visit")?);
        b = Session::restore(p.clone(), &b.save()?)?;
        assert_eq!(click(&mut a, "continue")?, click(&mut b, "continue")?);
    }
    let root = a.view()?.history.first().ok_or("missing root")?.id.clone();
    let len = a.view()?.history.len();
    a.checkout(&root)?;
    click(&mut a, "visit")?;
    assert_eq!(a.view()?.history.len(), len);
    Ok(())
}

#[test]
fn expression_short_circuit_and_large_integer_views_preserve_semantics() -> TestResult {
    let mut raw = source();
    raw["product"]["shared"]["visits"] = json!(i64::MAX);
    raw["packages"]["main"]["graphs"]["journey"]["nodes"]["start"]["data"]["choices"][1]["enabled_if"] = json!({
        "kind":"binary","op":"or","left":{"kind":"literal","value":true},
        "right":{"kind":"binary","op":"gt","left":{"kind":"binary","op":"add","left":{"kind":"read","scope":"shared","name":"visits"},"right":{"kind":"literal","value":1}},"right":{"kind":"literal","value":0}}
    });
    let s = Session::new(product(raw)?)?;
    assert!(
        s.view()?
            .actions
            .iter()
            .any(|a| a.id == "locked" && a.enabled)
    );
    assert_eq!(
        s.view()?
            .shared
            .iter()
            .find(|v| v.name == "visits")
            .ok_or("variable missing")?
            .value,
        i64::MAX.to_string()
    );
    // Also exercise the shared source helper in this integration-test binary.
    let mut raw = source();
    raw["format_version"] = json!(2);
    assert_eq!(compile_error(raw)?, "version");
    Ok(())
}

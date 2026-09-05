use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use narrata_node_tools::{load_bundle, load_project, read_text, verify_lock, write_text};
use narrata_nodes::{Bundle, NodeRegistry, Session, compile, parse_json};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn demo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../../products/gamebook-demo")
}

fn copy_project(target: &Path) -> std::io::Result<()> {
    fs::create_dir_all(target.join("packages"))?;
    for name in [
        "project.json",
        "project.lock.json",
        "packages/main.json",
        "packages/road.json",
        "packages/camp.json",
    ] {
        fs::copy(demo().join(name), target.join(name))?;
    }
    Ok(())
}

#[test]
fn three_packages_compose_outside_checkout_and_complete_the_story() -> TestResult {
    let directory = tempfile::tempdir()?;
    copy_project(directory.path())?;
    let compiled = load_project(&directory.path().join("project.json"))?;
    verify_lock(
        &compiled.product,
        &directory.path().join("project.lock.json"),
    )?;
    assert_eq!(compiled.product.lock().packages.len(), 3);
    let mut session = Session::new(Arc::new(compiled.product))?;
    for action in [
        "camp", "letter", "continue", "rest", "continue", "road", "help", "continue", "deliver",
    ] {
        let cursor = session.cursor()?.to_owned();
        session.select(&cursor, action)?;
    }
    let view = session.view()?;
    assert_eq!(view.outcome.as_deref(), Some("delivered"));
    assert_eq!(
        view.shared
            .iter()
            .find(|v| v.name == "rations")
            .ok_or("missing rations")?
            .value,
        "1"
    );
    assert_eq!(
        view.history
            .iter()
            .filter(|h| h.node.package == "camp")
            .count(),
        4
    );
    Ok(())
}

#[test]
fn changing_a_provider_changes_behavior_without_editing_consumers() -> TestResult {
    let directory = tempfile::tempdir()?;
    copy_project(directory.path())?;
    let original = load_project(&directory.path().join("project.json"))?;
    let main = read_text(&directory.path().join("packages/main.json"))?;
    let path = directory.path().join("packages/camp.json");
    let mut camp: serde_json::Value = parse_json(&read_text(&path)?)?;
    camp["id"] = serde_json::json!("example.alternate-camp");
    camp["graphs"]["visit"]["nodes"]["fire"]["data"]["choices"][1]["assignments"][0]["value"]["right"]
        ["value"] = serde_json::json!(2);
    write_text(&path, &serde_json::to_string_pretty(&camp)?)?;
    let changed = load_project(&directory.path().join("project.json"))?;
    assert_ne!(
        original.product.artifact_id(),
        changed.product.artifact_id()
    );
    assert!(
        verify_lock(
            &changed.product,
            &directory.path().join("project.lock.json")
        )
        .is_err()
    );
    assert_eq!(
        read_text(&directory.path().join("packages/main.json"))?,
        main
    );
    let mut session = Session::new(Arc::new(changed.product))?;
    for action in ["camp", "rest"] {
        let cursor = session.cursor()?.to_owned();
        session.select(&cursor, action)?;
    }
    assert_eq!(
        session
            .view()?
            .shared
            .iter()
            .find(|v| v.name == "rations")
            .ok_or("missing rations")?
            .value,
        "1"
    );
    Ok(())
}

#[test]
fn source_paths_cannot_escape_the_project_closure() -> TestResult {
    let directory = tempfile::tempdir()?;
    copy_project(directory.path())?;
    let manifest_path = directory.path().join("project.json");
    let mut project: serde_json::Value = parse_json(&read_text(&manifest_path)?)?;
    for path in [
        "../outside.json",
        "C:\\outside.json",
        "/outside.json",
        "packages/camp.json:stream",
    ] {
        project["packages"]["camp"] = serde_json::json!(path);
        write_text(&manifest_path, &serde_json::to_string(&project)?)?;
        assert_eq!(
            load_project(&manifest_path)
                .err()
                .ok_or("expected path rejection")?
                .code,
            "path"
        );
    }
    Ok(())
}

#[test]
fn generated_sample_is_reproducible_and_portable() -> TestResult {
    let composed = load_project(&demo().join("project.json"))?;
    let frozen = load_bundle(&demo().join("story.nar.json"))?;
    assert_eq!(composed.product.artifact_id(), frozen.product.artifact_id());
    let serialized = serde_json::to_string(composed.product.source())?;
    let bundled: Bundle = parse_json(&serialized)?;
    let recompiled = compile(bundled, &NodeRegistry::gamebook())?;
    assert_eq!(
        recompiled.product.artifact_id(),
        frozen.product.artifact_id()
    );
    Ok(())
}

#[test]
fn output_replacement_leaves_no_partial_file() -> TestResult {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("save.json");
    write_text(&path, "old")?;
    write_text(&path, "new")?;
    assert_eq!(read_text(&path)?, "new");
    assert_eq!(fs::read_dir(directory.path())?.count(), 1);
    Ok(())
}

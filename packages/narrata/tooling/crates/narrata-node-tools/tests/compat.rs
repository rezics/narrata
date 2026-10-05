//! The frozen R2 corpus (`fixtures/compat/nodes-r2`) stays readable, and the frozen R1 corpus
//! still migrates to exactly its bytes.

#![allow(clippy::panic, clippy::unwrap_used)]

use std::path::{Path, PathBuf};

use narrata_content_local::{ContentPack, LocalContent};
use narrata_kernel::codec::sha256;
use narrata_node_tools::{load_content, open_pack, pretty, read_bytes, read_text, run};
use narrata_nodes::{
    NameTable, Scalar, Session,
    r1::{RefText, migrate_save},
    view::Interaction,
};
use serde_json::Value;

const EXECUTION: &str = "execution:0190f2a0000070008000000000000001";
const SAVES: [(&str, usize, &str); 2] = [("linear", 10, "delivered"), ("branched", 15, "arrived")];

fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../..")
}

fn corpus() -> PathBuf {
    repository().join("fixtures/compat/nodes-r2")
}

fn r1_corpus() -> PathBuf {
    repository().join("fixtures/compat/nodes-r1")
}

fn manifest() -> Value {
    serde_json::from_str(&read_text(&corpus().join("manifest.json")).unwrap()).unwrap()
}

fn content() -> LocalContent {
    load_content(&[corpus().join("zh-Hans.json").display().to_string()]).unwrap()
}

fn migrate(pack: &Path, content: &LocalContent, save: &str) -> Session {
    let (program, names) = open_pack(pack).unwrap();
    let ref_text = |reference: &_| content.text(reference, &[]);
    let ref_text: RefText<'_> = &ref_text;
    let save = read_text(&r1_corpus().join(format!("{save}.save.json"))).unwrap();
    migrate_save(
        program,
        &names.unwrap(),
        &save,
        EXECUTION.parse().unwrap(),
        ref_text,
    )
    .unwrap()
}

#[test]
fn every_frozen_file_matches_its_recorded_digest() {
    let manifest = manifest();
    let artifacts = manifest["artifacts"].as_object().unwrap();
    assert_eq!(artifacts.len(), 5);
    for (name, entry) in artifacts {
        let bytes = read_bytes(&corpus().join(name)).unwrap();
        assert_eq!(
            hex::encode(sha256(&bytes)),
            entry["sha256"].as_str().unwrap(),
            "{name}"
        );
    }
}

#[test]
fn the_frozen_pack_opens_with_its_recorded_identity_and_r1_origin() {
    let manifest = manifest();
    let (program, names) = open_pack(&corpus().join("story.narpack")).unwrap();
    program.verify_artifact().unwrap();
    assert_eq!(
        program.artifact_id().to_string(),
        manifest["artifact_id"].as_str().unwrap()
    );
    let r1: Value =
        serde_json::from_str(&read_text(&r1_corpus().join("manifest.json")).unwrap()).unwrap();
    let origin = names.unwrap().migrated_from_r1.map(|id| hex::encode(id.0));
    assert_eq!(origin.as_deref(), r1["artifact_id"].as_str());
    assert_eq!(origin.as_deref(), manifest["migrated_from_r1"].as_str());
}

#[test]
fn r1_saves_still_migrate_to_the_frozen_exports() {
    let content = content();
    for (save, _, _) in SAVES {
        let session = migrate(&corpus().join("story.narpack"), &content, save);
        let frozen = read_text(&corpus().join(format!("{save}.export.json"))).unwrap();
        assert_eq!(session.export().unwrap(), frozen, "{save}");
    }
}

#[test]
fn frozen_exports_restore_without_replay_and_verify_from_the_root() {
    let (program, _) = open_pack(&corpus().join("story.narpack")).unwrap();
    for (save, commits, outcome) in SAVES {
        let text = read_text(&corpus().join(format!("{save}.export.json"))).unwrap();
        let session = Session::restore(program.clone(), &text).unwrap();
        assert_eq!(session.commits().count(), commits, "{save}");
        for (id, _) in session.commits() {
            session.verify_path(&id).unwrap();
        }
        let Interaction::Finished { outcome: ended, .. } = session.view(None).unwrap().interaction
        else {
            panic!("{save} should end");
        };
        assert_eq!(ended, outcome);
    }
}

#[test]
fn the_frozen_outline_is_the_content_packs_outline() {
    let pack = ContentPack::parse(&read_text(&corpus().join("zh-Hans.json")).unwrap()).unwrap();
    let outline = pretty(&narrata_content_local::outline(&pack)).unwrap();
    assert_eq!(
        outline,
        read_text(&corpus().join("zh-Hans.outline.json")).unwrap()
    );
}

/// Each commit as its parent's position, its frames by alias, the given shared values as text
/// and its outcome.
fn timeline(
    mut session: Session,
    names: &NameTable,
    content: &LocalContent,
    shared: &[String],
) -> Vec<String> {
    let commits: Vec<_> = session
        .commits()
        .map(|(id, commit)| (id, commit.parent))
        .collect();
    let text = |value: &Scalar| match value {
        Scalar::Bool(value) => value.to_string(),
        Scalar::Int(value) => value.to_string(),
        Scalar::Text(value) => value.clone(),
        Scalar::Ref(reference) => content.text(reference, &[]).unwrap(),
    };
    let mut out = Vec::new();
    for (id, parent) in &commits {
        session.checkout(id).unwrap();
        let state = session.state().unwrap();
        let parent = parent.and_then(|parent| commits.iter().position(|(id, _)| *id == parent));
        let frames: Vec<String> = state
            .frames
            .iter()
            .map(|frame| {
                format!(
                    "{}.{}.{}",
                    frame.graph.package,
                    frame.graph.graph,
                    names.node(&frame.graph, &frame.node).unwrap()
                )
            })
            .collect();
        let values: Vec<String> = shared
            .iter()
            .map(|name| format!("{name}={}", text(&state.shared[name])))
            .collect();
        let outcome = state.finished.as_ref().map(|finished| &finished.outcome);
        out.push(format!("{parent:?} {frames:?} {values:?} {outcome:?}"));
    }
    out
}

#[test]
fn the_r1_source_migration_and_the_frozen_pack_agree_on_both_r1_saves() {
    let directory = tempfile::tempdir().unwrap();
    let out = directory.path();
    let arg = |path: &Path| path.display().to_string();
    let project = r1_corpus().join("project/project.json");
    run(&["migrate-r1".into(), arg(&project), "--out".into(), arg(out)]).unwrap();
    let pure = out.join("story.narpack");
    let manifest = out.join("project.json");
    run(&["compose".into(), arg(&manifest), "--out".into(), arg(&pure)]).unwrap();
    let pure_content = load_content(&[arg(&out.join("content/zh-Hans.json"))]).unwrap();
    let frozen = corpus().join("story.narpack");
    let frozen_content = content();
    let (_, pure_names) = open_pack(&pure).unwrap();
    let (_, frozen_names) = open_pack(&frozen).unwrap();
    let (pure_names, frozen_names) = (pure_names.unwrap(), frozen_names.unwrap());
    for (save, commits, _) in SAVES {
        let migrated = migrate(&pure, &pure_content, save);
        // The R1 shared fields; the hand-cleaned work only adds fields of its own.
        let shared: Vec<String> = migrated.state().unwrap().shared.keys().cloned().collect();
        let expected = timeline(migrated, &pure_names, &pure_content, &shared);
        assert_eq!(expected.len(), commits);
        let actual = timeline(
            migrate(&frozen, &frozen_content, save),
            &frozen_names,
            &frozen_content,
            &shared,
        );
        assert_eq!(actual, expected, "{save}");
    }
}

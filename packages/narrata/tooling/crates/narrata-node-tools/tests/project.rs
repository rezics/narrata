#![allow(clippy::panic, clippy::unwrap_used)]

use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use narrata_content_local::{Block, ContentPack, Entry, LocalContent};
use narrata_node_tools::{
    act, compose, ids::Minter, load_content, load_project, open_pack, r1, read_text, render,
    verify_lock, write_text,
};
use narrata_nodes::{AuthoredId, Program, Session, SessionExport, r1::migrate_save};
use proptest::prelude::*;
use serde_json::{Value, json};

const R1_ARTIFACT: &str = "b73f74b82f33834c386134e1f5cd548d9ba304d2d299507d827c4fe7acad412b";
const EXECUTION: &str = "execution:0190f2a0000070008000000000000001";
const LINEAR: &str = "camp,letter,continue,rest,continue,road,help,continue,deliver";

fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../..")
}

fn r1_corpus() -> PathBuf {
    repository().join("fixtures/compat/nodes-r1")
}

fn demo() -> PathBuf {
    repository().join("products/gamebook-demo")
}

fn copy_demo(target: &Path) {
    for name in [
        "project.json",
        "project.lock.json",
        "packages/main.json",
        "packages/road.json",
        "packages/camp.json",
        "content/zh-Hans.json",
        "story.narpack",
    ] {
        fs::create_dir_all(target.join(name).parent().unwrap()).unwrap();
        fs::copy(demo().join(name), target.join(name)).unwrap();
    }
}

fn args(text: &[&str]) -> Vec<String> {
    text.iter().map(|value| (*value).to_owned()).collect()
}

fn run(text: &[&str]) -> Result<(), String> {
    narrata_node_tools::run(&args(text))
}

#[test]
fn the_r1_artifact_id_is_reproduced_from_the_frozen_sources() {
    let loaded = r1::load(&r1_corpus().join("project/project.json")).unwrap();
    assert_eq!(hex::encode(loaded.artifact_id().unwrap().0), R1_ARTIFACT);
}

#[test]
fn migrated_r1_sources_compose_and_keep_every_text_out_of_the_artifact() {
    let loaded = r1::load(&r1_corpus().join("project/project.json")).unwrap();
    let migrated = loaded
        .migrate("zh-Hans", &mut Minter::new(Vec::<AuthoredId>::new()))
        .unwrap();
    assert_eq!(
        migrated
            .manifest
            .migrated_from_r1
            .map(|id| hex::encode(id.0))
            .as_deref(),
        Some(R1_ARTIFACT)
    );
    let compilation = narrata_nodes::compile(
        &narrata_nodes::ProjectSource {
            manifest: migrated.manifest.clone(),
            packages: migrated.packages.clone(),
        },
        &narrata_nodes::NodeRegistry::gamebook(),
    )
    .unwrap();
    // Text lives only in the content pack; built-in R1 texts are extracted too.
    let texts: Vec<String> = migrated
        .content
        .entries
        .values()
        .flat_map(|entry| match entry {
            Entry::Text(text) => vec![text.text.clone()],
            Entry::Blocks(blocks) => blocks
                .blocks
                .iter()
                .filter_map(|block| match block {
                    narrata_content_local::Block::Text { text, .. } => Some(text.clone()),
                    narrata_content_local::Block::Marker { .. } => None,
                })
                .collect(),
        })
        .collect();
    assert!(texts.iter().any(|text| text == "旅程结束"));
    assert!(texts.iter().any(|text| text.contains("{greetings}")));
    let pack = String::from_utf8_lossy(&compilation.pack).into_owned();
    assert!(!pack.contains("篝火") && !pack.contains("旅程结束"));
}

/// Plays `actions` on a pack and returns every commit, in order, with its state ID.
fn commits(pack: &Path, actions: &str) -> Vec<String> {
    let (program, names) = open_pack(pack).unwrap();
    let mut session = Session::new(program, EXECUTION.parse().unwrap()).unwrap();
    act(&mut session, names.as_ref(), actions).unwrap();
    session
        .commits()
        .map(|(id, commit)| format!("{id} {}", commit.state))
        .collect()
}

#[test]
fn source_migration_and_both_r1_save_migrations_agree() {
    let directory = tempfile::tempdir().unwrap();
    let out = directory.path();
    run(&[
        "migrate-r1",
        r1_corpus().join("project/project.json").to_str().unwrap(),
        "--out",
        out.to_str().unwrap(),
    ])
    .unwrap();
    let pack = out.join("story.narpack");
    run(&[
        "compose",
        out.join("project.json").to_str().unwrap(),
        "--out",
        pack.to_str().unwrap(),
    ])
    .unwrap();
    let content = load_content(&[out.join("content/zh-Hans.json").display().to_string()]).unwrap();
    let ref_text = |reference: &narrata_kernel::content::ContentRef| content.text(reference, &[]);
    for save in ["linear", "branched"] {
        let (program, names) = open_pack(&pack).unwrap();
        let text = read_text(&r1_corpus().join(format!("{save}.save.json"))).unwrap();
        let session = migrate_save(
            program,
            &names.unwrap(),
            &text,
            EXECUTION.parse().unwrap(),
            &ref_text,
        )
        .unwrap();
        let migrated: Vec<String> = session
            .commits()
            .map(|(id, commit)| format!("{id} {}", commit.state))
            .collect();
        if save == "linear" {
            // Replaying the R1 actions on the migrated sources builds the same commits.
            assert_eq!(migrated, commits(&pack, LINEAR));
        } else {
            assert_eq!(migrated.len(), 15);
        }
        let finished = session.state().unwrap().finished.clone().unwrap();
        assert_eq!(
            finished.outcome,
            if save == "linear" {
                "delivered"
            } else {
                "arrived"
            }
        );
    }
}

#[test]
fn r1_saves_migrate_onto_the_demo_and_play_through_the_migrated_exports() {
    let (program, names) = open_pack(&demo().join("story.narpack")).unwrap();
    let names = names.unwrap();
    let content =
        load_content(&[demo().join("content/zh-Hans.json").display().to_string()]).unwrap();
    let ref_text = |reference: &narrata_kernel::content::ContentRef| content.text(reference, &[]);
    let text = read_text(&r1_corpus().join("linear.save.json")).unwrap();
    let session = migrate_save(
        program.clone(),
        &names,
        &text,
        EXECUTION.parse().unwrap(),
        &ref_text,
    )
    .unwrap();
    let mut played = Session::new(program.clone(), EXECUTION.parse().unwrap()).unwrap();
    act(&mut played, Some(&names), LINEAR).unwrap();
    assert_eq!(session.export().unwrap(), played.export().unwrap());
    let shown = render(&session, Some(&names), &content, Vec::new()).unwrap();
    assert!(
        shown.contains("# 旅程结束") && shown.contains("(outcome: delivered)"),
        "{shown}"
    );
    // A save of another R1 artifact is refused.
    let mut other: Value = serde_json::from_str(&text).unwrap();
    other["artifact_id"] = json!("00".repeat(32));
    let error = migrate_save(
        program,
        &names,
        &other.to_string(),
        EXECUTION.parse().unwrap(),
        &ref_text,
    )
    .err()
    .unwrap();
    assert_eq!(error.code, "incompatible_save");
}

#[test]
fn the_demo_composes_locked_and_reproduces_its_pack() {
    let directory = tempfile::tempdir().unwrap();
    copy_demo(directory.path());
    let mut project = load_project(&directory.path().join("project.json")).unwrap();
    let previous = fs::read(directory.path().join("story.narpack")).unwrap();
    let composed = compose(&mut project, Some(&previous), true).unwrap();
    verify_lock(&composed.compilation.lock, &project).unwrap();
    assert!(composed.compared && composed.appended.is_empty());
    assert_eq!(composed.compilation.pack, previous);
    assert_eq!(composed.compilation.lock.packages.len(), 3);
}

#[test]
fn the_local_choice_and_the_multiple_selection_leave_the_original_routes_unchanged() {
    let pack = demo().join("story.narpack");
    let (program, names) = open_pack(&pack).unwrap();
    let content =
        load_content(&[demo().join("content/zh-Hans.json").display().to_string()]).unwrap();
    let mut session = Session::new(program, EXECUTION.parse().unwrap()).unwrap();
    act(&mut session, names.as_ref(), "ledger,sign").unwrap();
    let shown = render(&session, names.as_ref(), &content, Vec::new()).unwrap();
    assert!(
        shown.contains("> 你写下自己的名字") && shown.contains("choose 0..2:"),
        "{shown}"
    );
    act(&mut session, names.as_ref(), "candle+flask").unwrap();
    let state = session.state().unwrap();
    assert_eq!(
        state.shared.get("packed"),
        Some(&narrata_nodes::Scalar::Int(2))
    );
    assert_eq!(
        state.shared.get("signed"),
        Some(&narrata_nodes::Scalar::Bool(true))
    );
    // Back at the station the ledger is gone and the R1 route continues as before.
    let shown = render(&session, names.as_ref(), &content, Vec::new()).unwrap();
    assert!(
        !shown.contains("[ledger]") && shown.contains("[road]"),
        "{shown}"
    );
    act(&mut session, names.as_ref(), LINEAR).unwrap();
    assert_eq!(
        session.state().unwrap().finished.as_ref().unwrap().outcome,
        "delivered"
    );
}

/// The artifact, every commit with its state and every rendered page of the linear route,
/// resolving each page in `language` before acting as a reader would.
fn read_through(content: &LocalContent, language: &str) -> (String, Vec<String>, String) {
    let (program, names) = open_pack(&demo().join("story.narpack")).unwrap();
    let artifact = program.artifact_id().to_string();
    let mut session = Session::new(program, EXECUTION.parse().unwrap()).unwrap();
    let mut pages = String::new();
    for step in LINEAR.split(',') {
        pages += &render(&session, names.as_ref(), content, vec![language.into()]).unwrap();
        act(&mut session, names.as_ref(), step).unwrap();
    }
    let commits = session
        .commits()
        .map(|(id, commit)| format!("{id} {}", commit.state))
        .collect();
    (artifact, commits, pages)
}

fn migrated_linear_save(content: &LocalContent) -> String {
    let (program, names) = open_pack(&demo().join("story.narpack")).unwrap();
    let save = read_text(&r1_corpus().join("linear.save.json")).unwrap();
    let ref_text = |reference: &narrata_kernel::content::ContentRef| content.text(reference, &[]);
    migrate_save(
        program,
        &names.unwrap(),
        &save,
        EXECUTION.parse().unwrap(),
        &ref_text,
    )
    .unwrap()
    .export()
    .unwrap()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(16))]

    /// Text lives only in content packs: reading through a second language whose every text is
    /// rewritten keeps the artifact, every commit and state, and the R1 save migration.
    #[test]
    fn rewritten_text_in_another_language_keeps_every_commit_and_state(
        language in "(en|fr|de|ja)",
        prefix in "[^{}\\p{C}]{1,12}",
    ) {
        let original =
            ContentPack::parse(&read_text(&demo().join("content/zh-Hans.json")).unwrap()).unwrap();
        let mut translated = original.clone();
        translated.language = language.clone();
        for entry in translated.entries.values_mut() {
            match entry {
                Entry::Text(entry) => entry.text = format!("{prefix}{}", entry.text),
                Entry::Blocks(entry) => {
                    for block in &mut entry.blocks {
                        if let Block::Text { text, .. } = block {
                            *text = format!("{prefix}{text}");
                        }
                    }
                }
            }
        }
        let mut only = LocalContent::new();
        only.add(original.clone()).unwrap();
        let mut both = LocalContent::new();
        both.add(original).unwrap();
        both.add(translated).unwrap();
        let (artifact, commits, pages) = read_through(&only, "zh-Hans");
        let (other_artifact, other_commits, other_pages) = read_through(&both, &language);
        prop_assert_eq!(artifact, other_artifact);
        prop_assert_eq!(commits, other_commits);
        prop_assert_ne!(pages, other_pages);
        prop_assert_eq!(migrated_linear_save(&only), migrated_linear_save(&both));
    }
}

#[test]
fn compose_appends_tombstones_for_deleted_ids_and_refuses_when_locked() {
    let directory = tempfile::tempdir().unwrap();
    copy_demo(directory.path());
    let road = directory.path().join("packages/road.json");
    let mut package: Value = serde_json::from_str(&read_text(&road).unwrap()).unwrap();
    let options =
        package["graphs"]["crossing"]["nodes"]["rocks"]["data"]["choice_points"][0]["options"]
            .as_array_mut()
            .unwrap();
    let removed = options.remove(1);
    write_text(&road, &serde_json::to_string_pretty(&package).unwrap()).unwrap();
    let previous = fs::read(directory.path().join("story.narpack")).unwrap();
    let mut project = load_project(&directory.path().join("project.json")).unwrap();
    let error = compose(&mut project, Some(&previous), true).err().unwrap();
    assert_eq!(error.code, "tombstones_needed");
    let composed = compose(&mut project, Some(&previous), false).unwrap();
    assert_eq!(composed.appended.len(), 1);
    assert_eq!(
        composed.appended[0].1.to_string(),
        removed["id"].as_str().unwrap()
    );
    let written: Value = serde_json::from_str(&read_text(&road).unwrap()).unwrap();
    assert_eq!(written["tombstones"][0], removed["id"]);
    // Removing the tombstone again is refused against the new artifact.
    let mut project = load_project(&directory.path().join("project.json")).unwrap();
    project
        .source
        .packages
        .get_mut("road")
        .unwrap()
        .tombstones
        .clear();
    let error = compose(&mut project, Some(&composed.compilation.pack), false)
        .err()
        .unwrap();
    assert_eq!(error.code, "tombstone_removed");
}

#[test]
fn compose_refuses_a_live_id_that_changed_owner() {
    let directory = tempfile::tempdir().unwrap();
    copy_demo(directory.path());
    let road = directory.path().join("packages/road.json");
    let mut package: Value = serde_json::from_str(&read_text(&road).unwrap()).unwrap();
    let nodes = &mut package["graphs"]["crossing"]["nodes"];
    // Move the fog passage's option into the trail passage's choice point.
    let moved = nodes["fog"]["data"]["choice_points"][0]["options"][0]["id"].clone();
    nodes["fog"]["data"]["choice_points"][0]["options"][0]["id"] =
        nodes["trail"]["data"]["choice_points"][0]["options"][0]["id"].clone();
    nodes["trail"]["data"]["choice_points"][0]["options"][0]["id"] = moved;
    write_text(&road, &serde_json::to_string_pretty(&package).unwrap()).unwrap();
    let previous = fs::read(directory.path().join("story.narpack")).unwrap();
    let mut project = load_project(&directory.path().join("project.json")).unwrap();
    assert_eq!(
        compose(&mut project, Some(&previous), false)
            .err()
            .unwrap()
            .code,
        "owner_changed"
    );
}

#[test]
fn ids_mints_only_missing_ids_and_compile_rejects_missing_ones() {
    let directory = tempfile::tempdir().unwrap();
    copy_demo(directory.path());
    let camp = directory.path().join("packages/camp.json");
    let mut package: Value = serde_json::from_str(&read_text(&camp).unwrap()).unwrap();
    let kept = package["graphs"]["visit"]["nodes"]["fire"]["id"].clone();
    let nodes = package["graphs"]["visit"]["nodes"].as_object_mut().unwrap();
    nodes
        .get_mut("rest")
        .unwrap()
        .as_object_mut()
        .unwrap()
        .remove("id");
    let point = &mut nodes.get_mut("letter").unwrap()["data"]["choice_points"][0];
    point.as_object_mut().unwrap().remove("id");
    point["options"][0].as_object_mut().unwrap().remove("id");
    write_text(&camp, &serde_json::to_string_pretty(&package).unwrap()).unwrap();
    let manifest = directory.path().join("project.json");
    let mut project = load_project(&manifest).unwrap();
    assert_eq!(
        compose(&mut project, None, false).err().unwrap().code,
        "missing_id"
    );
    run(&["ids", manifest.to_str().unwrap()]).unwrap();
    let written: Value = serde_json::from_str(&read_text(&camp).unwrap()).unwrap();
    assert_eq!(written["graphs"]["visit"]["nodes"]["fire"]["id"], kept);
    let minted = written["graphs"]["visit"]["nodes"]["rest"]["id"]
        .as_str()
        .unwrap();
    // A version 7 UUID.
    assert_eq!(&minted[5 + 12..5 + 13], "7");
    let mut project = load_project(&manifest).unwrap();
    compose(&mut project, None, false).unwrap();
}

#[test]
fn run_prints_resolved_text_and_round_trips_exports() {
    let directory = tempfile::tempdir().unwrap();
    let pack = demo().join("story.narpack");
    let content = demo().join("content/zh-Hans.json");
    let save = directory.path().join("journey.json");
    run(&[
        "run",
        pack.to_str().unwrap(),
        "--content",
        content.to_str().unwrap(),
        "--actions",
        "camp,letter",
        "--save",
        save.to_str().unwrap(),
        "--execution",
        EXECUTION,
    ])
    .unwrap();
    let export: SessionExport = serde_json::from_str(&read_text(&save).unwrap()).unwrap();
    assert_eq!(export.execution.to_string(), EXECUTION);
    let (program, _) = open_pack(&pack).unwrap();
    let restored = Session::restore(program, &read_text(&save).unwrap()).unwrap();
    assert_eq!(restored.cursor().unwrap(), export.cursor);
    run(&[
        "run",
        pack.to_str().unwrap(),
        "--load",
        save.to_str().unwrap(),
        "--actions",
        "continue",
    ])
    .unwrap();
    assert!(run(&["run", pack.to_str().unwrap(), "--actions", "nowhere"]).is_err());
}

#[test]
fn inspect_and_schemas_write_readable_json() {
    let directory = tempfile::tempdir().unwrap();
    let (program, names) = open_pack(&demo().join("story.narpack")).unwrap();
    let inspected = narrata_node_tools::inspect(&program, names.as_ref()).unwrap();
    assert_eq!(inspected["product"]["id"], "mountain-letter");
    assert_eq!(inspected["migrated_from_r1"], R1_ARTIFACT);
    assert!(
        inspected["graphs"][0]["nodes"]
            .as_object()
            .unwrap()
            .contains_key("station")
            || inspected["graphs"]
                .as_array()
                .unwrap()
                .iter()
                .any(|graph| graph["nodes"].get("station").is_some())
    );
    run(&["schemas", "--out", directory.path().to_str().unwrap()]).unwrap();
    for name in [
        "book-view.schema.json",
        "content-pack.schema.json",
        "session-export.schema.json",
    ] {
        let schema: Value =
            serde_json::from_str(&read_text(&directory.path().join(name)).unwrap()).unwrap();
        assert!(schema.get("$schema").is_some(), "{name}");
    }
}

#[test]
fn package_paths_stay_inside_the_project_closure() {
    let directory = tempfile::tempdir().unwrap();
    copy_demo(directory.path());
    let manifest = directory.path().join("project.json");
    let mut value: Value = serde_json::from_str(&read_text(&manifest).unwrap()).unwrap();
    value["packages"]["road"] = json!("../road.json");
    write_text(&manifest, &value.to_string()).unwrap();
    assert_eq!(load_project(&manifest).err().unwrap().code, "path");
}

#[test]
fn a_pack_without_names_still_runs_but_actions_need_aliases() {
    let (program, _) = open_pack(&demo().join("story.narpack")).unwrap();
    let bare = program.pack(None).unwrap();
    let (program, names) = Program::from_pack(&bare).unwrap();
    assert!(names.is_none());
    let mut session = Session::new(Arc::new(program), EXECUTION.parse().unwrap()).unwrap();
    assert_eq!(act(&mut session, None, "camp").err().unwrap().code, "names");
}

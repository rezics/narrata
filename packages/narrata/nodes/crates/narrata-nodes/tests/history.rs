#![allow(clippy::unwrap_used)]
mod support;
use narrata_nodes::{Commit, KIND_COMMIT, Program, Session, SessionExport, decode_state};
use std::{collections::BTreeMap, sync::Arc};

fn corpus(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../../../fixtures/compat")
        .join(name)
}

#[test]
fn interim_corpora_remap_commits_and_preserve_states_inputs_and_cursor_paths() {
    for (directory, save) in [
        ("nodes-r2", "linear.export.json"),
        ("nodes-r2", "branched.export.json"),
        ("nodes-r2-proposals", "session.export.json"),
    ] {
        let (program, names) =
            Program::from_pack(&std::fs::read(corpus(directory).join("story.narpack")).unwrap())
                .unwrap();
        let program = Arc::new(program);
        let text = std::fs::read_to_string(corpus(directory).join(save)).unwrap();
        let old: SessionExport = serde_json::from_str(&text).unwrap();
        let mut session = Session::restore(program.clone(), &text).unwrap();
        let mut remap = BTreeMap::new();
        let mut expected = BTreeMap::new();
        for bytes in old.objects.iter().map(|text| hex::decode(text).unwrap()) {
            if let Ok((mut commit, old_id)) = Commit::decode_interim(&bytes) {
                commit.parent = commit.parent.map(|parent| remap[&parent]);
                let new_id = commit.id();
                remap.insert(old_id, new_id);
                expected.insert(new_id, commit.clone());
                let object = session
                    .history()
                    .reader()
                    .require(
                        narrata_history::ObjectId::from_bytes(*new_id.as_bytes()),
                        Some(KIND_COMMIT),
                    )
                    .unwrap();
                assert_eq!(object.payload(), commit.encode());
                assert_eq!(
                    narrata_history::Commit::from_object(&object, KIND_COMMIT)
                        .unwrap()
                        .encode(),
                    commit.encode()
                );
            } else {
                let kind = u16::from_be_bytes([bytes[10], bytes[11]]);
                let object =
                    narrata_history::Object::from_bytes(&bytes, kind, 1, 1024 * 1024).unwrap();
                assert_eq!(
                    session
                        .history()
                        .get_object(object.id())
                        .unwrap()
                        .unwrap()
                        .bytes(),
                    bytes
                );
            }
        }
        assert_eq!(session.cursor().unwrap(), remap[&old.cursor]);
        assert_ne!(session.cursor().unwrap(), old.cursor);
        assert_eq!(
            session.commits().unwrap().collect::<BTreeMap<_, _>>(),
            expected
        );
        for (id, header) in &expected {
            session.checkout(id).unwrap();
            let state_bytes = old
                .objects
                .iter()
                .map(|text| hex::decode(text).unwrap())
                .find(|bytes| decode_state(&program, bytes).is_ok_and(|(_, id)| id == header.state))
                .unwrap();
            assert_eq!(
                session.state().unwrap(),
                &decode_state(&program, &state_bytes).unwrap().0
            );
            let view = session.view(names.as_ref()).unwrap();
            assert_eq!(view.cursor, *id);
            assert_eq!(view.depth, header.depth);
            assert!(
                view.history
                    .iter()
                    .any(|entry| entry.id == *id && entry.parent == header.parent && entry.current)
            );
            session.page().unwrap();
            session.verify_path(id).unwrap();
        }
        session.checkout(&remap[&old.cursor]).unwrap();
        let exported = session.export().unwrap();
        let restored = Session::restore(program, &exported).unwrap();
        assert_eq!(session.state().unwrap(), restored.state().unwrap());
        assert_eq!(session.page().unwrap(), restored.page().unwrap());
        assert_eq!(
            session.view(names.as_ref()).unwrap().interaction,
            restored.view(names.as_ref()).unwrap().interaction
        );
    }
}

#[test]
fn saves_reopen_and_checkpoints_are_checked() {
    let compilation = support::compiled();
    let (mut session, names) = support::session(&compilation);
    let program = session.program().clone();
    let execution = session.execution();
    let root = session.cursor().unwrap();
    support::choose(&mut session, &names, &["wave"]).unwrap();
    let chosen = session.cursor().unwrap();
    session
        .save(narrata_history::RefName::new("quick").unwrap())
        .unwrap();
    let before = session.view(Some(&names)).unwrap();
    let mut session = Session::open(program.clone(), execution, session.into_backend()).unwrap();
    assert_eq!(session.view(Some(&names)).unwrap(), before);
    session.checkout(&root).unwrap();
    support::choose(&mut session, &names, &["wave"]).unwrap();
    assert_eq!(session.cursor().unwrap(), chosen);
    let children = session.children(root, None, 1).unwrap();
    assert_eq!(children.items.len(), 1);
    assert!(!children.more);
    session.checkout(&root).unwrap();
    session
        .load_save(narrata_history::RefName::new("quick").unwrap())
        .unwrap();
    assert_eq!(session.view(Some(&names)).unwrap(), before);
    let text = session.export().unwrap();
    let mut bytes = hex::decode(&text).unwrap();
    assert!(bytes.starts_with(narrata_history::CHECKPOINT_MAGIC));
    let last = bytes.len() - 1;
    bytes[last] ^= 1;
    assert!(Session::restore(program, &hex::encode(bytes)).is_err());
    session.verify_path(&chosen).unwrap();
}

/// Set NARRATA_EMIT_NODE_HISTORY to an empty temporary directory to reproduce the corpus.
#[test]
fn checkpoint_corpus_is_deterministic_and_imports() {
    let emit = std::env::var_os("NARRATA_EMIT_NODE_HISTORY").map(std::path::PathBuf::from);
    let frozen = corpus("nodes-r2-history");
    let mut artifacts = serde_json::Map::new();
    for (name, directory, save) in [
        ("branched.checkpoint", "nodes-r2", "branched.export.json"),
        (
            "proposals.checkpoint",
            "nodes-r2-proposals",
            "session.export.json",
        ),
    ] {
        let (program, names) =
            Program::from_pack(&std::fs::read(corpus(directory).join("story.narpack")).unwrap())
                .unwrap();
        let program = Arc::new(program);
        let text = std::fs::read_to_string(corpus(directory).join(save)).unwrap();
        let session = Session::restore(program.clone(), &text).unwrap();
        let bytes = hex::decode(session.export().unwrap()).unwrap();
        assert_eq!(
            bytes,
            hex::decode(
                Session::restore(program.clone(), &text)
                    .unwrap()
                    .export()
                    .unwrap()
            )
            .unwrap()
        );
        let restored = Session::restore(program, &hex::encode(&bytes)).unwrap();
        assert_eq!(session.state().unwrap(), restored.state().unwrap());
        assert_eq!(session.page().unwrap(), restored.page().unwrap());
        assert_eq!(
            session.view(names.as_ref()).unwrap().interaction,
            restored.view(names.as_ref()).unwrap().interaction
        );
        restored.verify_path(&restored.cursor().unwrap()).unwrap();
        artifacts.insert(name.to_owned(), serde_json::json!({"sha256": hex::encode(narrata_kernel::codec::sha256(&bytes)),
            "bytes": bytes.len(), "artifact_id": session.program().artifact_id(), "cursor": session.cursor().unwrap(),
            "state": session.state().unwrap().id(), "program": format!("../{directory}/story.narpack")}));
        if let Some(emit) = &emit {
            std::fs::create_dir_all(emit).unwrap();
            std::fs::write(emit.join(name), &bytes).unwrap();
        } else {
            assert_eq!(std::fs::read(frozen.join(name)).unwrap(), bytes);
        }
    }
    let manifest = serde_json::to_vec_pretty(
        &serde_json::json!({"format": "kernel-checkpoint-v1", "artifacts": artifacts}),
    )
    .unwrap();
    if let Some(emit) = &emit {
        std::fs::write(emit.join("manifest.json"), manifest).unwrap();
    } else {
        assert_eq!(
            std::fs::read(frozen.join("manifest.json")).unwrap(),
            manifest
        );
    }
}

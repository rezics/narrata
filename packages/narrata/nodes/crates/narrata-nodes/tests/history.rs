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

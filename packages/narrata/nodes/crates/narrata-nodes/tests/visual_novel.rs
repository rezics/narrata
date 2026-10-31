#![allow(clippy::panic, clippy::unwrap_used)]

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
};

use narrata_kernel::codec::{EnvelopeLimits, encode_envelope, inspect_envelope};
use narrata_nodes::{
    ChunkSource, Error, Input, KIND_INPUT, KIND_MANIFEST, MediaKind, MediaRef, MusicAction,
    NodeRegistry, ObjectId, PackageSource, Program, ProjectManifest, ProjectSource, Result,
    Session, VISUAL_NOVEL_FORMAT_VERSION, VISUAL_NOVEL_SCHEMA, compile, decode_input, decode_state,
    plan::{GraphRef, NameTable},
    view::Interaction,
};
use narrata_storage::{MemoryBackend, StorageBackend};
use serde_json::{Value, json};

mod support;

fn node(n: u32) -> String {
    format!("node:{n:032x}")
}

fn point(n: u32) -> String {
    format!("choice-point:{n:032x}")
}

fn option(n: u32) -> String {
    format!("option:{n:032x}")
}

fn content(key: &str) -> Value {
    json!({"provider": "host", "key": key})
}

fn image(key: &str, revision: &str) -> Value {
    json!({"kind": "image", "provider": "demo", "key": key, "revision": revision})
}

fn audio(key: &str, revision: &str) -> Value {
    json!({"kind": "audio", "provider": "demo", "key": key, "revision": revision})
}

fn line(key: &str) -> Value {
    json!({
        "unit": content(key),
        "first": "line-start",
        "last": "line-end"
    })
}

fn node_source(id: u32, type_id: &str, data: Value) -> Value {
    json!({"id": node(id), "type_id": type_id, "data": data})
}

fn project_value() -> Value {
    json!({
        "manifest": {
            "format_version": VISUAL_NOVEL_FORMAT_VERSION,
            "product": {
                "id": "vn-demo",
                "entry": {"package": "main", "graph": "start"}
            },
            "packages": {"main": "main.json"}
        },
        "package": {
            "id": "vn-demo.main",
            "version": "1.0.0",
            "exports": ["start"],
            "graphs": {
                "start": {
                    "outcomes": ["done"],
                    "entry": "scene_one",
                    "nodes": {
                        "scene_one": node_source(1, "narrata.scene", json!({
                            "background": image("bg/room", "r1"),
                            "layers": [{
                                "id": "hero",
                                "order": 10,
                                "asset": image("portrait/hero-neutral", "v4")
                            }],
                            "music": {"kind": "play", "track": audio("music/theme", "a7")},
                            "next": "line_one"
                        })),
                        "line_one": node_source(2, "narrata.dialogue", json!({
                            "speaker": content("speaker.hero"),
                            "line": line("dialogue.one"),
                            "voice": audio("voice/one", "a1"),
                            "next": "line_two"
                        })),
                        "line_two": node_source(3, "narrata.dialogue", json!({
                            "speaker": content("speaker.friend"),
                            "line": line("dialogue.two"),
                            "next": "line_three"
                        })),
                        "line_three": node_source(4, "narrata.dialogue", json!({
                            "speaker": content("speaker.hero"),
                            "line": line("dialogue.three"),
                            "voice": audio("voice/three", "a3"),
                            "next": "route"
                        })),
                        "route": node_source(5, "narrata.passage", json!({
                            "choice_points": [{
                                "id": point(1),
                                "key": "route",
                                "options": [
                                    {
                                        "id": option(1),
                                        "key": "left",
                                        "label": content("option.left"),
                                        "outcome": {"kind": "branch", "target": "left"}
                                    },
                                    {
                                        "id": option(2),
                                        "key": "right",
                                        "label": content("option.right"),
                                        "outcome": {"kind": "branch", "target": "right"}
                                    }
                                ]
                            }]
                        })),
                        "left": node_source(6, "narrata.mutate", json!({
                            "assignments": [], "next": "scene_two"
                        })),
                        "right": node_source(7, "narrata.mutate", json!({
                            "assignments": [], "next": "scene_two"
                        })),
                        "scene_two": node_source(8, "narrata.scene", json!({
                            "background": image("bg/hall", "r2"),
                            "layers": [
                                {"id": "hero", "order": 10, "asset": image("portrait/hero-alert", "v5")},
                                {"id": "friend", "order": 20, "asset": image("portrait/friend", "v2")}
                            ],
                            "music": {"kind": "keep"},
                            "next": "line_four"
                        })),
                        "line_four": node_source(9, "narrata.dialogue", json!({
                            "speaker": content("speaker.friend"),
                            "line": line("dialogue.four"),
                            "voice": audio("voice/four", "a4"),
                            "next": "ending"
                        })),
                        "ending": node_source(10, "narrata.return", json!({"outcome": "done"}))
                    }
                }
            }
        }
    })
}

fn project_from(mut value: Value, edit: impl FnOnce(&mut Value)) -> ProjectSource {
    let mut package = value["package"].take();
    edit(&mut package);
    ProjectSource {
        manifest: serde_json::from_value(value["manifest"].take()).unwrap(),
        packages: BTreeMap::from([(
            "main".to_owned(),
            serde_json::from_value::<PackageSource>(package).unwrap(),
        )]),
    }
}

fn project() -> ProjectSource {
    project_from(project_value(), |_| {})
}

fn compiled() -> narrata_nodes::Compilation {
    compile(&project(), &NodeRegistry::visual_novel()).unwrap()
}

fn new_session(compilation: &narrata_nodes::Compilation) -> (Session, NameTable) {
    let (program, names) = Program::from_pack(&compilation.pack).unwrap();
    assert!(program.is_visual_novel());
    let execution = "execution:0190f2a0000070008000000000000001"
        .parse()
        .unwrap();
    (
        Session::new(Arc::new(program), execution).unwrap(),
        names.unwrap(),
    )
}

fn dialogue(
    session: &Session,
    names: &NameTable,
    expected_unit: &str,
) -> (narrata_nodes::CommitId, narrata_nodes::NodeId) {
    let view = session.view(Some(names)).unwrap();
    match view.interaction {
        Interaction::Advance {
            dialogue,
            speaker,
            line,
            voice,
            ..
        } => {
            assert_eq!(line.unit.key.as_str(), expected_unit);
            assert!(speaker.is_some());
            if expected_unit == "dialogue.one" {
                assert_eq!(
                    voice.as_ref().map(|voice| voice.key.as_str()),
                    Some("voice/one")
                );
            } else if expected_unit == "dialogue.two" {
                assert!(voice.is_none());
            }
            (view.cursor, dialogue)
        }
        _ => panic!("expected dialogue at {expected_unit}"),
    }
}

fn cold_reopen(session: &Session, names: &NameTable) -> Session {
    let state = session.state().unwrap().clone();
    let view = session.view(Some(names)).unwrap();
    let export = session.export().unwrap();
    let reopened = Session::restore(session.program().clone(), &export).unwrap();
    assert_eq!(reopened.state().unwrap(), &state);
    let restored = reopened.view(Some(names)).unwrap();
    assert_eq!(restored.interaction, view.interaction);
    assert_eq!(restored.scene, view.scene);
    assert_eq!(restored.music_action, view.music_action);
    reopened
}

fn deterministic_history(
    compilation: &narrata_nodes::Compilation,
) -> (
    Vec<(narrata_nodes::CommitId, narrata_nodes::Commit)>,
    String,
) {
    let (mut session, names) = new_session(compilation);
    loop {
        let view = session.view(Some(&names)).unwrap();
        match view.interaction {
            Interaction::Advance { dialogue, .. } => {
                session.advance(&view.cursor, dialogue).unwrap();
            }
            Interaction::Choose {
                choice_point,
                options,
                ..
            } => {
                let selected = options
                    .iter()
                    .find(|option| option.key.as_deref() == Some("left"))
                    .unwrap();
                session
                    .choose(&view.cursor, choice_point, vec![selected.id])
                    .unwrap();
            }
            Interaction::Finished { .. } => break,
        }
    }
    (
        session.commits().unwrap().collect(),
        session.export().unwrap(),
    )
}

#[test]
fn vn_format_three_is_explicit_and_compiles_to_deterministic_schema_two_objects() {
    let first = compiled();
    let second = compiled();
    assert_eq!(first.pack, second.pack);
    assert_eq!(first.lock.format_version, VISUAL_NOVEL_FORMAT_VERSION);
    assert_eq!(
        deterministic_history(&first),
        deterministic_history(&second)
    );

    let pack = narrata_nodes::Pack::decode(&first.pack).unwrap();
    let limits = EnvelopeLimits::default();
    assert_eq!(
        inspect_envelope(&pack.manifest, &limits)
            .unwrap()
            .schema_version,
        VISUAL_NOVEL_SCHEMA
    );
    assert!(pack.chunks.iter().all(|chunk| {
        inspect_envelope(chunk, &limits).unwrap().schema_version == VISUAL_NOVEL_SCHEMA
    }));
    assert_eq!(
        inspect_envelope(&pack.tombstones, &limits)
            .unwrap()
            .schema_version,
        1
    );

    let (session, names) = new_session(&first);
    let (cursor, dialogue) = dialogue(&session, &names, "dialogue.one");
    assert_eq!(
        inspect_envelope(&session.state().unwrap().envelope(), &limits)
            .unwrap()
            .schema_version,
        VISUAL_NOVEL_SCHEMA
    );
    assert!(matches!(
        decode_input(
            session.program(),
            &cursor,
            session.state().unwrap(),
            &Input::Advance { dialogue }.envelope()
        ),
        Ok((Input::Advance { .. }, _))
    ));

    let gamebook = support::compiled();
    let gamebook_pack = narrata_nodes::Pack::decode(&gamebook.pack).unwrap();
    assert_eq!(
        inspect_envelope(&gamebook_pack.manifest, &limits)
            .unwrap()
            .schema_version,
        1
    );
    assert!(
        gamebook_pack
            .chunks
            .iter()
            .all(|chunk| { inspect_envelope(chunk, &limits).unwrap().schema_version == 1 })
    );
}

#[test]
fn dialogue_scene_choice_restore_and_branch_rejoin_preserve_the_exact_target() {
    let compilation = compiled();
    let (mut session, names) = new_session(&compilation);

    let initial = session.view(Some(&names)).unwrap();
    assert_eq!(
        initial
            .scene
            .as_ref()
            .unwrap()
            .background
            .as_ref()
            .unwrap()
            .key,
        "bg/room"
    );
    assert_eq!(
        initial.scene.as_ref().unwrap().music.as_ref().unwrap().key,
        "music/theme"
    );
    assert_eq!(initial.scene.as_ref().unwrap().layers[0].id, "hero");
    session = cold_reopen(&session, &names);

    let (cursor_one, one) = dialogue(&session, &names, "dialogue.one");
    session.advance(&cursor_one, one).unwrap();
    session = cold_reopen(&session, &names);
    let (cursor_two, two) = dialogue(&session, &names, "dialogue.two");
    session.advance(&cursor_two, two).unwrap();
    session = cold_reopen(&session, &names);
    let (cursor_three, three) = dialogue(&session, &names, "dialogue.three");
    session.advance(&cursor_three, three).unwrap();
    session = cold_reopen(&session, &names);

    let choice = session.view(Some(&names)).unwrap();
    let Interaction::Choose {
        choice_point,
        options,
        ..
    } = choice.interaction
    else {
        panic!("expected a choice after the third line");
    };
    let choice_commit = choice.cursor;
    let left = options
        .iter()
        .find(|option| option.key.as_deref() == Some("left"))
        .unwrap()
        .id;
    session
        .choose(&choice_commit, choice_point, vec![left])
        .unwrap();
    session = cold_reopen(&session, &names);

    let scene_two = session.view(Some(&names)).unwrap();
    let target = scene_two.scene.as_ref().unwrap();
    assert_eq!(target.background.as_ref().unwrap().key, "bg/hall");
    assert_eq!(
        target
            .layers
            .iter()
            .map(|layer| layer.id.as_str())
            .collect::<Vec<_>>(),
        vec!["hero", "friend"]
    );
    assert_eq!(target.music.as_ref().unwrap().key, "music/theme");
    let (cursor_four, four) = dialogue(&session, &names, "dialogue.four");
    assert_eq!(
        scene_two.scene.as_ref().unwrap().music,
        Some(MediaRef {
            kind: MediaKind::Audio,
            provider: "demo".into(),
            key: "music/theme".into(),
            revision: "a7".into(),
        })
    );
    session = cold_reopen(&session, &names);
    session.advance(&cursor_four, four).unwrap();
    let completed = session.view(Some(&names)).unwrap();
    assert!(
        matches!(completed.interaction, Interaction::Finished { ref outcome, .. } if outcome == "done")
    );

    let previous_history: BTreeSet<_> = session.commits().unwrap().map(|(id, _)| id).collect();

    session.checkout(&choice_commit).unwrap();
    let restored_choice = session.view(Some(&names)).unwrap();
    assert_eq!(
        restored_choice
            .scene
            .as_ref()
            .unwrap()
            .background
            .as_ref()
            .unwrap()
            .key,
        "bg/room"
    );
    let Interaction::Choose {
        choice_point,
        options,
        ..
    } = restored_choice.interaction
    else {
        panic!("checkout must restore the choice");
    };
    let right = options
        .iter()
        .find(|option| option.key.as_deref() == Some("right"))
        .unwrap()
        .id;
    session
        .choose(&choice_commit, choice_point, vec![right])
        .unwrap();
    let merged = session.view(Some(&names)).unwrap();
    assert_eq!(
        merged
            .scene
            .as_ref()
            .unwrap()
            .background
            .as_ref()
            .unwrap()
            .key,
        "bg/hall"
    );
    assert_eq!(
        merged.scene.as_ref().unwrap().music.as_ref().unwrap().key,
        "music/theme"
    );
    assert!(
        matches!(merged.interaction, Interaction::Advance { ref line, .. } if line.unit.key.as_str() == "dialogue.four")
    );
    let _ = cold_reopen(&session, &names);
    let continued_history: BTreeSet<_> = session.commits().unwrap().map(|(id, _)| id).collect();
    assert!(previous_history.is_subset(&continued_history));
}

#[test]
fn scene_music_can_be_replaced_or_stopped_and_voice_is_line_scoped() {
    for (music, expected_track) in [
        (
            json!({"kind": "play", "track": audio("music/new-theme", "b2")}),
            Some("music/new-theme"),
        ),
        (json!({"kind": "stop"}), None),
    ] {
        let project = project_from(project_value(), |package| {
            package["graphs"]["start"]["nodes"]["scene_two"]["data"]["music"] = music;
        });
        let compilation = compile(&project, &NodeRegistry::visual_novel()).unwrap();
        let (mut session, names) = new_session(&compilation);
        loop {
            let view = session.view(Some(&names)).unwrap();
            match view.interaction {
                Interaction::Advance { ref line, .. }
                    if line.unit.key.as_str() == "dialogue.four" =>
                {
                    let track = view
                        .scene
                        .as_ref()
                        .and_then(|scene| scene.music.as_ref())
                        .map(|track| track.key.as_str());
                    assert_eq!(track, expected_track);
                    break;
                }
                Interaction::Advance { dialogue, .. } => {
                    session.advance(&view.cursor, dialogue).unwrap();
                }
                Interaction::Choose {
                    choice_point,
                    options,
                    ..
                } => {
                    let left = options
                        .iter()
                        .find(|option| option.key.as_deref() == Some("left"))
                        .unwrap();
                    session
                        .choose(&view.cursor, choice_point, vec![left.id])
                        .unwrap();
                }
                Interaction::Finished { .. } => panic!("the fourth line should be pending"),
            }
        }
    }
}

#[test]
fn same_track_play_differs_from_keep_and_survives_restore_and_checkout() {
    let track = MediaRef {
        kind: MediaKind::Audio,
        provider: "demo".into(),
        key: "music/theme".into(),
        revision: "a7".into(),
    };
    for (music, expected_action) in [
        (json!({"kind": "keep"}), MusicAction::Keep),
        (
            json!({"kind": "play", "track": audio("music/theme", "a7")}),
            MusicAction::Play {
                track: track.clone(),
            },
        ),
    ] {
        let project = project_from(project_value(), |package| {
            package["graphs"]["start"]["nodes"]["scene_two"]["data"]["music"] = music;
        });
        let compilation = compile(&project, &NodeRegistry::visual_novel()).unwrap();
        let (mut session, names) = new_session(&compilation);
        for key in ["dialogue.one", "dialogue.two", "dialogue.three"] {
            let (cursor, dialogue) = dialogue(&session, &names, key);
            session.advance(&cursor, dialogue).unwrap();
        }

        let choice = session.view(Some(&names)).unwrap();
        let Interaction::Choose {
            choice_point,
            options,
            ..
        } = choice.interaction
        else {
            panic!("expected the route choice");
        };
        let left = options
            .iter()
            .find(|option| option.key.as_deref() == Some("left"))
            .unwrap()
            .id;
        session
            .choose(&choice.cursor, choice_point, vec![left])
            .unwrap();

        let transition_cursor = session.cursor().unwrap();
        let transition = session.view(Some(&names)).unwrap();
        assert_eq!(
            transition.scene.as_ref().unwrap().music,
            Some(track.clone())
        );
        assert_eq!(transition.music_action, Some(expected_action.clone()));
        assert_eq!(
            serde_json::to_value(&transition).unwrap()["music_action"],
            serde_json::to_value(&expected_action).unwrap()
        );
        let session = cold_reopen(&session, &names);
        assert_eq!(
            session.view(Some(&names)).unwrap().music_action,
            Some(expected_action.clone())
        );

        let (cursor, dialogue) = dialogue(&session, &names, "dialogue.four");
        let mut session = session;
        session.advance(&cursor, dialogue).unwrap();
        let after_dialogue = session.view(Some(&names)).unwrap();
        assert_eq!(after_dialogue.music_action, None);
        assert!(
            serde_json::to_value(&after_dialogue)
                .unwrap()
                .get("music_action")
                .is_none()
        );
        assert_eq!(
            after_dialogue.scene.as_ref().unwrap().music,
            Some(track.clone())
        );

        session.checkout(&transition_cursor).unwrap();
        let checked_out = session.view(Some(&names)).unwrap();
        assert_eq!(
            checked_out.scene.as_ref().unwrap().music,
            Some(track.clone())
        );
        assert_eq!(checked_out.music_action, Some(expected_action));
    }
}

#[test]
fn new_scene_clears_omitted_background_and_old_only_character_after_restore_and_checkout() {
    let project = project_from(project_value(), |package| {
        package["graphs"]["start"]["nodes"]["scene_one"]["data"]["layers"]
            .as_array_mut()
            .unwrap()
            .push(json!({
                "id": "old-only",
                "order": 20,
                "asset": image("portrait/old-only", "v1")
            }));
        package["graphs"]["start"]["nodes"]["scene_two"]["data"]["background"] = Value::Null;
    });
    let compilation = compile(&project, &NodeRegistry::visual_novel()).unwrap();
    let (mut session, names) = new_session(&compilation);
    let initial = session.view(Some(&names)).unwrap();
    assert_eq!(
        initial
            .scene
            .as_ref()
            .unwrap()
            .background
            .as_ref()
            .unwrap()
            .key,
        "bg/room"
    );
    assert!(
        initial
            .scene
            .as_ref()
            .unwrap()
            .layers
            .iter()
            .any(|layer| layer.id == "old-only")
    );

    for key in ["dialogue.one", "dialogue.two", "dialogue.three"] {
        let (cursor, dialogue) = dialogue(&session, &names, key);
        session.advance(&cursor, dialogue).unwrap();
    }
    let choice = session.view(Some(&names)).unwrap();
    let Interaction::Choose {
        choice_point,
        options,
        ..
    } = choice.interaction
    else {
        panic!("expected the route choice");
    };
    let left = options
        .iter()
        .find(|option| option.key.as_deref() == Some("left"))
        .unwrap()
        .id;
    session
        .choose(&choice.cursor, choice_point, vec![left])
        .unwrap();
    let transition_cursor = session.cursor().unwrap();

    fn assert_cleared(view: &narrata_nodes::SessionView) {
        let target = view.scene.as_ref().unwrap();
        assert_eq!(target.background, None);
        assert_eq!(
            target
                .layers
                .iter()
                .map(|layer| layer.id.as_str())
                .collect::<Vec<_>>(),
            vec!["hero", "friend"]
        );
        assert!(!target.layers.iter().any(|layer| layer.id == "old-only"));
    }

    assert_cleared(&session.view(Some(&names)).unwrap());
    session = cold_reopen(&session, &names);
    assert_cleared(&session.view(Some(&names)).unwrap());
    let (cursor, dialogue) = dialogue(&session, &names, "dialogue.four");
    session.advance(&cursor, dialogue).unwrap();
    session.checkout(&transition_cursor).unwrap();
    assert_cleared(&session.view(Some(&names)).unwrap());
}

#[test]
fn same_parent_advance_reuses_one_commit_and_moved_cursor_rejects_stale_clicks() {
    let compilation = compiled();
    let (mut session, names) = new_session(&compilation);
    let (root, dialogue) = dialogue(&session, &names, "dialogue.one");
    let initial_count = session.commits().unwrap().count();
    assert_eq!(
        decode_input(
            session.program(),
            &root,
            session.state().unwrap(),
            &Input::Advance {
                dialogue: node(88).parse().unwrap()
            }
            .envelope()
        )
        .unwrap_err()
        .code,
        "action"
    );
    let next = session.advance(&root, dialogue).unwrap();
    assert_eq!(session.commits().unwrap().count(), initial_count + 1);
    let stale = session.advance(&root, dialogue).unwrap_err();
    assert_eq!(stale.code, "stale_input");
    assert_eq!(session.cursor().unwrap(), next);
    assert_eq!(session.commits().unwrap().count(), initial_count + 1);

    session.checkout(&root).unwrap();
    assert_eq!(session.advance(&root, dialogue).unwrap(), next);
    assert_eq!(session.commits().unwrap().count(), initial_count + 1);
}

#[test]
fn another_session_stale_advance_uses_shared_cursor_without_writing() {
    let compilation = compiled();
    let (program, names) = Program::from_pack(&compilation.pack).unwrap();
    let program = Arc::new(program);
    let execution = "execution:0190f2a0000070008000000000000001"
        .parse()
        .unwrap();
    let backend = MemoryBackend::new();
    let mut first = Session::open(program.clone(), execution, backend.share()).unwrap();
    let mut second = Session::open(program.clone(), execution, backend.share()).unwrap();
    let (old_cursor, dialogue) = dialogue(&second, names.as_ref().unwrap(), "dialogue.one");
    assert_eq!(second.cursor().unwrap(), old_cursor);

    let advanced = first.advance(&old_cursor, dialogue).unwrap();
    let objects_before_stale = backend.scan_objects(None, 1024).unwrap().digests;
    assert_eq!(first.commits().unwrap().count(), 2);
    assert_eq!(second.cursor().unwrap(), old_cursor);
    assert_eq!(
        second.advance(&old_cursor, dialogue).unwrap_err().code,
        "stale_input"
    );
    assert_eq!(
        backend.scan_objects(None, 1024).unwrap().digests,
        objects_before_stale
    );
    assert_eq!(second.cursor().unwrap(), old_cursor);

    let durable = Session::open(program, execution, backend.share()).unwrap();
    assert_eq!(durable.cursor().unwrap(), advanced);
    assert_eq!(durable.state().unwrap(), first.state().unwrap());
    assert_eq!(durable.commits().unwrap().count(), 2);
}

#[test]
fn invalid_media_state_interactions_tags_and_schemas_are_rejected() {
    let bad_category = project_from(project_value(), |package| {
        package["graphs"]["start"]["nodes"]["scene_one"]["data"]["background"]["kind"] =
            json!("audio");
    });
    assert_eq!(
        compile(&bad_category, &NodeRegistry::visual_novel())
            .unwrap_err()
            .code,
        "resource_kind"
    );
    let bad_reference = project_from(project_value(), |package| {
        package["graphs"]["start"]["nodes"]["scene_one"]["data"]["background"]["key"] =
            json!("https://media.invalid/image");
    });
    assert_eq!(
        compile(&bad_reference, &NodeRegistry::visual_novel())
            .unwrap_err()
            .code,
        "resource_reference"
    );

    let compilation = compiled();
    let (session, names) = new_session(&compilation);
    let (cursor, dialogue) = dialogue(&session, &names, "dialogue.one");
    let mut bad_state = session.state().unwrap().clone();
    bad_state
        .scene
        .as_mut()
        .unwrap()
        .background
        .as_mut()
        .unwrap()
        .kind = MediaKind::Audio;
    assert_eq!(
        decode_state(session.program(), &bad_state.envelope())
            .unwrap_err()
            .code,
        "invalid_state"
    );
    let mut inconsistent_state = session.state().unwrap().clone();
    inconsistent_state.frames.last_mut().unwrap().at = Some(point(90).parse().unwrap());
    assert_eq!(
        decode_state(session.program(), &inconsistent_state.envelope())
            .unwrap_err()
            .code,
        "invalid_state"
    );

    let mut unknown_input_tag = Input::Advance { dialogue }.encode();
    unknown_input_tag[1] = 23;
    let unknown_input = encode_envelope(KIND_INPUT, VISUAL_NOVEL_SCHEMA, &unknown_input_tag);
    assert_eq!(
        decode_input(
            session.program(),
            &cursor,
            session.state().unwrap(),
            &unknown_input
        )
        .unwrap_err()
        .code,
        "decode"
    );
    let wrong_schema = encode_envelope(KIND_INPUT, 1, &Input::Advance { dialogue }.encode());
    assert_eq!(
        decode_input(
            session.program(),
            &cursor,
            session.state().unwrap(),
            &wrong_schema
        )
        .unwrap_err()
        .code,
        "decode"
    );

    let pack = narrata_nodes::Pack::decode(&compilation.pack).unwrap();
    let unknown_manifest = encode_envelope(KIND_MANIFEST, 77, &pack.manifest[56..]);
    assert_eq!(
        Program::from_manifest(
            &unknown_manifest,
            Box::new(narrata_nodes::MemorySource::default())
        )
        .unwrap_err()
        .code,
        "decode"
    );
}

#[derive(Clone, Default)]
struct DelayedChunks(Arc<Mutex<BTreeMap<ObjectId, Vec<u8>>>>);

impl DelayedChunks {
    fn insert(&self, id: ObjectId, bytes: Vec<u8>) {
        self.0.lock().unwrap().insert(id, bytes);
    }
}

impl ChunkSource for DelayedChunks {
    fn load(&self, id: &ObjectId) -> Result<Vec<u8>> {
        self.0.lock().unwrap().get(id).cloned().ok_or_else(|| {
            Error::new(
                "missing_chunk",
                id.to_string(),
                "chunk has not been delivered",
            )
        })
    }
}

#[test]
fn missing_logic_chunk_does_not_advance_and_retry_reuses_the_confirmed_input() {
    let source = json!({
        "manifest": {
            "format_version": VISUAL_NOVEL_FORMAT_VERSION,
            "product": {"id": "lazy-vn", "entry": {"package": "main", "graph": "start"}},
            "packages": {"main": "main.json"}
        },
        "package": {
            "id": "lazy-vn.main", "version": "1", "exports": ["start"],
            "graphs": {
                "start": {
                    "outcomes": ["done"], "entry": "line",
                    "nodes": {
                        "line": node_source(40, "narrata.dialogue", json!({
                            "line": line("lazy.one"), "next": "call"
                        })),
                        "call": node_source(41, "narrata.call", json!({
                            "target": {"kind": "local", "graph": "helper"},
                            "on_return": {"done": "ending"}
                        })),
                        "ending": node_source(42, "narrata.return", json!({"outcome": "done"}))
                    }
                },
                "helper": {
                    "outcomes": ["done"], "entry": "line",
                    "nodes": {
                        "line": node_source(43, "narrata.dialogue", json!({
                            "line": line("lazy.two"), "next": "return"
                        })),
                        "return": node_source(44, "narrata.return", json!({"outcome": "done"}))
                    }
                }
            }
        }
    });
    let manifest: ProjectManifest = serde_json::from_value(source["manifest"].clone()).unwrap();
    let package: PackageSource = serde_json::from_value(source["package"].clone()).unwrap();
    let compiled = compile(
        &ProjectSource {
            manifest,
            packages: BTreeMap::from([("main".into(), package)]),
        },
        &NodeRegistry::visual_novel(),
    )
    .unwrap();
    let pack = narrata_nodes::Pack::decode(&compiled.pack).unwrap();
    let (complete, names) = Program::from_pack(&compiled.pack).unwrap();
    let start = GraphRef {
        package: "main".into(),
        graph: "start".into(),
    };
    let helper = GraphRef {
        package: "main".into(),
        graph: "helper".into(),
    };
    let start_chunk = complete.manifest().graphs[&start].chunk as usize;
    let helper_chunk = complete.manifest().graphs[&helper].chunk as usize;
    let delayed = DelayedChunks::default();
    delayed.insert(
        complete.manifest().chunks[start_chunk],
        pack.chunks[start_chunk].clone(),
    );
    let program =
        Arc::new(Program::from_manifest(&pack.manifest, Box::new(delayed.clone())).unwrap());
    let execution = "execution:0190f2a0000070008000000000000001"
        .parse()
        .unwrap();
    let mut session = Session::new(program, execution).unwrap();
    let view = session.view(names.as_ref()).unwrap();
    let Interaction::Advance { dialogue, .. } = view.interaction else {
        panic!("expected first line")
    };
    let before = view.cursor;
    assert_eq!(
        session.advance(&before, dialogue).unwrap_err().code,
        "missing_chunk"
    );
    assert_eq!(session.cursor().unwrap(), before);
    assert_eq!(session.commits().unwrap().count(), 1);

    delayed.insert(
        complete.manifest().chunks[helper_chunk],
        pack.chunks[helper_chunk].clone(),
    );
    session.advance(&before, dialogue).unwrap();
    let continued = session.view(names.as_ref()).unwrap();
    assert!(
        matches!(continued.interaction, Interaction::Advance { ref line, .. } if line.unit.key.as_str() == "lazy.two")
    );
    assert_eq!(session.state().unwrap().frames.len(), 2);
    assert_eq!(session.commits().unwrap().count(), 2);
}

#[test]
fn source_format_three_requires_the_visual_novel_registry() {
    let source = project();
    assert!(compile(&source, &NodeRegistry::gamebook()).is_err());
    let mut source = source;
    source.manifest.format_version = 2;
    assert!(compile(&source, &NodeRegistry::visual_novel()).is_err());
}

#[test]
fn frozen_vn_corpus_rebuilds_and_imports_its_choice_history() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../../../fixtures/compat/nodes-vn-v1");
    let source_bytes = std::fs::read(root.join("source.json")).unwrap();
    let source_json: Value = serde_json::from_slice(&source_bytes).unwrap();
    let source = project_from(source_json, |_| {});
    let compilation = compile(&source, &NodeRegistry::visual_novel()).unwrap();
    let frozen_pack = std::fs::read(root.join("story.narpack")).unwrap();
    let frozen_export = std::fs::read_to_string(root.join("save.export.json")).unwrap();
    assert_eq!(compilation.pack, frozen_pack);

    let (program, names) = Program::from_pack(&frozen_pack).unwrap();
    let program = Arc::new(program);
    let restored = Session::restore(program.clone(), &frozen_export).unwrap();
    assert_eq!(restored.export().unwrap(), frozen_export);
    let current = restored.view(names.as_ref()).unwrap();
    let expected_view: narrata_nodes::SessionView =
        serde_json::from_slice(&std::fs::read(root.join("view.json")).unwrap()).unwrap();
    assert_eq!(current.interaction, expected_view.interaction);
    assert_eq!(current.scene, expected_view.scene);
    assert!(
        matches!(current.interaction, Interaction::Advance { ref line, .. } if line.unit.key.as_str() == "dialogue.four")
    );
    assert_eq!(
        current
            .scene
            .as_ref()
            .unwrap()
            .background
            .as_ref()
            .unwrap()
            .key
            .as_str(),
        "bg/hall"
    );
    assert_eq!(
        current
            .scene
            .as_ref()
            .unwrap()
            .music
            .as_ref()
            .unwrap()
            .key
            .as_str(),
        "music/theme"
    );

    let host: Value =
        serde_json::from_slice(&std::fs::read(root.join("host-fixture.json")).unwrap()).unwrap();
    for key in [
        "speaker.hero",
        "speaker.friend",
        "dialogue.one",
        "dialogue.two",
        "dialogue.three",
        "dialogue.four",
        "option.left",
        "option.right",
    ] {
        assert!(
            host["content"][format!("host:{key}")].is_string(),
            "missing host content {key}"
        );
    }
    assert!(host["media"].as_array().unwrap().iter().any(|asset| {
        asset["kind"] == "audio" && asset["key"] == "music/theme" && asset["revision"] == "a7"
    }));
    assert!(!frozen_export.contains("Mira"));
    assert!(!frozen_export.contains("https://"));

    let mut session = restored;
    let choice = session
        .commits()
        .unwrap()
        .find(|(_, commit)| commit.depth == 3)
        .map(|(id, _)| id)
        .unwrap();
    session.checkout(&choice).unwrap();
    let view = session.view(names.as_ref()).unwrap();
    let Interaction::Choose {
        choice_point,
        options,
        ..
    } = view.interaction
    else {
        panic!("the imported history should restore the choice safe point");
    };
    let right = options
        .iter()
        .find(|option| option.key.as_deref() == Some("right"))
        .unwrap()
        .id;
    session.choose(&choice, choice_point, vec![right]).unwrap();
    assert!(
        matches!(session.view(names.as_ref()).unwrap().interaction, Interaction::Advance { ref line, .. } if line.unit.key.as_str() == "dialogue.four")
    );
}

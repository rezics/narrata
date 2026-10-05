//! Frozen compatibility evidence is generated twice before its first creation. Once present,
//! the corpus is read-only; normal tests compare every byte and restore its saved sessions.
use super::*;
use narrata_nodes::view::Interaction;
use std::{collections::BTreeMap, path::Path, sync::Arc};

type Files = BTreeMap<String, Vec<u8>>;
fn json_bytes(value: &impl serde::Serialize) -> Vec<u8> {
    let mut bytes = narrata_nodes::canonical_json(value).unwrap();
    bytes.push(b'\n');
    bytes
}
fn save(files: &mut Files, scenario: &str, name: &str, bytes: Vec<u8>) {
    files.insert(format!("{scenario}/{name}"), bytes);
}

fn capture(
    files: &mut Files,
    scenario: &str,
    old: &Compilation,
    new: &Compilation,
    session: Session,
    recoveries: Vec<Recovery>,
) {
    let (registry, id) = setup(old, new, recoveries.clone());
    capture_registered(
        files, scenario, old, new, session, &registry, id, recoveries, None,
    );
}
#[allow(clippy::too_many_arguments)]
fn capture_registered(
    files: &mut Files,
    scenario: &str,
    old: &Compilation,
    new: &Compilation,
    mut s: Session,
    registry: &EvolutionRegistry,
    id: MigrationId,
    recoveries: Vec<Recovery>,
    source_bytes: Option<Vec<u8>>,
) {
    save(files, scenario, "source.narpack", old.pack.clone());
    save(files, scenario, "target.narpack", new.pack.clone());
    save(
        files,
        scenario,
        "source.checkpoint",
        source_bytes.unwrap_or_else(|| hex::decode(s.export().unwrap()).unwrap()),
    );
    save(
        files,
        scenario,
        "declarations.json",
        json_bytes(&RecoveryDeclarationsV1::new(old.program.artifact_id(), recoveries).unwrap()),
    );
    let descriptor = registry.descriptor(id).unwrap();
    save(files, scenario, "descriptor.cbor", descriptor.encode());
    save(files, scenario, "descriptor.json", json_bytes(descriptor));
    let assessment = assess(&s, registry, &new.program, id);
    save(files, scenario, "assessment.json", json_bytes(&assessment));
    if matches!(
        assessment,
        AssessmentV1::Automatic { .. } | AssessmentV1::NeedsConfirmation { .. }
    ) {
        let p = plan(assessment);
        save(files, scenario, "report.cbor", p.report.encode().unwrap());
        let result = s.apply_evolution(registry, &p, PathLimits::default());
        assert!(matches!(result, ApplyV1::Migrated { .. }));
        save(files, scenario, "apply.json", json_bytes(&result));
        // Shallow input retains its boundary after migrating; use an explicit shallow export.
        let bytes = if scenario == "shallow-history" {
            s.history()
                .export_shallow(
                    narrata_history::ObjectId::from_bytes(*s.cursor().unwrap().as_bytes()),
                    1,
                    &Default::default(),
                )
                .unwrap()
                .to_bytes()
                .unwrap()
        } else {
            hex::decode(s.export().unwrap()).unwrap()
        };
        save(files, scenario, "target.checkpoint", bytes);
    }
}

fn vectors() -> Files {
    let mut files = Files::new();
    let old = compiled();
    let new = changed();
    let (mut s, names) = session(&old);
    choose(&mut s, &names, &["wave"]).unwrap();
    capture(&mut files, "unseen-rule", &old, &new, s, vec![]);

    let new = try_variant(|_, main, _| { set(main, &format!("{ROUTE}/options/1/outcome"), "target", json!("detour")); set(main, "/graphs/start/nodes", "detour", json!({"id":node(30),"type_id":"narrata.mutate","data":{"assignments":[],"next":"count"}})); }).unwrap();
    let (mut s, names) = session(&old);
    for choices in [&["wave"][..], &["rope"], &["right"]] {
        choose(&mut s, &names, choices).unwrap();
    }
    capture(&mut files, "target-rejoins", &old, &new, s, vec![]);

    let new = try_variant(|_, main, _| {
        at(main, &format!("{MULTI}/options"))
            .as_array_mut()
            .unwrap()
            .swap(0, 1);
    })
    .unwrap();
    let (mut s, names) = session(&old);
    choose(&mut s, &names, &["wave"]).unwrap();
    choose(&mut s, &names, &["rope", "lamp"]).unwrap();
    capture(&mut files, "multi-reordered", &old, &new, s, vec![]);

    let new = deleted_gate();
    let recoveries = vec![
        mapping(DeletedId::Node(node(1).parse().unwrap())),
        mapping(DeletedId::ChoicePoint(point(1).parse().unwrap())),
    ];
    capture(
        &mut files,
        "deleted-current",
        &old,
        &new,
        session(&old).0,
        recoveries,
    );
    capture(
        &mut files,
        "no-declarations",
        &old,
        &new,
        session(&old).0,
        vec![],
    );

    let new = try_variant(|_, main, _| {
        add_recovery_passage(main);
        at(main, &format!("{GATE}/choice_points"))
            .as_array_mut()
            .unwrap()
            .remove(0);
        main["tombstones"] = json!([point(1), option(1), option(2)]);
    })
    .unwrap();
    capture(
        &mut files,
        "deleted-choice-point",
        &old,
        &new,
        session(&old).0,
        vec![mapping(DeletedId::ChoicePoint(point(1).parse().unwrap()))],
    );

    let new = try_variant(|_, main, _| {
        add_recovery_passage(main);
        at(main, "/graphs/start/nodes")
            .as_object_mut()
            .unwrap()
            .remove("visit");
        set(
            main,
            &format!("{ROUTE}/options/0/outcome"),
            "target",
            json!("safe"),
        );
        main["tombstones"] = json!([node(2)]);
    })
    .unwrap();
    let (mut s, names) = session(&old);
    for choices in [&["wave"][..], &["rope"], &["left"]] {
        choose(&mut s, &names, choices).unwrap();
    }
    capture(
        &mut files,
        "deleted-lower-call",
        &old,
        &new,
        s,
        vec![mapping(DeletedId::Node(node(2).parse().unwrap()))],
    );

    let new = try_variant(|_, main, _| {
        add_recovery_passage(main);
        at(main, "/graphs/start/nodes")
            .as_object_mut()
            .unwrap()
            .remove("won");
        set(
            main,
            "/graphs/start/nodes/count/data",
            "when_true",
            json!("safe"),
        );
        main["tombstones"] = json!([node(5)]);
    })
    .unwrap();
    let (mut s, names) = session(&old);
    for choices in [&["wave"][..], &["rope"], &["right"]] {
        choose(&mut s, &names, choices).unwrap();
    }
    capture(
        &mut files,
        "deleted-ending",
        &old,
        &new,
        s,
        vec![mapping(DeletedId::Node(node(5).parse().unwrap()))],
    );

    let new = try_variant(|_, main, _| {
        add_recovery_passage(main);
        let mut second = at(main, "/graphs/start/nodes/safe").clone();
        second["id"] = json!(node(21));
        second["data"]["choice_points"][0]["id"] = json!(point(21));
        second["data"]["choice_points"][0]["options"][0]["id"] = json!(option(21));
        set(main, "/graphs/start/nodes", "other", second);
        set(main, "/graphs/start", "entry", json!("safe"));
        at(main, "/graphs/start/nodes")
            .as_object_mut()
            .unwrap()
            .remove("gate");
        main["tombstones"] = json!([
            node(1),
            point(1),
            point(2),
            point(3),
            option(1),
            option(2),
            option(3),
            option(4),
            option(5),
            option(6),
            option(7)
        ]);
    })
    .unwrap();
    let conflicting = vec![
        mapping(DeletedId::Node(node(1).parse().unwrap())),
        Recovery {
            deleted: DeletedId::ChoicePoint(point(1).parse().unwrap()),
            target: RecoveryTarget {
                node: node(21).parse().unwrap(),
                choice_point: point(21).parse().unwrap(),
            },
        },
    ];
    capture(
        &mut files,
        "conflicting-declarations",
        &old,
        &new,
        session(&old).0,
        conflicting,
    );

    let new = try_variant(|manifest, main, _| {
        manifest["product"]["shared"]["coins"] = json!(false);
        main["graphs"]["start"]["shared"]["coins"] = json!("bool");
        set(main, &format!("{POINT}/options/0"), "effects", json!([]));
        set(main, &format!("{MULTI}/options/0"), "effects", json!([]));
        set(
            main,
            &format!("{MULTI}/options/2"),
            "enabled_if",
            literal(json!(true)),
        );
        set(
            main,
            "/graphs/start/nodes/count/data",
            "condition",
            literal(json!(true)),
        );
    })
    .unwrap();
    capture(
        &mut files,
        "variable-type",
        &old,
        &new,
        session(&old).0,
        vec![],
    );

    let new = changed();
    let (registry, id) = setup(&old, &new, vec![]);
    let (mut s, names) = session(&old);
    choose(&mut s, &names, &["wave"]).unwrap();
    let bytes = s
        .history()
        .export_shallow(
            narrata_history::ObjectId::from_bytes(*s.cursor().unwrap().as_bytes()),
            0,
            &Default::default(),
        )
        .unwrap()
        .to_bytes()
        .unwrap();
    let s = Session::restore_with_registry(&registry, &hex::encode(&bytes)).unwrap();
    capture_registered(
        &mut files,
        "shallow-history",
        &old,
        &new,
        s,
        &registry,
        id,
        vec![],
        Some(bytes),
    );

    let proposal_old = try_variant(|_, main, _| {
        set(main, POINT, "proposals", json!(true));
    })
    .unwrap();
    let proposal_new = try_variant(|_, main, _| {
        set(main, POINT, "proposals", json!(true));
        set(
            main,
            "/graphs/start/nodes/tally/data",
            "assignments",
            json!([add("local", "visits", 2)]),
        );
    })
    .unwrap();
    let request: ProposalRequest = serde_json::from_value(json!({"choice_point":point(1),"options":[{"label":content("proposed"),"outcome":{"kind":"local","rejoin":"b2"}}]})).unwrap();
    let (mut s, _) = session(&proposal_old);
    s.propose(&s.cursor().unwrap(), &request).unwrap();
    let dynamic_id = s.state().unwrap().frames[0].overlay.options[&point(1).parse().unwrap()][0].id;
    capture(
        &mut files,
        "proposal-retained",
        &proposal_old,
        &proposal_new,
        s,
        vec![],
    );
    let collision = try_variant(|_, main, _| {
        set(main, POINT, "proposals", json!(true));
        let mut extra = at(main, &format!("{POINT}/options/0")).clone();
        extra["id"] = json!(dynamic_id);
        extra["key"] = json!("collision");
        at(main, &format!("{POINT}/options"))
            .as_array_mut()
            .unwrap()
            .push(extra);
    })
    .unwrap();
    let (mut s, _) = session(&proposal_old);
    s.propose(&s.cursor().unwrap(), &request).unwrap();
    capture(
        &mut files,
        "proposal-collision",
        &proposal_old,
        &collision,
        s,
        vec![],
    );

    let next = try_variant(|_, main, _| {
        set(
            main,
            "/graphs/start/nodes/tally/data",
            "assignments",
            json!([add("local", "visits", 3)]),
        );
    })
    .unwrap();
    let (mut registry, first) = setup(&old, &changed(), vec![]);
    registry.register_program(open(&next.pack).0).unwrap();
    let previous = changed();
    let second = registry
        .register_descriptor(
            EvolutionDescriptorV1::new(
                previous.program.artifact_id(),
                next.program.artifact_id(),
                vec![],
            )
            .unwrap(),
        )
        .unwrap();
    let (mut s, _) = session(&old);
    let p = plan(assess(&s, &registry, &previous.program, first));
    assert!(matches!(
        s.apply_evolution(&registry, &p, PathLimits::default()),
        ApplyV1::Migrated { .. }
    ));
    save(
        &mut files,
        "republished-after-migration",
        "ancestor.narpack",
        old.pack.clone(),
    );
    save(
        &mut files,
        "republished-after-migration",
        "ancestor-descriptor.cbor",
        registry.descriptor(first).unwrap().encode(),
    );
    capture_registered(
        &mut files,
        "republished-after-migration",
        &previous,
        &next,
        s,
        &registry,
        second,
        vec![],
        None,
    );

    let alias = try_variant(|_, main, _| {
        set(main, POINT, "key", json!("hello"));
    })
    .unwrap();
    assert_eq!(alias.program.artifact_id(), old.program.artifact_id());
    let (s, _) = session(&old);
    let mut registry = EvolutionRegistry::new();
    registry.register_program(open(&old.pack).0).unwrap();
    save(
        &mut files,
        "content-alias-exact",
        "source.narpack",
        old.pack.clone(),
    );
    save(
        &mut files,
        "content-alias-exact",
        "target.narpack",
        alias.pack.clone(),
    );
    save(
        &mut files,
        "content-alias-exact",
        "source.checkpoint",
        hex::decode(s.export().unwrap()).unwrap(),
    );
    save(
        &mut files,
        "content-alias-exact",
        "assessment.json",
        json_bytes(&s.assess_evolution(
            &registry,
            alias.program.artifact_id(),
            &[],
            PathLimits::default(),
        )),
    );

    // Content belongs to the host. The same artifact resolves new body text independently;
    // reference changes alter its identity and are assessed as ordinary structural revisions.
    let reference = try_variant(|_, main, _| {
        set(main, GATE, "title", content("replacement.title"));
    })
    .unwrap();
    capture(
        &mut files,
        "content-reference",
        &old,
        &reference,
        session(&old).0,
        vec![],
    );

    let (s, _) = session(&old);
    save(
        &mut files,
        "old-save-continue",
        "source.narpack",
        old.pack.clone(),
    );
    save(
        &mut files,
        "old-save-continue",
        "source.checkpoint",
        hex::decode(s.export().unwrap()).unwrap(),
    );
    save(
        &mut files,
        "old-save-continue",
        "legacy.json",
        json_bytes(&legacy_export(&s)),
    );
    let summaries: Vec<_> = files.iter().map(|(name, bytes)| json!({"path":name,"bytes":bytes.len(),"sha256":hex::encode(narrata_kernel::codec::sha256(bytes))})).collect();
    files.insert(
        "manifest.json".into(),
        json_bytes(&json!({"format_version":1,"files":summaries})),
    );
    files
}

#[test]
fn frozen_evolution_vectors_are_deterministic_and_old_and_migrated_saves_continue() {
    let files = vectors();
    assert_eq!(files, vectors(), "two generations must be byte-identical");
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../../../fixtures/compat/nodes-evolution-v1");
    for (name, bytes) in &files {
        assert_eq!(
            &std::fs::read(root.join(name)).unwrap(),
            bytes,
            "frozen drift: {name}"
        );
    }
    for (name, bytes) in &files {
        if name.ends_with("source.checkpoint") || name.ends_with("target.checkpoint") {
            let directory = name.rsplit_once('/').unwrap().0;
            let mut registry = EvolutionRegistry::new();
            for pack in ["source.narpack", "target.narpack", "ancestor.narpack"] {
                if let Some(bytes) = files.get(&format!("{directory}/{pack}")) {
                    registry
                        .register_program(Arc::new(Program::from_pack(bytes).unwrap().0))
                        .unwrap();
                }
            }
            for descriptor in ["descriptor.cbor", "ancestor-descriptor.cbor"] {
                if let Some(bytes) = files.get(&format!("{directory}/{descriptor}")) {
                    registry
                        .register_descriptor(EvolutionDescriptorV1::decode(bytes).unwrap())
                        .unwrap();
                }
            }
            let mut restored =
                Session::restore_with_registry(&registry, &hex::encode(bytes)).unwrap();
            if !directory.eq("shallow-history") {
                let again = restored.export().unwrap();
                assert_eq!(
                    Session::restore_with_registry(&registry, &again)
                        .unwrap()
                        .state()
                        .unwrap(),
                    restored.state().unwrap()
                );
                restored.view(None).unwrap();
                assert_eq!(
                    registry
                        .compare_path(
                            restored.history(),
                            restored.cursor().unwrap(),
                            restored.program().artifact_id(),
                            PathLimits::default()
                        )
                        .unwrap(),
                    PathComparisonV1::Equivalent
                );
                let cursor = restored.cursor().unwrap();
                let commits = restored.commits().unwrap().collect::<Vec<_>>();
                for (id, header) in commits {
                    restored.presentation(&id).unwrap();
                    restored.checkout(&id).unwrap();
                    assert_eq!(restored.program().artifact_id(), header.artifact);
                    let exported = restored.export().unwrap();
                    assert_eq!(
                        Session::restore_with_registry(&registry, &exported)
                            .unwrap()
                            .cursor()
                            .unwrap(),
                        id
                    );
                }
                restored.checkout(&cursor).unwrap();
            }
            // Each restored interaction can continue by a valid set or is a legal ending.
            let current = restored.cursor().unwrap();
            if let Interaction::Choose {
                choice_point,
                min,
                options,
                ..
            } = restored.interaction(None).unwrap()
            {
                let chosen = options
                    .iter()
                    .filter(|o| o.enabled)
                    .take(usize::from(min))
                    .map(|o| o.id)
                    .collect();
                restored.choose(&current, choice_point, chosen).unwrap();
            }
        }
        if name.ends_with("legacy.json") {
            let directory = name.rsplit_once('/').unwrap().0;
            let program = Arc::new(
                Program::from_pack(&files[&format!("{directory}/source.narpack")])
                    .unwrap()
                    .0,
            );
            let mut restored =
                Session::restore(program, std::str::from_utf8(bytes).unwrap()).unwrap();
            restored
                .choose(
                    &restored.cursor().unwrap(),
                    point(1).parse().unwrap(),
                    vec![option(1).parse().unwrap()],
                )
                .unwrap();
        }
    }
}

#[test]
#[ignore = "one-time creation; refuses to overwrite a frozen corpus"]
fn create_frozen_evolution_corpus() {
    let first = vectors();
    let second = vectors();
    assert_eq!(first, second);
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../..");
    let root = repo.join("fixtures/compat/nodes-evolution-v1");
    assert!(!root.exists(), "frozen corpus is read-only");
    for (which, files) in [("first", &first), ("second", &second)] {
        let temp = repo.join(format!(".temp/evolution-corpus/{which}"));
        for (name, bytes) in files {
            let path = temp.join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes).unwrap();
        }
    }
    for (name, bytes) in first {
        let path = root.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }
}

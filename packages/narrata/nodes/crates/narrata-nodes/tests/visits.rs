#![cfg(feature = "trace")]
#![allow(clippy::unwrap_used, clippy::panic)]
mod support;

use narrata_nodes::{
    Session,
    history::{evolution::*, visits::*},
};
use support::*;

fn registry(s: &Session) -> EvolutionRegistry {
    let mut registry = EvolutionRegistry::new();
    registry.register_program(s.program().clone()).unwrap();
    registry
}

#[test]
fn replay_observes_local_multiselect_automatic_call_return_and_ending_without_writes() {
    let (mut s, names) = session(&compiled());
    for keys in [
        &["wave"][..],
        &["rope", "lamp"][..],
        &["left"][..],
        &["continue"][..],
    ] {
        choose(&mut s, &names, keys).unwrap();
    }
    let before = s.export().unwrap();
    let registry = registry(&s);
    let visits = registry
        .visited_from_save(s.history(), s.cursor().unwrap(), PathLimits::default())
        .unwrap();
    let source = &visits.report().sources[0];
    assert!(source.complete);
    assert!(source.missing.is_empty());
    let events = &source.segments[0].events;
    let visited: Vec<_> = events
        .iter()
        .filter(|e| matches!(e.action, VisitActionV1::Visit { .. }))
        .map(|e| e.node.to_string())
        .collect();
    assert_eq!(
        visited,
        [
            node(1),
            node(2),
            node(11),
            node(12),
            node(3),
            node(4),
            node(5)
        ]
    );
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e.action, VisitActionV1::Selected { .. }))
            .count(),
        5
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(&e.action, VisitActionV1::Call { instance: 2, .. }))
    );
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e.action, VisitActionV1::Return { .. }))
            .count(),
        2
    );
    assert!(events.iter().any(|e| matches!(&e.action, VisitActionV1::Segment { segment, role: None } if segment.unit.key.as_str() == "ending")));
    assert_eq!(s.export().unwrap(), before);
    assert_eq!(
        visits.report(),
        registry
            .visited_from_save(s.history(), s.cursor().unwrap(), PathLimits::default())
            .unwrap()
            .report()
    );
}

#[test]
fn checkout_excludes_sibling_history_and_explicit_merge_retains_cursor_provenance() {
    let (mut s, names) = session(&compiled());
    choose(&mut s, &names, &["wave"]).unwrap();
    choose(&mut s, &names, &["rope", "lamp"]).unwrap();
    let root = s.cursor().unwrap();
    choose(&mut s, &names, &["left"]).unwrap();
    let left = s.cursor().unwrap();
    s.checkout(&root).unwrap();
    choose(&mut s, &names, &["right"]).unwrap();
    let registry = registry(&s);
    let right = registry
        .visited_from_save(s.history(), s.cursor().unwrap(), PathLimits::default())
        .unwrap();
    assert!(
        !right.report().sources[0].segments[0]
            .events
            .iter()
            .any(|e| e.node.to_string() == node(11))
    );
    let left = registry
        .visited_from_save(s.history(), left, PathLimits::default())
        .unwrap();
    let merged = CheckedVisits::merge([right, left]).unwrap();
    assert_eq!(merged.report().sources.len(), 2);
    assert_ne!(
        merged.report().sources[0].cursor,
        merged.report().sources[1].cursor
    );
}

#[test]
fn shallow_missing_artifact_and_limits_are_incomplete() {
    let (mut s, names) = session(&compiled());
    choose(&mut s, &names, &["wave"]).unwrap();
    let registry = registry(&s);
    let root = narrata_history::ObjectId::from_bytes(*s.cursor().unwrap().as_bytes());
    let bytes = s
        .history()
        .export_shallow(root, 0, &std::collections::BTreeSet::new())
        .unwrap()
        .to_bytes()
        .unwrap();
    let shallow = Session::restore(s.program().clone(), &hex::encode(bytes)).unwrap();
    let source = registry
        .visited_from_save(
            shallow.history(),
            shallow.cursor().unwrap(),
            PathLimits::default(),
        )
        .unwrap();
    assert!(!source.report().sources[0].complete);
    assert!(matches!(
        source.report().sources[0].missing.as_slice(),
        [VisitMissingV1::Object { .. }]
    ));
    let source = EvolutionRegistry::new()
        .visited_from_save(s.history(), s.cursor().unwrap(), PathLimits::default())
        .unwrap();
    assert!(matches!(
        source.report().sources[0].missing.as_slice(),
        [VisitMissingV1::Artifact { .. }]
    ));
    for limits in [
        PathLimits {
            object_bytes: 0,
            ..PathLimits::default()
        },
        PathLimits {
            transitions: 0,
            ..PathLimits::default()
        },
        PathLimits {
            automatic_steps: 0,
            ..PathLimits::default()
        },
    ] {
        let source = registry
            .visited_from_save(s.history(), s.cursor().unwrap(), limits)
            .unwrap();
        assert!(!source.report().sources[0].complete);
        assert_eq!(source.report().sources[0].missing, [VisitMissingV1::Limit]);
    }
}

#[test]
fn migration_segments_keep_exact_artifacts_and_require_descriptor_evidence() {
    let old = compiled();
    let new = try_variant(|_, main, _| {
        set(
            main,
            "/graphs/start/nodes/tally/data",
            "assignments",
            serde_json::json!([add("local", "visits", 2)]),
        )
    })
    .unwrap();
    let (mut s, names) = session(&old);
    choose(&mut s, &names, &["wave"]).unwrap();
    let mut registry = registry(&s);
    registry.register_program(open(&new.pack).0).unwrap();
    let id = registry
        .register_descriptor(
            EvolutionDescriptorV1::new(
                old.program.artifact_id(),
                new.program.artifact_id(),
                vec![],
            )
            .unwrap(),
        )
        .unwrap();
    let assessment = s.assess_evolution(
        &registry,
        new.program.artifact_id(),
        &[id],
        PathLimits::default(),
    );
    let AssessmentV1::Automatic { plan } = assessment else {
        panic!("{assessment:?}")
    };
    assert!(matches!(
        s.apply_evolution(&registry, &plan, PathLimits::default()),
        ApplyV1::Migrated { .. }
    ));
    let visits = registry
        .visited_from_save(s.history(), s.cursor().unwrap(), PathLimits::default())
        .unwrap();
    let source = &visits.report().sources[0];
    assert!(source.complete);
    assert_eq!(
        source
            .segments
            .iter()
            .map(|s| s.artifact_id)
            .collect::<Vec<_>>(),
        [old.program.artifact_id(), new.program.artifact_id()]
    );
    assert!(
        source.segments[1]
            .events
            .iter()
            .any(|e| matches!(e.action, VisitActionV1::Segment { .. }))
    );
    let mut missing = EvolutionRegistry::new();
    missing.register_program(open(&old.pack).0).unwrap();
    missing.register_program(open(&new.pack).0).unwrap();
    let visits = missing
        .visited_from_save(s.history(), s.cursor().unwrap(), PathLimits::default())
        .unwrap();
    assert!(!visits.report().sources[0].complete);
    assert!(matches!(
        visits.report().sources[0].missing.as_slice(),
        [VisitMissingV1::Descriptor { .. }]
    ));
}

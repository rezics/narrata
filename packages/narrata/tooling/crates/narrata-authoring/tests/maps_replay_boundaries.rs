#![allow(clippy::unwrap_used, clippy::panic)]
#[path = "../../../../nodes/crates/narrata-nodes/tests/support/mod.rs"]
mod support;
use narrata_authoring::{
    maps::*,
    project_relations,
    relations::{ReaderRelationAllowlist, RelationRow},
};
use narrata_nodes::{CommitId, Session, history::evolution::*};
use serde_json::json;
use support::*;

fn registry(s: &Session) -> EvolutionRegistry {
    let mut r = EvolutionRegistry::new();
    r.register_program(s.program().clone()).unwrap();
    r
}

#[test]
fn originating_choice_or_next_survives_an_intervening_bodyless_choice() {
    for starts_with_choice in [true, false] {
        let compilation = try_variant(|_, main, _| {
            set(main, "/graphs/start/nodes", "bodyless", json!({"id":node(20),"type_id":"narrata.passage","data":{"choice_points":[{"id":point(20),"key":"bridge","options":[{"id":option(20),"key":"onward","outcome":{"kind":"branch","target":"visit"}}]}]}}));
            if starts_with_choice {
                set(main, "/graphs/start/nodes/gate/data/choice_points/2/options/0/outcome", "target", json!("bodyless"));
            } else {
                set(main, GATE, "choice_points", json!([]));
                set(main, GATE, "next", json!("bodyless"));
            }
        }).unwrap();
        let (mut s, names) = session(&compilation);
        if starts_with_choice {
            for keys in [&["wave"][..], &["rope", "lamp"][..], &["left"][..]] {
                choose(&mut s, &names, keys).unwrap();
            }
        }
        let projection = project_relations(s.program()).unwrap();
        let r = registry(&s);
        let before = r
            .visited_from_save(s.history(), s.cursor().unwrap(), PathLimits::default())
            .unwrap();
        let before = ReaderRelationAllowlist::from_visits(&projection, &before).unwrap();
        assert!(
            !before
                .rows()
                .iter()
                .any(|r| matches!(r, RelationRow::Edge(_)))
        );
        choose(&mut s, &names, &["onward"]).unwrap();
        let visits = r
            .visited_from_save(s.history(), s.cursor().unwrap(), PathLimits::default())
            .unwrap();
        let allowed = ReaderRelationAllowlist::from_visits(&projection, &visits).unwrap();
        let actual: Vec<_> = allowed
            .rows()
            .iter()
            .filter(|r| matches!(r, RelationRow::Edge(_)))
            .collect();
        let expected: Vec<_> = projection.rows().iter().filter(|r| matches!(r, RelationRow::Edge(e) if e.source_node.to_string() == node(1) && e.target_node.to_string() == node(11) && e.option.map(|id| id.to_string()) == starts_with_choice.then(|| option(6)))).collect();
        assert_eq!(actual, expected);
        assert_eq!(actual.len(), 1);
    }
}

#[test]
fn checked_cursor_provenance_keeps_empty_incomplete_maps_and_sparse_results() {
    let compilation = try_variant(|_, main, _| {
        set(main, GATE, "choice_points", json!([{"id":point(1),"key":"loop","options":[{"id":option(1),"key":"again","outcome":{"kind":"branch","target":"gate"}}]}]));
    }).unwrap();
    let (mut s, names) = session(&compilation);
    for _ in 0..70 {
        choose(&mut s, &names, &["again"]).unwrap();
    }
    let r = registry(&s);
    let projection = project_relations(s.program()).unwrap();
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
    let shallow = Session::restore(s.program().clone(), &hex::encode(bytes)).unwrap();
    for visits in [
        r.visited_from_save(
            shallow.history(),
            shallow.cursor().unwrap(),
            PathLimits::default(),
        )
        .unwrap(),
        r.visited_from_save(
            s.history(),
            s.cursor().unwrap(),
            PathLimits {
                object_bytes: 0,
                ..PathLimits::default()
            },
        )
        .unwrap(),
        r.visited_from_save(
            s.history(),
            s.cursor().unwrap(),
            PathLimits {
                transitions: 0,
                ..PathLimits::default()
            },
        )
        .unwrap(),
    ] {
        assert!(visits.report().sources[0].segments.is_empty());
        assert_eq!(
            visits.report().sources[0].artifact_id,
            Some(s.program().artifact_id())
        );
        let map = reader_map(&projection, &visits).unwrap();
        assert!(map.nodes.is_empty() && map.units.is_empty());
        assert!(!map.sources[0].complete);
        assert_eq!(map.sources[0].missing, visits.report().sources[0].missing);
        let allow = ReaderRelationAllowlist::from_visits(&projection, &visits).unwrap();
        let sparse = allow.check_rows(vec![]).unwrap();
        assert_eq!(reader_map_from_subset(&sparse, &visits).unwrap(), map);
    }
    let unknown = r
        .visited_from_save(
            s.history(),
            CommitId::from_bytes([99; 32]),
            PathLimits::default(),
        )
        .unwrap();
    assert_eq!(unknown.report().sources[0].artifact_id, None);
    assert!(!unknown.report().sources[0].missing.is_empty());
    assert!(reader_map(&projection, &unknown).is_err());
    assert!(ReaderRelationAllowlist::from_visits(&projection, &unknown).is_err());
    let wrong = project_relations(&compiled().program).unwrap();
    let visits = r
        .visited_from_save(
            s.history(),
            s.cursor().unwrap(),
            PathLimits {
                object_bytes: 0,
                ..PathLimits::default()
            },
        )
        .unwrap();
    assert!(reader_map(&wrong, &visits).is_err());
}

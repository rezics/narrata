#![allow(clippy::panic, clippy::unwrap_used)]

use narrata_core::{
    SourceAnchor, SourceDocumentId, SourceLocation, SourceMapError, SourceMapSidecar, StableId,
    StableIdKind, diagnostic::SourceSpan,
};

fn anchor(path: &str, start: u32) -> SourceAnchor {
    SourceAnchor {
        id: StableId::new(StableIdKind::Instruction, [1; 16]),
        location: SourceLocation {
            document: SourceDocumentId::from_u128(1),
            path: path.to_owned(),
            span: SourceSpan {
                start,
                end: start + 4,
            },
        },
    }
}

#[test]
fn source_map_round_trips_without_changing_persistent_ids() {
    let sidecar = SourceMapSidecar::new(vec![anchor("chapter/intro.nar", 20)]).unwrap();
    let restored = SourceMapSidecar::from_json(&sidecar.to_json().unwrap()).unwrap();
    assert_eq!(restored, sidecar);
    assert_eq!(restored.entries()[0].id.bytes, [1; 16]);
}

#[test]
fn duplicate_id_reports_both_source_locations() {
    let error = SourceMapSidecar::new(vec![
        anchor("chapter/intro.nar", 20),
        anchor("chapter/renamed.nar", 80),
    ])
    .unwrap_err();
    assert!(matches!(
        error,
        SourceMapError::Duplicate { first, second, .. }
            if first.span.start == 20 && second.span.start == 80
    ));
}

#[test]
fn sidecar_rejects_unknown_fields_and_parent_paths() {
    assert!(SourceMapSidecar::new(vec![anchor("../secret.nar", 0)]).is_err());
    let json = br#"{"version":1,"entries":[],"surprise":true}"#;
    assert!(SourceMapSidecar::from_json(json).is_err());
}

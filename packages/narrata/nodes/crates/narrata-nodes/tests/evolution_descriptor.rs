#![allow(clippy::unwrap_used, clippy::panic)]

use narrata_nodes::{ArtifactId, history::evolution::*};
use serde_json::json;
use std::path::Path;

fn recovery(n: u8) -> Recovery {
    Recovery {
        deleted: DeletedId::Node(narrata_nodes::NodeId::from_bytes([n; 16])),
        target: RecoveryTarget {
            node: narrata_nodes::NodeId::from_bytes([3; 16]),
            choice_point: narrata_nodes::ChoicePointId::from_bytes([4; 16]),
        },
    }
}
#[test]
fn strict_metadata_and_canonical_descriptor_identity() {
    let from = ArtifactId::from_bytes([1; 32]);
    let to = ArtifactId::from_bytes([2; 32]);
    let descriptor = EvolutionDescriptorV1::new(from, to, vec![recovery(2), recovery(1)]).unwrap();
    let reordered = EvolutionDescriptorV1::new(from, to, vec![recovery(1), recovery(2)]).unwrap();
    assert_eq!(descriptor.id(), reordered.id());
    assert_eq!(descriptor.encode(), reordered.encode());
    assert_eq!(
        EvolutionDescriptorV1::decode(&descriptor.encode()).unwrap(),
        descriptor
    );
    let text = serde_json::to_string_pretty(&descriptor).unwrap();
    assert_eq!(EvolutionDescriptorV1::from_json(&text).unwrap(), descriptor);
    assert!(EvolutionDescriptorV1::new(from, from, vec![]).is_err());
    assert!(RecoveryDeclarationsV1::new(from, vec![recovery(1), recovery(1)]).is_err());
    assert!(RecoveryDeclarationsV1::new(from, vec![recovery(1); MAX_RECOVERIES + 1]).is_err());
    for invalid in [
        text.replace("\"format_version\": 1", "\"format_version\": 2"),
        text.replacen('{', "{\"unexpected\":true,", 1),
        text.replacen('{', "{\"format_version\":1,", 1),
        format!("{}{}", text, text),
    ] {
        assert!(EvolutionDescriptorV1::from_json(&invalid).is_err());
    }
    assert!(EvolutionDescriptorV1::from_json(&" ".repeat(MAX_DESCRIPTOR_BYTES + 1)).is_err());
    let mut trailing = descriptor.encode();
    trailing.push(0);
    assert!(EvolutionDescriptorV1::decode(&trailing).is_err());
    let mut nonminimal = descriptor.encode();
    nonminimal.splice(2..3, [0x18, 1]);
    assert!(EvolutionDescriptorV1::decode(&nonminimal).is_err());
    let mut wrong_kind = serde_json::to_value(&descriptor).unwrap();
    wrong_kind["recoveries"][0]["deleted"] = json!("option:01010101010101010101010101010101");
    assert!(EvolutionDescriptorV1::from_json(&wrong_kind.to_string()).is_err());
}

#[test]
fn revision_json_retains_all_u64_bits_and_rejects_numbers() {
    let value = CursorRevision(u64::MAX);
    let text = serde_json::to_string(&value).unwrap();
    assert_eq!(text, "\"18446744073709551615\"");
    assert_eq!(
        serde_json::from_str::<CursorRevision>(&text).unwrap(),
        value
    );
    for invalid in ["1", "\"01\"", "\"-1\"", "\"18446744073709551616\""] {
        assert!(serde_json::from_str::<CursorRevision>(invalid).is_err());
    }
}

#[test]
fn checked_evolution_schemas_match_their_owning_types() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schemas");
    for (name, schema) in schemas() {
        let expected = serde_json::to_string_pretty(&schema).unwrap() + "\n";
        assert_eq!(
            std::fs::read_to_string(root.join(name)).unwrap(),
            expected,
            "schema drift: {name}"
        );
    }
}

#[test]
#[ignore = "explicit schema generation"]
fn generate_evolution_schemas() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schemas");
    for (name, schema) in schemas() {
        std::fs::write(
            root.join(name),
            serde_json::to_string_pretty(&schema).unwrap() + "\n",
        )
        .unwrap();
    }
}

#![allow(clippy::unwrap_used)]

use super::*;

fn encode_unsigned(value: &u64) -> Vec<u8> {
    let mut writer = CborWriter::new();
    writer.unsigned(*value);
    writer.into_bytes()
}

#[test]
fn integer_heads_are_minimal_and_signed_extremes_round_trip() {
    for (value, expected) in [
        (0, "00"),
        (23, "17"),
        (24, "1818"),
        (255, "18ff"),
        (256, "190100"),
        (65535, "19ffff"),
        (65536, "1a00010000"),
        (u32::MAX as u64, "1affffffff"),
        (u32::MAX as u64 + 1, "1b0000000100000000"),
        (u64::MAX, "1bffffffffffffffff"),
    ] {
        let bytes = encode_unsigned(&value);
        assert_eq!(hex::encode(&bytes), expected);
        assert_eq!(
            decode_checked(
                &bytes,
                &Default::default(),
                CborReader::unsigned,
                encode_unsigned
            )
            .unwrap(),
            value
        );
    }
    for value in [i64::MIN, -65537, -256, -24, -1, 0, i64::MAX] {
        let mut writer = CborWriter::new();
        writer.signed(value);
        let bytes = writer.into_bytes();
        let mut reader = CborReader::new(&bytes);
        assert_eq!(reader.signed().unwrap(), value);
        reader.finish().unwrap();
    }
    assert_eq!(
        CborReader::new(&hex::decode("1b8000000000000000").unwrap()).signed(),
        Err(DecodeError::IntegerOverflow)
    );
    assert_eq!(
        CborReader::new(&hex::decode("3b8000000000000000").unwrap()).signed(),
        Err(DecodeError::IntegerOverflow)
    );
}

#[test]
fn reader_writer_cover_strings_collections_booleans_and_null() {
    let mut writer = CborWriter::new();
    writer.map(1);
    writer.unsigned(0);
    writer.array(5);
    writer.bytes(&[1, 2]);
    writer.text("é");
    writer.boolean(false);
    writer.boolean(true);
    writer.null();
    let bytes = writer.into_bytes();
    assert_eq!(hex::encode(&bytes), "a1008542010262c3a9f4f5f6");
    let mut reader = CborReader::new(&bytes);
    assert_eq!(reader.map_len().unwrap(), 1);
    assert_eq!(reader.unsigned().unwrap(), 0);
    assert_eq!(reader.array_len().unwrap(), 5);
    assert_eq!(reader.bytes_exact::<2>().unwrap(), [1, 2]);
    assert_eq!(reader.text(2).unwrap(), "é");
    assert!(!reader.boolean().unwrap());
    assert!(reader.boolean().unwrap());
    assert_eq!(reader.optional(CborReader::unsigned).unwrap(), None);
    reader.finish().unwrap();
    let mut reader = CborReader::new(&[1]);
    assert_eq!(reader.optional(CborReader::unsigned).unwrap(), Some(1));
    reader.finish().unwrap();
    let mut composed = CborWriter::new();
    composed.text("é");
    let mut decomposed = CborWriter::new();
    decomposed.text("e\u{301}");
    assert_ne!(composed.into_bytes(), decomposed.into_bytes());
    assert_eq!(
        CborReader::new(&[0x61, 0xff]).text(1),
        Err(DecodeError::InvalidUtf8)
    );
    assert_eq!(
        CborReader::new(&[0x42, 0]).bytes(2),
        Err(DecodeError::UnexpectedEnd)
    );
    assert_eq!(
        CborReader::new(&[0x41, 0]).bytes_exact::<2>(),
        Err(DecodeError::Schema("fixed byte string length"))
    );
}

fn preflight(bytes: &[u8], limits: &DecodeLimits) -> DecodeError {
    // A schema marker proves that structural validation allowed the callback.
    decode_checked::<()>(
        bytes,
        limits,
        |_| Err(DecodeError::Schema("probe")),
        |_| vec![],
    )
    .unwrap_err()
}

#[test]
fn checked_decode_rejects_constructs_outside_the_profile_before_schema_decode() {
    let limits = DecodeLimits::default();
    for input in [
        "1800",
        "1900ff",
        "1a0000ffff",
        "1b00000000ffffffff",
        "5800",
        "7800",
        "9800",
        "b800", // non-minimal lengths
        "9f00ff",
        "5f40ff",
        "7f60ff",
        "bf0000ff", // indefinite lengths
        "c000",
        "f90000",
        "fa00000000",
        "fb0000000000000000", // tag and floats
        "f7",
        "f818",
        "1c",
        "1d",
        "1e", // unsupported simple/reserved items
        "a200000001",
        "a201000000",
        "a16000", // duplicate, unsorted, non-unsigned keys
        "00ff",
        "61ff",
        "8200",
        "", // trailing, invalid UTF-8, truncated
    ] {
        let bytes = hex::decode(input).unwrap();
        assert_ne!(
            preflight(&bytes, &limits),
            DecodeError::Schema("probe"),
            "{input}"
        );
    }
    for input in [
        "00",
        "20",
        "40",
        "60",
        "80",
        "a0",
        "f4",
        "f5",
        "f6",
        "a2008001a100f6",
    ] {
        assert_eq!(
            preflight(&hex::decode(input).unwrap(), &limits),
            DecodeError::Schema("probe"),
            "{input}"
        );
    }
}

#[test]
fn checked_decode_enforces_round_trip_and_callback_consumption() {
    assert_eq!(
        decode_checked(&[1], &Default::default(), CborReader::unsigned, |_| vec![0]),
        Err(DecodeError::NonCanonical("round-trip mismatch"))
    );
    assert_eq!(
        decode_checked(
            &[0x81, 1],
            &Default::default(),
            CborReader::array_len,
            |_| vec![0x81, 1]
        ),
        Err(DecodeError::TrailingBytes)
    );
    assert_eq!(
        decode_checked::<()>(
            &[1],
            &Default::default(),
            |_| Err(DecodeError::Schema("domain")),
            |_| vec![]
        ),
        Err(DecodeError::Schema("domain"))
    );
    // The checked helper can return borrowed data without copying the payload.
    assert_eq!(
        decode_checked(
            &[0x61, b'a'],
            &Default::default(),
            |r| r.text(1),
            |s| {
                let mut writer = CborWriter::new();
                writer.text(s);
                writer.into_bytes()
            }
        )
        .unwrap(),
        "a"
    );
}

#[test]
fn checked_decode_enforces_depth_and_length_budgets_before_schema_decode() {
    let limits = DecodeLimits {
        max_payload_bytes: 8,
        max_depth: 2,
        max_string_bytes: 1,
        max_collection_items: 2,
        max_total_items: 3,
    };
    for (input, expected) in [
        ("000000000000000000", "payload bytes"),
        ("818100", "CBOR depth"),
        ("420001", "string bytes"),
        ("626162", "string bytes"),
        ("83000000", "collection items"),
        ("a3000001000200", "collection items"),
        ("a200000100", "CBOR items"),
    ] {
        assert_eq!(
            preflight(&hex::decode(input).unwrap(), &limits),
            DecodeError::Limit(expected)
        );
    }
    assert_eq!(preflight(&[0x81, 0], &limits), DecodeError::Schema("probe"));
    assert_eq!(
        preflight(
            &[0],
            &DecodeLimits {
                max_depth: 0,
                ..limits
            }
        ),
        DecodeError::Limit("CBOR depth")
    );
    assert_eq!(
        preflight(
            &[0],
            &DecodeLimits {
                max_total_items: 0,
                ..limits
            }
        ),
        DecodeError::Limit("CBOR items")
    );
    // Traversal uses a bounded explicit stack rather than recursive calls.
    let mut nested = vec![0x81; 10_000];
    nested.push(0);
    assert_eq!(
        preflight(
            &nested,
            &DecodeLimits {
                max_depth: 10_001,
                ..Default::default()
            }
        ),
        DecodeError::Schema("probe")
    );
}

const MANIFEST: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../../../fixtures/compat/stage5-v0/manifest.json"
));

fn manifest_value(field: &str) -> &str {
    MANIFEST
        .split_once(&format!("\"{field}\": \""))
        .unwrap()
        .1
        .split('"')
        .next()
        .unwrap()
}

#[test]
fn frozen_stage5_envelopes_digests_and_object_ids_are_unchanged() {
    let fixtures = [
        (
            "program-v0.hex",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../../../../fixtures/compat/stage5-v0/program-v0.hex"
            )),
        ),
        (
            "snapshot-v0.hex",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../../../../fixtures/compat/stage5-v0/snapshot-v0.hex"
            )),
        ),
        (
            "receipt-v1.hex",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../../../../fixtures/compat/stage5-v0/receipt-v1.hex"
            )),
        ),
        (
            "commit-v1.hex",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../../../../fixtures/compat/stage5-v0/commit-v1.hex"
            )),
        ),
        (
            "checkpoint-bundle-v1.hex",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../../../../fixtures/compat/stage5-v0/checkpoint-bundle-v1.hex"
            )),
        ),
        (
            "timeline-archive-v1.hex",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../../../../fixtures/compat/stage5-v0/timeline-archive-v1.hex"
            )),
        ),
    ];
    for (name, input) in fixtures {
        let bytes = hex::decode(input.trim()).unwrap();
        let expected_hash = MANIFEST
            .split_once(&format!("\"{name}\""))
            .unwrap()
            .1
            .split_once("\"sha256\": \"")
            .unwrap()
            .1
            .split('"')
            .next()
            .unwrap();
        assert_eq!(hex::encode(sha256(&bytes)), expected_hash, "{name}");
        if name.starts_with("checkpoint-") || name.starts_with("timeline-") {
            continue;
        }
        let envelope = inspect_envelope(&bytes, &Default::default()).unwrap();
        assert_eq!(
            encode_envelope(envelope.kind, envelope.schema_version, envelope.payload),
            bytes
        );
        if name == "program-v0.hex" {
            assert_eq!(
                format!(
                    "artifact:{}",
                    hex::encode(digest_bytes("program-artifact", 0, envelope.payload))
                ),
                manifest_value("program_artifact_id")
            );
        }
        let field = match name {
            "receipt-v1.hex" => Some(("receipt_id", "stored-receipt:")),
            "commit-v1.hex" => Some(("commit_id", "commit:")),
            _ => None,
        };
        if let Some((field, prefix)) = field {
            assert_eq!(
                format!(
                    "{prefix}{}",
                    hex::encode(object_id(
                        envelope.kind,
                        envelope.schema_version,
                        envelope.payload
                    ))
                ),
                manifest_value(field)
            );
        }
    }
    let bytes = hex::decode(
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../../../fixtures/codec/value-v0.cbor.hex"
        ))
        .trim(),
    )
    .unwrap();
    assert_eq!(
        hex::encode(sha256(&bytes)),
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../../../fixtures/codec/value-v0.sha256"
        ))
        .trim()
    );
}

#[test]
fn digest_frames_separate_domains_schemas_and_kinds() {
    assert_ne!(digest_bytes("ab", 0, b"c"), digest_bytes("a", 0, b"bc"));
    assert_ne!(digest_bytes("a", 0, b"b"), digest_bytes("a", 1, b"b"));
    assert_ne!(object_id(1, 0, b"x"), object_id(2, 0, b"x"));
    assert_ne!(object_id(1, 0, b"x"), object_id(1, 1, b"x"));
    assert_ne!(object_id(1, 0, b"x"), object_id(1, 0, b"y"));
}

#[test]
fn envelopes_accept_domain_kind_codes_and_reject_corruption_and_versions() {
    let limits = EnvelopeLimits::default();
    for kind in [0, 1, 11, 12, u16::MAX] {
        let bytes = encode_envelope(kind, 7, &[0]);
        let envelope = decode_envelope(&bytes, kind, 7, &limits).unwrap();
        assert_eq!(envelope.kind, kind);
        assert_eq!(envelope.payload, &[0]);
        assert!(decode_envelope_versions(&bytes, kind, &[0, 7], &limits).is_ok());
        assert!(decode_envelope_versions(&bytes, kind, &[0], &limits).is_err());
        assert_eq!(
            decode_envelope(&bytes, kind.wrapping_add(1), 7, &limits).unwrap_err(),
            DecodeError::Envelope("wrong object kind")
        );
    }
    let bytes = encode_envelope(42, 0, &[0]);
    for (offset, expected) in [
        (0, DecodeError::Envelope("wrong magic")),
        (
            9,
            DecodeError::UnsupportedVersion {
                axis: "envelope",
                version: 1,
            },
        ),
        (15, DecodeError::Envelope("unsupported flags")),
        (
            23,
            DecodeError::Envelope("payload length mismatch or trailing bytes"),
        ),
        (24, DecodeError::Envelope("payload digest mismatch")),
        (56, DecodeError::Envelope("payload digest mismatch")),
    ] {
        let mut bad = bytes.clone();
        bad[offset] ^= 1;
        assert_eq!(inspect_envelope(&bad, &limits).unwrap_err(), expected);
    }
    assert_eq!(
        inspect_envelope(&bytes[..55], &limits).unwrap_err(),
        DecodeError::Envelope("truncated header")
    );
    assert_eq!(
        inspect_envelope(
            &bytes,
            &EnvelopeLimits {
                max_envelope_bytes: 56,
                ..limits
            }
        )
        .unwrap_err(),
        DecodeError::Limit("envelope bytes")
    );
    assert_eq!(
        inspect_envelope(
            &bytes,
            &EnvelopeLimits {
                max_payload_bytes: 0,
                ..limits
            }
        )
        .unwrap_err(),
        DecodeError::Limit("payload bytes")
    );
    let mut trailing = bytes.clone();
    trailing.push(0);
    assert!(inspect_envelope(&trailing, &limits).is_err());
    let mut overflow = bytes;
    overflow[16..24].copy_from_slice(&u64::MAX.to_be_bytes());
    assert_eq!(
        inspect_envelope(
            &overflow,
            &EnvelopeLimits {
                max_payload_bytes: u64::MAX,
                ..limits
            }
        )
        .unwrap_err(),
        DecodeError::LengthOverflow
    );
}

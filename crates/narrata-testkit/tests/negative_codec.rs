#![allow(clippy::panic, clippy::unwrap_used)]

use narrata_core::{
    Value,
    codec::{
        DecodeError, ObjectKind, decode_canonical_value, decode_envelope, encode_canonical_value,
        encode_envelope,
    },
    limits::{DecodeLimits, ProgramLoadLimits},
    program::encode_program_artifact,
};
use narrata_testkit::generator::hello_v0;

#[test]
fn all_restricted_cbor_forms_are_rejected() {
    let limits = DecodeLimits::default();
    for bytes in [
        hex::decode("82180000").unwrap(),
        hex::decode("9f00ff").unwrap(),
        hex::decode("8205bfff").unwrap(),
        hex::decode("82037f6161ff").unwrap(),
        hex::decode("820361ff").unwrap(),
        hex::decode("810000").unwrap(),
        hex::decode("8118ff").unwrap(),
    ] {
        assert!(decode_canonical_value(&bytes, &limits).is_err());
    }
    let nested = encode_canonical_value(&Value::List(vec![Value::List(vec![Value::Null])]));
    let shallow = DecodeLimits {
        max_value_depth: 1,
        ..DecodeLimits::default()
    };
    assert!(matches!(
        decode_canonical_value(&nested, &shallow),
        Err(DecodeError::Limit(_))
    ));
}

#[test]
fn envelope_kind_schema_length_digest_and_trailing_bytes_are_checked() {
    let payload = encode_canonical_value(&Value::Null);
    let envelope = encode_envelope(ObjectKind::Value, 0, &payload);
    assert!(decode_envelope(&envelope, ObjectKind::Program, 0, &Default::default()).is_err());
    assert!(decode_envelope(&envelope, ObjectKind::Value, 1, &Default::default()).is_err());

    let mut length = envelope.clone();
    length[16..24].copy_from_slice(&u64::MAX.to_be_bytes());
    assert!(decode_envelope(&length, ObjectKind::Value, 0, &Default::default()).is_err());

    let mut digest = envelope.clone();
    let last = digest.len().saturating_sub(1);
    digest[last] ^= 1;
    assert!(decode_envelope(&digest, ObjectKind::Value, 0, &Default::default()).is_err());

    let mut trailing = envelope.clone();
    trailing.push(0);
    assert!(decode_envelope(&trailing, ObjectKind::Value, 0, &Default::default()).is_err());
}

#[test]
fn program_unknown_duplicate_and_unsorted_fields_are_rejected() {
    let encoded = encode_program_artifact(&hello_v0());
    let payload = encoded[56..].to_vec();

    let mut unsorted = payload.clone();
    unsorted[1..5].copy_from_slice(&[1, 0, 0, 0]);
    let unsorted = encode_envelope(ObjectKind::Program, 0, &unsorted);
    assert!(narrata_core::load_program(&unsorted, &ProgramLoadLimits::default()).is_err());

    for extra_key in [0_u8, 9_u8] {
        let mut extra = payload.clone();
        extra[0] = 0xaa;
        extra.extend_from_slice(&[extra_key, 0]);
        let extra = encode_envelope(ObjectKind::Program, 0, &extra);
        assert!(narrata_core::load_program(&extra, &ProgramLoadLimits::default()).is_err());
    }
}

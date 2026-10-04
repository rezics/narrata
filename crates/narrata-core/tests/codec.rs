#![allow(clippy::panic, clippy::unwrap_used)]

use std::collections::BTreeMap;

use narrata_core::{
    FieldId, Value,
    codec::{DecodeError, decode_canonical_value, encode_canonical_value},
    limits::DecodeLimits,
    value::{ValueLimitError, validate_value},
};
use proptest::prelude::*;

#[test]
fn value_round_trip_and_record_order_are_canonical() {
    let mut first = BTreeMap::new();
    first.insert(FieldId::from_u128(2), Value::I64(-1));
    first.insert(FieldId::from_u128(1), Value::from("é"));
    let mut second = BTreeMap::new();
    second.insert(FieldId::from_u128(1), Value::from("é"));
    second.insert(FieldId::from_u128(2), Value::I64(-1));
    let first = Value::Record(first);
    let second = Value::Record(second);
    let bytes = encode_canonical_value(&first);
    assert_eq!(bytes, encode_canonical_value(&second));
    assert_eq!(
        decode_canonical_value(&bytes, &Default::default()).unwrap(),
        first
    );
}

#[test]
fn exact_utf8_does_not_normalize_unicode() {
    let composed = encode_canonical_value(&Value::from("é"));
    let decomposed = encode_canonical_value(&Value::from("e\u{301}"));
    assert_ne!(composed, decomposed);
}

#[test]
fn rejects_non_minimal_and_indefinite_cbor() {
    let limits = DecodeLimits::default();
    assert!(matches!(
        decode_canonical_value(&[0x82, 0x18, 0x00, 0x00], &limits),
        Err(DecodeError::NonCanonical(_))
    ));
    assert!(matches!(
        decode_canonical_value(&[0x9f, 0x00, 0xff], &limits),
        Err(DecodeError::Unsupported(_))
    ));
}

#[test]
fn value_limits_return_errors_without_panicking() {
    let value = Value::List(vec![Value::List(vec![Value::Null])]);
    let limits = DecodeLimits {
        max_value_depth: 2,
        ..DecodeLimits::default()
    };
    assert_eq!(validate_value(&value, &limits), Err(ValueLimitError::Depth));
}

proptest! {
    #[test]
    fn i64_round_trip(value in any::<i64>()) {
        let value = Value::I64(value);
        let encoded = encode_canonical_value(&value);
        prop_assert_eq!(decode_canonical_value(&encoded, &Default::default()).unwrap(), value);
    }
}

#[test]
fn kernel_kind_codes_remain_closed_at_the_core_boundary() {
    use narrata_core::codec::{ObjectKind, encode_envelope, inspect_envelope};

    for code in [0, 12, u16::MAX] {
        let mut bytes = narrata_kernel::codec::encode_envelope(code, 0, &[0]);
        assert!(narrata_kernel::codec::inspect_envelope(&bytes, &Default::default()).is_ok());
        assert_eq!(
            inspect_envelope(&bytes, &Default::default()).unwrap_err(),
            DecodeError::Envelope("unknown object kind")
        );
        // Kind validation retains its original precedence over payload checks.
        bytes[56] ^= 1;
        assert_eq!(
            inspect_envelope(&bytes, &Default::default()).unwrap_err(),
            DecodeError::Envelope("unknown object kind")
        );
    }
    for code in 1..=11 {
        let kind = ObjectKind::from_code(code).unwrap();
        let bytes = encode_envelope(kind, 0, &[0]);
        assert_eq!(
            inspect_envelope(&bytes, &Default::default()).unwrap().kind,
            kind
        );
    }
}

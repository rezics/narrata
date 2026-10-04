#![allow(clippy::unwrap_used)]

use proptest::prelude::*;

use super::*;

#[test]
fn identifiers_enforce_their_alphabets_and_byte_limits() {
    assert!(ProviderId::new("local").is_ok());
    assert!(ProviderId::new("rezics-2").is_ok());
    for bad in ["", "Local", "a_b", "é", &"a".repeat(33)] {
        assert_eq!(ProviderId::new(bad), Err(ContentError::Provider), "{bad}");
    }
    assert!(ContentKey::new("章节/3#段落").is_ok());
    assert!(ContentKey::new("a".repeat(256)).is_ok());
    // 86 three-byte characters exceed 256 bytes although they are only 86 characters.
    for bad in ["", "tab\there", "line\n", "\u{85}", &"界".repeat(86)] {
        assert_eq!(ContentKey::new(bad), Err(ContentError::Key), "{bad:?}");
    }
    assert!(AnchorId::new("p-1 ~").is_ok());
    for bad in ["", "é", "\u{7f}", &"a".repeat(129)] {
        assert_eq!(AnchorId::new(bad), Err(ContentError::Anchor), "{bad:?}");
    }
}

#[test]
fn canonical_bytes_are_fixed() {
    let reference = ContentRef::new("local", "k").unwrap();
    assert_eq!(hex::encode(reference.to_bytes()), "a200656c6f63616c01616b");
    let segment = Segment {
        unit: reference.clone(),
        first: None,
        last: Some(AnchorId::new("z").unwrap()),
    };
    assert_eq!(
        hex::encode(segment.to_bytes()),
        "a200a200656c6f63616c01616b02617a"
    );
    assert_eq!(Segment::from_bytes(&segment.to_bytes()).unwrap(), segment);
    assert_eq!(
        Segment::from_bytes(&Segment::unit(reference.clone()).to_bytes()).unwrap(),
        Segment::unit(reference)
    );
}

#[test]
fn checked_decoding_rejects_invalid_values_and_shapes() {
    let decode = |text: &str| ContentRef::from_bytes(&hex::decode(text).unwrap());
    // Upper-case provider.
    assert_eq!(
        decode("a200654c6f63616c01616b"),
        Err(DecodeError::Schema("content provider"))
    );
    // Swapped key order.
    assert!(decode("a201616b00656c6f63616c").is_err());
    // Missing key, unknown field, trailing byte.
    assert!(decode("a100656c6f63616c").is_err());
    assert!(decode("a200656c6f63616c02616b").is_err());
    assert!(decode("a200656c6f63616c01616b00").is_err());
    // Empty key.
    assert!(decode("a200656c6f63616c0160").is_err());
    // A segment anchor with a control character.
    assert!(
        Segment::from_bytes(&hex::decode("a200a200656c6f63616c01616b01610a").unwrap()).is_err()
    );
}

fn reference() -> impl Strategy<Value = ContentRef> {
    ("[a-z0-9-]{1,32}", "[^\\p{Cc}]{1,40}").prop_filter_map("bounded", |(provider, key)| {
        ContentRef::new(&provider, &key).ok()
    })
}

fn anchor() -> impl Strategy<Value = AnchorId> {
    "[ -~]{1,128}".prop_map(|value| AnchorId::new(value).unwrap())
}

proptest! {
    #[test]
    fn segments_round_trip_and_tampering_is_rejected(
        unit in reference(),
        first in proptest::option::of(anchor()),
        last in proptest::option::of(anchor()),
        position in any::<prop::sample::Index>(),
        flip in 1_u8..,
    ) {
        let segment = Segment { unit, first, last };
        let bytes = segment.to_bytes();
        prop_assert_eq!(Segment::from_bytes(&bytes).unwrap(), segment.clone());
        let mut tampered = bytes.clone();
        let index = position.index(tampered.len());
        tampered[index] ^= flip;
        if let Ok(decoded) = Segment::from_bytes(&tampered) {
            // A flip inside text can produce another valid segment, never the same one.
            prop_assert_ne!(&decoded, &segment);
            prop_assert_eq!(decoded.to_bytes(), tampered);
        }
    }
}

#[cfg(feature = "serde")]
#[test]
fn json_form_validates_like_the_constructors() {
    let reference: ContentRef =
        serde_json::from_str(r#"{"provider":"local","key":"main.station"}"#).unwrap();
    assert_eq!(reference, ContentRef::new("local", "main.station").unwrap());
    assert!(serde_json::from_str::<ContentRef>(r#"{"provider":"Local","key":"x"}"#).is_err());
    assert!(
        serde_json::from_str::<ContentRef>(r#"{"provider":"local","key":"x","extra":1}"#).is_err()
    );
    let segment: Segment =
        serde_json::from_str(r#"{"unit":{"provider":"local","key":"u"},"first":"a"}"#).unwrap();
    assert_eq!(
        serde_json::to_string(&segment).unwrap(),
        r#"{"unit":{"provider":"local","key":"u"},"first":"a"}"#
    );
}

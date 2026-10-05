//! The host messages decode exactly what they encode, and nothing else. The vectors in
//! `protocol-vectors.json` were computed by an encoder independent of this crate and are shared
//! with the TypeScript adapter's tests.

#![allow(clippy::panic, clippy::unwrap_used)]

use std::sync::Arc;

use narrata_kernel::codec::DecodeError;
use narrata_storage::{KeyValue, ObjectDigest, Revision};
use narrata_storage_host::{
    Flush, FlushReply, LoadRequest, Loaded, Persist, Range, StoreId, StoreState,
};
use serde_json::Value;

fn vectors() -> Value {
    serde_json::from_str(include_str!("protocol-vectors.json")).unwrap()
}

fn bytes(value: &Value) -> Vec<u8> {
    hex::decode(value.as_str().unwrap()).unwrap()
}

fn optional(value: &Value) -> Option<Vec<u8>> {
    (!value.is_null()).then(|| bytes(value))
}

fn number(value: &Value) -> u64 {
    value.as_u64().unwrap()
}

fn list<T>(value: &Value, item: impl Fn(&Value) -> T) -> Vec<T> {
    value.as_array().unwrap().iter().map(item).collect()
}

fn digest(value: &Value) -> ObjectDigest {
    ObjectDigest::from_bytes(bytes(value).try_into().unwrap())
}

fn store_id(value: &Value) -> StoreId {
    StoreId::from_bytes(bytes(value).try_into().unwrap())
}

fn store(value: &Value) -> StoreState {
    StoreState {
        id: store_id(&value[0]),
        revision: number(&value[1]),
    }
}

fn key_value(value: &Value, revision: &Value) -> KeyValue {
    KeyValue {
        value: bytes(value),
        revision: Revision::new(number(revision)).unwrap(),
    }
}

fn range(value: &Value) -> Range {
    Range {
        lower: bytes(&value[0]),
        upper: optional(&value[1]),
        limit: number(&value[2]),
    }
}

fn shared(value: &Value) -> Arc<[u8]> {
    bytes(value).into()
}

fn load_request(vector: &Value) -> LoadRequest {
    LoadRequest {
        keys: list(&vector["keys"], bytes),
        objects: list(&vector["objects"], digest),
        key_ranges: list(&vector["keyRanges"], range),
        object_ranges: list(&vector["objectRanges"], range),
    }
}

fn loaded(vector: &Value) -> Loaded {
    Loaded {
        store: store(&vector["store"]),
        keys: list(&vector["keys"], |entry| {
            let value = (!entry[1].is_null()).then(|| key_value(&entry[1][0], &entry[1][1]));
            (bytes(&entry[0]), value)
        }),
        objects: list(&vector["objects"], |entry| {
            (digest(&entry[0]), optional(&entry[1]).map(Arc::from))
        }),
        key_ranges: list(&vector["keyRanges"], |entry| {
            let entries = list(&entry[1], |entry| {
                (bytes(&entry[0]), key_value(&entry[1], &entry[2]))
            });
            (range(&entry[0]), entries)
        }),
        object_ranges: list(&vector["objectRanges"], |entry| {
            (range(&entry[0]), list(&entry[1], digest))
        }),
    }
}

fn flush(vector: &Value) -> Flush {
    Flush {
        store: store_id(&vector["store"]),
        batches: list(&vector["batches"], |batch| Persist {
            base: number(&batch[0]),
            revision: number(&batch[1]),
            put_objects: list(&batch[2], |entry| (digest(&entry[0]), shared(&entry[1]))),
            delete_objects: list(&batch[3], digest),
            put_keys: list(&batch[4], |entry| (bytes(&entry[0]), bytes(&entry[1]))),
            delete_keys: list(&batch[5], bytes),
        }),
    }
}

/// Checks that `message` encodes to the vector's bytes and that those bytes decode back to it.
fn round_trip<T: std::fmt::Debug + PartialEq>(
    name: &str,
    vector: &Value,
    message: &T,
    encode: impl Fn(&T) -> Vec<u8>,
    decode: impl Fn(&[u8]) -> Result<T, DecodeError>,
) {
    let encoded = hex::encode(encode(message));
    assert_eq!(encoded, vector["hex"].as_str().unwrap(), "{name}");
    assert_eq!(
        decode(&bytes(&vector["hex"])).as_ref(),
        Ok(message),
        "{name}"
    );
}

#[test]
fn messages_match_the_shared_vectors() {
    let vectors = vectors();
    let request = load_request(&vectors["loadRequest"]);
    round_trip(
        "load request",
        &vectors["loadRequest"],
        &request,
        LoadRequest::encode,
        LoadRequest::decode,
    );
    let answer = loaded(&vectors["loaded"]);
    round_trip(
        "loaded",
        &vectors["loaded"],
        &answer,
        Loaded::encode,
        Loaded::decode,
    );
    let batches = flush(&vectors["flush"]);
    round_trip(
        "flush",
        &vectors["flush"],
        &batches,
        Flush::encode,
        Flush::decode,
    );
    let persisted = FlushReply::Persisted {
        revision: number(&vectors["persisted"]["revision"]),
    };
    round_trip(
        "persisted",
        &vectors["persisted"],
        &persisted,
        FlushReply::encode,
        FlushReply::decode,
    );
    let conflict = FlushReply::Conflict(store(&vectors["conflict"]["store"]));
    round_trip(
        "conflict",
        &vectors["conflict"],
        &conflict,
        FlushReply::encode,
        FlushReply::decode,
    );
}

fn schema<T: std::fmt::Debug>(result: Result<T, DecodeError>, case: &str) {
    assert!(
        matches!(result, Err(DecodeError::Schema(_))),
        "{case}: {result:?}"
    );
}

#[test]
fn decoding_refuses_unordered_or_inconsistent_messages() {
    let vectors = vectors();
    let request = load_request(&vectors["loadRequest"]);
    let reversed = LoadRequest {
        keys: vec![vec![0, 4, 2], vec![0, 4, 1]],
        ..request.clone()
    };
    schema(LoadRequest::decode(&reversed.encode()), "unordered keys");
    let empty_range = LoadRequest {
        key_ranges: vec![Range {
            lower: vec![0, 5],
            upper: Some(vec![0, 5]),
            limit: 1,
        }],
        ..request.clone()
    };
    schema(LoadRequest::decode(&empty_range.encode()), "empty range");
    let no_limit = LoadRequest {
        object_ranges: vec![Range {
            lower: Vec::new(),
            upper: None,
            limit: 0,
        }],
        ..request
    };
    schema(LoadRequest::decode(&no_limit.encode()), "zero limit");

    let answer = loaded(&vectors["loaded"]);
    let mut outside = answer.clone();
    outside.key_ranges[0].1[0].0 = vec![0, 5];
    schema(
        Loaded::decode(&outside.encode()),
        "range entry outside its range",
    );
    let mut ahead = answer.clone();
    ahead.store.revision = 1;
    schema(
        Loaded::decode(&ahead.encode()),
        "revision ahead of the store",
    );
    let one_key = Loaded {
        keys: vec![answer.keys[0].clone()],
        objects: Vec::new(),
        key_ranges: Vec::new(),
        object_ranges: Vec::new(),
        ..answer
    };
    let mut zero = one_key.encode();
    // The key's revision precedes the three empty lists that end the message.
    let revision_at = zero.len() - 4;
    assert_eq!(zero[revision_at..], [9, 0x80, 0x80, 0x80]);
    zero[revision_at] = 0;
    schema(Loaded::decode(&zero), "zero revision");

    let batches = flush(&vectors["flush"]);
    let mut gap = batches.clone();
    gap.batches[1].base = 11;
    gap.batches[1].revision = 12;
    schema(Flush::decode(&gap.encode()), "batches not consecutive");
    let mut skip = batches.clone();
    skip.batches[1].revision = 12;
    schema(Flush::decode(&skip.encode()), "revision skips");
    let mut both = batches.clone();
    both.batches[0].delete_keys = vec![both.batches[0].put_keys[0].0.clone()];
    schema(Flush::decode(&both.encode()), "key put and deleted");
    let mut short = batches.clone();
    short.batches[0].delete_keys = vec![vec![0]];
    schema(Flush::decode(&short.encode()), "key shorter than its space");
    let none = Flush {
        batches: Vec::new(),
        ..batches
    };
    schema(Flush::decode(&none.encode()), "no batch");
}

#[test]
fn decoding_refuses_non_canonical_bytes_and_other_versions() {
    let persisted = FlushReply::Persisted { revision: 11 }.encode();
    assert_eq!(persisted, [0x83, 0x01, 0x00, 0x0B]);
    let mut trailing = persisted.clone();
    trailing.push(0);
    assert!(FlushReply::decode(&trailing).is_err());
    assert!(matches!(
        FlushReply::decode(&[0x83, 0x18, 0x01, 0x00, 0x0B]),
        Err(DecodeError::NonCanonical(_))
    ));
    assert!(matches!(
        FlushReply::decode(&[0x83, 0x02, 0x00, 0x0B]),
        Err(DecodeError::UnsupportedVersion { version: 2, .. })
    ));
    assert!(matches!(
        FlushReply::decode(&[0x83, 0x01, 0x01, 0x0B]),
        Err(DecodeError::Schema(_))
    ));
    assert!(matches!(
        FlushReply::decode(&[0x9F, 0x01, 0x00, 0x0B, 0xFF]),
        Err(DecodeError::Unsupported(_))
    ));
    assert!(LoadRequest::decode(&[0x84, 0x01, 0x80, 0x80, 0x80]).is_err());
}

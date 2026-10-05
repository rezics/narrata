//! Messages between a cache and its host (ADR 0017).
//!
//! Every message is one CBOR array in the ADR 0003 profile whose first item is the protocol
//! version. Lists are strictly ascending, so a message has one encoding and decoding checks it
//! byte for byte. Keys travel as storage keys, `space (u16, big-endian) ‖ key`, which order the
//! same way the contract orders `(space, key)`; a host stores and ranges over them without
//! knowing key spaces.

use std::sync::Arc;

use narrata_kernel::codec::{CborReader, CborWriter, DecodeError, DecodeLimits, decode_checked};
use narrata_storage::{KeySpace, KeyValue, ObjectDigest, Revision};

pub const PROTOCOL_VERSION: u64 = 1;

/// Bounds for decoding messages. Values are bounded by the cache's limits, not here.
const LIMITS: DecodeLimits = DecodeLimits {
    max_payload_bytes: 1 << 34,
    max_depth: 8,
    max_string_bytes: 1 << 32,
    max_collection_items: 1 << 24,
    max_total_items: 1 << 27,
};

/// One incarnation of a host's store. A store created again after the browser evicted it gets
/// another id, so revisions counted in the old one never match the new one.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct StoreId([u8; 16]);

impl StoreId {
    pub const fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    pub const fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }
}

/// Which store a host holds and the revision of the last batch it persisted; 0 when nothing was
/// ever persisted, so the store is empty.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StoreState {
    pub id: StoreId,
    pub revision: u64,
}

/// `space (u16, big-endian) ‖ key`.
pub fn storage_key(space: KeySpace, key: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(2 + key.len());
    bytes.extend_from_slice(&space.get().to_be_bytes());
    bytes.extend_from_slice(key);
    bytes
}

/// The space and key of a storage key; `None` when it is shorter than a space.
pub fn split_storage_key(bytes: &[u8]) -> Option<(KeySpace, &[u8])> {
    let (space, key) = bytes.split_first_chunk::<2>()?;
    Some((KeySpace::new(u16::from_be_bytes(*space)), key))
}

/// Storage keys or object digests from `lower` up to but excluding `upper` (to the end when
/// `None`), at most `limit` of them. A host answers with every entry in the range when there
/// are fewer than `limit`, so a shorter answer covers the whole range.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Range {
    pub lower: Vec<u8>,
    pub upper: Option<Vec<u8>>,
    pub limit: u64,
}

impl Range {
    pub fn contains(&self, key: &[u8]) -> bool {
        key >= self.lower.as_slice() && self.upper.as_ref().is_none_or(|upper| key < upper)
    }

    fn check(&self) -> Result<(), &'static str> {
        if self.limit == 0 {
            return Err("range limit must be positive");
        }
        if self
            .upper
            .as_ref()
            .is_some_and(|upper| *upper <= self.lower)
        {
            return Err("range upper bound must follow its lower bound");
        }
        Ok(())
    }

    /// Checks entries a host returned for this range.
    fn check_entries<'a>(&self, keys: impl Iterator<Item = &'a [u8]>) -> Result<(), &'static str> {
        let mut count = 0_u64;
        let mut previous: Option<&[u8]> = None;
        for key in keys {
            if !self.contains(key) {
                return Err("range entry outside its range");
            }
            if previous.is_some_and(|previous| previous >= key) {
                return Err("range entries out of order");
            }
            previous = Some(key);
            count += 1;
        }
        if count > self.limit {
            return Err("range answered with more entries than its limit");
        }
        Ok(())
    }
}

/// What a cache lacks: single keys and objects, and key or digest ranges it cannot scan yet. An
/// empty request still asks for the store's state.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LoadRequest {
    /// Storage keys, ascending.
    pub keys: Vec<Vec<u8>>,
    /// Ascending.
    pub objects: Vec<ObjectDigest>,
    pub key_ranges: Vec<Range>,
    /// Digest ranges; their answers list digests without bytes.
    pub object_ranges: Vec<Range>,
}

/// A host's answer to a [`LoadRequest`], read in one transaction. It describes itself, so a cache
/// does not need to remember what it asked.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Loaded {
    pub store: StoreState,
    pub keys: Vec<(Vec<u8>, Option<KeyValue>)>,
    pub objects: Vec<(ObjectDigest, Option<Arc<[u8]>>)>,
    pub key_ranges: Vec<(Range, RangeEntries)>,
    pub object_ranges: Vec<(Range, Vec<ObjectDigest>)>,
}

/// The storage keys and values a host found in a key range, ascending.
pub type RangeEntries = Vec<(Vec<u8>, KeyValue)>;

/// Batches a cache accepted and its host has not yet confirmed, oldest first.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Flush {
    /// The store the batches were checked against.
    pub store: StoreId,
    /// Consecutive: each starts at the revision the previous one ends at.
    pub batches: Vec<Persist>,
}

/// The effect of one batch on the host's store. Every batch that changes the store moves its
/// revision by one, so a host can tell from the revision alone whether another writer came
/// first; keys a batch puts take its revision.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Persist {
    /// The store revision the batch was checked against.
    pub base: u64,
    /// `base + 1`.
    pub revision: u64,
    /// Objects the store lacks, ascending.
    pub put_objects: Vec<(ObjectDigest, Arc<[u8]>)>,
    pub delete_objects: Vec<ObjectDigest>,
    /// Storage keys and values, ascending.
    pub put_keys: Vec<(Vec<u8>, Vec<u8>)>,
    pub delete_keys: Vec<Vec<u8>>,
}

/// How a host's transaction for a [`Flush`] ended.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FlushReply {
    /// Every batch is persisted; the store is at `revision`.
    Persisted { revision: u64 },
    /// The store was not where the first batch expected it, so nothing was written.
    Conflict(StoreState),
}

impl LoadRequest {
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
            && self.objects.is_empty()
            && self.key_ranges.is_empty()
            && self.object_ranges.is_empty()
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut writer = CborWriter::new();
        writer.array(5);
        writer.unsigned(PROTOCOL_VERSION);
        list(&mut writer, &self.keys, |writer, key| writer.bytes(key));
        list(&mut writer, &self.objects, |writer, digest| {
            writer.bytes(digest.as_bytes());
        });
        for ranges in [&self.key_ranges, &self.object_ranges] {
            list(&mut writer, ranges, write_range);
        }
        writer.into_bytes()
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        decode_checked(
            bytes,
            &LIMITS,
            |reader| {
                header(reader, 5)?;
                let request = Self {
                    keys: read_list(reader, |reader| Ok(reader.bytes(u64::MAX)?.to_vec()))?,
                    objects: read_list(reader, read_digest)?,
                    key_ranges: read_list(reader, read_range)?,
                    object_ranges: read_list(reader, read_range)?,
                };
                request.check().map_err(DecodeError::Schema)?;
                Ok(request)
            },
            Self::encode,
        )
    }

    fn check(&self) -> Result<(), &'static str> {
        ascending(&self.keys, "requested keys are out of order")?;
        ascending(&self.objects, "requested objects are out of order")?;
        for ranges in [&self.key_ranges, &self.object_ranges] {
            ascending(ranges, "requested ranges are out of order")?;
            ranges.iter().try_for_each(Range::check)?;
        }
        Ok(())
    }
}

impl Loaded {
    pub fn encode(&self) -> Vec<u8> {
        let mut writer = CborWriter::new();
        writer.array(7);
        writer.unsigned(PROTOCOL_VERSION);
        writer.bytes(self.store.id.as_bytes());
        writer.unsigned(self.store.revision);
        list(&mut writer, &self.keys, |writer, (key, value)| {
            writer.array(2);
            writer.bytes(key);
            match value {
                Some(value) => {
                    writer.array(2);
                    writer.bytes(&value.value);
                    writer.unsigned(value.revision.get());
                }
                None => writer.null(),
            }
        });
        list(&mut writer, &self.objects, |writer, (digest, bytes)| {
            writer.array(2);
            writer.bytes(digest.as_bytes());
            match bytes {
                Some(bytes) => writer.bytes(bytes),
                None => writer.null(),
            }
        });
        list(&mut writer, &self.key_ranges, |writer, (range, entries)| {
            writer.array(2);
            write_range(writer, range);
            list(writer, entries, |writer, (key, value)| {
                writer.array(3);
                writer.bytes(key);
                writer.bytes(&value.value);
                writer.unsigned(value.revision.get());
            });
        });
        list(
            &mut writer,
            &self.object_ranges,
            |writer, (range, digests)| {
                writer.array(2);
                write_range(writer, range);
                list(writer, digests, |writer, digest| {
                    writer.bytes(digest.as_bytes());
                });
            },
        );
        writer.into_bytes()
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        decode_checked(
            bytes,
            &LIMITS,
            |reader| {
                header(reader, 7)?;
                let store = StoreState {
                    id: StoreId(reader.bytes_exact()?),
                    revision: reader.unsigned()?,
                };
                let keys = read_list(reader, |reader| {
                    pair(reader)?;
                    let key = reader.bytes(u64::MAX)?.to_vec();
                    let value = reader.optional(|reader| {
                        pair(reader)?;
                        read_value(reader)
                    })?;
                    Ok((key, value))
                })?;
                let objects = read_list(reader, |reader| {
                    pair(reader)?;
                    let digest = read_digest(reader)?;
                    let bytes = reader.optional(|reader| Ok(reader.bytes(u64::MAX)?.into()))?;
                    Ok((digest, bytes))
                })?;
                let key_ranges = read_list(reader, |reader| {
                    pair(reader)?;
                    let range = read_range(reader)?;
                    let entries = read_list(reader, |reader| {
                        exact(reader, 3)?;
                        let key = reader.bytes(u64::MAX)?.to_vec();
                        Ok((key, read_value(reader)?))
                    })?;
                    Ok((range, entries))
                })?;
                let object_ranges = read_list(reader, |reader| {
                    pair(reader)?;
                    Ok((read_range(reader)?, read_list(reader, read_digest)?))
                })?;
                let loaded = Self {
                    store,
                    keys,
                    objects,
                    key_ranges,
                    object_ranges,
                };
                loaded.check().map_err(DecodeError::Schema)?;
                Ok(loaded)
            },
            Self::encode,
        )
    }

    /// Checks order, range bounds, and that no revision passes the store's.
    pub fn check(&self) -> Result<(), &'static str> {
        let issued = |value: &KeyValue| {
            if value.revision.get() <= self.store.revision {
                Ok(())
            } else {
                Err("key revision passes the store revision")
            }
        };
        let keys = self.keys.iter().map(|(key, _)| key).collect::<Vec<_>>();
        ascending(&keys, "loaded keys are out of order")?;
        self.keys
            .iter()
            .filter_map(|(_, value)| value.as_ref())
            .try_for_each(issued)?;
        let objects = self
            .objects
            .iter()
            .map(|(digest, _)| digest)
            .collect::<Vec<_>>();
        ascending(&objects, "loaded objects are out of order")?;
        if self.store.revision == 0 && self.objects.iter().any(|(_, bytes)| bytes.is_some()) {
            return Err("an empty store holds no object");
        }
        let ranges = self
            .key_ranges
            .iter()
            .map(|(range, _)| range)
            .collect::<Vec<_>>();
        ascending(&ranges, "loaded key ranges are out of order")?;
        for (range, entries) in &self.key_ranges {
            range.check()?;
            range.check_entries(entries.iter().map(|(key, _)| key.as_slice()))?;
            entries.iter().try_for_each(|(_, value)| issued(value))?;
        }
        let ranges = self
            .object_ranges
            .iter()
            .map(|(range, _)| range)
            .collect::<Vec<_>>();
        ascending(&ranges, "loaded object ranges are out of order")?;
        for (range, digests) in &self.object_ranges {
            range.check()?;
            range.check_entries(digests.iter().map(|digest| digest.as_bytes().as_slice()))?;
            if self.store.revision == 0 && !digests.is_empty() {
                return Err("an empty store holds no object");
            }
        }
        Ok(())
    }
}

impl Flush {
    pub fn encode(&self) -> Vec<u8> {
        let mut writer = CborWriter::new();
        writer.array(3);
        writer.unsigned(PROTOCOL_VERSION);
        writer.bytes(self.store.as_bytes());
        list(&mut writer, &self.batches, |writer, batch| {
            writer.array(6);
            writer.unsigned(batch.base);
            writer.unsigned(batch.revision);
            list(writer, &batch.put_objects, |writer, (digest, bytes)| {
                writer.array(2);
                writer.bytes(digest.as_bytes());
                writer.bytes(bytes);
            });
            list(writer, &batch.delete_objects, |writer, digest| {
                writer.bytes(digest.as_bytes());
            });
            list(writer, &batch.put_keys, |writer, (key, value)| {
                writer.array(2);
                writer.bytes(key);
                writer.bytes(value);
            });
            list(writer, &batch.delete_keys, |writer, key| writer.bytes(key));
        });
        writer.into_bytes()
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        decode_checked(
            bytes,
            &LIMITS,
            |reader| {
                header(reader, 3)?;
                let store = StoreId(reader.bytes_exact()?);
                let batches = read_list(reader, |reader| {
                    exact(reader, 6)?;
                    Ok(Persist {
                        base: reader.unsigned()?,
                        revision: reader.unsigned()?,
                        put_objects: read_list(reader, |reader| {
                            pair(reader)?;
                            Ok((read_digest(reader)?, reader.bytes(u64::MAX)?.into()))
                        })?,
                        delete_objects: read_list(reader, read_digest)?,
                        put_keys: read_list(reader, |reader| {
                            pair(reader)?;
                            let key = reader.bytes(u64::MAX)?.to_vec();
                            Ok((key, reader.bytes(u64::MAX)?.to_vec()))
                        })?,
                        delete_keys: read_list(reader, |reader| {
                            Ok(reader.bytes(u64::MAX)?.to_vec())
                        })?,
                    })
                })?;
                let flush = Self { store, batches };
                flush.check().map_err(DecodeError::Schema)?;
                Ok(flush)
            },
            Self::encode,
        )
    }

    /// The revision the store reaches once every batch is persisted.
    pub fn revision(&self) -> Option<u64> {
        self.batches.last().map(|batch| batch.revision)
    }

    fn check(&self) -> Result<(), &'static str> {
        if self.batches.is_empty() {
            return Err("a flush carries at least one batch");
        }
        let mut previous: Option<u64> = None;
        for batch in &self.batches {
            if batch.base.checked_add(1) != Some(batch.revision) {
                return Err("a batch moves the store revision by one");
            }
            if previous.is_some_and(|previous| previous != batch.base) {
                return Err("flushed batches are not consecutive");
            }
            previous = Some(batch.revision);
            let put = batch
                .put_objects
                .iter()
                .map(|(digest, _)| digest)
                .collect::<Vec<_>>();
            ascending(&put, "put objects are out of order")?;
            ascending(&batch.delete_objects, "deleted objects are out of order")?;
            if batch
                .delete_objects
                .iter()
                .any(|digest| put.binary_search(&digest).is_ok())
            {
                return Err("object put and deleted in one batch");
            }
            let put = batch
                .put_keys
                .iter()
                .map(|(key, _)| key)
                .collect::<Vec<_>>();
            ascending(&put, "put keys are out of order")?;
            ascending(&batch.delete_keys, "deleted keys are out of order")?;
            if batch
                .delete_keys
                .iter()
                .any(|key| put.binary_search(&key).is_ok())
            {
                return Err("key put and deleted in one batch");
            }
            let mut keys = put.iter().copied().chain(&batch.delete_keys);
            if keys.any(|key| split_storage_key(key).is_none()) {
                return Err("storage key shorter than its key space");
            }
        }
        Ok(())
    }
}

impl FlushReply {
    pub fn encode(&self) -> Vec<u8> {
        let mut writer = CborWriter::new();
        match self {
            Self::Persisted { revision } => {
                writer.array(3);
                writer.unsigned(PROTOCOL_VERSION);
                writer.unsigned(0);
                writer.unsigned(*revision);
            }
            Self::Conflict(store) => {
                writer.array(4);
                writer.unsigned(PROTOCOL_VERSION);
                writer.unsigned(1);
                writer.bytes(store.id.as_bytes());
                writer.unsigned(store.revision);
            }
        }
        writer.into_bytes()
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        decode_checked(
            bytes,
            &LIMITS,
            |reader| {
                let length = reader.array_len()?;
                version(reader)?;
                match (length, reader.unsigned()?) {
                    (3, 0) => Ok(Self::Persisted {
                        revision: reader.unsigned()?,
                    }),
                    (4, 1) => Ok(Self::Conflict(StoreState {
                        id: StoreId(reader.bytes_exact()?),
                        revision: reader.unsigned()?,
                    })),
                    _ => Err(DecodeError::Schema("unknown flush reply")),
                }
            },
            Self::encode,
        )
    }
}

fn list<T>(writer: &mut CborWriter, items: &[T], mut item: impl FnMut(&mut CborWriter, &T)) {
    writer.array(items.len() as u64);
    for value in items {
        item(writer, value);
    }
}

fn write_range(writer: &mut CborWriter, range: &Range) {
    writer.array(3);
    writer.bytes(&range.lower);
    match &range.upper {
        Some(upper) => writer.bytes(upper),
        None => writer.null(),
    }
    writer.unsigned(range.limit);
}

fn version(reader: &mut CborReader<'_>) -> Result<(), DecodeError> {
    match reader.unsigned()? {
        PROTOCOL_VERSION => Ok(()),
        other => Err(DecodeError::UnsupportedVersion {
            axis: "storage host protocol",
            version: u16::try_from(other).unwrap_or(u16::MAX),
        }),
    }
}

fn header(reader: &mut CborReader<'_>, length: u64) -> Result<(), DecodeError> {
    exact(reader, length)?;
    version(reader)
}

fn exact(reader: &mut CborReader<'_>, length: u64) -> Result<(), DecodeError> {
    if reader.array_len()? == length {
        Ok(())
    } else {
        Err(DecodeError::Schema("message array length"))
    }
}

fn pair(reader: &mut CborReader<'_>) -> Result<(), DecodeError> {
    exact(reader, 2)
}

fn read_list<'a, T>(
    reader: &mut CborReader<'a>,
    mut item: impl FnMut(&mut CborReader<'a>) -> Result<T, DecodeError>,
) -> Result<Vec<T>, DecodeError> {
    // The structural preflight bounded the length; it is not a capacity to trust.
    let length = reader.array_len()?;
    let mut items = Vec::new();
    for _ in 0..length {
        items.push(item(reader)?);
    }
    Ok(items)
}

fn read_digest(reader: &mut CborReader<'_>) -> Result<ObjectDigest, DecodeError> {
    Ok(ObjectDigest::from_bytes(reader.bytes_exact()?))
}

fn read_value(reader: &mut CborReader<'_>) -> Result<KeyValue, DecodeError> {
    let value = reader.bytes(u64::MAX)?.to_vec();
    let revision =
        Revision::new(reader.unsigned()?).ok_or(DecodeError::Schema("key revision is zero"))?;
    Ok(KeyValue { value, revision })
}

fn read_range(reader: &mut CborReader<'_>) -> Result<Range, DecodeError> {
    exact(reader, 3)?;
    Ok(Range {
        lower: reader.bytes(u64::MAX)?.to_vec(),
        upper: reader.optional(|reader| Ok(reader.bytes(u64::MAX)?.to_vec()))?,
        limit: reader.unsigned()?,
    })
}

fn ascending<T: Ord>(items: &[T], what: &'static str) -> Result<(), &'static str> {
    if items.windows(2).all(|pair| pair[0] < pair[1]) {
        Ok(())
    } else {
        Err(what)
    }
}

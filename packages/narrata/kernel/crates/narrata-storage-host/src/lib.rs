//! Kernel storage for asynchronous hosts such as a browser's IndexedDB (ADR 0017).
//!
//! The storage contract is synchronous (ADR 0012). A [`CacheBackend`] keeps it so by answering
//! from the part of the host's store it has loaded: a call that reads anything else fails with
//! [`narrata_storage::StorageError::NotLoaded`] and records what it needed. The host loads that
//! ([`LoadRequest`], [`Loaded`]) and runs the operation again. Engine operations read before
//! they write and a batch is checked before it changes anything, so the attempt that stopped
//! had no effect.
//!
//! Accepted batches wait in order until the host persists them in one transaction that checks
//! the store's revision ([`Flush`], [`FlushReply`]). The host publishes nothing that depends on
//! a batch before it is confirmed; if another writer moved the store first, the cache drops
//! everything and the host reloads. The messages are canonical CBOR (ADR 0003).

#![forbid(unsafe_code)]

mod cache;
mod known;
mod protocol;
#[cfg(feature = "testing")]
pub mod testing;

pub use cache::{CacheBackend, HostError};
pub use protocol::{
    Flush, FlushReply, LoadRequest, Loaded, PROTOCOL_VERSION, Persist, Range, RangeEntries,
    StoreExport, StoreId, StoreState, split_storage_key, storage_key,
};

mod canonical;
mod checked;
mod digest;
mod envelope;
mod error;

#[cfg(test)]
mod tests;

pub use canonical::{CborReader, CborWriter};
pub use checked::{DecodeLimits, decode_checked};
pub use digest::{digest_bytes, object_id, sha256};
pub use envelope::{
    Envelope, EnvelopeLimits, decode_envelope, decode_envelope_versions, encode_envelope,
    inspect_envelope, inspect_envelope_with_kind,
};
pub use error::DecodeError;

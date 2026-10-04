//! A counter domain for tests, benchmarks and conformance corpora: the state is an integer no
//! larger than the artifact's ceiling, and an input adds a positive increment. Its kinds come from
//! the range ADR 0015 reserves for test domains, which no product store registers.

use narrata_kernel::codec::{CborWriter, DecodeError, DecodeLimits, decode_checked, sha256};
use thiserror::Error;

use crate::{ArtifactId, Domain};

pub const COUNTER_COMMIT_KIND: u16 = 0xFF10;
pub const COUNTER_STATE_KIND: u16 = 0xFF11;
pub const COUNTER_INPUT_KIND: u16 = 0xFF12;

/// The counter artifact: every state stays at or below `ceiling`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Counter {
    ceiling: u64,
}

/// The step refused an increment that would pass the ceiling.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
#[error("counter would pass its ceiling")]
pub struct Overflow;

impl Counter {
    pub const fn new(ceiling: u64) -> Self {
        Self { ceiling }
    }

    pub const fn ceiling(&self) -> u64 {
        self.ceiling
    }

    /// The counter's semantics; the history layer never calls it, sessions pass it to
    /// [`crate::Session::advance`].
    pub fn step(&self, state: &u64, increment: &u64) -> Result<u64, Overflow> {
        state
            .checked_add(*increment)
            .filter(|next| *next <= self.ceiling)
            .ok_or(Overflow)
    }
}

fn encode_unsigned(value: u64) -> Vec<u8> {
    let mut writer = CborWriter::new();
    writer.unsigned(value);
    writer.into_bytes()
}

fn decode_unsigned(payload: &[u8]) -> Result<u64, DecodeError> {
    decode_checked(
        payload,
        &DecodeLimits::default(),
        |reader| reader.unsigned(),
        |value| encode_unsigned(*value),
    )
}

impl Domain for Counter {
    const COMMIT_KIND: u16 = COUNTER_COMMIT_KIND;
    const STATE_KIND: u16 = COUNTER_STATE_KIND;
    const STATE_SCHEMA: u16 = 1;
    const INPUT_KIND: u16 = COUNTER_INPUT_KIND;
    const INPUT_SCHEMA: u16 = 1;

    type State = u64;
    type Input = u64;

    fn artifact_id(&self) -> ArtifactId {
        let mut bytes = b"narrata-test-counter\0".to_vec();
        bytes.extend_from_slice(&self.ceiling.to_be_bytes());
        ArtifactId::from_bytes(sha256(&bytes))
    }

    fn encode_state(&self, state: &u64) -> Vec<u8> {
        encode_unsigned(*state)
    }

    fn decode_state(&self, payload: &[u8]) -> Result<u64, DecodeError> {
        let state = decode_unsigned(payload)?;
        if state > self.ceiling {
            return Err(DecodeError::Schema("counter state passes the ceiling"));
        }
        Ok(state)
    }

    fn encode_input(&self, input: &u64) -> Vec<u8> {
        encode_unsigned(*input)
    }

    fn decode_input(&self, payload: &[u8]) -> Result<u64, DecodeError> {
        let increment = decode_unsigned(payload)?;
        if increment == 0 || increment > self.ceiling {
            return Err(DecodeError::Schema("counter increment is out of range"));
        }
        Ok(increment)
    }
}

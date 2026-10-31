use narrata_kernel::codec::DecodeError;

use crate::ArtifactId;

/// A narrative domain whose sessions the history layer stores (ADR 0015).
///
/// An implementor is usually the loaded artifact itself: decoding checks state and input bytes
/// against it, and decoding is the only way the history layer produces a state. The kernel never
/// runs the domain's semantics; [`crate::Session::advance`] runs the step the caller passes, and
/// only when the transition is new.
///
/// Payloads are the domain's canonical ADR 0003 encodings: `decode_state(encode_state(s))` must
/// return a state that encodes to the same bytes, which the history layer checks before storing.
pub trait Domain {
    /// Kind codes of the domain's objects, from the allocation in ADR 0015.
    const COMMIT_KIND: u16;
    const STATE_KIND: u16;
    const STATE_SCHEMA: u16;
    const INPUT_KIND: u16;
    const INPUT_SCHEMA: u16;

    type State;
    type Input;

    /// Identity of the artifact every commit of a session names.
    fn artifact_id(&self) -> ArtifactId;

    fn encode_state(&self, state: &Self::State) -> Vec<u8>;

    /// Schema of this encoded state. Domains with a versioned payload can select a new schema
    /// for new states while continuing to read their earlier objects.
    fn state_schema(&self, _state: &Self::State) -> u16 {
        Self::STATE_SCHEMA
    }

    /// Whether this kind has a decoder for `schema`; the artifact-specific decoder still
    /// decides whether the current artifact accepts it.
    fn supports_state_schema(schema: u16) -> bool {
        schema == Self::STATE_SCHEMA
    }

    /// Decodes a stored state and checks it against the artifact.
    fn decode_state(&self, payload: &[u8]) -> Result<Self::State, DecodeError>;

    fn decode_state_schema(&self, schema: u16, payload: &[u8]) -> Result<Self::State, DecodeError> {
        if Self::supports_state_schema(schema) {
            self.decode_state(payload)
        } else {
            Err(DecodeError::Schema("unsupported state schema"))
        }
    }

    fn encode_input(&self, input: &Self::Input) -> Vec<u8>;

    fn input_schema(&self, _input: &Self::Input) -> u16 {
        Self::INPUT_SCHEMA
    }

    fn supports_input_schema(schema: u16) -> bool {
        schema == Self::INPUT_SCHEMA
    }

    /// Decodes a stored input and checks it against the artifact.
    fn decode_input(&self, payload: &[u8]) -> Result<Self::Input, DecodeError>;

    fn decode_input_schema(&self, schema: u16, payload: &[u8]) -> Result<Self::Input, DecodeError> {
        if Self::supports_input_schema(schema) {
            self.decode_input(payload)
        } else {
            Err(DecodeError::Schema("unsupported input schema"))
        }
    }
}

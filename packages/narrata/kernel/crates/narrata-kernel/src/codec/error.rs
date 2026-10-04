use thiserror::Error;

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum DecodeError {
    #[error("unexpected end of CBOR input")]
    UnexpectedEnd,
    #[error("trailing bytes after CBOR object")]
    TrailingBytes,
    #[error("CBOR length cannot be represented")]
    LengthOverflow,
    #[error("CBOR integer cannot be represented")]
    IntegerOverflow,
    #[error("expected CBOR major type {expected}, found {actual}")]
    Type { expected: u8, actual: u8 },
    #[error("unsupported CBOR construct: {0}")]
    Unsupported(&'static str),
    #[error("non-canonical CBOR: {0}")]
    NonCanonical(&'static str),
    #[error("invalid UTF-8 text string")]
    InvalidUtf8,
    #[error("schema violation: {0}")]
    Schema(&'static str),
    #[error("decode limit exceeded: {0}")]
    Limit(&'static str),
    #[error("envelope error: {0}")]
    Envelope(&'static str),
    #[error("unsupported {axis} version {version}")]
    UnsupportedVersion { axis: &'static str, version: u16 },
}

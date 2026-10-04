use thiserror::Error;

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum WireError {
    #[error("unexpected end of input")]
    UnexpectedEnd,
    #[error("trailing bytes")]
    TrailingBytes,
    #[error("length cannot be represented")]
    LengthOverflow,
    #[error("integer cannot be represented")]
    IntegerOverflow,
    #[error("expected CBOR major type {expected}, found {actual}")]
    Type { expected: u8, actual: u8 },
    #[error("unsupported CBOR construct")]
    Unsupported,
    #[error("non-canonical CBOR")]
    NonCanonical,
    #[error("invalid UTF-8")]
    InvalidUtf8,
    #[error("schema violation: {0}")]
    Schema(&'static str),
    #[error("decode limit exceeded: {0}")]
    Limit(&'static str),
}

// The kernel owns the byte profile; this adapter preserves the store wire API
// and its existing error variants and messages.
pub(crate) use narrata_core::codec::CborWriter as Writer;
use narrata_core::codec::{CborReader, DecodeError};

impl From<DecodeError> for WireError {
    fn from(error: DecodeError) -> Self {
        match error {
            DecodeError::UnexpectedEnd => Self::UnexpectedEnd,
            DecodeError::TrailingBytes => Self::TrailingBytes,
            DecodeError::LengthOverflow => Self::LengthOverflow,
            DecodeError::IntegerOverflow => Self::IntegerOverflow,
            DecodeError::Type { expected, actual } => Self::Type { expected, actual },
            DecodeError::Unsupported(_) => Self::Unsupported,
            DecodeError::NonCanonical(_) => Self::NonCanonical,
            DecodeError::InvalidUtf8 => Self::InvalidUtf8,
            DecodeError::Schema(message) => Self::Schema(message),
            DecodeError::Limit("byte string length") => Self::Limit("byte string"),
            DecodeError::Limit("text string length") => Self::Limit("text string"),
            DecodeError::Limit(message) => Self::Limit(message),
            DecodeError::Envelope(message) => Self::Schema(message),
            DecodeError::UnsupportedVersion { .. } => Self::Unsupported,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Reader<'a>(CborReader<'a>);

impl<'a> Reader<'a> {
    pub(crate) fn new(bytes: &'a [u8]) -> Self {
        Self(CborReader::new(bytes))
    }

    pub(crate) fn finish(self) -> Result<(), WireError> {
        self.0.finish().map_err(Into::into)
    }

    pub(crate) fn unsigned(&mut self) -> Result<u64, WireError> {
        self.0.unsigned().map_err(Into::into)
    }

    pub(crate) fn bytes(&mut self, max: u64) -> Result<&'a [u8], WireError> {
        self.0.bytes(max).map_err(Into::into)
    }

    pub(crate) fn bytes_exact<const N: usize>(&mut self) -> Result<[u8; N], WireError> {
        self.0.bytes_exact().map_err(Into::into)
    }

    pub(crate) fn text(&mut self, max: u64) -> Result<&'a str, WireError> {
        self.0.text(max).map_err(Into::into)
    }

    pub(crate) fn array(&mut self) -> Result<u64, WireError> {
        self.0.array_len().map_err(Into::into)
    }

    pub(crate) fn map(&mut self) -> Result<u64, WireError> {
        self.0.map_len().map_err(Into::into)
    }

    pub(crate) fn optional<T>(
        &mut self,
        decode: impl FnOnce(&mut Self) -> Result<T, WireError>,
    ) -> Result<Option<T>, WireError> {
        // The kernel consumes the null marker and leaves non-null values for the
        // store's schema decoder, whose failures retain their original type.
        if self
            .0
            .optional(|_| Ok(()))
            .map_err(WireError::from)?
            .is_none()
        {
            Ok(None)
        } else {
            decode(self).map(Some)
        }
    }
}

pub(crate) fn expect_map(reader: &mut Reader<'_>, length: u64) -> Result<(), WireError> {
    if reader.map()? == length {
        Ok(())
    } else {
        Err(WireError::Schema("map field count"))
    }
}

pub(crate) fn expect_array(reader: &mut Reader<'_>, length: u64) -> Result<(), WireError> {
    if reader.array()? == length {
        Ok(())
    } else {
        Err(WireError::Schema("array field count"))
    }
}

pub(crate) fn key(reader: &mut Reader<'_>, expected: u64) -> Result<(), WireError> {
    if reader.unsigned()? == expected {
        Ok(())
    } else {
        Err(WireError::Schema("map key order"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kernel_adapter_preserves_store_error_variants_and_limits() {
        assert_eq!(
            Reader::new(&[0x18, 0]).unsigned(),
            Err(WireError::NonCanonical)
        );
        assert_eq!(Reader::new(&[0x9f]).array(), Err(WireError::Unsupported));
        assert_eq!(
            Reader::new(&[0x42, 0, 1]).bytes(1),
            Err(WireError::Limit("byte string"))
        );
        assert_eq!(
            Reader::new(&[0x62, b'a', b'b']).text(1),
            Err(WireError::Limit("text string"))
        );
        assert_eq!(
            Reader::new(&[0x61, 0xff]).text(1),
            Err(WireError::InvalidUtf8)
        );
        assert_eq!(
            Reader::new(&[]).optional(Reader::unsigned),
            Err(WireError::UnexpectedEnd)
        );
        assert_eq!(Reader::new(&[0xf6]).optional(Reader::unsigned), Ok(None));
        let mut reader = Reader::new(&[1]);
        assert_eq!(reader.optional(Reader::unsigned), Ok(Some(1)));
        assert_eq!(reader.finish(), Ok(()));
        assert_eq!(
            Reader::new(&[0]).optional::<()>(|_| Err(WireError::Schema("domain"))),
            Err(WireError::Schema("domain"))
        );
    }
}

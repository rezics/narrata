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

#[derive(Clone, Debug, Default)]
pub(crate) struct Writer {
    bytes: Vec<u8>,
}

impl Writer {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }

    fn head(&mut self, major: u8, value: u64) {
        let prefix = major << 5;
        match value {
            0..=23 => self.bytes.push(prefix | value as u8),
            24..=0xff => {
                self.bytes.push(prefix | 24);
                self.bytes.push(value as u8);
            }
            0x100..=0xffff => {
                self.bytes.push(prefix | 25);
                self.bytes.extend_from_slice(&(value as u16).to_be_bytes());
            }
            0x1_0000..=0xffff_ffff => {
                self.bytes.push(prefix | 26);
                self.bytes.extend_from_slice(&(value as u32).to_be_bytes());
            }
            _ => {
                self.bytes.push(prefix | 27);
                self.bytes.extend_from_slice(&value.to_be_bytes());
            }
        }
    }

    pub(crate) fn unsigned(&mut self, value: u64) {
        self.head(0, value);
    }

    pub(crate) fn bytes(&mut self, value: &[u8]) {
        self.head(2, value.len() as u64);
        self.bytes.extend_from_slice(value);
    }

    pub(crate) fn text(&mut self, value: &str) {
        self.head(3, value.len() as u64);
        self.bytes.extend_from_slice(value.as_bytes());
    }

    pub(crate) fn array(&mut self, length: u64) {
        self.head(4, length);
    }

    pub(crate) fn map(&mut self, length: u64) {
        self.head(5, length);
    }

    pub(crate) fn null(&mut self) {
        self.bytes.push(0xf6);
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Reader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Reader<'a> {
    pub(crate) fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    pub(crate) fn finish(self) -> Result<(), WireError> {
        if self.offset == self.bytes.len() {
            Ok(())
        } else {
            Err(WireError::TrailingBytes)
        }
    }

    fn byte(&mut self) -> Result<u8, WireError> {
        let value = self
            .bytes
            .get(self.offset)
            .copied()
            .ok_or(WireError::UnexpectedEnd)?;
        self.offset = self.offset.saturating_add(1);
        Ok(value)
    }

    fn take(&mut self, length: usize) -> Result<&'a [u8], WireError> {
        let end = self
            .offset
            .checked_add(length)
            .ok_or(WireError::LengthOverflow)?;
        let value = self
            .bytes
            .get(self.offset..end)
            .ok_or(WireError::UnexpectedEnd)?;
        self.offset = end;
        Ok(value)
    }

    fn head(&mut self) -> Result<(u8, u64), WireError> {
        let initial = self.byte()?;
        let major = initial >> 5;
        let additional = initial & 0x1f;
        let value = match additional {
            0..=23 => u64::from(additional),
            24 => {
                let value = u64::from(self.byte()?);
                if value < 24 {
                    return Err(WireError::NonCanonical);
                }
                value
            }
            25 => {
                let bytes = self.take(2)?;
                let value = u64::from(u16::from_be_bytes([bytes[0], bytes[1]]));
                if value <= 0xff {
                    return Err(WireError::NonCanonical);
                }
                value
            }
            26 => {
                let bytes = self.take(4)?;
                let value = u64::from(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]));
                if value <= 0xffff {
                    return Err(WireError::NonCanonical);
                }
                value
            }
            27 => {
                let bytes = self.take(8)?;
                let value = u64::from_be_bytes([
                    bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
                ]);
                if value <= 0xffff_ffff {
                    return Err(WireError::NonCanonical);
                }
                value
            }
            _ => return Err(WireError::Unsupported),
        };
        Ok((major, value))
    }

    fn major(&mut self, expected: u8) -> Result<u64, WireError> {
        let (actual, value) = self.head()?;
        if actual == expected {
            Ok(value)
        } else {
            Err(WireError::Type { expected, actual })
        }
    }

    pub(crate) fn unsigned(&mut self) -> Result<u64, WireError> {
        self.major(0)
    }

    pub(crate) fn bytes(&mut self, max: u64) -> Result<&'a [u8], WireError> {
        let length = self.major(2)?;
        if length > max {
            return Err(WireError::Limit("byte string"));
        }
        self.take(usize::try_from(length).map_err(|_| WireError::LengthOverflow)?)
    }

    pub(crate) fn bytes_exact<const N: usize>(&mut self) -> Result<[u8; N], WireError> {
        let bytes = self.bytes(N as u64)?;
        if bytes.len() != N {
            return Err(WireError::Schema("fixed byte string length"));
        }
        let mut result = [0; N];
        result.copy_from_slice(bytes);
        Ok(result)
    }

    pub(crate) fn text(&mut self, max: u64) -> Result<&'a str, WireError> {
        let length = self.major(3)?;
        if length > max {
            return Err(WireError::Limit("text string"));
        }
        let bytes = self.take(usize::try_from(length).map_err(|_| WireError::LengthOverflow)?)?;
        std::str::from_utf8(bytes).map_err(|_| WireError::InvalidUtf8)
    }

    pub(crate) fn array(&mut self) -> Result<u64, WireError> {
        self.major(4)
    }

    pub(crate) fn map(&mut self) -> Result<u64, WireError> {
        self.major(5)
    }

    pub(crate) fn optional<T>(
        &mut self,
        decode: impl FnOnce(&mut Self) -> Result<T, WireError>,
    ) -> Result<Option<T>, WireError> {
        match self.bytes.get(self.offset).copied() {
            Some(0xf6) => {
                self.offset = self.offset.saturating_add(1);
                Ok(None)
            }
            Some(_) => decode(self).map(Some),
            None => Err(WireError::UnexpectedEnd),
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

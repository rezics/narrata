use super::DecodeError;

/// Writes minimal, definite-length CBOR primitives. The schema encoder chooses
/// container contents and field order; use `decode_checked` to verify a payload.
#[derive(Clone, Debug, Default)]
pub struct CborWriter {
    bytes: Vec<u8>,
}

impl CborWriter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn into_bytes(self) -> Vec<u8> {
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

    pub fn unsigned(&mut self, value: u64) {
        self.head(0, value);
    }

    pub fn signed(&mut self, value: i64) {
        if value >= 0 {
            self.unsigned(value as u64);
        } else {
            self.head(1, (-1_i128 - i128::from(value)) as u64);
        }
    }

    pub fn bytes(&mut self, value: &[u8]) {
        self.head(2, value.len() as u64);
        self.bytes.extend_from_slice(value);
    }

    pub fn text(&mut self, value: &str) {
        self.head(3, value.len() as u64);
        self.bytes.extend_from_slice(value.as_bytes());
    }

    pub fn array(&mut self, length: u64) {
        self.head(4, length);
    }

    pub fn map(&mut self, length: u64) {
        self.head(5, length);
    }

    pub fn boolean(&mut self, value: bool) {
        self.bytes.push(if value { 0xf5 } else { 0xf4 });
    }

    pub fn null(&mut self) {
        self.bytes.push(0xf6);
    }
}

/// Reads minimal, definite-length CBOR primitives with explicit string limits.
/// Schema decoders enforce field sets, key order and collection/depth budgets,
/// and call `finish` to reject trailing bytes. `decode_checked` provides a full
/// structural preflight and canonical round-trip check around such a decoder.
#[derive(Clone, Debug)]
pub struct CborReader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> CborReader<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    pub fn finish(self) -> Result<(), DecodeError> {
        if self.offset == self.bytes.len() {
            Ok(())
        } else {
            Err(DecodeError::TrailingBytes)
        }
    }

    fn byte(&mut self) -> Result<u8, DecodeError> {
        let byte = self
            .bytes
            .get(self.offset)
            .copied()
            .ok_or(DecodeError::UnexpectedEnd)?;
        self.offset = self.offset.saturating_add(1);
        Ok(byte)
    }

    fn take(&mut self, length: usize) -> Result<&'a [u8], DecodeError> {
        let end = self
            .offset
            .checked_add(length)
            .ok_or(DecodeError::LengthOverflow)?;
        let value = self
            .bytes
            .get(self.offset..end)
            .ok_or(DecodeError::UnexpectedEnd)?;
        self.offset = end;
        Ok(value)
    }

    fn head(&mut self) -> Result<(u8, u64), DecodeError> {
        let initial = self.byte()?;
        let major = initial >> 5;
        let additional = initial & 0x1f;
        let value = match additional {
            0..=23 => u64::from(additional),
            24 => {
                let value = u64::from(self.byte()?);
                if value < 24 {
                    return Err(DecodeError::NonCanonical("non-minimal integer/length"));
                }
                value
            }
            25 => {
                let bytes = self.take(2)?;
                let value = u64::from(u16::from_be_bytes([bytes[0], bytes[1]]));
                if value <= 0xff {
                    return Err(DecodeError::NonCanonical("non-minimal integer/length"));
                }
                value
            }
            26 => {
                let bytes = self.take(4)?;
                let value = u64::from(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]));
                if value <= 0xffff {
                    return Err(DecodeError::NonCanonical("non-minimal integer/length"));
                }
                value
            }
            27 => {
                let bytes = self.take(8)?;
                let value = u64::from_be_bytes([
                    bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
                ]);
                if value <= 0xffff_ffff {
                    return Err(DecodeError::NonCanonical("non-minimal integer/length"));
                }
                value
            }
            31 => return Err(DecodeError::Unsupported("indefinite-length item")),
            _ => {
                return Err(DecodeError::Unsupported(
                    "reserved CBOR additional information",
                ));
            }
        };
        Ok((major, value))
    }

    fn expect_major(&mut self, expected: u8) -> Result<u64, DecodeError> {
        let (major, value) = self.head()?;
        if major == expected {
            Ok(value)
        } else {
            Err(DecodeError::Type {
                expected,
                actual: major,
            })
        }
    }

    pub fn unsigned(&mut self) -> Result<u64, DecodeError> {
        self.expect_major(0)
    }

    pub fn signed(&mut self) -> Result<i64, DecodeError> {
        let (major, value) = self.head()?;
        match major {
            0 => i64::try_from(value).map_err(|_| DecodeError::IntegerOverflow),
            1 => {
                let signed = -1_i128 - i128::from(value);
                i64::try_from(signed).map_err(|_| DecodeError::IntegerOverflow)
            }
            _ => Err(DecodeError::Type {
                expected: 0,
                actual: major,
            }),
        }
    }

    pub fn bytes(&mut self, max: u64) -> Result<&'a [u8], DecodeError> {
        let length = self.expect_major(2)?;
        if length > max {
            return Err(DecodeError::Limit("byte string length"));
        }
        let length = usize::try_from(length).map_err(|_| DecodeError::LengthOverflow)?;
        self.take(length)
    }

    pub fn bytes_exact<const N: usize>(&mut self) -> Result<[u8; N], DecodeError> {
        let bytes = self.bytes(N as u64)?;
        if bytes.len() != N {
            return Err(DecodeError::Schema("fixed byte string length"));
        }
        let mut result = [0_u8; N];
        result.copy_from_slice(bytes);
        Ok(result)
    }

    pub fn text(&mut self, max: u64) -> Result<&'a str, DecodeError> {
        let length = self.expect_major(3)?;
        if length > max {
            return Err(DecodeError::Limit("text string length"));
        }
        let length = usize::try_from(length).map_err(|_| DecodeError::LengthOverflow)?;
        std::str::from_utf8(self.take(length)?).map_err(|_| DecodeError::InvalidUtf8)
    }

    pub fn array_len(&mut self) -> Result<u64, DecodeError> {
        self.expect_major(4)
    }

    pub fn map_len(&mut self) -> Result<u64, DecodeError> {
        self.expect_major(5)
    }

    pub fn boolean(&mut self) -> Result<bool, DecodeError> {
        match self.byte()? {
            0xf4 => Ok(false),
            0xf5 => Ok(true),
            _ => Err(DecodeError::Schema("boolean")),
        }
    }

    pub fn optional<T>(
        &mut self,
        decode: impl FnOnce(&mut Self) -> Result<T, DecodeError>,
    ) -> Result<Option<T>, DecodeError> {
        match self.bytes.get(self.offset).copied() {
            Some(0xf6) => {
                self.offset = self.offset.saturating_add(1);
                Ok(None)
            }
            Some(_) => decode(self).map(Some),
            None => Err(DecodeError::UnexpectedEnd),
        }
    }

    // Used by the iterative checked-decode preflight, without allocating strings
    // or collections from untrusted lengths.
    pub(super) fn structural_item(
        &mut self,
        max_string_bytes: u64,
        max_collection_items: u64,
    ) -> Result<Option<(u64, bool)>, DecodeError> {
        let (major, value) = self.head()?;
        match major {
            0 => Ok(None),
            1 => {
                i64::try_from(-1_i128 - i128::from(value))
                    .map_err(|_| DecodeError::IntegerOverflow)?;
                Ok(None)
            }
            2 | 3 => {
                if value > max_string_bytes {
                    return Err(DecodeError::Limit("string bytes"));
                }
                let length = usize::try_from(value).map_err(|_| DecodeError::LengthOverflow)?;
                let bytes = self.take(length)?;
                if major == 3 {
                    std::str::from_utf8(bytes).map_err(|_| DecodeError::InvalidUtf8)?;
                }
                Ok(None)
            }
            4 | 5 => {
                if value > max_collection_items {
                    return Err(DecodeError::Limit("collection items"));
                }
                let is_map = major == 5;
                let remaining = if is_map {
                    value.checked_mul(2).ok_or(DecodeError::LengthOverflow)?
                } else {
                    value
                };
                Ok(Some((remaining, is_map)))
            }
            7 if matches!(value, 20..=22) => Ok(None),
            _ => Err(DecodeError::Unsupported("CBOR item outside profile")),
        }
    }
}

use thiserror::Error;

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum IdParseError {
    #[error("expected identity namespace '{expected}'")]
    WrongNamespace { expected: &'static str },
    #[error("identity payload must contain exactly {expected} hexadecimal characters")]
    WrongLength { expected: usize },
    #[error("identity payload is not valid hexadecimal")]
    InvalidHex,
}

pub fn parse<const N: usize>(input: &str, prefix: &'static str) -> Result<[u8; N], IdParseError> {
    let payload = input
        .strip_prefix(prefix)
        .ok_or(IdParseError::WrongNamespace { expected: prefix })?;
    if payload.len() != N * 2 {
        return Err(IdParseError::WrongLength { expected: N * 2 });
    }
    let mut bytes = [0_u8; N];
    hex::decode_to_slice(payload, &mut bytes).map_err(|_| IdParseError::InvalidHex)?;
    Ok(bytes)
}

#[doc(hidden)]
pub fn format(
    prefix: &str,
    bytes: &[u8],
    formatter: &mut std::fmt::Formatter<'_>,
) -> std::fmt::Result {
    write!(formatter, "{}{}", prefix, hex::encode(bytes))
}

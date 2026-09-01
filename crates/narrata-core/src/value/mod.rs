mod checked;
mod kind;
mod validate;
mod wire;

pub use checked::Value;
pub use kind::ValueKindV0;
pub use validate::{ValueLimitError, validate_value};
pub(crate) use wire::{decode_value, encode_value};

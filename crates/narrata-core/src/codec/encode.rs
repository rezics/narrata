use crate::value::Value;

use super::CborWriter;

pub fn encode_canonical_value(value: &Value) -> Vec<u8> {
    let mut writer = CborWriter::new();
    crate::value::encode_value(&mut writer, value);
    writer.into_bytes()
}

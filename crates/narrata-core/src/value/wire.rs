use std::{collections::BTreeMap, sync::Arc};

use crate::{
    codec::{CborReader, CborWriter, DecodeError},
    identity::{EntityId, FieldId, TypeId, VariantId},
    limits::DecodeLimits,
};

use super::Value;

pub(crate) fn encode_value(writer: &mut CborWriter, value: &Value) {
    match value {
        Value::Null => {
            writer.array(1);
            writer.unsigned(0);
        }
        Value::Bool(value) => {
            writer.array(2);
            writer.unsigned(1);
            writer.boolean(*value);
        }
        Value::I64(value) => {
            writer.array(2);
            writer.unsigned(2);
            writer.signed(*value);
        }
        Value::String(value) => {
            writer.array(2);
            writer.unsigned(3);
            writer.text(value);
        }
        Value::List(items) => {
            writer.array(2);
            writer.unsigned(4);
            writer.array(items.len() as u64);
            for item in items {
                encode_value(writer, item);
            }
        }
        Value::Record(fields) => {
            writer.array(2);
            writer.unsigned(5);
            writer.map(fields.len() as u64);
            for (field, value) in fields {
                writer.bytes(field.as_bytes());
                encode_value(writer, value);
            }
        }
        Value::Variant {
            type_id,
            variant_id,
            payload,
        } => {
            writer.array(4);
            writer.unsigned(6);
            writer.bytes(type_id.as_bytes());
            writer.bytes(variant_id.as_bytes());
            encode_value(writer, payload);
        }
        Value::Entity(id) => {
            writer.array(2);
            writer.unsigned(7);
            writer.bytes(id.as_bytes());
        }
    }
}

pub(crate) fn decode_value(
    reader: &mut CborReader<'_>,
    limits: &DecodeLimits,
    depth: u32,
    nodes: &mut u64,
) -> Result<Value, DecodeError> {
    if depth > limits.max_value_depth {
        return Err(DecodeError::Limit("value depth"));
    }
    *nodes = nodes.saturating_add(1);
    if *nodes > limits.max_total_value_nodes {
        return Err(DecodeError::Limit("value nodes"));
    }
    let length = reader.array_len()?;
    let tag = reader.unsigned()?;
    match (tag, length) {
        (0, 1) => Ok(Value::Null),
        (1, 2) => Ok(Value::Bool(reader.boolean()?)),
        (2, 2) => Ok(Value::I64(reader.signed()?)),
        (3, 2) => {
            let text = reader.text(limits.max_string_bytes)?;
            Ok(Value::String(Arc::from(text)))
        }
        (4, 2) => {
            let length = reader.array_len()?;
            if length > limits.max_collection_items {
                return Err(DecodeError::Limit("list items"));
            }
            let capacity = usize::try_from(length).map_err(|_| DecodeError::LengthOverflow)?;
            let mut items = Vec::with_capacity(capacity);
            for _ in 0..length {
                items.push(decode_value(
                    reader,
                    limits,
                    depth.saturating_add(1),
                    nodes,
                )?);
            }
            Ok(Value::List(items))
        }
        (5, 2) => {
            let length = reader.map_len()?;
            if length > limits.max_collection_items {
                return Err(DecodeError::Limit("record fields"));
            }
            let mut fields = BTreeMap::new();
            let mut previous = None;
            for _ in 0..length {
                let raw = reader.bytes_exact::<16>()?;
                if previous.is_some_and(|prior| prior >= raw) {
                    return Err(DecodeError::NonCanonical("record key order"));
                }
                previous = Some(raw);
                let value = decode_value(reader, limits, depth.saturating_add(1), nodes)?;
                fields.insert(FieldId::from_bytes(raw), value);
            }
            Ok(Value::Record(fields))
        }
        (6, 4) => Ok(Value::Variant {
            type_id: TypeId::from_bytes(reader.bytes_exact::<16>()?),
            variant_id: VariantId::from_bytes(reader.bytes_exact::<16>()?),
            payload: Box::new(decode_value(
                reader,
                limits,
                depth.saturating_add(1),
                nodes,
            )?),
        }),
        (7, 2) => Ok(Value::Entity(EntityId::from_bytes(
            reader.bytes_exact::<16>()?,
        ))),
        _ => Err(DecodeError::Schema("value tag or field count")),
    }
}

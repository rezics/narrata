use thiserror::Error;

use crate::limits::DecodeLimits;

use super::Value;

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum ValueLimitError {
    #[error("value nesting exceeds limit")]
    Depth,
    #[error("string length exceeds limit")]
    StringBytes,
    #[error("collection size exceeds limit")]
    CollectionItems,
    #[error("total value node count exceeds limit")]
    TotalNodes,
}

pub fn validate_value(value: &Value, limits: &DecodeLimits) -> Result<(), ValueLimitError> {
    let mut nodes = 0_u64;
    let mut pending = vec![(value, 1_u32)];
    while let Some((current, depth)) = pending.pop() {
        if depth > limits.max_value_depth {
            return Err(ValueLimitError::Depth);
        }
        nodes = nodes.saturating_add(1);
        if nodes > limits.max_total_value_nodes {
            return Err(ValueLimitError::TotalNodes);
        }
        match current {
            Value::String(text) if text.len() as u64 > limits.max_string_bytes => {
                return Err(ValueLimitError::StringBytes);
            }
            Value::List(items) => {
                if items.len() as u64 > limits.max_collection_items {
                    return Err(ValueLimitError::CollectionItems);
                }
                pending.extend(items.iter().map(|item| (item, depth.saturating_add(1))));
            }
            Value::Record(fields) => {
                if fields.len() as u64 > limits.max_collection_items {
                    return Err(ValueLimitError::CollectionItems);
                }
                pending.extend(fields.values().map(|item| (item, depth.saturating_add(1))));
            }
            Value::Variant { payload, .. } => {
                pending.push((payload, depth.saturating_add(1)));
            }
            Value::Null | Value::Bool(_) | Value::I64(_) | Value::String(_) | Value::Entity(_) => {}
        }
    }
    Ok(())
}

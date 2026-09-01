use std::{collections::BTreeMap, sync::Arc};

use crate::identity::{EntityId, FieldId, TypeId, VariantId};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    I64(i64),
    String(Arc<str>),
    List(Vec<Value>),
    Record(BTreeMap<FieldId, Value>),
    Variant {
        type_id: TypeId,
        variant_id: VariantId,
        payload: Box<Value>,
    },
    Entity(EntityId),
}

impl Value {
    pub fn kind(&self) -> super::ValueKindV0 {
        match self {
            Self::Null => super::ValueKindV0::Null,
            Self::Bool(_) => super::ValueKindV0::Bool,
            Self::I64(_) => super::ValueKindV0::I64,
            Self::String(_) => super::ValueKindV0::String,
            Self::List(_) => super::ValueKindV0::List,
            Self::Record(_) => super::ValueKindV0::Record,
            Self::Variant { .. } => super::ValueKindV0::Variant,
            Self::Entity(_) => super::ValueKindV0::Entity,
        }
    }

    pub fn logical_units(&self) -> u64 {
        let mut total = 0_u64;
        let mut pending = vec![self];
        while let Some(value) = pending.pop() {
            total = total.saturating_add(1);
            match value {
                Self::String(text) => {
                    total = total.saturating_add(text.len() as u64);
                }
                Self::List(items) => {
                    total = total.saturating_add(items.len() as u64);
                    pending.extend(items);
                }
                Self::Record(fields) => {
                    total = total.saturating_add(fields.len() as u64);
                    pending.extend(fields.values());
                }
                Self::Variant { payload, .. } => pending.push(payload),
                Self::Null | Self::Bool(_) | Self::I64(_) | Self::Entity(_) => {}
            }
        }
        total
    }
}

impl From<&str> for Value {
    fn from(value: &str) -> Self {
        Self::String(Arc::from(value))
    }
}

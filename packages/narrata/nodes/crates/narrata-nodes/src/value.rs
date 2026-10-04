use narrata_kernel::content::ContentRef;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A state value. `text` is data that rules compare (decision 2); translatable names travel as
/// `ref`, which only supports equality.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(untagged)]
pub enum Scalar {
    Bool(bool),
    Int(i64),
    Text(String),
    Ref(ContentRef),
}

#[derive(
    Clone, Copy, Debug, Deserialize, Eq, JsonSchema, Ord, PartialEq, PartialOrd, Serialize,
)]
#[serde(rename_all = "snake_case")]
pub enum ScalarType {
    Bool,
    Int,
    Text,
    Ref,
}

impl Scalar {
    pub fn kind(&self) -> ScalarType {
        match self {
            Self::Bool(_) => ScalarType::Bool,
            Self::Int(_) => ScalarType::Int,
            Self::Text(_) => ScalarType::Text,
            Self::Ref(_) => ScalarType::Ref,
        }
    }

    pub fn view(&self) -> ViewScalar {
        match self {
            Self::Bool(value) => ViewScalar::Bool(*value),
            Self::Int(value) => ViewScalar::Int(value.to_string()),
            Self::Text(value) => ViewScalar::Text(value.clone()),
            Self::Ref(value) => ViewScalar::Ref(value.clone()),
        }
    }
}

/// Exchange form of a value. Integers are decimal strings so that 64-bit values survive
/// JavaScript; `ref` values are resolved by the host's content provider.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum ViewScalar {
    Bool(bool),
    Int(String),
    Text(String),
    Ref(ContentRef),
}

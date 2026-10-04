//! Node-stack identities (ADR 0013 §3). Authored IDs are 16 bytes and minted by tooling,
//! never by `compile`; object IDs are `object_id` digests of canonical payloads.

use std::borrow::Cow;

use narrata_kernel::{authored_id, derived_id};
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

authored_id!(NodeId, "node:");
authored_id!(ChoicePointId, "choice-point:");
authored_id!(OptionId, "option:");
authored_id!(ExecutionId, "execution:");

derived_id!(ArtifactId, "artifact:");
derived_id!(CommitId, "commit:");
derived_id!(ObjectId, "object:");

macro_rules! text_id {
    ($name:ident, $pattern:literal) => {
        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.collect_str(self)
            }
        }

        /// Only the canonical lower-case text is accepted, so one ID has one spelling.
        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let text = Cow::<str>::deserialize(deserializer)?;
                let id: Self = text.parse().map_err(serde::de::Error::custom)?;
                if id.to_string() != text {
                    return Err(serde::de::Error::custom(concat!(
                        "identity must be written as ",
                        $pattern
                    )));
                }
                Ok(id)
            }
        }

        impl JsonSchema for $name {
            fn schema_name() -> Cow<'static, str> {
                stringify!($name).into()
            }

            fn json_schema(_: &mut SchemaGenerator) -> Schema {
                json_schema!({"type": "string", "pattern": $pattern})
            }
        }
    };
}

text_id!(NodeId, "^node:[0-9a-f]{32}$");
text_id!(ChoicePointId, "^choice-point:[0-9a-f]{32}$");
text_id!(OptionId, "^option:[0-9a-f]{32}$");
text_id!(ExecutionId, "^execution:[0-9a-f]{32}$");
text_id!(ArtifactId, "^artifact:[0-9a-f]{64}$");
text_id!(CommitId, "^commit:[0-9a-f]{64}$");
text_id!(ObjectId, "^object:[0-9a-f]{64}$");

/// An authored identity of any of the three tombstoned kinds.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum AuthoredId {
    Node(NodeId),
    ChoicePoint(ChoicePointId),
    Option(OptionId),
}

impl std::fmt::Display for AuthoredId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Node(id) => id.fmt(formatter),
            Self::ChoicePoint(id) => id.fmt(formatter),
            Self::Option(id) => id.fmt(formatter),
        }
    }
}

impl std::str::FromStr for AuthoredId {
    type Err = narrata_kernel::identity::IdParseError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        if text.starts_with("node:") {
            text.parse().map(Self::Node)
        } else if text.starts_with("choice-point:") {
            text.parse().map(Self::ChoicePoint)
        } else {
            text.parse().map(Self::Option)
        }
    }
}

text_id!(AuthoredId, "^(node|choice-point|option):[0-9a-f]{32}$");

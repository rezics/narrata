//! JSON exchange form. Deserialization applies the same validation as the constructors.

use std::borrow::Cow;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::{AnchorId, ContentKey, ContentRef, ProviderId, Segment};

macro_rules! text_serde {
    ($name:ident, $schema:expr) => {
        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(self.as_str())
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let value = String::deserialize(deserializer)?;
                Self::new(value).map_err(serde::de::Error::custom)
            }
        }

        impl JsonSchema for $name {
            fn schema_name() -> Cow<'static, str> {
                stringify!($name).into()
            }

            fn json_schema(_: &mut SchemaGenerator) -> Schema {
                $schema
            }
        }
    };
}

// JSON Schema counts characters rather than bytes, so the byte limits stay a domain check.
text_serde!(
    ProviderId,
    json_schema!({"type": "string", "pattern": "^[a-z0-9-]{1,32}$"})
);
text_serde!(
    ContentKey,
    json_schema!({"type": "string", "minLength": 1, "maxLength": 256, "pattern": "^[^\\u0000-\\u001f\\u007f-\\u009f]+$"})
);
text_serde!(
    AnchorId,
    json_schema!({"type": "string", "pattern": "^[ -~]{1,128}$"})
);

#[derive(Deserialize, JsonSchema, Serialize)]
#[serde(rename = "ContentRef", deny_unknown_fields)]
struct ContentRefForm {
    provider: ProviderId,
    key: ContentKey,
}

#[derive(Deserialize, JsonSchema, Serialize)]
#[serde(rename = "Segment", deny_unknown_fields)]
struct SegmentForm {
    unit: ContentRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    first: Option<AnchorId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last: Option<AnchorId>,
}

impl Serialize for ContentRef {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        ContentRefForm {
            provider: self.provider.clone(),
            key: self.key.clone(),
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ContentRef {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let form = ContentRefForm::deserialize(deserializer)?;
        Ok(Self {
            provider: form.provider,
            key: form.key,
        })
    }
}

impl JsonSchema for ContentRef {
    fn schema_name() -> Cow<'static, str> {
        "ContentRef".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        ContentRefForm::json_schema(generator)
    }
}

impl Serialize for Segment {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        SegmentForm {
            unit: self.unit.clone(),
            first: self.first.clone(),
            last: self.last.clone(),
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Segment {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let form = SegmentForm::deserialize(deserializer)?;
        Ok(Self {
            unit: form.unit,
            first: form.first,
            last: form.last,
        })
    }
}

impl JsonSchema for Segment {
    fn schema_name() -> Cow<'static, str> {
        "Segment".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        SegmentForm::json_schema(generator)
    }
}

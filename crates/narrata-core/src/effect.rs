use std::{collections::BTreeMap, fmt, sync::Arc};

use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::{
    ActionId, CommitId, EffectId, EffectPayloadDigest, EffectRequestDigest, ExecutionId, FieldId,
    InputPayloadDigest, InstructionId, TypeId, Value, ValueKindV0, VariantId,
    codec::{CborWriter, digest_bytes},
    scene::ResumeSupport,
};

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum CapabilityIdError {
    #[error("capability ID must be 1..=64 lowercase ASCII letters, digits, '.', '_' or '-'")]
    Invalid,
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct CapabilityId(Arc<str>);

impl CapabilityId {
    pub fn new(value: impl AsRef<str>) -> Result<Self, CapabilityIdError> {
        let value = value.as_ref();
        if value.is_empty()
            || value.len() > 64
            || !value.bytes().all(|byte| {
                byte.is_ascii_lowercase()
                    || byte.is_ascii_digit()
                    || matches!(byte, b'.' | b'_' | b'-')
            })
        {
            return Err(CapabilityIdError::Invalid);
        }
        Ok(Self(Arc::from(value)))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for CapabilityId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct CapabilityVersion(u16);

impl CapabilityVersion {
    pub const fn new(value: u16) -> Option<Self> {
        if value == 0 { None } else { Some(Self(value)) }
    }

    pub const fn get(self) -> u16 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum CapabilityRequirement {
    Optional = 0,
    Required = 1,
}

impl CapabilityRequirement {
    pub(crate) const fn from_u64(value: u64) -> Option<Self> {
        match value {
            0 => Some(Self::Optional),
            1 => Some(Self::Required),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum DeliveryPolicy {
    Reconcile = 0,
    RecordedQuery = 1,
    AtLeastOnceIdempotent = 2,
    HostTransactional = 3,
}

impl DeliveryPolicy {
    pub(crate) const fn from_u64(value: u64) -> Option<Self> {
        match value {
            0 => Some(Self::Reconcile),
            1 => Some(Self::RecordedQuery),
            2 => Some(Self::AtLeastOnceIdempotent),
            3 => Some(Self::HostTransactional),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RewindPolicy {
    Reapply,
    ReuseRecordedResponse,
    Barrier,
    Compensatable { capability: CapabilityId },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ValueSchemaV0 {
    Any,
    Null,
    Bool,
    I64,
    String,
    Entity,
    List(Box<ValueSchemaV0>),
    Record(BTreeMap<FieldId, ValueSchemaV0>),
    Variant {
        type_id: TypeId,
        variants: BTreeMap<VariantId, ValueSchemaV0>,
    },
}

impl ValueSchemaV0 {
    pub fn accepts(&self, value: &Value) -> bool {
        match (self, value) {
            (Self::Any, _) | (Self::Null, Value::Null) | (Self::Bool, Value::Bool(_)) => true,
            (Self::I64, Value::I64(_))
            | (Self::String, Value::String(_))
            | (Self::Entity, Value::Entity(_)) => true,
            (Self::List(item), Value::List(items)) => items.iter().all(|value| item.accepts(value)),
            (Self::Record(schema), Value::Record(fields)) => {
                schema.len() == fields.len()
                    && schema.iter().all(|(id, expected)| {
                        fields.get(id).is_some_and(|value| expected.accepts(value))
                    })
            }
            (
                Self::Variant { type_id, variants },
                Value::Variant {
                    type_id: actual_type,
                    variant_id,
                    payload,
                },
            ) => {
                type_id == actual_type
                    && variants
                        .get(variant_id)
                        .is_some_and(|schema| schema.accepts(payload))
            }
            _ => false,
        }
    }

    pub fn static_kind(&self) -> Option<ValueKindV0> {
        match self {
            Self::Any => None,
            Self::Null => Some(ValueKindV0::Null),
            Self::Bool => Some(ValueKindV0::Bool),
            Self::I64 => Some(ValueKindV0::I64),
            Self::String => Some(ValueKindV0::String),
            Self::Entity => Some(ValueKindV0::Entity),
            Self::List(_) => Some(ValueKindV0::List),
            Self::Record(_) => Some(ValueKindV0::Record),
            Self::Variant { .. } => Some(ValueKindV0::Variant),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CapabilityDeclV0 {
    pub id: CapabilityId,
    pub version: CapabilityVersion,
    pub requirement: CapabilityRequirement,
    pub request_schema: ValueSchemaV0,
    pub response_schema: ValueSchemaV0,
    pub delivery: DeliveryPolicy,
    pub rewind: RewindPolicy,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostCapability {
    pub id: CapabilityId,
    pub version: CapabilityVersion,
    pub request_schema: Option<ValueSchemaV0>,
    pub response_schema: Option<ValueSchemaV0>,
    pub allowed_policies: Vec<(DeliveryPolicy, RewindPolicy)>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SceneResumeCapabilities {
    pub animation: ResumeSupport,
    pub audio: ResumeSupport,
}

impl Default for SceneResumeCapabilities {
    fn default() -> Self {
        Self {
            animation: ResumeSupport::BestEffort,
            audio: ResumeSupport::BestEffort,
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct HostCapabilities {
    capabilities: BTreeMap<CapabilityId, HostCapability>,
    scene_resume: SceneResumeCapabilities,
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum HostCapabilitiesError {
    #[error(transparent)]
    InvalidId(#[from] CapabilityIdError),
    #[error("host capability {capability} is declared more than once")]
    Duplicate { capability: CapabilityId },
}

impl HostCapabilities {
    pub fn new(
        capabilities: impl IntoIterator<Item = HostCapability>,
    ) -> Result<Self, HostCapabilitiesError> {
        let mut registry = BTreeMap::new();
        for capability in capabilities {
            let id = capability.id.clone();
            if registry.insert(id.clone(), capability).is_some() {
                return Err(HostCapabilitiesError::Duplicate { capability: id });
            }
        }
        Ok(Self {
            capabilities: registry,
            scene_resume: SceneResumeCapabilities::default(),
        })
    }

    pub fn get(&self, id: &CapabilityId) -> Option<&HostCapability> {
        self.capabilities.get(id)
    }

    pub fn with_scene_resume(mut self, animation: ResumeSupport, audio: ResumeSupport) -> Self {
        self.scene_resume = SceneResumeCapabilities { animation, audio };
        self
    }

    pub const fn scene_resume(&self) -> SceneResumeCapabilities {
        self.scene_resume
    }
}

pub fn builtin_host_capabilities() -> Result<HostCapabilities, HostCapabilitiesError> {
    let version = CapabilityVersion::new(1).ok_or(CapabilityIdError::Invalid)?;
    let mut capabilities = Vec::new();
    for id in ["visual.dialogue", "visual.choice", "visual.scene"] {
        capabilities.push(HostCapability {
            id: CapabilityId::new(id)?,
            version,
            request_schema: None,
            response_schema: Some(ValueSchemaV0::Null),
            allowed_policies: vec![(DeliveryPolicy::Reconcile, RewindPolicy::Reapply)],
        });
    }
    capabilities.push(HostCapability {
        id: CapabilityId::new("host.query")?,
        version,
        request_schema: None,
        response_schema: None,
        allowed_policies: vec![(
            DeliveryPolicy::RecordedQuery,
            RewindPolicy::ReuseRecordedResponse,
        )],
    });
    capabilities.push(HostCapability {
        id: CapabilityId::new("host.command")?,
        version,
        request_schema: None,
        response_schema: None,
        allowed_policies: vec![
            (DeliveryPolicy::AtLeastOnceIdempotent, RewindPolicy::Reapply),
            (DeliveryPolicy::AtLeastOnceIdempotent, RewindPolicy::Barrier),
            (DeliveryPolicy::HostTransactional, RewindPolicy::Reapply),
            (DeliveryPolicy::HostTransactional, RewindPolicy::Barrier),
        ],
    });
    HostCapabilities::new(capabilities)
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct NegotiatedCapabilities {
    capabilities: BTreeMap<CapabilityId, CapabilityDeclV0>,
    scene_resume: SceneResumeCapabilities,
}

impl NegotiatedCapabilities {
    pub fn get(&self, id: &CapabilityId) -> Option<&CapabilityDeclV0> {
        self.capabilities.get(id)
    }

    pub fn contains(&self, id: &CapabilityId) -> bool {
        self.capabilities.contains_key(id)
    }

    pub const fn scene_resume(&self) -> SceneResumeCapabilities {
        self.scene_resume
    }
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum CapabilityNegotiationError {
    #[error("required capability {capability}@{version} is missing")]
    MissingRequired {
        capability: CapabilityId,
        version: u16,
    },
    #[error("capability {capability} version is incompatible: program {required}, host {provided}")]
    VersionMismatch {
        capability: CapabilityId,
        required: u16,
        provided: u16,
    },
    #[error("capability {capability} schema is incompatible")]
    SchemaMismatch { capability: CapabilityId },
    #[error("host safety policy forbids the requested policy for {capability}")]
    PolicyForbidden { capability: CapabilityId },
}

pub fn negotiate_capabilities(
    declarations: &[CapabilityDeclV0],
    host: &HostCapabilities,
) -> Result<NegotiatedCapabilities, Vec<CapabilityNegotiationError>> {
    let mut negotiated = BTreeMap::new();
    let mut errors = Vec::new();
    for declaration in declarations {
        let Some(available) = host.get(&declaration.id) else {
            if declaration.requirement == CapabilityRequirement::Required {
                errors.push(CapabilityNegotiationError::MissingRequired {
                    capability: declaration.id.clone(),
                    version: declaration.version.get(),
                });
            }
            continue;
        };
        if available.version != declaration.version {
            errors.push(CapabilityNegotiationError::VersionMismatch {
                capability: declaration.id.clone(),
                required: declaration.version.get(),
                provided: available.version.get(),
            });
            continue;
        }
        if available
            .request_schema
            .as_ref()
            .is_some_and(|schema| schema != &declaration.request_schema)
            || available
                .response_schema
                .as_ref()
                .is_some_and(|schema| schema != &declaration.response_schema)
        {
            errors.push(CapabilityNegotiationError::SchemaMismatch {
                capability: declaration.id.clone(),
            });
            continue;
        }
        if !available.allowed_policies.iter().any(|(delivery, rewind)| {
            delivery == &declaration.delivery && rewind == &declaration.rewind
        }) {
            errors.push(CapabilityNegotiationError::PolicyForbidden {
                capability: declaration.id.clone(),
            });
            continue;
        }
        negotiated.insert(declaration.id.clone(), declaration.clone());
    }
    if errors.is_empty() {
        Ok(NegotiatedCapabilities {
            capabilities: negotiated,
            scene_resume: host.scene_resume(),
        })
    } else {
        Err(errors)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EffectRequestV0 {
    pub id: EffectId,
    pub execution: ExecutionId,
    pub capability: CapabilityId,
    pub capability_version: CapabilityVersion,
    pub payload: Value,
    pub payload_digest: EffectPayloadDigest,
    pub request_digest: EffectRequestDigest,
    pub delivery: DeliveryPolicy,
    pub rewind: RewindPolicy,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EffectResponseV0 {
    pub effect: EffectId,
    pub request_digest: EffectRequestDigest,
    pub capability: CapabilityId,
    pub capability_version: CapabilityVersion,
    pub payload: Value,
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum EffectResponseError {
    #[error("effect response does not match the pending Effect ID")]
    Effect,
    #[error("effect response does not match the complete request digest")]
    RequestDigest,
    #[error("effect response capability or version does not match")]
    Capability,
    #[error("effect response payload does not match the declared response schema")]
    ResponseSchema,
}

pub fn effect_payload_digest(payload: &Value) -> EffectPayloadDigest {
    let mut writer = CborWriter::new();
    crate::value::encode_value(&mut writer, payload);
    EffectPayloadDigest::from_bytes(digest_bytes("effect-payload", 0, &writer.into_bytes()))
}

pub fn effect_request_digest(
    capability: &CapabilityId,
    version: CapabilityVersion,
    payload_digest: EffectPayloadDigest,
    delivery: DeliveryPolicy,
    rewind: &RewindPolicy,
) -> EffectRequestDigest {
    let mut writer = CborWriter::new();
    writer.array(5);
    writer.text(capability.as_str());
    writer.unsigned(u64::from(version.get()));
    writer.bytes(payload_digest.as_bytes());
    writer.unsigned(delivery as u64);
    encode_rewind_policy(&mut writer, rewind);
    EffectRequestDigest::from_bytes(digest_bytes("effect-request", 0, &writer.into_bytes()))
}

#[allow(clippy::too_many_arguments)]
pub fn derive_effect_id(
    execution: ExecutionId,
    parent_commit: CommitId,
    input: InputPayloadDigest,
    instruction: InstructionId,
    occurrence: u64,
    request_digest: EffectRequestDigest,
) -> EffectId {
    let mut hasher = Sha256::new();
    hasher.update(b"narrata-effect\0");
    hasher.update(execution.as_bytes());
    hasher.update(parent_commit.as_bytes());
    hasher.update(input.as_bytes());
    hasher.update(instruction.as_bytes());
    hasher.update(occurrence.to_be_bytes());
    hasher.update(request_digest.as_bytes());
    let digest = hasher.finalize();
    let mut bytes = [0_u8; 32];
    bytes.copy_from_slice(&digest);
    EffectId::from_bytes(bytes)
}

#[allow(clippy::too_many_arguments)]
pub fn derive_statechart_effect_id(
    execution: ExecutionId,
    parent_commit: CommitId,
    input: InputPayloadDigest,
    action: ActionId,
    occurrence: u64,
    request_digest: EffectRequestDigest,
) -> EffectId {
    let mut hasher = Sha256::new();
    hasher.update(b"narrata-statechart-effect\0");
    hasher.update(execution.as_bytes());
    hasher.update(parent_commit.as_bytes());
    hasher.update(input.as_bytes());
    hasher.update(action.as_bytes());
    hasher.update(occurrence.to_be_bytes());
    hasher.update(request_digest.as_bytes());
    let digest = hasher.finalize();
    let mut bytes = [0_u8; 32];
    bytes.copy_from_slice(&digest);
    EffectId::from_bytes(bytes)
}

pub(crate) fn encode_rewind_policy(writer: &mut CborWriter, policy: &RewindPolicy) {
    match policy {
        RewindPolicy::Reapply => {
            writer.array(1);
            writer.unsigned(0);
        }
        RewindPolicy::ReuseRecordedResponse => {
            writer.array(1);
            writer.unsigned(1);
        }
        RewindPolicy::Barrier => {
            writer.array(1);
            writer.unsigned(2);
        }
        RewindPolicy::Compensatable { capability } => {
            writer.array(2);
            writer.unsigned(3);
            writer.text(capability.as_str());
        }
    }
}

use crate::{
    codec::{CborWriter, digest_bytes},
    effect::{EffectResponseError, EffectResponseV0},
    identity::{ChoiceId, InputId, InputPayloadDigest, InteractionId},
    runtime::PendingEffectV0,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RuntimeInputV0 {
    Start {
        request_id: InputId,
    },
    Advance {
        request_id: InputId,
        interaction_id: InteractionId,
    },
    Select {
        request_id: InputId,
        interaction_id: InteractionId,
        choice_id: ChoiceId,
    },
    EffectResponse {
        request_id: InputId,
        response: EffectResponseV0,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CheckedRuntimeInput(RuntimeInputV0);

impl CheckedRuntimeInput {
    pub fn start(request_id: InputId) -> Self {
        Self(RuntimeInputV0::Start { request_id })
    }

    pub fn advance(request_id: InputId, interaction_id: InteractionId) -> Self {
        Self(RuntimeInputV0::Advance {
            request_id,
            interaction_id,
        })
    }

    pub fn select(request_id: InputId, interaction_id: InteractionId, choice_id: ChoiceId) -> Self {
        Self(RuntimeInputV0::Select {
            request_id,
            interaction_id,
            choice_id,
        })
    }

    pub fn effect_response(
        request_id: InputId,
        pending: &PendingEffectV0,
        program: &crate::program::CheckedProgram,
        response: EffectResponseV0,
    ) -> Result<Self, EffectResponseError> {
        if response.effect != pending.request.id {
            return Err(EffectResponseError::Effect);
        }
        if response.request_digest != pending.request.request_digest {
            return Err(EffectResponseError::RequestDigest);
        }
        if response.capability != pending.request.capability
            || response.capability_version != pending.request.capability_version
        {
            return Err(EffectResponseError::Capability);
        }
        if !program
            .capability(&pending.request.capability)
            .is_some_and(|capability| capability.response_schema.accepts(&response.payload))
        {
            return Err(EffectResponseError::ResponseSchema);
        }
        Ok(Self(RuntimeInputV0::EffectResponse {
            request_id,
            response,
        }))
    }

    pub fn request_id(&self) -> InputId {
        match self.0 {
            RuntimeInputV0::Start { request_id }
            | RuntimeInputV0::Advance { request_id, .. }
            | RuntimeInputV0::Select { request_id, .. }
            | RuntimeInputV0::EffectResponse { request_id, .. } => request_id,
        }
    }

    pub fn as_v0(&self) -> &RuntimeInputV0 {
        &self.0
    }

    pub fn payload_digest(&self) -> InputPayloadDigest {
        input_payload_digest(self)
    }
}

pub(crate) fn input_payload_digest(input: &CheckedRuntimeInput) -> InputPayloadDigest {
    let mut writer = CborWriter::new();
    match input.as_v0() {
        RuntimeInputV0::Start { request_id } => {
            writer.array(2);
            writer.unsigned(0);
            writer.bytes(request_id.as_bytes());
        }
        RuntimeInputV0::Advance {
            request_id,
            interaction_id,
        } => {
            writer.array(3);
            writer.unsigned(1);
            writer.bytes(request_id.as_bytes());
            writer.bytes(interaction_id.as_bytes());
        }
        RuntimeInputV0::Select {
            request_id,
            interaction_id,
            choice_id,
        } => {
            writer.array(4);
            writer.unsigned(2);
            writer.bytes(request_id.as_bytes());
            writer.bytes(interaction_id.as_bytes());
            writer.bytes(choice_id.as_bytes());
        }
        RuntimeInputV0::EffectResponse {
            request_id,
            response,
        } => {
            writer.array(7);
            writer.unsigned(3);
            writer.bytes(request_id.as_bytes());
            writer.bytes(response.effect.as_bytes());
            writer.bytes(response.request_digest.as_bytes());
            writer.text(response.capability.as_str());
            writer.unsigned(u64::from(response.capability_version.get()));
            crate::value::encode_value(&mut writer, &response.payload);
        }
    }
    InputPayloadDigest::from_bytes(digest_bytes(
        "runtime-input-payload",
        0,
        &writer.into_bytes(),
    ))
}

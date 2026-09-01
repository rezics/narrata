use crate::{
    codec::{CborWriter, digest_bytes},
    identity::{ChoiceId, InputId, InputPayloadDigest, InteractionId},
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

    pub fn request_id(&self) -> InputId {
        match self.0 {
            RuntimeInputV0::Start { request_id }
            | RuntimeInputV0::Advance { request_id, .. }
            | RuntimeInputV0::Select { request_id, .. } => request_id,
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
    }
    InputPayloadDigest::from_bytes(digest_bytes(
        "runtime-input-payload",
        0,
        &writer.into_bytes(),
    ))
}

use std::sync::Arc;

use sha2::{Digest, Sha256};

use crate::{
    identity::{
        ChoiceId, ExecutionId, InputPayloadDigest, InstructionId, InteractionId, StateDigest,
    },
    value::Value,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingChoiceItemV0 {
    pub id: ChoiceId,
    pub label: Arc<str>,
    pub(crate) target: InstructionId,
}

impl PendingChoiceItemV0 {
    pub fn target_for_runtime(&self) -> InstructionId {
        self.target
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PendingInteractionV0 {
    Say {
        interaction_id: InteractionId,
        origin_instruction: InstructionId,
        origin_parent_state: StateDigest,
        origin_input_digest: InputPayloadDigest,
        occurrence: u64,
        speaker: Option<Arc<str>>,
        text: Arc<str>,
        resume_to: InstructionId,
    },
    Choice {
        interaction_id: InteractionId,
        origin_instruction: InstructionId,
        origin_parent_state: StateDigest,
        origin_input_digest: InputPayloadDigest,
        occurrence: u64,
        prompt: Option<Arc<str>>,
        offered: Vec<PendingChoiceItemV0>,
    },
}

impl PendingInteractionV0 {
    pub fn interaction_id(&self) -> InteractionId {
        match self {
            Self::Say { interaction_id, .. } | Self::Choice { interaction_id, .. } => {
                *interaction_id
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SayView {
    pub interaction_id: InteractionId,
    pub speaker: Option<Arc<str>>,
    pub text: Arc<str>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChoiceViewItem {
    pub id: ChoiceId,
    pub label: Arc<str>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChoiceView {
    pub interaction_id: InteractionId,
    pub prompt: Option<Arc<str>>,
    pub choices: Vec<ChoiceViewItem>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DraftResult {
    AwaitSay(SayView),
    AwaitChoice(ChoiceView),
    Finished(Value),
}

pub(crate) fn interaction_view(pending: &PendingInteractionV0) -> DraftResult {
    match pending {
        PendingInteractionV0::Say {
            interaction_id,
            speaker,
            text,
            ..
        } => DraftResult::AwaitSay(SayView {
            interaction_id: *interaction_id,
            speaker: speaker.clone(),
            text: text.clone(),
        }),
        PendingInteractionV0::Choice {
            interaction_id,
            prompt,
            offered,
            ..
        } => DraftResult::AwaitChoice(ChoiceView {
            interaction_id: *interaction_id,
            prompt: prompt.clone(),
            choices: offered
                .iter()
                .map(|choice| ChoiceViewItem {
                    id: choice.id,
                    label: choice.label.clone(),
                })
                .collect(),
        }),
    }
}

pub(crate) fn derive_interaction_id(
    execution: ExecutionId,
    parent: StateDigest,
    input: InputPayloadDigest,
    instruction: InstructionId,
    occurrence: u64,
    kind: u8,
) -> InteractionId {
    let mut hasher = Sha256::new();
    hasher.update(b"NARRATA-INTERACTION\0");
    hasher.update(execution.as_bytes());
    hasher.update(parent.as_bytes());
    hasher.update(input.as_bytes());
    hasher.update(instruction.as_bytes());
    hasher.update(occurrence.to_be_bytes());
    hasher.update([kind]);
    let result = hasher.finalize();
    let mut digest = [0_u8; 32];
    digest.copy_from_slice(&result);
    InteractionId::from_bytes(digest)
}

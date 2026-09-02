use std::sync::Arc;

use sha2::{Digest, Sha256};

use crate::{
    CommitId,
    effect::EffectRequestV0,
    identity::{
        ActionId, ChoiceId, EventTypeId, ExecutionId, GlobalId, InputPayloadDigest, InstructionId,
        InteractionId, StateDigest,
    },
    value::Value,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingEffectV0 {
    pub request: EffectRequestV0,
    pub path: EffectPathV0,
    pub origin_parent_commit: CommitId,
    pub origin_parent_state: StateDigest,
    pub origin_input_digest: InputPayloadDigest,
    pub occurrence: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EffectPathV0 {
    Flow {
        origin: InstructionId,
        resume_to: InstructionId,
    },
    Statechart {
        site: ActionId,
        response_to: Option<GlobalId>,
        response_event: Option<EventTypeId>,
    },
}

impl PendingEffectV0 {
    pub fn flow_origin(&self) -> Option<InstructionId> {
        match self.path {
            EffectPathV0::Flow { origin, .. } => Some(origin),
            EffectPathV0::Statechart { .. } => None,
        }
    }

    pub fn flow_continuation(&self) -> Option<InstructionId> {
        match self.path {
            EffectPathV0::Flow { resume_to, .. } => Some(resume_to),
            EffectPathV0::Statechart { .. } => None,
        }
    }

    pub fn statechart_continuation(&self) -> Option<(Option<GlobalId>, Option<EventTypeId>)> {
        match self.path {
            EffectPathV0::Statechart {
                response_to,
                response_event,
                ..
            } => Some((response_to, response_event)),
            EffectPathV0::Flow { .. } => None,
        }
    }

    pub fn statechart_site(&self) -> Option<ActionId> {
        match self.path {
            EffectPathV0::Statechart { site, .. } => Some(site),
            EffectPathV0::Flow { .. } => None,
        }
    }
}

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
    AwaitEffect(EffectRequestV0),
    Finished(Value),
    StatechartStable(crate::statechart::StatechartView),
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

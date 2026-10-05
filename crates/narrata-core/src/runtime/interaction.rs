use std::sync::Arc;

use sha2::{Digest, Sha256};

use crate::{
    CommitId,
    effect::EffectRequestV0,
    identity::{
        ActionId, ChoiceId, EventTypeId, ExecutionId, GlobalId, InputPayloadDigest, InstructionId,
        InteractionId, StateDigest,
    },
    program::{CheckedProgram, ContentEntryV1, ContentIndex, ContentRef, Segment},
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

/// A presentation operand as a pending interaction records it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PendingContent {
    /// Snapshot schema 0: the reader text copied from a format 0 Program.
    LegacyText(Arc<str>),
    /// Snapshot schema 1: a format 1 content-table index (ADR 0018).
    Content(ContentIndex),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingChoiceItemV0 {
    pub id: ChoiceId,
    pub label: PendingContent,
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
        speaker: Option<PendingContent>,
        text: PendingContent,
        resume_to: InstructionId,
    },
    Choice {
        interaction_id: InteractionId,
        origin_instruction: InstructionId,
        origin_parent_state: StateDigest,
        origin_input_digest: InputPayloadDigest,
        occurrence: u64,
        prompt: Option<PendingContent>,
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

    pub fn occurrence(&self) -> u64 {
        match self {
            Self::Say { occurrence, .. } | Self::Choice { occurrence, .. } => *occurrence,
        }
    }

    /// The presentation operands in order: speaker and text, or prompt and labels.
    pub fn contents(&self) -> impl Iterator<Item = &PendingContent> {
        let (first, second, labels) = match self {
            Self::Say { speaker, text, .. } => (speaker.as_ref(), Some(text), &[][..]),
            Self::Choice {
                prompt, offered, ..
            } => (prompt.as_ref(), None, offered.as_slice()),
        };
        first
            .into_iter()
            .chain(second)
            .chain(labels.iter().map(|item| &item.label))
    }
}

/// What the host presents for one operand. Format 1 Programs give content references that the
/// host resolves; only format 0 Programs still carry text.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ContentView {
    Ref(ContentRef),
    Segment(Segment),
    LegacyText(Arc<str>),
}

/// `occurrence` with the Execution and the Commit forms the presentation key
/// `(ExecutionId, CommitId, occurrence)` of everything this interaction presents.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SayView {
    pub interaction_id: InteractionId,
    pub occurrence: u64,
    pub speaker: Option<ContentView>,
    pub text: ContentView,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChoiceViewItem {
    pub id: ChoiceId,
    pub label: ContentView,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChoiceView {
    pub interaction_id: InteractionId,
    pub occurrence: u64,
    pub prompt: Option<ContentView>,
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

/// The view of a pending interaction. Content indices resolve against `program`; `None` means
/// the interaction names an entry `program` does not have.
pub fn pending_view(
    program: &CheckedProgram,
    pending: &PendingInteractionV0,
) -> Option<DraftResult> {
    let view = |content: &PendingContent| match content {
        PendingContent::LegacyText(text) => Some(ContentView::LegacyText(text.clone())),
        PendingContent::Content(index) => match program.content(*index)? {
            ContentEntryV1::Ref(reference) => Some(ContentView::Ref(reference.clone())),
            ContentEntryV1::Segment(segment) => Some(ContentView::Segment(segment.clone())),
        },
    };
    Some(match pending {
        PendingInteractionV0::Say {
            interaction_id,
            occurrence,
            speaker,
            text,
            ..
        } => DraftResult::AwaitSay(SayView {
            interaction_id: *interaction_id,
            occurrence: *occurrence,
            speaker: match speaker {
                Some(speaker) => Some(view(speaker)?),
                None => None,
            },
            text: view(text)?,
        }),
        PendingInteractionV0::Choice {
            interaction_id,
            occurrence,
            prompt,
            offered,
            ..
        } => DraftResult::AwaitChoice(ChoiceView {
            interaction_id: *interaction_id,
            occurrence: *occurrence,
            prompt: match prompt {
                Some(prompt) => Some(view(prompt)?),
                None => None,
            },
            choices: offered
                .iter()
                .map(|choice| {
                    Some(ChoiceViewItem {
                        id: choice.id,
                        label: view(&choice.label)?,
                    })
                })
                .collect::<Option<_>>()?,
        }),
    })
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

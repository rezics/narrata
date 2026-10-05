use std::{collections::BTreeMap, path::Path, str::FromStr, sync::Arc};

use narrata_core::{
    ChoiceId, ExecutionId, InputId,
    limits::{MacrostepLimits, ProgramLoadLimits},
    program::{encode_program_artifact, load_program},
    runtime::{
        CheckedRuntimeInput, DraftResult, PendingInteractionV0, RuntimeStateV0, SliceBudget,
        TransitionDraft, new_execution,
    },
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    backend::{BackendError, ConformanceBackend, NativeBackend},
    generator::{branch_call_choice_v0, branch_call_choice_v1, hello_v0, hello_v1},
};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ConformanceFixture {
    pub schema: u32,
    pub story: String,
    pub execution_id: String,
    pub inputs: Vec<FixtureInput>,
    #[serde(default)]
    pub expected_state_digests: Vec<String>,
    #[serde(default)]
    pub expected_receipt_digests: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FixtureInput {
    Start {
        request_id: String,
    },
    Advance {
        request_id: String,
    },
    Select {
        request_id: String,
        choice_id: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConformanceManifest {
    pub state_digests: Vec<String>,
    pub receipt_digests: Vec<String>,
}

#[derive(Debug, Error)]
pub enum FixtureError {
    #[error("cannot read fixture: {0}")]
    Io(#[from] std::io::Error),
    #[error("cannot parse fixture JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid identity: {0}")]
    Identity(String),
    #[error("unknown built-in story '{0}'")]
    UnknownStory(String),
    #[error("Program is invalid: {0}")]
    Program(String),
    #[error("Runtime initialization failed: {0}")]
    Init(String),
    #[error(transparent)]
    Backend(#[from] BackendError),
    #[error("fixture input does not match current pending interaction")]
    InputState,
    #[error("conformance mismatch at {field}[{index}]: expected {expected}, actual {actual}")]
    Mismatch {
        field: &'static str,
        index: usize,
        expected: String,
        actual: String,
    },
    #[error("InputId conflict in ephemeral session")]
    InputConflict,
}

pub fn load_fixture(path: &Path) -> Result<ConformanceFixture, FixtureError> {
    Ok(serde_json::from_slice(&std::fs::read(path)?)?)
}

pub fn run_fixture(fixture: &ConformanceFixture) -> Result<ConformanceManifest, FixtureError> {
    run_fixture_with_backend(fixture, &NativeBackend, SliceBudget::unlimited())
}

pub fn run_fixture_with_backend(
    fixture: &ConformanceFixture,
    backend: &impl ConformanceBackend,
    slice: SliceBudget,
) -> Result<ConformanceManifest, FixtureError> {
    let artifact = match fixture.story.as_str() {
        "hello-v0" => hello_v0(),
        "branch-call-choice-v0" => branch_call_choice_v0(),
        "hello-v1" => hello_v1(),
        "branch-call-choice-v1" => branch_call_choice_v1(),
        other => return Err(FixtureError::UnknownStory(other.to_owned())),
    };
    let bytes = encode_program_artifact(&artifact);
    let program = load_program(&bytes, &ProgramLoadLimits::default())
        .map_err(|error| FixtureError::Program(error.to_string()))?;
    let execution = ExecutionId::from_str(&fixture.execution_id)
        .map_err(|error| FixtureError::Identity(error.to_string()))?;
    let mut state = Arc::new(
        new_execution(&program, execution)
            .map_err(|error| FixtureError::Init(error.to_string()))?,
    );
    let mut session = EphemeralSession::default();
    let mut manifest = ConformanceManifest {
        state_digests: Vec::new(),
        receipt_digests: Vec::new(),
    };
    for fixture_input in &fixture.inputs {
        let input = resolve_input(fixture_input, &state)?;
        let draft = session.apply(
            backend,
            program.clone(),
            state.clone(),
            input,
            MacrostepLimits::default(),
            slice,
        )?;
        manifest
            .state_digests
            .push(draft.next_state_digest().to_string());
        manifest
            .receipt_digests
            .push(draft.receipt_digest().to_string());
        state = Arc::new(draft.next_state().clone());
    }
    assert_expected(
        "state_digests",
        &fixture.expected_state_digests,
        &manifest.state_digests,
    )?;
    assert_expected(
        "receipt_digests",
        &fixture.expected_receipt_digests,
        &manifest.receipt_digests,
    )?;
    Ok(manifest)
}

fn resolve_input(
    fixture: &FixtureInput,
    state: &RuntimeStateV0,
) -> Result<CheckedRuntimeInput, FixtureError> {
    match fixture {
        FixtureInput::Start { request_id } => Ok(CheckedRuntimeInput::start(parse_id(request_id)?)),
        FixtureInput::Advance { request_id } => {
            let interaction = state
                .pending()
                .map(PendingInteractionV0::interaction_id)
                .ok_or(FixtureError::InputState)?;
            Ok(CheckedRuntimeInput::advance(
                parse_id(request_id)?,
                interaction,
            ))
        }
        FixtureInput::Select {
            request_id,
            choice_id,
        } => {
            let pending = state.pending().ok_or(FixtureError::InputState)?;
            let PendingInteractionV0::Choice {
                interaction_id,
                offered,
                ..
            } = pending
            else {
                return Err(FixtureError::InputState);
            };
            let choice = ChoiceId::from_str(choice_id)
                .map_err(|error| FixtureError::Identity(error.to_string()))?;
            if !offered.iter().any(|item| item.id == choice) {
                return Err(FixtureError::InputState);
            }
            Ok(CheckedRuntimeInput::select(
                parse_id(request_id)?,
                *interaction_id,
                choice,
            ))
        }
    }
}

fn parse_id(value: &str) -> Result<InputId, FixtureError> {
    InputId::from_str(value).map_err(|error| FixtureError::Identity(error.to_string()))
}

fn assert_expected(
    field: &'static str,
    expected: &[String],
    actual: &[String],
) -> Result<(), FixtureError> {
    if expected.is_empty() {
        return Ok(());
    }
    for index in 0..expected.len().max(actual.len()) {
        if expected.get(index) != actual.get(index) {
            return Err(FixtureError::Mismatch {
                field,
                index,
                expected: expected
                    .get(index)
                    .cloned()
                    .unwrap_or_else(|| "<missing>".to_owned()),
                actual: actual
                    .get(index)
                    .cloned()
                    .unwrap_or_else(|| "<missing>".to_owned()),
            });
        }
    }
    Ok(())
}

#[derive(Clone, Debug, Default)]
pub struct EphemeralSession {
    completed: BTreeMap<InputId, (narrata_core::InputPayloadDigest, TransitionDraft)>,
}

impl EphemeralSession {
    pub fn apply(
        &mut self,
        backend: &impl ConformanceBackend,
        program: Arc<narrata_core::CheckedProgram>,
        state: Arc<RuntimeStateV0>,
        input: CheckedRuntimeInput,
        limits: MacrostepLimits,
        slice: SliceBudget,
    ) -> Result<TransitionDraft, FixtureError> {
        let request_id = input.request_id();
        let input_digest = input.payload_digest();
        if let Some((digest, draft)) = self.completed.get(&request_id) {
            if *digest == input_digest {
                return Ok(draft.clone());
            }
            return Err(FixtureError::InputConflict);
        }
        let draft = backend.transition(program, state, input, limits, slice)?;
        self.completed
            .insert(request_id, (input_digest, draft.clone()));
        Ok(draft)
    }
}

pub fn result_name(result: &DraftResult) -> &'static str {
    match result {
        DraftResult::AwaitSay(_) => "say",
        DraftResult::AwaitChoice(_) => "choice",
        DraftResult::AwaitEffect(_) => "effect",
        DraftResult::Finished(_) => "finished",
        DraftResult::StatechartStable(_) => "statechart-stable",
    }
}

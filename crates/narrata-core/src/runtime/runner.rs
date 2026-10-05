use std::{collections::BTreeMap, sync::Arc};

use thiserror::Error;

use crate::{
    CommitId,
    codec::digest_bytes,
    effect::{
        derive_effect_id, derive_statechart_effect_id, effect_payload_digest, effect_request_digest,
    },
    identity::{InputId, InputPayloadDigest, InstructionId, ReceiptDigest, StateDigest},
    limits::MacrostepLimits,
    program::{
        BinaryOpV0, CheckedProgram, ContentOperand, OpV0, ReturnModeV0, SlotRefV0, UnaryOpV0,
    },
    statechart::{
        ChartRunV0, InvokedFlowV0, StatechartStepOutcome, StatechartTraceEventV0,
        StatechartTraceKindV0, StatechartView, begin_chart, begin_chart_effect_response,
        execute_chart_step, finish_invoked_flow,
    },
    value::Value,
};

use super::{
    CheckedRuntimeInput, DraftResult, FrameStateV0, PendingChoiceItemV0, PendingContent,
    PendingEffectV0, PendingInteractionV0, ReceiptResultKindV0, RuntimeInputV0, RuntimeStateV0,
    RuntimeStatusV0, SliceBudget, TraceEventV0, TransitionReceiptV0, Turn, VmStateV0,
    derive_interaction_id, input_payload_digest, pending_view,
    receipt::{ReceiptMetrics, encode_receipt_payload, receipt_v0},
};

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum TransitionStartError {
    #[error("Runtime State belongs to a different Program Artifact")]
    ProgramMismatch,
    #[error("Runtime State uses incompatible semantics")]
    SemanticsMismatch,
    #[error("input is not valid for the current Runtime status: {0}")]
    InvalidInput(&'static str),
    #[error("Runtime turn counter overflow")]
    TurnOverflow,
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum RuntimeFault {
    #[error("macrostep instruction hard limit exceeded")]
    InstructionLimit,
    #[error("macrostep call hard limit exceeded")]
    CallLimit,
    #[error("macrostep logical allocation hard limit exceeded")]
    LogicalAllocationLimit,
    #[error("runtime call-depth limit exceeded")]
    CallDepthLimit,
    #[error("runtime evaluation stack-depth limit exceeded")]
    StackDepthLimit,
    #[error("runtime live-value limit exceeded")]
    LiveValueLimit,
    #[error("checked Program reference or Runtime State is inconsistent")]
    InvalidState,
    #[error("runtime operand kind mismatch")]
    KindMismatch,
    #[error("deterministic integer arithmetic fault")]
    Arithmetic,
    #[error("Choice offers no visible items")]
    NoVisibleChoices,
    #[error("Effect execution requires a persistent parent Commit identity")]
    MissingParentCommit,
    #[error("Effect payload does not match the negotiated capability schema")]
    EffectSchemaMismatch,
    #[error("Statechart microstep hard limit exceeded")]
    MicrostepLimit,
    #[error("Statechart internal event hard limit exceeded")]
    InternalEventLimit,
    #[error("Statechart runtime-state hard limit exceeded")]
    StatechartStateLimit,
    #[error("parallel Statechart transitions write the same global")]
    ParallelWriteConflict,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SliceProgress {
    pub instruction_count: u64,
    pub call_count: u64,
    pub logical_alloc_units: u64,
    pub microstep_count: u64,
    pub internal_event_count: u64,
    pub last_trace: Option<TraceEventV0>,
}

#[derive(Clone, Debug)]
pub enum SliceOutcome {
    Yielded {
        runner: TransitionRunner,
        progress: SliceProgress,
    },
    Completed(TransitionDraft),
    Faulted(RuntimeFault),
}

#[derive(Clone, Debug)]
pub struct TransitionDraft {
    parent: StateDigest,
    input_id: InputId,
    input_payload_digest: InputPayloadDigest,
    next_state: RuntimeStateV0,
    next_state_digest: StateDigest,
    receipt: TransitionReceiptV0,
    receipt_digest: ReceiptDigest,
    result: DraftResult,
    trace: Vec<TraceEventV0>,
    statechart_trace: Vec<StatechartTraceEventV0>,
}

impl TransitionDraft {
    pub fn parent(&self) -> StateDigest {
        self.parent
    }

    pub fn input_id(&self) -> InputId {
        self.input_id
    }

    pub fn input_payload_digest(&self) -> InputPayloadDigest {
        self.input_payload_digest
    }

    pub fn next_state(&self) -> &RuntimeStateV0 {
        &self.next_state
    }

    pub fn next_state_digest(&self) -> StateDigest {
        self.next_state_digest
    }

    pub fn receipt(&self) -> &TransitionReceiptV0 {
        &self.receipt
    }

    pub fn receipt_digest(&self) -> ReceiptDigest {
        self.receipt_digest
    }

    pub fn result(&self) -> &DraftResult {
        &self.result
    }

    pub fn trace(&self) -> &[TraceEventV0] {
        &self.trace
    }

    pub fn statechart_trace(&self) -> &[StatechartTraceEventV0] {
        &self.statechart_trace
    }

    pub fn into_next_state(self) -> RuntimeStateV0 {
        self.next_state
    }
}

#[derive(Clone, Debug)]
struct WorkingStateV0 {
    state: RuntimeStateV0,
}

#[derive(Clone, Debug)]
pub struct TransitionRunner {
    program: Arc<CheckedProgram>,
    parent_digest: StateDigest,
    parent_commit: Option<CommitId>,
    input: CheckedRuntimeInput,
    input_digest: InputPayloadDigest,
    limits: MacrostepLimits,
    working: WorkingStateV0,
    instruction_count: u64,
    call_count: u64,
    logical_alloc_units: u64,
    trace: Vec<TraceEventV0>,
    statechart_trace: Vec<StatechartTraceEventV0>,
    chart_run: Option<ChartRunV0>,
    suspended_chart: Option<ChartRunV0>,
    microstep_count: u64,
    internal_event_count: u64,
}

enum BeginMode {
    Flow(VmStateV0),
    Chart(ChartRunV0),
}

enum StepOutcome {
    Continue,
    SafePoint,
}

impl TransitionRunner {
    pub(crate) fn begin(
        program: Arc<CheckedProgram>,
        parent: Arc<RuntimeStateV0>,
        input: CheckedRuntimeInput,
        limits: MacrostepLimits,
        parent_commit: Option<CommitId>,
    ) -> Result<Self, TransitionStartError> {
        if parent.program_artifact_id != program.artifact_id() {
            return Err(TransitionStartError::ProgramMismatch);
        }
        if parent.semantics_version != program.artifact().semantics_version {
            return Err(TransitionStartError::SemanticsMismatch);
        }
        let parent_digest = crate::snapshot::state_digest(&parent);
        let input_digest = input_payload_digest(&input);
        let next_turn = parent
            .turn
            .0
            .checked_add(1)
            .ok_or(TransitionStartError::TurnOverflow)?;
        let mut state = parent.as_ref().clone();
        let mode =
            if parent.statechart.is_some() {
                match (&parent.status, input.as_v0()) {
                    (RuntimeStatusV0::StatechartStable, RuntimeInputV0::Start { .. })
                        if parent.turn == Turn(0) =>
                    {
                        BeginMode::Chart(begin_chart(&state, None))
                    }
                    (RuntimeStatusV0::StatechartStable, RuntimeInputV0::Event { event, .. })
                        if program
                            .statechart()
                            .is_some_and(|chart| chart.events.binary_search(event).is_ok()) =>
                    {
                        BeginMode::Chart(begin_chart(&state, Some(*event)))
                    }
                    (
                        RuntimeStatusV0::AwaitingStatechartEffect { pending },
                        RuntimeInputV0::EffectResponse { response, .. },
                    ) if response.effect == pending.request.id
                        && response.request_digest == pending.request.request_digest
                        && response.capability == pending.request.capability
                        && response.capability_version == pending.request.capability_version
                        && program.capability(&pending.request.capability).is_some_and(
                            |capability| capability.response_schema.accepts(&response.payload),
                        ) =>
                    {
                        let run = begin_chart_effect_response(&mut state, pending, response)
                            .map_err(|_| {
                                TransitionStartError::InvalidInput(
                                    "Statechart Effect response violates its continuation",
                                )
                            })?;
                        BeginMode::Chart(run)
                    }
                    (
                        RuntimeStatusV0::Awaiting {
                            vm,
                            pending:
                                PendingInteractionV0::Say {
                                    interaction_id,
                                    resume_to,
                                    ..
                                },
                        },
                        RuntimeInputV0::Advance {
                            interaction_id: supplied,
                            ..
                        },
                    ) if interaction_id == supplied => {
                        let mut vm = vm.clone();
                        let frame = vm
                            .frames
                            .last_mut()
                            .ok_or(TransitionStartError::InvalidInput("missing active frame"))?;
                        frame.instruction = *resume_to;
                        BeginMode::Flow(vm)
                    }
                    (
                        RuntimeStatusV0::Awaiting {
                            vm,
                            pending:
                                PendingInteractionV0::Choice {
                                    interaction_id,
                                    offered,
                                    ..
                                },
                        },
                        RuntimeInputV0::Select {
                            interaction_id: supplied,
                            choice_id,
                            ..
                        },
                    ) if interaction_id == supplied => {
                        let target = offered
                            .iter()
                            .find(|choice| choice.id == *choice_id)
                            .map(PendingChoiceItemV0::target_for_runtime)
                            .ok_or(TransitionStartError::InvalidInput(
                                "ChoiceId was not offered",
                            ))?;
                        let mut vm = vm.clone();
                        let frame = vm
                            .frames
                            .last_mut()
                            .ok_or(TransitionStartError::InvalidInput("missing active frame"))?;
                        frame.instruction = target;
                        BeginMode::Flow(vm)
                    }
                    (
                        RuntimeStatusV0::AwaitingEffect { vm, pending },
                        RuntimeInputV0::EffectResponse { response, .. },
                    ) if response.effect == pending.request.id
                        && response.request_digest == pending.request.request_digest
                        && response.capability == pending.request.capability
                        && response.capability_version == pending.request.capability_version
                        && program.capability(&pending.request.capability).is_some_and(
                            |capability| capability.response_schema.accepts(&response.payload),
                        ) =>
                    {
                        let mut vm = vm.clone();
                        let frame = vm
                            .frames
                            .last_mut()
                            .ok_or(TransitionStartError::InvalidInput("missing active frame"))?;
                        if frame.evaluation_stack.len() as u64 >= limits.runtime.max_stack_depth {
                            return Err(TransitionStartError::InvalidInput(
                                "Effect response exceeds stack limit",
                            ));
                        }
                        frame.instruction = pending.flow_continuation().ok_or(
                            TransitionStartError::InvalidInput("Effect is not a Flow continuation"),
                        )?;
                        frame.evaluation_stack.push(response.payload.clone());
                        BeginMode::Flow(vm)
                    }
                    (RuntimeStatusV0::StatechartFinished, _) => {
                        return Err(TransitionStartError::InvalidInput(
                            "finished Statechart accepts no input",
                        ));
                    }
                    _ => {
                        return Err(TransitionStartError::InvalidInput(
                            "input kind does not match Statechart safe point",
                        ));
                    }
                }
            } else {
                let ready_vm = match (&parent.status, input.as_v0()) {
                    (RuntimeStatusV0::Ready { vm }, RuntimeInputV0::Start { .. })
                        if parent.turn == Turn(0) =>
                    {
                        vm.clone()
                    }
                    (
                        RuntimeStatusV0::Awaiting {
                            vm,
                            pending:
                                PendingInteractionV0::Say {
                                    interaction_id,
                                    resume_to,
                                    ..
                                },
                        },
                        RuntimeInputV0::Advance {
                            interaction_id: supplied,
                            ..
                        },
                    ) if interaction_id == supplied => {
                        let mut vm = vm.clone();
                        let frame = vm
                            .frames
                            .last_mut()
                            .ok_or(TransitionStartError::InvalidInput("missing active frame"))?;
                        frame.instruction = *resume_to;
                        vm
                    }
                    (
                        RuntimeStatusV0::AwaitingEffect { vm, pending },
                        RuntimeInputV0::EffectResponse { response, .. },
                    ) if response.effect == pending.request.id
                        && response.request_digest == pending.request.request_digest
                        && response.capability == pending.request.capability
                        && response.capability_version == pending.request.capability_version
                        && program.capability(&pending.request.capability).is_some_and(
                            |capability| capability.response_schema.accepts(&response.payload),
                        ) =>
                    {
                        let mut vm = vm.clone();
                        let frame = vm
                            .frames
                            .last_mut()
                            .ok_or(TransitionStartError::InvalidInput("missing active frame"))?;
                        if frame.evaluation_stack.len() as u64 >= limits.runtime.max_stack_depth {
                            return Err(TransitionStartError::InvalidInput(
                                "Effect response exceeds stack limit",
                            ));
                        }
                        frame.instruction = pending.flow_continuation().ok_or(
                            TransitionStartError::InvalidInput("Effect is not a Flow continuation"),
                        )?;
                        frame.evaluation_stack.push(response.payload.clone());
                        vm
                    }
                    (
                        RuntimeStatusV0::Awaiting {
                            vm,
                            pending:
                                PendingInteractionV0::Choice {
                                    interaction_id,
                                    offered,
                                    ..
                                },
                        },
                        RuntimeInputV0::Select {
                            interaction_id: supplied,
                            choice_id,
                            ..
                        },
                    ) if interaction_id == supplied => {
                        let target = offered
                            .iter()
                            .find(|choice| choice.id == *choice_id)
                            .map(PendingChoiceItemV0::target_for_runtime)
                            .ok_or(TransitionStartError::InvalidInput(
                                "ChoiceId was not offered",
                            ))?;
                        let mut vm = vm.clone();
                        let frame = vm
                            .frames
                            .last_mut()
                            .ok_or(TransitionStartError::InvalidInput("missing active frame"))?;
                        frame.instruction = target;
                        vm
                    }
                    (RuntimeStatusV0::Finished { .. }, _) => {
                        return Err(TransitionStartError::InvalidInput(
                            "finished execution accepts no input",
                        ));
                    }
                    _ => {
                        return Err(TransitionStartError::InvalidInput(
                            "input kind or InteractionId does not match pending state",
                        ));
                    }
                };
                BeginMode::Flow(ready_vm)
            };
        state.turn = Turn(next_turn);
        let chart_run = match mode {
            BeginMode::Flow(vm) => {
                state.status = RuntimeStatusV0::Ready { vm };
                None
            }
            BeginMode::Chart(run) => {
                state.status = RuntimeStatusV0::StatechartStable;
                Some(run)
            }
        };
        Ok(Self {
            program,
            parent_digest,
            parent_commit,
            input,
            input_digest,
            limits,
            working: WorkingStateV0 { state },
            instruction_count: 0,
            call_count: 0,
            logical_alloc_units: 0,
            trace: Vec::new(),
            statechart_trace: Vec::new(),
            chart_run,
            suspended_chart: None,
            microstep_count: 0,
            internal_event_count: 0,
        })
    }

    pub fn run_slice(mut self, budget: SliceBudget) -> SliceOutcome {
        let mut remaining = budget.instructions();
        loop {
            if remaining.is_some_and(|value| value == 0) {
                let progress = self.progress();
                return SliceOutcome::Yielded {
                    runner: self,
                    progress,
                };
            }
            if let Some(value) = &mut remaining {
                *value = value.saturating_sub(1);
            }
            let step = if self.chart_run.is_some() {
                self.execute_chart_one()
            } else {
                if self.instruction_count >= self.limits.max_instructions {
                    return SliceOutcome::Faulted(RuntimeFault::InstructionLimit);
                }
                self.instruction_count = self.instruction_count.saturating_add(1);
                self.execute_one()
            };
            match step {
                Ok(StepOutcome::Continue) => {}
                Ok(StepOutcome::SafePoint) => return self.complete(),
                Err(error) => return SliceOutcome::Faulted(error),
            }
            if let Err(error) = self.check_live_limits() {
                return SliceOutcome::Faulted(error);
            }
        }
    }

    pub fn progress(&self) -> SliceProgress {
        SliceProgress {
            instruction_count: self.instruction_count,
            call_count: self.call_count,
            logical_alloc_units: self.logical_alloc_units,
            microstep_count: self.microstep_count,
            internal_event_count: self.internal_event_count,
            last_trace: self.trace.last().cloned(),
        }
    }

    fn execute_chart_one(&mut self) -> Result<StepOutcome, RuntimeFault> {
        let mut run = self.chart_run.take().ok_or(RuntimeFault::InvalidState)?;
        let outcome = execute_chart_step(
            &self.program,
            &mut self.working.state,
            &mut run,
            &self.limits,
            &mut self.statechart_trace,
        )?;
        self.microstep_count = run.microstep_count;
        self.internal_event_count = run.internal_event_count;
        match outcome {
            StatechartStepOutcome::Continue => {
                self.chart_run = Some(run);
                Ok(StepOutcome::Continue)
            }
            StatechartStepOutcome::StartFlow {
                site,
                flow,
                result_to,
                done_event,
            } => {
                self.start_invoked_flow(site, flow, result_to, done_event)?;
                self.suspended_chart = Some(run);
                Ok(StepOutcome::Continue)
            }
            StatechartStepOutcome::AwaitEffect {
                site,
                capability,
                payload,
                response_to,
                response_event,
            } => {
                self.statechart_effect(site, &capability, payload, response_to, response_event)?;
                self.statechart_trace.push(StatechartTraceEventV0 {
                    microstep: run.microstep_count,
                    kind: StatechartTraceKindV0::Await,
                    event: None,
                    state: None,
                    transition: None,
                    action: Some(site),
                });
                Ok(StepOutcome::SafePoint)
            }
            StatechartStepOutcome::Stable => {
                self.working.state.status = RuntimeStatusV0::StatechartStable;
                Ok(StepOutcome::SafePoint)
            }
            StatechartStepOutcome::Finished => {
                self.working.state.status = RuntimeStatusV0::StatechartFinished;
                Ok(StepOutcome::SafePoint)
            }
        }
    }

    fn start_invoked_flow(
        &mut self,
        site: crate::ActionId,
        flow_id: crate::FlowId,
        result_to: Option<crate::GlobalId>,
        done_event: Option<crate::EventTypeId>,
    ) -> Result<(), RuntimeFault> {
        let flow = self
            .program
            .flow(flow_id)
            .cloned()
            .ok_or(RuntimeFault::InvalidState)?;
        if !flow.parameters.is_empty() || flow.return_kind.is_some() != result_to.is_some() {
            return Err(RuntimeFault::InvalidState);
        }
        let mut locals = BTreeMap::new();
        for local in &flow.locals {
            self.charge(&local.default)?;
            locals.insert(local.id, local.default.clone());
        }
        let chart = self
            .working
            .state
            .statechart
            .as_mut()
            .ok_or(RuntimeFault::InvalidState)?;
        if chart.invocation.is_some() {
            return Err(RuntimeFault::InvalidState);
        }
        chart.invocation = Some(InvokedFlowV0 {
            site,
            flow: flow_id,
            result_to,
            done_event,
            deferred_events: Default::default(),
        });
        self.working.state.status = RuntimeStatusV0::Ready {
            vm: VmStateV0 {
                frames: vec![FrameStateV0 {
                    flow: flow_id,
                    instruction: flow.entry,
                    return_to: None,
                    locals,
                    evaluation_stack: Vec::new(),
                }],
            },
        };
        self.statechart_trace.push(StatechartTraceEventV0 {
            microstep: self.microstep_count,
            kind: StatechartTraceKindV0::Invoke,
            event: None,
            state: None,
            transition: None,
            action: Some(site),
        });
        Ok(())
    }

    fn statechart_effect(
        &mut self,
        site: crate::ActionId,
        capability: &crate::CapabilityId,
        payload: Value,
        response_to: Option<crate::GlobalId>,
        response_event: Option<crate::EventTypeId>,
    ) -> Result<(), RuntimeFault> {
        let declaration = self
            .program
            .capability(capability)
            .cloned()
            .ok_or(RuntimeFault::InvalidState)?;
        if !declaration.request_schema.accepts(&payload) {
            return Err(RuntimeFault::EffectSchemaMismatch);
        }
        let parent_commit = self
            .parent_commit
            .ok_or(RuntimeFault::MissingParentCommit)?;
        let payload_digest = effect_payload_digest(&payload);
        let request_digest = effect_request_digest(
            &declaration.id,
            declaration.version,
            payload_digest,
            declaration.delivery,
            &declaration.rewind,
        );
        let occurrence = self.working.state.interaction_counter;
        let id = derive_statechart_effect_id(
            self.working.state.execution_id,
            parent_commit,
            self.input_digest,
            site,
            occurrence,
            request_digest,
        );
        self.working.state.interaction_counter = occurrence
            .checked_add(1)
            .ok_or(RuntimeFault::InvalidState)?;
        let request = crate::EffectRequestV0 {
            id,
            execution: self.working.state.execution_id,
            capability: declaration.id,
            capability_version: declaration.version,
            payload,
            payload_digest,
            request_digest,
            delivery: declaration.delivery,
            rewind: declaration.rewind,
        };
        self.working.state.status = RuntimeStatusV0::AwaitingStatechartEffect {
            pending: PendingEffectV0 {
                request,
                path: super::EffectPathV0::Statechart {
                    site,
                    response_to,
                    response_event,
                },
                origin_parent_commit: parent_commit,
                origin_parent_state: self.parent_digest,
                origin_input_digest: self.input_digest,
                occurrence,
            },
        };
        Ok(())
    }

    fn execute_one(&mut self) -> Result<StepOutcome, RuntimeFault> {
        let (flow_id, instruction_id, frame_depth, stack_depth) = {
            let vm = self.ready_vm()?;
            let frame = vm.frames.last().ok_or(RuntimeFault::InvalidState)?;
            (
                frame.flow,
                frame.instruction,
                vm.frames.len(),
                frame.evaluation_stack.len(),
            )
        };
        let instruction = self
            .program
            .instruction(flow_id, instruction_id)
            .cloned()
            .ok_or(RuntimeFault::InvalidState)?;
        self.trace.push(TraceEventV0 {
            turn: self.working.state.turn.0,
            flow: flow_id,
            instruction: instruction_id,
            opcode: instruction.op.kind_name(),
            frame_depth,
            stack_depth,
            status: "running",
            state_digest: None,
            receipt_digest: None,
        });
        match instruction.op {
            OpV0::Const { constant, next } => {
                let value = self
                    .program
                    .constant(constant)
                    .cloned()
                    .ok_or(RuntimeFault::InvalidState)?;
                self.charge(&value)?;
                self.push(value)?;
                self.top_frame_mut()?.instruction = next;
            }
            OpV0::Load { slot, next } => {
                let value = self.load_slot(slot)?.clone();
                self.charge(&value)?;
                self.push(value)?;
                self.top_frame_mut()?.instruction = next;
            }
            OpV0::Store { slot, next } => {
                let value = self.pop()?;
                self.store_slot(slot, value)?;
                self.top_frame_mut()?.instruction = next;
            }
            OpV0::Unary { op, next } => {
                let value = self.pop()?;
                let result = match (op, value) {
                    (UnaryOpV0::Not, Value::Bool(value)) => Value::Bool(!value),
                    (UnaryOpV0::Neg, Value::I64(value)) => {
                        Value::I64(value.checked_neg().ok_or(RuntimeFault::Arithmetic)?)
                    }
                    _ => return Err(RuntimeFault::KindMismatch),
                };
                self.push(result)?;
                self.top_frame_mut()?.instruction = next;
            }
            OpV0::Binary { op, next } => {
                let right = self.pop()?;
                let left = self.pop()?;
                let result = evaluate_binary(op, left, right)?;
                self.push(result)?;
                self.top_frame_mut()?.instruction = next;
            }
            OpV0::Jump { target } => self.top_frame_mut()?.instruction = target,
            OpV0::JumpIfFalse { if_true, if_false } => {
                let condition = match self.pop()? {
                    Value::Bool(value) => value,
                    _ => return Err(RuntimeFault::KindMismatch),
                };
                self.top_frame_mut()?.instruction = if condition { if_true } else { if_false };
            }
            OpV0::Call {
                flow,
                argument_count,
                return_to,
            } => self.call(flow, argument_count, return_to)?,
            OpV0::Return { value } => self.return_from_flow(value)?,
            OpV0::Say {
                speaker,
                text,
                next,
            } => {
                self.say(instruction_id, speaker, text, next)?;
                return Ok(StepOutcome::SafePoint);
            }
            OpV0::Choice { prompt, choices } => {
                self.choice(instruction_id, prompt, &choices)?;
                return Ok(StepOutcome::SafePoint);
            }
            OpV0::Effect { capability, next } => {
                self.effect(instruction_id, &capability, next)?;
                return Ok(StepOutcome::SafePoint);
            }
            OpV0::ReconcileScene { target, next } => {
                self.require_safe_stacks()?;
                self.working.state.scene = Some(target);
                self.top_frame_mut()?.instruction = next;
            }
            OpV0::Raise { event, next } => {
                crate::statechart::queue_invoked_flow_event(&mut self.working.state, event)?;
                self.top_frame_mut()?.instruction = next;
            }
            OpV0::Finish { value } => {
                self.finish(value)?;
                return Ok(StepOutcome::SafePoint);
            }
        }
        Ok(StepOutcome::Continue)
    }

    fn ready_vm(&self) -> Result<&VmStateV0, RuntimeFault> {
        match &self.working.state.status {
            RuntimeStatusV0::Ready { vm } => Ok(vm),
            RuntimeStatusV0::Awaiting { .. }
            | RuntimeStatusV0::AwaitingEffect { .. }
            | RuntimeStatusV0::Finished { .. }
            | RuntimeStatusV0::StatechartStable
            | RuntimeStatusV0::AwaitingStatechartEffect { .. }
            | RuntimeStatusV0::StatechartFinished => Err(RuntimeFault::InvalidState),
        }
    }

    fn ready_vm_mut(&mut self) -> Result<&mut VmStateV0, RuntimeFault> {
        match &mut self.working.state.status {
            RuntimeStatusV0::Ready { vm } => Ok(vm),
            RuntimeStatusV0::Awaiting { .. }
            | RuntimeStatusV0::AwaitingEffect { .. }
            | RuntimeStatusV0::Finished { .. }
            | RuntimeStatusV0::StatechartStable
            | RuntimeStatusV0::AwaitingStatechartEffect { .. }
            | RuntimeStatusV0::StatechartFinished => Err(RuntimeFault::InvalidState),
        }
    }

    fn top_frame_mut(&mut self) -> Result<&mut FrameStateV0, RuntimeFault> {
        self.ready_vm_mut()?
            .frames
            .last_mut()
            .ok_or(RuntimeFault::InvalidState)
    }

    fn push(&mut self, value: Value) -> Result<(), RuntimeFault> {
        let maximum = self.limits.runtime.max_stack_depth;
        let frame = self.top_frame_mut()?;
        if frame.evaluation_stack.len() as u64 >= maximum {
            return Err(RuntimeFault::StackDepthLimit);
        }
        frame.evaluation_stack.push(value);
        Ok(())
    }

    fn pop(&mut self) -> Result<Value, RuntimeFault> {
        self.top_frame_mut()?
            .evaluation_stack
            .pop()
            .ok_or(RuntimeFault::InvalidState)
    }

    fn load_slot(&self, slot: SlotRefV0) -> Result<&Value, RuntimeFault> {
        match slot {
            SlotRefV0::Global(id) => self
                .working
                .state
                .globals
                .get(&id)
                .ok_or(RuntimeFault::InvalidState),
            SlotRefV0::Local(id) => self
                .ready_vm()?
                .frames
                .last()
                .and_then(|frame| frame.locals.get(&id))
                .ok_or(RuntimeFault::InvalidState),
        }
    }

    fn store_slot(&mut self, slot: SlotRefV0, value: Value) -> Result<(), RuntimeFault> {
        match slot {
            SlotRefV0::Global(id) => {
                let target = self
                    .working
                    .state
                    .globals
                    .get_mut(&id)
                    .ok_or(RuntimeFault::InvalidState)?;
                *target = value;
            }
            SlotRefV0::Local(id) => {
                let target = self
                    .top_frame_mut()?
                    .locals
                    .get_mut(&id)
                    .ok_or(RuntimeFault::InvalidState)?;
                *target = value;
            }
        }
        Ok(())
    }

    fn call(
        &mut self,
        flow_id: crate::identity::FlowId,
        argument_count: u16,
        return_to: InstructionId,
    ) -> Result<(), RuntimeFault> {
        if self.call_count >= self.limits.max_calls {
            return Err(RuntimeFault::CallLimit);
        }
        if self.ready_vm()?.frames.len() as u64 >= self.limits.runtime.max_call_depth {
            return Err(RuntimeFault::CallDepthLimit);
        }
        let callee = self
            .program
            .flow(flow_id)
            .cloned()
            .ok_or(RuntimeFault::InvalidState)?;
        if callee.parameters.len() != usize::from(argument_count) {
            return Err(RuntimeFault::InvalidState);
        }
        let mut arguments = Vec::with_capacity(usize::from(argument_count));
        for _ in 0..argument_count {
            arguments.push(self.pop()?);
        }
        arguments.reverse();
        let mut locals = BTreeMap::new();
        for (parameter, argument) in callee.parameters.iter().zip(arguments) {
            locals.insert(parameter.id, argument);
        }
        for local in &callee.locals {
            self.charge(&local.default)?;
            locals.insert(local.id, local.default.clone());
        }
        self.call_count = self.call_count.saturating_add(1);
        self.ready_vm_mut()?.frames.push(FrameStateV0 {
            flow: flow_id,
            instruction: callee.entry,
            return_to: Some(return_to),
            locals,
            evaluation_stack: Vec::new(),
        });
        Ok(())
    }

    fn return_from_flow(&mut self, mode: ReturnModeV0) -> Result<(), RuntimeFault> {
        let result = match mode {
            ReturnModeV0::None => None,
            ReturnModeV0::Stack => Some(self.pop()?),
        };
        if !self.top_frame_mut()?.evaluation_stack.is_empty() {
            return Err(RuntimeFault::InvalidState);
        }
        let is_invocation_root = self
            .working
            .state
            .statechart
            .as_ref()
            .and_then(|chart| chart.invocation.as_ref())
            .is_some()
            && self.ready_vm()?.frames.len() == 1;
        if is_invocation_root {
            let frame = self
                .ready_vm_mut()?
                .frames
                .pop()
                .ok_or(RuntimeFault::InvalidState)?;
            if frame.return_to.is_some() {
                return Err(RuntimeFault::InvalidState);
            }
            let run = match self.suspended_chart.take() {
                Some(run) => {
                    finish_invoked_flow(&mut self.working.state, result)?;
                    run
                }
                None => finish_invoked_flow(&mut self.working.state, result)?,
            };
            self.working.state.status = RuntimeStatusV0::StatechartStable;
            self.microstep_count = run.microstep_count;
            self.internal_event_count = run.internal_event_count;
            self.chart_run = Some(run);
            self.statechart_trace.push(StatechartTraceEventV0 {
                microstep: self.microstep_count,
                kind: StatechartTraceKindV0::Resume,
                event: None,
                state: None,
                transition: None,
                action: None,
            });
            return Ok(());
        }
        let frame = self
            .ready_vm_mut()?
            .frames
            .pop()
            .ok_or(RuntimeFault::InvalidState)?;
        let return_to = frame.return_to.ok_or(RuntimeFault::InvalidState)?;
        self.top_frame_mut()?.instruction = return_to;
        if let Some(value) = result {
            self.push(value)?;
        }
        Ok(())
    }

    fn say(
        &mut self,
        origin_instruction: InstructionId,
        speaker: Option<ContentOperand>,
        text: ContentOperand,
        resume_to: InstructionId,
    ) -> Result<(), RuntimeFault> {
        self.require_safe_stacks()?;
        let speaker = speaker.map(|operand| self.present(operand)).transpose()?;
        let text = self.present(text)?;
        let occurrence = self.working.state.interaction_counter;
        let interaction_id = derive_interaction_id(
            self.working.state.execution_id,
            self.parent_digest,
            self.input_digest,
            origin_instruction,
            occurrence,
            0,
        );
        self.working.state.interaction_counter = occurrence
            .checked_add(1)
            .ok_or(RuntimeFault::InvalidState)?;
        let vm = self.ready_vm()?.clone();
        self.working.state.status = RuntimeStatusV0::Awaiting {
            vm,
            pending: PendingInteractionV0::Say {
                interaction_id,
                origin_instruction,
                origin_parent_state: self.parent_digest,
                origin_input_digest: self.input_digest,
                occurrence,
                speaker,
                text,
                resume_to,
            },
        };
        Ok(())
    }

    fn choice(
        &mut self,
        origin_instruction: InstructionId,
        prompt: Option<ContentOperand>,
        choices: &[crate::program::ChoiceArmV0],
    ) -> Result<(), RuntimeFault> {
        self.require_safe_stacks()?;
        let flow = self
            .ready_vm()?
            .frames
            .last()
            .map(|frame| frame.flow)
            .ok_or(RuntimeFault::InvalidState)?;
        let prompt = prompt.map(|operand| self.present(operand)).transpose()?;
        let mut offered = Vec::new();
        for choice in choices {
            let visible = match choice.visible_if {
                Some(slot) => match self.load_slot(slot)? {
                    Value::Bool(value) => *value,
                    _ => return Err(RuntimeFault::KindMismatch),
                },
                None => true,
            };
            if visible {
                let label = self.present(choice.label)?;
                offered.push(PendingChoiceItemV0 {
                    id: choice.id,
                    label,
                    target: choice.target,
                });
            }
        }
        if offered.is_empty() {
            return Err(RuntimeFault::NoVisibleChoices);
        }
        if self.program.flow(flow).is_none() {
            return Err(RuntimeFault::InvalidState);
        }
        let occurrence = self.working.state.interaction_counter;
        let interaction_id = derive_interaction_id(
            self.working.state.execution_id,
            self.parent_digest,
            self.input_digest,
            origin_instruction,
            occurrence,
            1,
        );
        self.working.state.interaction_counter = occurrence
            .checked_add(1)
            .ok_or(RuntimeFault::InvalidState)?;
        let vm = self.ready_vm()?.clone();
        self.working.state.status = RuntimeStatusV0::Awaiting {
            vm,
            pending: PendingInteractionV0::Choice {
                interaction_id,
                origin_instruction,
                origin_parent_state: self.parent_digest,
                origin_input_digest: self.input_digest,
                occurrence,
                prompt,
                offered,
            },
        };
        Ok(())
    }

    fn finish(&mut self, mode: ReturnModeV0) -> Result<(), RuntimeFault> {
        let result = match mode {
            ReturnModeV0::None => Value::Null,
            ReturnModeV0::Stack => self.pop()?,
        };
        self.require_safe_stacks()?;
        let final_frames = self.ready_vm()?.frames.clone();
        self.working.state.status = RuntimeStatusV0::Finished {
            result,
            final_frames,
        };
        Ok(())
    }

    fn effect(
        &mut self,
        origin_instruction: InstructionId,
        capability: &crate::CapabilityId,
        resume_to: InstructionId,
    ) -> Result<(), RuntimeFault> {
        let payload = self.pop()?;
        self.require_safe_stacks()?;
        let declaration = self
            .program
            .capability(capability)
            .cloned()
            .ok_or(RuntimeFault::InvalidState)?;
        if !declaration.request_schema.accepts(&payload) {
            return Err(RuntimeFault::EffectSchemaMismatch);
        }
        let parent_commit = self
            .parent_commit
            .ok_or(RuntimeFault::MissingParentCommit)?;
        let payload_digest = effect_payload_digest(&payload);
        let request_digest = effect_request_digest(
            &declaration.id,
            declaration.version,
            payload_digest,
            declaration.delivery,
            &declaration.rewind,
        );
        let occurrence = self.working.state.interaction_counter;
        let id = derive_effect_id(
            self.working.state.execution_id,
            parent_commit,
            self.input_digest,
            origin_instruction,
            occurrence,
            request_digest,
        );
        self.working.state.interaction_counter = occurrence
            .checked_add(1)
            .ok_or(RuntimeFault::InvalidState)?;
        let request = crate::EffectRequestV0 {
            id,
            execution: self.working.state.execution_id,
            capability: declaration.id,
            capability_version: declaration.version,
            payload,
            payload_digest,
            request_digest,
            delivery: declaration.delivery,
            rewind: declaration.rewind,
        };
        let vm = self.ready_vm()?.clone();
        self.working.state.status = RuntimeStatusV0::AwaitingEffect {
            vm,
            pending: PendingEffectV0 {
                request,
                path: super::EffectPathV0::Flow {
                    origin: origin_instruction,
                    resume_to,
                },
                origin_parent_commit: parent_commit,
                origin_parent_state: self.parent_digest,
                origin_input_digest: self.input_digest,
                occurrence,
            },
        };
        Ok(())
    }

    /// Records a presentation operand and charges for it: format 0 copies the text and pays
    /// for its bytes; format 1 records the content index for one unit, so counters in the
    /// Receipt never depend on text (ADR 0018).
    fn present(&mut self, operand: ContentOperand) -> Result<PendingContent, RuntimeFault> {
        match operand {
            ContentOperand::Constant(index) => match self.program.constant(index) {
                Some(Value::String(text)) => {
                    let text = text.clone();
                    self.charge_units(1_u64.saturating_add(text.len() as u64))?;
                    Ok(PendingContent::LegacyText(text))
                }
                _ => Err(RuntimeFault::InvalidState),
            },
            ContentOperand::Content(index) => {
                self.program
                    .content(index)
                    .ok_or(RuntimeFault::InvalidState)?;
                self.charge_units(1)?;
                Ok(PendingContent::Content(index))
            }
        }
    }

    fn require_safe_stacks(&self) -> Result<(), RuntimeFault> {
        if self
            .ready_vm()?
            .frames
            .iter()
            .all(|frame| frame.evaluation_stack.is_empty())
        {
            Ok(())
        } else {
            Err(RuntimeFault::InvalidState)
        }
    }

    fn charge(&mut self, value: &Value) -> Result<(), RuntimeFault> {
        self.charge_units(value.logical_units())
    }

    fn charge_units(&mut self, units: u64) -> Result<(), RuntimeFault> {
        self.logical_alloc_units = self
            .logical_alloc_units
            .checked_add(units)
            .ok_or(RuntimeFault::LogicalAllocationLimit)?;
        if self.logical_alloc_units > self.limits.max_logical_alloc_units {
            Err(RuntimeFault::LogicalAllocationLimit)
        } else {
            Ok(())
        }
    }

    fn check_live_limits(&self) -> Result<(), RuntimeFault> {
        let mut total = self
            .working
            .state
            .globals
            .values()
            .map(Value::logical_units)
            .sum::<u64>();
        if let Some(vm) = self.working.state.vm() {
            for frame in &vm.frames {
                total = total
                    .saturating_add(frame.locals.values().map(Value::logical_units).sum::<u64>());
                total = total.saturating_add(
                    frame
                        .evaluation_stack
                        .iter()
                        .map(Value::logical_units)
                        .sum::<u64>(),
                );
            }
        }
        if let Some(chart) = &self.working.state.statechart {
            let history_entries = chart.history.values().map(Vec::len).sum::<usize>();
            let internal_events = chart
                .internal_queue
                .len()
                .saturating_add(chart.deferred_events.len())
                .saturating_add(
                    chart
                        .invocation
                        .as_ref()
                        .map_or(0, |invocation| invocation.deferred_events.len()),
                );
            if chart.active.len() as u64 > self.limits.runtime.max_active_states
                || history_entries as u64 > self.limits.runtime.max_history_entries
                || chart.deferred_work.len() as u64 > self.limits.runtime.max_deferred_work
            {
                return Err(RuntimeFault::StatechartStateLimit);
            }
            if internal_events as u64 > self.limits.runtime.max_internal_events {
                return Err(RuntimeFault::InternalEventLimit);
            }
            total = total.saturating_add(
                chart
                    .deferred_work
                    .iter()
                    .filter_map(|work| match work {
                        crate::statechart::DeferredStatechartWorkV0::Effect { payload, .. } => {
                            Some(payload.logical_units())
                        }
                        crate::statechart::DeferredStatechartWorkV0::Flow { .. } => None,
                    })
                    .sum::<u64>(),
            );
        }
        if let Some(pending) = self.working.state.pending_effect() {
            total = total.saturating_add(pending.request.payload.logical_units());
        }
        if total > self.limits.runtime.max_total_live_values {
            Err(RuntimeFault::LiveValueLimit)
        } else {
            Ok(())
        }
    }

    fn complete(mut self) -> SliceOutcome {
        if let Err(error) = self.check_live_limits() {
            return SliceOutcome::Faulted(error);
        }
        let next_state_digest = crate::snapshot::state_digest(&self.working.state);
        let (result, result_kind) = match &self.working.state.status {
            RuntimeStatusV0::Awaiting { pending, .. } => {
                let Some(result) = pending_view(&self.program, pending) else {
                    return SliceOutcome::Faulted(RuntimeFault::InvalidState);
                };
                let kind = match result {
                    DraftResult::AwaitSay(_) => ReceiptResultKindV0::Say,
                    DraftResult::AwaitChoice(_) => ReceiptResultKindV0::Choice,
                    DraftResult::AwaitEffect(_) => ReceiptResultKindV0::Effect,
                    DraftResult::Finished(_) => ReceiptResultKindV0::Finished,
                    DraftResult::StatechartStable(_) => ReceiptResultKindV0::StatechartStable,
                };
                (result, kind)
            }
            RuntimeStatusV0::AwaitingEffect { pending, .. } => (
                DraftResult::AwaitEffect(pending.request.clone()),
                ReceiptResultKindV0::Effect,
            ),
            RuntimeStatusV0::AwaitingStatechartEffect { pending } => (
                DraftResult::AwaitEffect(pending.request.clone()),
                ReceiptResultKindV0::Effect,
            ),
            RuntimeStatusV0::StatechartStable => {
                let view = self
                    .working
                    .state
                    .statechart
                    .as_ref()
                    .map(StatechartView::from)
                    .ok_or(RuntimeFault::InvalidState);
                let Ok(view) = view else {
                    return SliceOutcome::Faulted(RuntimeFault::InvalidState);
                };
                (
                    DraftResult::StatechartStable(view),
                    ReceiptResultKindV0::StatechartStable,
                )
            }
            RuntimeStatusV0::StatechartFinished => (
                DraftResult::Finished(Value::Null),
                ReceiptResultKindV0::Finished,
            ),
            RuntimeStatusV0::Finished { result, .. } => (
                DraftResult::Finished(result.clone()),
                ReceiptResultKindV0::Finished,
            ),
            RuntimeStatusV0::Ready { .. } => {
                return SliceOutcome::Faulted(RuntimeFault::InvalidState);
            }
        };
        let receipt = receipt_v0(
            self.input.request_id(),
            self.input_digest,
            self.parent_digest,
            next_state_digest,
            result_kind,
            ReceiptMetrics {
                instruction_count: self.instruction_count,
                call_count: self.call_count,
                logical_alloc_units: self.logical_alloc_units,
                microstep_count: self.microstep_count,
                internal_event_count: self.internal_event_count,
            },
        );
        let receipt_digest = ReceiptDigest::from_bytes(digest_bytes(
            "transition-receipt",
            0,
            &encode_receipt_payload(&receipt),
        ));
        if let Some(last) = self.trace.last_mut() {
            last.status = match result_kind {
                ReceiptResultKindV0::Say => "awaiting-say",
                ReceiptResultKindV0::Choice => "awaiting-choice",
                ReceiptResultKindV0::Finished => "finished",
                ReceiptResultKindV0::Effect => "awaiting-effect",
                ReceiptResultKindV0::StatechartStable => "statechart-stable",
            };
            last.state_digest = Some(next_state_digest);
            last.receipt_digest = Some(receipt_digest);
        }
        SliceOutcome::Completed(TransitionDraft {
            parent: self.parent_digest,
            input_id: self.input.request_id(),
            input_payload_digest: self.input_digest,
            next_state: self.working.state,
            next_state_digest,
            receipt,
            receipt_digest,
            result,
            trace: self.trace,
            statechart_trace: self.statechart_trace,
        })
    }
}

fn evaluate_binary(op: BinaryOpV0, left: Value, right: Value) -> Result<Value, RuntimeFault> {
    match op {
        BinaryOpV0::Eq => Ok(Value::Bool(left == right)),
        BinaryOpV0::Ne => Ok(Value::Bool(left != right)),
        BinaryOpV0::Add
        | BinaryOpV0::Sub
        | BinaryOpV0::Mul
        | BinaryOpV0::Div
        | BinaryOpV0::Rem
        | BinaryOpV0::Lt
        | BinaryOpV0::Le
        | BinaryOpV0::Gt
        | BinaryOpV0::Ge => {
            let (Value::I64(left), Value::I64(right)) = (left, right) else {
                return Err(RuntimeFault::KindMismatch);
            };
            match op {
                BinaryOpV0::Add => left.checked_add(right).map(Value::I64),
                BinaryOpV0::Sub => left.checked_sub(right).map(Value::I64),
                BinaryOpV0::Mul => left.checked_mul(right).map(Value::I64),
                BinaryOpV0::Div => left.checked_div(right).map(Value::I64),
                BinaryOpV0::Rem => left.checked_rem(right).map(Value::I64),
                BinaryOpV0::Lt => Some(Value::Bool(left < right)),
                BinaryOpV0::Le => Some(Value::Bool(left <= right)),
                BinaryOpV0::Gt => Some(Value::Bool(left > right)),
                BinaryOpV0::Ge => Some(Value::Bool(left >= right)),
                BinaryOpV0::Eq | BinaryOpV0::Ne => None,
            }
            .ok_or(RuntimeFault::Arithmetic)
        }
    }
}

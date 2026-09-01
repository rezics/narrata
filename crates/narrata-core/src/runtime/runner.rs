use std::{collections::BTreeMap, sync::Arc};

use thiserror::Error;

use crate::{
    codec::digest_bytes,
    identity::{InputId, InputPayloadDigest, InstructionId, ReceiptDigest, StateDigest},
    limits::MacrostepLimits,
    program::{BinaryOpV0, CheckedProgram, OpV0, ReturnModeV0, SlotRefV0, UnaryOpV0},
    value::Value,
};

use super::{
    CheckedRuntimeInput, DraftResult, FrameStateV0, PendingChoiceItemV0, PendingInteractionV0,
    ReceiptResultKindV0, RuntimeInputV0, RuntimeStateV0, RuntimeStatusV0, SliceBudget,
    TraceEventV0, TransitionReceiptV0, Turn, VmStateV0, derive_interaction_id,
    input_payload_digest, interaction_view,
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
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SliceProgress {
    pub instruction_count: u64,
    pub call_count: u64,
    pub logical_alloc_units: u64,
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
    input: CheckedRuntimeInput,
    input_digest: InputPayloadDigest,
    limits: MacrostepLimits,
    working: WorkingStateV0,
    instruction_count: u64,
    call_count: u64,
    logical_alloc_units: u64,
    trace: Vec<TraceEventV0>,
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
        state.turn = Turn(next_turn);
        state.status = RuntimeStatusV0::Ready { vm: ready_vm };
        Ok(Self {
            program,
            parent_digest,
            input,
            input_digest,
            limits,
            working: WorkingStateV0 { state },
            instruction_count: 0,
            call_count: 0,
            logical_alloc_units: 0,
            trace: Vec::new(),
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
            if self.instruction_count >= self.limits.max_instructions {
                return SliceOutcome::Faulted(RuntimeFault::InstructionLimit);
            }
            self.instruction_count = self.instruction_count.saturating_add(1);
            if let Some(value) = &mut remaining {
                *value = value.saturating_sub(1);
            }
            match self.execute_one() {
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
            last_trace: self.trace.last().cloned(),
        }
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
            RuntimeStatusV0::Awaiting { .. } | RuntimeStatusV0::Finished { .. } => {
                Err(RuntimeFault::InvalidState)
            }
        }
    }

    fn ready_vm_mut(&mut self) -> Result<&mut VmStateV0, RuntimeFault> {
        match &mut self.working.state.status {
            RuntimeStatusV0::Ready { vm } => Ok(vm),
            RuntimeStatusV0::Awaiting { .. } | RuntimeStatusV0::Finished { .. } => {
                Err(RuntimeFault::InvalidState)
            }
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
        speaker: Option<crate::program::ConstIndex>,
        text: crate::program::ConstIndex,
        resume_to: InstructionId,
    ) -> Result<(), RuntimeFault> {
        self.require_safe_stacks()?;
        let speaker = speaker.map(|index| self.constant_text(index)).transpose()?;
        let text = self.constant_text(text)?;
        self.charge_text(speaker.as_deref())?;
        self.charge_text(Some(&text))?;
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
        prompt: Option<crate::program::ConstIndex>,
        choices: &[crate::program::ChoiceArmV0],
    ) -> Result<(), RuntimeFault> {
        self.require_safe_stacks()?;
        let flow = self
            .ready_vm()?
            .frames
            .last()
            .map(|frame| frame.flow)
            .ok_or(RuntimeFault::InvalidState)?;
        let prompt = prompt.map(|index| self.constant_text(index)).transpose()?;
        self.charge_text(prompt.as_deref())?;
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
                let label = self.constant_text(choice.label)?;
                self.charge_text(Some(&label))?;
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

    fn constant_text(&self, index: crate::program::ConstIndex) -> Result<Arc<str>, RuntimeFault> {
        match self.program.constant(index) {
            Some(Value::String(text)) => Ok(text.clone()),
            _ => Err(RuntimeFault::InvalidState),
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

    fn charge_text(&mut self, value: Option<&str>) -> Result<(), RuntimeFault> {
        self.charge_units(value.map_or(0, |text| 1_u64.saturating_add(text.len() as u64)))
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
        if total > self.limits.runtime.max_total_live_values {
            Err(RuntimeFault::LiveValueLimit)
        } else {
            Ok(())
        }
    }

    fn complete(mut self) -> SliceOutcome {
        let next_state_digest = crate::snapshot::state_digest(&self.working.state);
        let (result, result_kind) = match &self.working.state.status {
            RuntimeStatusV0::Awaiting { pending, .. } => {
                let result = interaction_view(pending);
                let kind = match result {
                    DraftResult::AwaitSay(_) => ReceiptResultKindV0::Say,
                    DraftResult::AwaitChoice(_) => ReceiptResultKindV0::Choice,
                    DraftResult::Finished(_) => ReceiptResultKindV0::Finished,
                };
                (result, kind)
            }
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

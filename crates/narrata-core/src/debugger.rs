use std::collections::BTreeSet;

use crate::{
    FieldId, GlobalId, InstructionId, RuntimeStateV0, Value,
    runtime::{FrameStateV0, RuntimeStatusV0, SliceProgress},
};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RedactionPolicy {
    pub globals: BTreeSet<GlobalId>,
    pub fields: BTreeSet<FieldId>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InspectedValue {
    Visible(Value),
    Redacted,
    List(Vec<InspectedValue>),
    Record(Vec<(FieldId, InspectedValue)>),
    Variant {
        type_id: crate::TypeId,
        variant_id: crate::VariantId,
        payload: Box<InspectedValue>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InspectedFrame {
    pub flow: crate::FlowId,
    pub instruction: InstructionId,
    pub return_to: Option<InstructionId>,
    pub locals: Vec<(crate::LocalId, InspectedValue)>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeInspection {
    pub status: &'static str,
    pub globals: Vec<(GlobalId, InspectedValue)>,
    pub frames: Vec<InspectedFrame>,
    pub active_states: Vec<crate::StateId>,
}

pub fn inspect_runtime(state: &RuntimeStateV0, redaction: &RedactionPolicy) -> RuntimeInspection {
    RuntimeInspection {
        status: status_name(&state.status),
        globals: state
            .globals
            .iter()
            .map(|(id, value)| {
                (
                    *id,
                    if redaction.globals.contains(id) {
                        InspectedValue::Redacted
                    } else {
                        inspect_value(value, redaction)
                    },
                )
            })
            .collect(),
        frames: frames(&state.status)
            .iter()
            .map(|frame| inspect_frame(frame, redaction))
            .collect(),
        active_states: state
            .statechart
            .as_ref()
            .map(|chart| chart.active.iter().copied().collect())
            .unwrap_or_default(),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InstructionSliceBreakpoint {
    pub instruction: InstructionId,
}

impl InstructionSliceBreakpoint {
    pub fn matches(self, progress: &SliceProgress) -> bool {
        progress
            .last_trace
            .as_ref()
            .is_some_and(|trace| trace.instruction == self.instruction)
    }
}

fn inspect_frame(frame: &FrameStateV0, redaction: &RedactionPolicy) -> InspectedFrame {
    InspectedFrame {
        flow: frame.flow,
        instruction: frame.instruction,
        return_to: frame.return_to,
        locals: frame
            .locals
            .iter()
            .map(|(id, value)| (*id, inspect_value(value, redaction)))
            .collect(),
    }
}

fn inspect_value(value: &Value, redaction: &RedactionPolicy) -> InspectedValue {
    match value {
        Value::List(values) => InspectedValue::List(
            values
                .iter()
                .map(|value| inspect_value(value, redaction))
                .collect(),
        ),
        Value::Record(values) => InspectedValue::Record(
            values
                .iter()
                .map(|(id, value)| {
                    (
                        *id,
                        if redaction.fields.contains(id) {
                            InspectedValue::Redacted
                        } else {
                            inspect_value(value, redaction)
                        },
                    )
                })
                .collect(),
        ),
        Value::Variant {
            type_id,
            variant_id,
            payload,
        } => InspectedValue::Variant {
            type_id: *type_id,
            variant_id: *variant_id,
            payload: Box::new(inspect_value(payload, redaction)),
        },
        value => InspectedValue::Visible(value.clone()),
    }
}

fn frames(status: &RuntimeStatusV0) -> &[FrameStateV0] {
    match status {
        RuntimeStatusV0::Ready { vm }
        | RuntimeStatusV0::Awaiting { vm, .. }
        | RuntimeStatusV0::AwaitingEffect { vm, .. } => &vm.frames,
        RuntimeStatusV0::Finished { final_frames, .. } => final_frames,
        RuntimeStatusV0::StatechartStable
        | RuntimeStatusV0::AwaitingStatechartEffect { .. }
        | RuntimeStatusV0::StatechartFinished => &[],
    }
}

fn status_name(status: &RuntimeStatusV0) -> &'static str {
    match status {
        RuntimeStatusV0::Ready { .. } => "ready",
        RuntimeStatusV0::Awaiting { .. } => "awaiting-interaction",
        RuntimeStatusV0::AwaitingEffect { .. } => "awaiting-effect",
        RuntimeStatusV0::StatechartStable => "statechart-stable",
        RuntimeStatusV0::AwaitingStatechartEffect { .. } => "awaiting-statechart-effect",
        RuntimeStatusV0::StatechartFinished => "statechart-finished",
        RuntimeStatusV0::Finished { .. } => "finished",
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::{
        ExecutionId, ProgramArtifactId,
        runtime::Turn,
        version::{SEMANTICS_V0, SNAPSHOT_SCHEMA_V1},
    };

    #[test]
    fn inspection_redacts_selected_globals_and_nested_fields() {
        let secret = GlobalId::from_u128(1);
        let record = GlobalId::from_u128(2);
        let hidden = FieldId::from_u128(1);
        let visible = FieldId::from_u128(2);
        let state = RuntimeStateV0 {
            snapshot_schema: SNAPSHOT_SCHEMA_V1,
            semantics_version: SEMANTICS_V0,
            execution_id: ExecutionId::from_u128(1),
            program_artifact_id: ProgramArtifactId::from_bytes([1; 32]),
            turn: Turn(0),
            interaction_counter: 0,
            globals: BTreeMap::from([
                (secret, Value::from("token")),
                (
                    record,
                    Value::Record(BTreeMap::from([
                        (hidden, Value::from("private")),
                        (visible, Value::I64(7)),
                    ])),
                ),
            ]),
            scene: None,
            statechart: None,
            status: RuntimeStatusV0::StatechartStable,
        };
        let inspected = inspect_runtime(
            &state,
            &RedactionPolicy {
                globals: BTreeSet::from([secret]),
                fields: BTreeSet::from([hidden]),
            },
        );
        assert_eq!(inspected.globals[0].1, InspectedValue::Redacted);
        assert!(matches!(
            &inspected.globals[1].1,
            InspectedValue::Record(fields)
                if fields[0].1 == InspectedValue::Redacted
                    && fields[1].1 == InspectedValue::Visible(Value::I64(7))
        ));
    }
}

#![allow(clippy::panic, clippy::unwrap_used)]

use narrata_core::{
    ChoiceId, FlowId, InstructionId, Value,
    diagnostic::codes::{
        CONTROL_FLOW_INVALID, DUPLICATE_ID, KIND_MISMATCH, MISSING_REFERENCE, STACK_INVALID,
    },
    limits::ProgramLoadLimits,
    program::{ChoiceArmV0, ConstIndex, OpV0, ReturnModeV0, validate_program},
};
use narrata_testkit::generator::{branch_call_choice_v0, hello_v0};

#[test]
fn malformed_programs_fail_before_runtime_with_stable_diagnostics() {
    let mut cases = Vec::new();

    let mut duplicate_flow = hello_v0();
    duplicate_flow.flows.push(duplicate_flow.flows[0].clone());
    cases.push((duplicate_flow, DUPLICATE_ID));

    let mut duplicate_instruction = hello_v0();
    duplicate_instruction.flows[0].instructions[1].id =
        duplicate_instruction.flows[0].instructions[0].id;
    cases.push((duplicate_instruction, DUPLICATE_ID));

    let mut missing_entry = hello_v0();
    missing_entry.entry_flow = FlowId::from_u128(999);
    cases.push((missing_entry, MISSING_REFERENCE));

    let mut dangling = hello_v0();
    dangling.flows[0].instructions[0].op = OpV0::Say {
        speaker: None,
        text: ConstIndex(0),
        next: InstructionId::from_u128(999),
    };
    cases.push((dangling, MISSING_REFERENCE));

    let mut wrong_text = hello_v0();
    wrong_text.constants[0] = Value::I64(1);
    cases.push((wrong_text, KIND_MISMATCH));

    let mut return_in_entry = hello_v0();
    return_in_entry.flows[0].instructions[0].op = OpV0::Return {
        value: ReturnModeV0::None,
    };
    cases.push((return_in_entry, CONTROL_FLOW_INVALID));

    let mut finish_subflow = branch_call_choice_v0();
    finish_subflow.flows[1].instructions[0].op = OpV0::Finish {
        value: ReturnModeV0::None,
    };
    cases.push((finish_subflow, CONTROL_FLOW_INVALID));

    let mut duplicate_choice = branch_call_choice_v0();
    if let OpV0::Choice { choices, .. } = &mut duplicate_choice.flows[0].instructions[11].op {
        choices[1].id = choices[0].id;
    }
    cases.push((duplicate_choice, DUPLICATE_ID));

    let mut bad_choice_target = branch_call_choice_v0();
    if let OpV0::Choice { choices, .. } = &mut bad_choice_target.flows[0].instructions[11].op {
        choices[0].target = InstructionId::from_u128(999);
    }
    cases.push((bad_choice_target, MISSING_REFERENCE));

    for (artifact, expected_code) in cases {
        let diagnostics = validate_program(artifact, &ProgramLoadLimits::default()).unwrap_err();
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == expected_code)
        );
        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| !diagnostic.path.to_string().is_empty())
        );
    }
}

#[test]
fn safe_point_with_nonempty_stack_and_stack_underflow_are_rejected() {
    let mut nonempty = hello_v0();
    let say = nonempty.flows[0].instructions[0].clone();
    let const_id = InstructionId::from_u128(500);
    nonempty.flows[0].entry = const_id;
    nonempty.flows[0]
        .instructions
        .push(narrata_core::program::InstructionRecordV0 {
            id: const_id,
            op: OpV0::Const {
                constant: ConstIndex(0),
                next: say.id,
            },
        });
    let diagnostics = validate_program(nonempty, &Default::default()).unwrap_err();
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == STACK_INVALID)
    );

    let mut underflow = hello_v0();
    underflow.flows[0].instructions[0].op = OpV0::Store {
        slot: narrata_core::program::SlotRefV0::Global(narrata_core::GlobalId::from_u128(1)),
        next: underflow.flows[0].instructions[1].id,
    };
    let diagnostics = validate_program(underflow, &Default::default()).unwrap_err();
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == MISSING_REFERENCE
                || diagnostic.code == STACK_INVALID)
    );
}

#[test]
fn choice_ids_are_program_global_not_per_instruction() {
    let mut artifact = branch_call_choice_v0();
    artifact.flows[0]
        .instructions
        .push(narrata_core::program::InstructionRecordV0 {
            id: InstructionId::from_u128(999),
            op: OpV0::Choice {
                prompt: None,
                choices: vec![ChoiceArmV0 {
                    id: ChoiceId::from_u128(1),
                    label: ConstIndex(3),
                    visible_if: None,
                    target: InstructionId::from_u128(20),
                }],
            },
        });
    let diagnostics = validate_program(artifact, &Default::default()).unwrap_err();
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == DUPLICATE_ID)
    );
}

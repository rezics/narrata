use std::collections::{BTreeMap, VecDeque};

use crate::{
    diagnostic::{
        Diagnostic, DiagnosticClass, DiagnosticPath,
        codes::{KIND_MISMATCH, STACK_INVALID},
    },
    identity::{FlowId, InstructionId},
    value::ValueKindV0,
};

use super::{BinaryOpV0, CheckedProgram, OpV0, ReturnModeV0, UnaryOpV0};

pub(crate) fn analyze(
    program: &CheckedProgram,
) -> Result<BTreeMap<FlowId, usize>, Vec<Diagnostic>> {
    let mut maxima = BTreeMap::new();
    let mut diagnostics = Vec::new();
    for flow in &program.artifact.flows {
        match analyze_flow(program, flow.id) {
            Ok(maximum) => {
                maxima.insert(flow.id, maximum);
            }
            Err(mut errors) => diagnostics.append(&mut errors),
        }
    }
    if diagnostics.is_empty() {
        Ok(maxima)
    } else {
        Err(diagnostics)
    }
}

fn analyze_flow(program: &CheckedProgram, flow_id: FlowId) -> Result<usize, Vec<Diagnostic>> {
    let Some(flow) = program.flow(flow_id) else {
        return Ok(0);
    };
    let mut incoming: BTreeMap<InstructionId, Vec<ValueKindV0>> = BTreeMap::new();
    incoming.insert(flow.entry, Vec::new());
    let mut queue = VecDeque::from([flow.entry]);
    let mut maximum = 0_usize;
    let mut diagnostics = Vec::new();
    while let Some(id) = queue.pop_front() {
        let Some(instruction) = program.instruction(flow_id, id) else {
            continue;
        };
        let Some(mut stack) = incoming.get(&id).cloned() else {
            continue;
        };
        let path = DiagnosticPath::root()
            .field("flows")
            .object(flow_id)
            .field("instructions")
            .object(id);
        let result = apply(program, flow_id, &instruction.op, &mut stack, path.clone());
        let successors = match result {
            Ok(successors) => successors,
            Err(error) => {
                diagnostics.push(error);
                continue;
            }
        };
        maximum = maximum.max(stack.len());
        for successor in successors {
            match incoming.get(&successor) {
                Some(previous) if previous != &stack => diagnostics.push(Diagnostic::new(
                    DiagnosticClass::Validation,
                    STACK_INVALID,
                    path.clone(),
                    format!("stack shape at merge into {successor} differs"),
                )),
                Some(_) => {}
                None => {
                    incoming.insert(successor, stack.clone());
                    queue.push_back(successor);
                }
            }
        }
    }
    if diagnostics.is_empty() {
        Ok(maximum)
    } else {
        Err(diagnostics)
    }
}

fn apply(
    program: &CheckedProgram,
    flow_id: FlowId,
    op: &OpV0,
    stack: &mut Vec<ValueKindV0>,
    path: DiagnosticPath,
) -> Result<Vec<InstructionId>, Diagnostic> {
    match op {
        OpV0::Const { constant, next } => {
            let kind = program
                .constant(*constant)
                .map(crate::value::Value::kind)
                .ok_or_else(|| {
                    Diagnostic::new(
                        DiagnosticClass::Validation,
                        KIND_MISMATCH,
                        path.clone(),
                        "constant index is invalid",
                    )
                })?;
            stack.push(kind);
            Ok(vec![*next])
        }
        OpV0::Load { slot, next } => {
            let kind = program.slot_kind(flow_id, *slot).ok_or_else(|| {
                Diagnostic::new(
                    DiagnosticClass::Validation,
                    KIND_MISMATCH,
                    path.clone(),
                    "slot does not exist",
                )
            })?;
            stack.push(kind);
            Ok(vec![*next])
        }
        OpV0::Store { slot, next } => {
            let expected = program.slot_kind(flow_id, *slot).ok_or_else(|| {
                Diagnostic::new(
                    DiagnosticClass::Validation,
                    KIND_MISMATCH,
                    path.clone(),
                    "slot does not exist",
                )
            })?;
            pop_kind(stack, expected, path)?;
            Ok(vec![*next])
        }
        OpV0::Unary { op, next } => {
            let expected = match op {
                UnaryOpV0::Not => ValueKindV0::Bool,
                UnaryOpV0::Neg => ValueKindV0::I64,
            };
            pop_kind(stack, expected, path)?;
            stack.push(expected);
            Ok(vec![*next])
        }
        OpV0::Binary { op, next } => {
            match op {
                BinaryOpV0::Eq | BinaryOpV0::Ne => {
                    let right = pop(stack, path.clone())?;
                    let left = pop(stack, path.clone())?;
                    if left != right {
                        return Err(kind_error(path, left, right));
                    }
                    stack.push(ValueKindV0::Bool);
                }
                BinaryOpV0::Add
                | BinaryOpV0::Sub
                | BinaryOpV0::Mul
                | BinaryOpV0::Div
                | BinaryOpV0::Rem => {
                    pop_kind(stack, ValueKindV0::I64, path.clone())?;
                    pop_kind(stack, ValueKindV0::I64, path)?;
                    stack.push(ValueKindV0::I64);
                }
                BinaryOpV0::Lt | BinaryOpV0::Le | BinaryOpV0::Gt | BinaryOpV0::Ge => {
                    pop_kind(stack, ValueKindV0::I64, path.clone())?;
                    pop_kind(stack, ValueKindV0::I64, path)?;
                    stack.push(ValueKindV0::Bool);
                }
            }
            Ok(vec![*next])
        }
        OpV0::Jump { target } => Ok(vec![*target]),
        OpV0::JumpIfFalse { if_true, if_false } => {
            pop_kind(stack, ValueKindV0::Bool, path)?;
            Ok(vec![*if_true, *if_false])
        }
        OpV0::Call {
            flow,
            argument_count: _,
            return_to,
        } => {
            let callee = program.flow(*flow).ok_or_else(|| {
                Diagnostic::new(
                    DiagnosticClass::Validation,
                    KIND_MISMATCH,
                    path.clone(),
                    "callee does not exist",
                )
            })?;
            for parameter in callee.parameters.iter().rev() {
                pop_kind(stack, parameter.kind, path.clone())?;
            }
            if let Some(kind) = callee.return_kind {
                stack.push(kind);
            }
            Ok(vec![*return_to])
        }
        OpV0::Return { value } => {
            let flow = program.flow(flow_id).ok_or_else(|| {
                Diagnostic::new(
                    DiagnosticClass::Validation,
                    STACK_INVALID,
                    path.clone(),
                    "flow does not exist",
                )
            })?;
            check_terminal_stack(stack, *value, flow.return_kind, path)?;
            Ok(Vec::new())
        }
        OpV0::Say { next, .. } => {
            require_empty(stack, path)?;
            Ok(vec![*next])
        }
        OpV0::Choice { choices, .. } => {
            require_empty(stack, path)?;
            Ok(choices.iter().map(|choice| choice.target).collect())
        }
        OpV0::Finish { value } => {
            let expected = match value {
                ReturnModeV0::None => None,
                ReturnModeV0::Stack => stack.last().copied(),
            };
            check_terminal_stack(stack, *value, expected, path)?;
            Ok(Vec::new())
        }
    }
}

fn check_terminal_stack(
    stack: &mut Vec<ValueKindV0>,
    mode: ReturnModeV0,
    expected: Option<ValueKindV0>,
    path: DiagnosticPath,
) -> Result<(), Diagnostic> {
    match (mode, expected) {
        (ReturnModeV0::None, None) => require_empty(stack, path),
        (ReturnModeV0::Stack, Some(kind)) => {
            pop_kind(stack, kind, path.clone())?;
            require_empty(stack, path)
        }
        _ => Err(Diagnostic::new(
            DiagnosticClass::Validation,
            KIND_MISMATCH,
            path,
            "return mode does not match flow return kind",
        )),
    }
}

fn require_empty(stack: &[ValueKindV0], path: DiagnosticPath) -> Result<(), Diagnostic> {
    if stack.is_empty() {
        Ok(())
    } else {
        Err(Diagnostic::new(
            DiagnosticClass::Validation,
            STACK_INVALID,
            path,
            "safe point requires an empty evaluation stack",
        ))
    }
}

fn pop(stack: &mut Vec<ValueKindV0>, path: DiagnosticPath) -> Result<ValueKindV0, Diagnostic> {
    stack.pop().ok_or_else(|| {
        Diagnostic::new(
            DiagnosticClass::Validation,
            STACK_INVALID,
            path,
            "evaluation stack underflow",
        )
    })
}

fn pop_kind(
    stack: &mut Vec<ValueKindV0>,
    expected: ValueKindV0,
    path: DiagnosticPath,
) -> Result<(), Diagnostic> {
    let actual = pop(stack, path.clone())?;
    if actual == expected {
        Ok(())
    } else {
        Err(kind_error(path, expected, actual))
    }
}

fn kind_error(path: DiagnosticPath, expected: ValueKindV0, actual: ValueKindV0) -> Diagnostic {
    Diagnostic::new(
        DiagnosticClass::Validation,
        KIND_MISMATCH,
        path,
        format!("expected {expected:?}, found {actual:?}"),
    )
}

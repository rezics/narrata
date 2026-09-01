use std::{collections::BTreeMap, error::Error, fmt};

use crate::{
    codec::{DecodeError, digest_bytes},
    diagnostic::{
        Diagnostic, DiagnosticClass, DiagnosticPath,
        codes::{
            CONTROL_FLOW_INVALID, DUPLICATE_ID, KIND_MISMATCH, LIMIT_EXCEEDED, MISSING_REFERENCE,
            UNSUPPORTED_VERSION,
        },
    },
    identity::{ChoiceId, FlowId, InstructionId, ProgramArtifactId},
    limits::ProgramLoadLimits,
    version::{PROGRAM_FORMAT_V0, SEMANTICS_V0},
};

use super::{
    CheckedProgram, OpV0, ProgramArtifactV0, ReturnModeV0, SlotRefV0, decode_program_artifact,
    stack_analysis,
};

#[derive(Clone, Debug)]
pub enum ProgramLoadError {
    Decode(DecodeError),
    Validation(Vec<Diagnostic>),
}

impl fmt::Display for ProgramLoadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Decode(error) => write!(formatter, "program decode failed: {error}"),
            Self::Validation(errors) => {
                write!(
                    formatter,
                    "program validation failed with {} error(s)",
                    errors.len()
                )?;
                for error in errors {
                    write!(formatter, "\n{error}")?;
                }
                Ok(())
            }
        }
    }
}

impl Error for ProgramLoadError {}

pub fn load_program(
    bytes: &[u8],
    limits: &ProgramLoadLimits,
) -> Result<std::sync::Arc<CheckedProgram>, ProgramLoadError> {
    let artifact = decode_program_artifact(bytes, limits).map_err(ProgramLoadError::Decode)?;
    validate_program(artifact, limits)
        .map(std::sync::Arc::new)
        .map_err(ProgramLoadError::Validation)
}

pub fn validate_program(
    artifact: ProgramArtifactV0,
    limits: &ProgramLoadLimits,
) -> Result<CheckedProgram, Vec<Diagnostic>> {
    let mut diagnostics = Vec::new();
    if artifact.format_version != PROGRAM_FORMAT_V0 {
        diagnostics.push(Diagnostic::new(
            DiagnosticClass::Incompatible,
            UNSUPPORTED_VERSION,
            DiagnosticPath::root().field("format_version"),
            "unsupported Program format version",
        ));
    }
    if artifact.semantics_version != SEMANTICS_V0 {
        diagnostics.push(Diagnostic::new(
            DiagnosticClass::Incompatible,
            UNSUPPORTED_VERSION,
            DiagnosticPath::root().field("semantics_version"),
            "unsupported semantics version",
        ));
    }
    check_counts(&artifact, limits, &mut diagnostics);

    let mut flow_indices = BTreeMap::new();
    let mut instruction_indices = BTreeMap::new();
    let mut global_indices = BTreeMap::new();
    let mut choice_ids = BTreeMap::<ChoiceId, (FlowId, InstructionId)>::new();
    for (index, global) in artifact.globals.iter().enumerate() {
        if global_indices.insert(global.id, index).is_some() {
            diagnostics.push(duplicate("globals", global.id));
        }
        if global.default.kind() != global.kind {
            diagnostics.push(kind_mismatch(
                DiagnosticPath::root().field("globals").index(index),
                "global default does not match its declared kind",
            ));
        }
    }
    for (flow_index, flow) in artifact.flows.iter().enumerate() {
        if flow_indices.insert(flow.id, flow_index).is_some() {
            diagnostics.push(duplicate("flows", flow.id));
        }
        let flow_path = DiagnosticPath::root().field("flows").index(flow_index);
        let mut locals = BTreeMap::new();
        for (index, local) in flow.parameters.iter().chain(&flow.locals).enumerate() {
            if locals.insert(local.id, index).is_some() {
                diagnostics.push(
                    Diagnostic::new(
                        DiagnosticClass::Validation,
                        DUPLICATE_ID,
                        flow_path.clone().field("locals").index(index),
                        "duplicate local identity",
                    )
                    .with_id(local.id),
                );
            }
            if local.default.kind() != local.kind {
                diagnostics.push(kind_mismatch(
                    flow_path.clone().field("locals").index(index),
                    "local default does not match its declared kind",
                ));
            }
        }
        let mut indices = BTreeMap::new();
        for (instruction_index, instruction) in flow.instructions.iter().enumerate() {
            if indices.insert(instruction.id, instruction_index).is_some() {
                diagnostics.push(
                    Diagnostic::new(
                        DiagnosticClass::Validation,
                        DUPLICATE_ID,
                        flow_path
                            .clone()
                            .field("instructions")
                            .index(instruction_index),
                        "duplicate instruction identity in flow",
                    )
                    .with_id(instruction.id),
                );
            }
            if let OpV0::Choice { choices, .. } = &instruction.op {
                for choice in choices {
                    if choice_ids
                        .insert(choice.id, (flow.id, instruction.id))
                        .is_some()
                    {
                        diagnostics.push(
                            Diagnostic::new(
                                DiagnosticClass::Validation,
                                DUPLICATE_ID,
                                flow_path.clone().field("choices"),
                                "duplicate ChoiceId across Program",
                            )
                            .with_id(choice.id),
                        );
                    }
                }
            }
        }
        instruction_indices.insert(flow.id, indices);
    }

    if !flow_indices.contains_key(&artifact.entry_flow) {
        diagnostics.push(
            Diagnostic::new(
                DiagnosticClass::Validation,
                MISSING_REFERENCE,
                DiagnosticPath::root().field("entry_flow"),
                "entry Flow does not exist",
            )
            .with_id(artifact.entry_flow),
        );
    } else if artifact
        .flows
        .get(
            *flow_indices
                .get(&artifact.entry_flow)
                .unwrap_or(&usize::MAX),
        )
        .is_some_and(|flow| !flow.parameters.is_empty())
    {
        diagnostics.push(Diagnostic::new(
            DiagnosticClass::Validation,
            CONTROL_FLOW_INVALID,
            DiagnosticPath::root().field("entry_flow"),
            "entry Flow cannot declare parameters",
        ));
    }

    let payload = super::wire::encode_program_payload(&artifact);
    let artifact_id = ProgramArtifactId::from_bytes(digest_bytes("program-artifact", 0, &payload));
    let mut checked = CheckedProgram {
        artifact,
        artifact_id,
        flow_indices,
        instruction_indices,
        global_indices,
        stack_limits: BTreeMap::new(),
    };
    if diagnostics.is_empty() {
        validate_references(&checked, &mut diagnostics);
    }
    if diagnostics.is_empty() {
        match stack_analysis::analyze(&checked) {
            Ok(limits_by_flow)
                if limits_by_flow
                    .values()
                    .all(|maximum| *maximum as u64 <= limits.runtime.max_stack_depth) =>
            {
                checked.stack_limits = limits_by_flow;
            }
            Ok(_) => diagnostics.push(Diagnostic::new(
                DiagnosticClass::LimitExceeded,
                LIMIT_EXCEEDED,
                DiagnosticPath::root().field("flows"),
                "statically required evaluation stack exceeds Runtime limit",
            )),
            Err(mut errors) => diagnostics.append(&mut errors),
        }
    }
    if diagnostics.is_empty() {
        Ok(checked)
    } else {
        Err(diagnostics)
    }
}

fn check_counts(
    artifact: &ProgramArtifactV0,
    limits: &ProgramLoadLimits,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let instruction_count: u64 = artifact
        .flows
        .iter()
        .map(|flow| flow.instructions.len() as u64)
        .sum();
    let too_many = artifact.flows.len() as u64 > limits.program.max_flows
        || artifact.constants.len() as u64 > limits.program.max_constants
        || artifact.globals.len() as u64 > limits.program.max_globals
        || instruction_count > limits.program.max_instructions
        || artifact.flows.iter().any(|flow| {
            flow.parameters.len().saturating_add(flow.locals.len()) as u64
                > limits.program.max_locals_per_flow
        })
        || artifact.flows.iter().any(|flow| {
            flow.instructions.iter().any(|instruction| {
                matches!(
                    &instruction.op,
                    OpV0::Choice { choices, .. }
                        if choices.len() as u64 > limits.program.max_choices_per_instruction
                )
            })
        });
    if too_many {
        diagnostics.push(Diagnostic::new(
            DiagnosticClass::LimitExceeded,
            LIMIT_EXCEEDED,
            DiagnosticPath::root(),
            "Program exceeds configured limits",
        ));
    }
    if !artifact.capabilities.is_empty() || !artifact.external_content.is_empty() {
        diagnostics.push(Diagnostic::new(
            DiagnosticClass::Validation,
            CONTROL_FLOW_INVALID,
            DiagnosticPath::root(),
            "Stage 1 capability and external content declarations must be empty",
        ));
    }
}

fn validate_references(program: &CheckedProgram, diagnostics: &mut Vec<Diagnostic>) {
    for flow in &program.artifact.flows {
        if program.instruction(flow.id, flow.entry).is_none() {
            diagnostics.push(missing(
                DiagnosticPath::root()
                    .field("flows")
                    .object(flow.id)
                    .field("entry"),
                flow.entry,
            ));
        }
        for instruction in &flow.instructions {
            let path = DiagnosticPath::root()
                .field("flows")
                .object(flow.id)
                .field("instructions")
                .object(instruction.id);
            match &instruction.op {
                OpV0::Const { constant, next } => {
                    constant_ref(program, *constant, None, path.clone(), diagnostics);
                    target_ref(program, flow.id, *next, path.clone(), diagnostics);
                }
                OpV0::Load { slot, next } | OpV0::Store { slot, next } => {
                    slot_ref(program, flow.id, *slot, path.clone(), diagnostics);
                    target_ref(program, flow.id, *next, path.clone(), diagnostics);
                }
                OpV0::Unary { next, .. } | OpV0::Binary { next, .. } => {
                    target_ref(program, flow.id, *next, path.clone(), diagnostics)
                }
                OpV0::Jump { target: jump } => {
                    target_ref(program, flow.id, *jump, path.clone(), diagnostics)
                }
                OpV0::JumpIfFalse { if_true, if_false } => {
                    target_ref(program, flow.id, *if_true, path.clone(), diagnostics);
                    target_ref(program, flow.id, *if_false, path.clone(), diagnostics);
                }
                OpV0::Call {
                    flow: callee,
                    argument_count,
                    return_to,
                } => {
                    target_ref(program, flow.id, *return_to, path.clone(), diagnostics);
                    match program.flow(*callee) {
                        Some(callee_flow)
                            if callee_flow.parameters.len() == usize::from(*argument_count) => {}
                        Some(_) => diagnostics.push(kind_mismatch(
                            path.clone(),
                            "Call argument count does not match callee parameters",
                        )),
                        None => diagnostics.push(missing(path.clone(), *callee)),
                    }
                }
                OpV0::Return { value } => {
                    if flow.id == program.artifact.entry_flow {
                        diagnostics.push(control(
                            path.clone(),
                            "Return is forbidden in the entry Flow",
                        ));
                    }
                    validate_return_mode(*value, flow.return_kind, path, diagnostics);
                }
                OpV0::Say {
                    speaker,
                    text,
                    next,
                } => {
                    if let Some(speaker) = speaker {
                        constant_ref(
                            program,
                            *speaker,
                            Some(crate::value::ValueKindV0::String),
                            path.clone(),
                            diagnostics,
                        );
                    }
                    constant_ref(
                        program,
                        *text,
                        Some(crate::value::ValueKindV0::String),
                        path.clone(),
                        diagnostics,
                    );
                    target_ref(program, flow.id, *next, path.clone(), diagnostics);
                }
                OpV0::Choice { prompt, choices } => {
                    if let Some(prompt) = prompt {
                        constant_ref(
                            program,
                            *prompt,
                            Some(crate::value::ValueKindV0::String),
                            path.clone(),
                            diagnostics,
                        );
                    }
                    for choice in choices {
                        constant_ref(
                            program,
                            choice.label,
                            Some(crate::value::ValueKindV0::String),
                            path.clone(),
                            diagnostics,
                        );
                        if let Some(slot) = choice.visible_if {
                            slot_ref(program, flow.id, slot, path.clone(), diagnostics);
                            if program.slot_kind(flow.id, slot)
                                != Some(crate::value::ValueKindV0::Bool)
                            {
                                diagnostics.push(kind_mismatch(
                                    path.clone(),
                                    "Choice visibility slot must be Bool",
                                ));
                            }
                        }
                        target_ref(program, flow.id, choice.target, path.clone(), diagnostics);
                    }
                }
                OpV0::Finish { value } => {
                    if flow.id != program.artifact.entry_flow {
                        diagnostics.push(control(
                            path.clone(),
                            "Finish is only allowed in the entry Flow",
                        ));
                    }
                    if *value == ReturnModeV0::None {
                        // Valid and yields Null.
                    }
                }
            }
        }
    }
}

fn validate_return_mode(
    mode: ReturnModeV0,
    kind: Option<crate::value::ValueKindV0>,
    path: DiagnosticPath,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if !matches!(
        (mode, kind),
        (ReturnModeV0::None, None) | (ReturnModeV0::Stack, Some(_))
    ) {
        diagnostics.push(kind_mismatch(
            path,
            "Return mode does not match Flow return kind",
        ));
    }
}

fn constant_ref(
    program: &CheckedProgram,
    index: super::ConstIndex,
    expected: Option<crate::value::ValueKindV0>,
    path: DiagnosticPath,
    diagnostics: &mut Vec<Diagnostic>,
) {
    match program.constant(index) {
        Some(value) if expected.is_none_or(|kind| value.kind() == kind) => {}
        Some(_) => diagnostics.push(kind_mismatch(path, "constant has the wrong Value kind")),
        None => diagnostics.push(Diagnostic::new(
            DiagnosticClass::Validation,
            MISSING_REFERENCE,
            path,
            format!("constant index {} does not exist", index.0),
        )),
    }
}

fn slot_ref(
    program: &CheckedProgram,
    flow: FlowId,
    slot: SlotRefV0,
    path: DiagnosticPath,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if program.slot_kind(flow, slot).is_none() {
        diagnostics.push(Diagnostic::new(
            DiagnosticClass::Validation,
            MISSING_REFERENCE,
            path,
            "slot reference does not exist in this Flow",
        ));
    }
}

fn target_ref(
    program: &CheckedProgram,
    flow: FlowId,
    instruction: InstructionId,
    path: DiagnosticPath,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if program.instruction(flow, instruction).is_none() {
        diagnostics.push(missing(path, instruction));
    }
}

fn duplicate(section: &'static str, id: impl ToString) -> Diagnostic {
    Diagnostic::new(
        DiagnosticClass::Validation,
        DUPLICATE_ID,
        DiagnosticPath::root().field(section),
        "duplicate identity",
    )
    .with_id(id)
}

fn missing(path: DiagnosticPath, id: impl ToString) -> Diagnostic {
    Diagnostic::new(
        DiagnosticClass::Validation,
        MISSING_REFERENCE,
        path,
        "referenced identity does not exist in the current Flow or Program",
    )
    .with_id(id)
}

fn kind_mismatch(path: DiagnosticPath, message: &'static str) -> Diagnostic {
    Diagnostic::new(DiagnosticClass::Validation, KIND_MISMATCH, path, message)
}

fn control(path: DiagnosticPath, message: &'static str) -> Diagnostic {
    Diagnostic::new(
        DiagnosticClass::Validation,
        CONTROL_FLOW_INVALID,
        path,
        message,
    )
}

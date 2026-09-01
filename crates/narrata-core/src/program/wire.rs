use crate::{
    codec::{CborReader, CborWriter, DecodeError, ObjectKind, decode_envelope, encode_envelope},
    identity::{ChoiceId, FlowId, GlobalId, InstructionId, LocalId, ProgramId},
    limits::ProgramLoadLimits,
    value::{Value, ValueKindV0},
    version::{ProgramFormatVersion, SemanticsVersion},
};

use super::{
    BinaryOpV0, CapabilityDeclV0, ChoiceArmV0, ConstIndex, ExternalContentDeclV0, FlowV0,
    GlobalDeclV0, InstructionRecordV0, LocalDeclV0, OpV0, ReturnModeV0, SlotRefV0, UnaryOpV0,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProgramArtifactV0 {
    pub format_version: ProgramFormatVersion,
    pub semantics_version: SemanticsVersion,
    pub program_id: ProgramId,
    pub entry_flow: FlowId,
    pub constants: Vec<Value>,
    pub globals: Vec<GlobalDeclV0>,
    pub flows: Vec<FlowV0>,
    pub capabilities: Vec<CapabilityDeclV0>,
    pub external_content: Vec<ExternalContentDeclV0>,
}

pub fn encode_program_artifact(artifact: &ProgramArtifactV0) -> Vec<u8> {
    let payload = encode_program_payload(artifact);
    encode_envelope(ObjectKind::Program, artifact.format_version.get(), &payload)
}

pub(crate) fn encode_program_payload(artifact: &ProgramArtifactV0) -> Vec<u8> {
    let mut writer = CborWriter::new();
    writer.map(9);
    writer.unsigned(0);
    writer.unsigned(u64::from(artifact.format_version.get()));
    writer.unsigned(1);
    writer.unsigned(u64::from(artifact.semantics_version.get()));
    writer.unsigned(2);
    writer.bytes(artifact.program_id.as_bytes());
    writer.unsigned(3);
    writer.bytes(artifact.entry_flow.as_bytes());
    writer.unsigned(4);
    writer.array(artifact.constants.len() as u64);
    for value in &artifact.constants {
        crate::value::encode_value(&mut writer, value);
    }
    writer.unsigned(5);
    writer.array(artifact.globals.len() as u64);
    for global in &artifact.globals {
        encode_global(&mut writer, global);
    }
    writer.unsigned(6);
    writer.array(artifact.flows.len() as u64);
    for flow in &artifact.flows {
        encode_flow(&mut writer, flow);
    }
    writer.unsigned(7);
    writer.array(0);
    writer.unsigned(8);
    writer.array(0);
    writer.into_bytes()
}

pub fn decode_program_artifact(
    bytes: &[u8],
    limits: &ProgramLoadLimits,
) -> Result<ProgramArtifactV0, DecodeError> {
    let envelope = decode_envelope(bytes, ObjectKind::Program, 0, &limits.decode)?;
    let artifact = decode_program_payload(envelope.payload, limits)?;
    if encode_program_payload(&artifact) != envelope.payload {
        return Err(DecodeError::NonCanonical(
            "program payload round-trip mismatch",
        ));
    }
    Ok(artifact)
}

fn decode_program_payload(
    bytes: &[u8],
    limits: &ProgramLoadLimits,
) -> Result<ProgramArtifactV0, DecodeError> {
    let mut reader = CborReader::new(bytes);
    expect_map(&mut reader, 9)?;
    let format_version = ProgramFormatVersion::new(read_u16(&mut reader)?);
    expect_key(&mut reader, 1)?;
    let semantics_version = SemanticsVersion::new(read_u16(&mut reader)?);
    expect_key(&mut reader, 2)?;
    let program_id = ProgramId::from_bytes(reader.bytes_exact::<16>()?);
    expect_key(&mut reader, 3)?;
    let entry_flow = FlowId::from_bytes(reader.bytes_exact::<16>()?);
    expect_key(&mut reader, 4)?;
    let constants_len = bounded_array(
        &mut reader,
        limits.program.max_constants,
        "program constants",
    )?;
    let constants_capacity =
        usize::try_from(constants_len).map_err(|_| DecodeError::LengthOverflow)?;
    let mut constants = Vec::with_capacity(constants_capacity);
    let mut value_nodes = 0_u64;
    for _ in 0..constants_len {
        constants.push(crate::value::decode_value(
            &mut reader,
            &limits.decode,
            1,
            &mut value_nodes,
        )?);
    }
    expect_key(&mut reader, 5)?;
    let globals_len = bounded_array(&mut reader, limits.program.max_globals, "program globals")?;
    let globals_capacity = usize::try_from(globals_len).map_err(|_| DecodeError::LengthOverflow)?;
    let mut globals = Vec::with_capacity(globals_capacity);
    for _ in 0..globals_len {
        globals.push(decode_global(&mut reader, limits, &mut value_nodes)?);
    }
    expect_key(&mut reader, 6)?;
    let flows_len = bounded_array(&mut reader, limits.program.max_flows, "program flows")?;
    let flows_capacity = usize::try_from(flows_len).map_err(|_| DecodeError::LengthOverflow)?;
    let mut flows = Vec::with_capacity(flows_capacity);
    let mut total_instructions = 0_u64;
    for _ in 0..flows_len {
        flows.push(decode_flow(
            &mut reader,
            limits,
            &mut value_nodes,
            &mut total_instructions,
        )?);
    }
    expect_key(&mut reader, 7)?;
    if reader.array_len()? != 0 {
        return Err(DecodeError::Schema("Stage 1 capabilities must be empty"));
    }
    expect_key(&mut reader, 8)?;
    if reader.array_len()? != 0 {
        return Err(DecodeError::Schema(
            "Stage 1 external content declarations must be empty",
        ));
    }
    reader.finish()?;
    Ok(ProgramArtifactV0 {
        format_version,
        semantics_version,
        program_id,
        entry_flow,
        constants,
        globals,
        flows,
        capabilities: Vec::new(),
        external_content: Vec::new(),
    })
}

fn encode_global(writer: &mut CborWriter, global: &GlobalDeclV0) {
    writer.array(3);
    writer.bytes(global.id.as_bytes());
    writer.unsigned(global.kind as u64);
    crate::value::encode_value(writer, &global.default);
}

fn decode_global(
    reader: &mut CborReader<'_>,
    limits: &ProgramLoadLimits,
    value_nodes: &mut u64,
) -> Result<GlobalDeclV0, DecodeError> {
    expect_array(reader, 3, "global declaration")?;
    let id = GlobalId::from_bytes(reader.bytes_exact::<16>()?);
    let kind = decode_kind(reader)?;
    let default = crate::value::decode_value(reader, &limits.decode, 1, value_nodes)?;
    Ok(GlobalDeclV0 { id, kind, default })
}

fn encode_local(writer: &mut CborWriter, local: &LocalDeclV0) {
    writer.array(3);
    writer.bytes(local.id.as_bytes());
    writer.unsigned(local.kind as u64);
    crate::value::encode_value(writer, &local.default);
}

fn decode_local(
    reader: &mut CborReader<'_>,
    limits: &ProgramLoadLimits,
    value_nodes: &mut u64,
) -> Result<LocalDeclV0, DecodeError> {
    expect_array(reader, 3, "local declaration")?;
    Ok(LocalDeclV0 {
        id: LocalId::from_bytes(reader.bytes_exact::<16>()?),
        kind: decode_kind(reader)?,
        default: crate::value::decode_value(reader, &limits.decode, 1, value_nodes)?,
    })
}

fn encode_flow(writer: &mut CborWriter, flow: &FlowV0) {
    writer.array(6);
    writer.bytes(flow.id.as_bytes());
    writer.array(flow.parameters.len() as u64);
    for local in &flow.parameters {
        encode_local(writer, local);
    }
    writer.array(flow.locals.len() as u64);
    for local in &flow.locals {
        encode_local(writer, local);
    }
    match flow.return_kind {
        Some(kind) => writer.unsigned(kind as u64),
        None => writer.null(),
    }
    writer.bytes(flow.entry.as_bytes());
    writer.array(flow.instructions.len() as u64);
    for instruction in &flow.instructions {
        writer.array(2);
        writer.bytes(instruction.id.as_bytes());
        encode_op(writer, &instruction.op);
    }
}

fn decode_flow(
    reader: &mut CborReader<'_>,
    limits: &ProgramLoadLimits,
    value_nodes: &mut u64,
    total_instructions: &mut u64,
) -> Result<FlowV0, DecodeError> {
    expect_array(reader, 6, "flow")?;
    let id = FlowId::from_bytes(reader.bytes_exact::<16>()?);
    let parameters_len = bounded_array(
        reader,
        limits.program.max_locals_per_flow,
        "flow parameters",
    )?;
    let parameters_capacity =
        usize::try_from(parameters_len).map_err(|_| DecodeError::LengthOverflow)?;
    let mut parameters = Vec::with_capacity(parameters_capacity);
    for _ in 0..parameters_len {
        parameters.push(decode_local(reader, limits, value_nodes)?);
    }
    let locals_len = bounded_array(reader, limits.program.max_locals_per_flow, "flow locals")?;
    if parameters_len.saturating_add(locals_len) > limits.program.max_locals_per_flow {
        return Err(DecodeError::Limit("total flow locals"));
    }
    let locals_capacity = usize::try_from(locals_len).map_err(|_| DecodeError::LengthOverflow)?;
    let mut locals = Vec::with_capacity(locals_capacity);
    for _ in 0..locals_len {
        locals.push(decode_local(reader, limits, value_nodes)?);
    }
    let return_kind = reader.optional(decode_kind)?;
    let entry = InstructionId::from_bytes(reader.bytes_exact::<16>()?);
    let instructions_len = reader.array_len()?;
    *total_instructions = total_instructions.saturating_add(instructions_len);
    if *total_instructions > limits.program.max_instructions {
        return Err(DecodeError::Limit("program instructions"));
    }
    let instructions_capacity =
        usize::try_from(instructions_len).map_err(|_| DecodeError::LengthOverflow)?;
    let mut instructions = Vec::with_capacity(instructions_capacity);
    for _ in 0..instructions_len {
        expect_array(reader, 2, "instruction")?;
        instructions.push(InstructionRecordV0 {
            id: InstructionId::from_bytes(reader.bytes_exact::<16>()?),
            op: decode_op(reader, limits)?,
        });
    }
    Ok(FlowV0 {
        id,
        parameters,
        locals,
        return_kind,
        entry,
        instructions,
    })
}

fn encode_slot(writer: &mut CborWriter, slot: SlotRefV0) {
    writer.array(2);
    match slot {
        SlotRefV0::Global(id) => {
            writer.unsigned(0);
            writer.bytes(id.as_bytes());
        }
        SlotRefV0::Local(id) => {
            writer.unsigned(1);
            writer.bytes(id.as_bytes());
        }
    }
}

fn decode_slot(reader: &mut CborReader<'_>) -> Result<SlotRefV0, DecodeError> {
    expect_array(reader, 2, "slot reference")?;
    match reader.unsigned()? {
        0 => Ok(SlotRefV0::Global(GlobalId::from_bytes(
            reader.bytes_exact::<16>()?,
        ))),
        1 => Ok(SlotRefV0::Local(LocalId::from_bytes(
            reader.bytes_exact::<16>()?,
        ))),
        _ => Err(DecodeError::Schema("slot reference tag")),
    }
}

fn encode_optional_index(writer: &mut CborWriter, index: Option<ConstIndex>) {
    match index {
        Some(index) => writer.unsigned(u64::from(index.0)),
        None => writer.null(),
    }
}

fn encode_op(writer: &mut CborWriter, op: &OpV0) {
    match op {
        OpV0::Const { constant, next } => {
            writer.array(3);
            writer.unsigned(0);
            writer.unsigned(u64::from(constant.0));
            writer.bytes(next.as_bytes());
        }
        OpV0::Load { slot, next } | OpV0::Store { slot, next } => {
            writer.array(3);
            writer.unsigned(if matches!(op, OpV0::Load { .. }) {
                1
            } else {
                2
            });
            encode_slot(writer, *slot);
            writer.bytes(next.as_bytes());
        }
        OpV0::Unary { op, next } => {
            writer.array(3);
            writer.unsigned(3);
            writer.unsigned(*op as u64);
            writer.bytes(next.as_bytes());
        }
        OpV0::Binary { op, next } => {
            writer.array(3);
            writer.unsigned(4);
            writer.unsigned(*op as u64);
            writer.bytes(next.as_bytes());
        }
        OpV0::Jump { target } => {
            writer.array(2);
            writer.unsigned(5);
            writer.bytes(target.as_bytes());
        }
        OpV0::JumpIfFalse { if_true, if_false } => {
            writer.array(3);
            writer.unsigned(6);
            writer.bytes(if_true.as_bytes());
            writer.bytes(if_false.as_bytes());
        }
        OpV0::Call {
            flow,
            argument_count,
            return_to,
        } => {
            writer.array(4);
            writer.unsigned(7);
            writer.bytes(flow.as_bytes());
            writer.unsigned(u64::from(*argument_count));
            writer.bytes(return_to.as_bytes());
        }
        OpV0::Return { value } => {
            writer.array(2);
            writer.unsigned(8);
            writer.unsigned(*value as u64);
        }
        OpV0::Say {
            speaker,
            text,
            next,
        } => {
            writer.array(4);
            writer.unsigned(9);
            encode_optional_index(writer, *speaker);
            writer.unsigned(u64::from(text.0));
            writer.bytes(next.as_bytes());
        }
        OpV0::Choice { prompt, choices } => {
            writer.array(3);
            writer.unsigned(10);
            encode_optional_index(writer, *prompt);
            writer.array(choices.len() as u64);
            for choice in choices {
                writer.array(4);
                writer.bytes(choice.id.as_bytes());
                writer.unsigned(u64::from(choice.label.0));
                match choice.visible_if {
                    Some(slot) => encode_slot(writer, slot),
                    None => writer.null(),
                }
                writer.bytes(choice.target.as_bytes());
            }
        }
        OpV0::Finish { value } => {
            writer.array(2);
            writer.unsigned(11);
            writer.unsigned(*value as u64);
        }
    }
}

fn decode_op(reader: &mut CborReader<'_>, limits: &ProgramLoadLimits) -> Result<OpV0, DecodeError> {
    let length = reader.array_len()?;
    let tag = reader.unsigned()?;
    let instruction = |bytes| InstructionId::from_bytes(bytes);
    match (tag, length) {
        (0, 3) => Ok(OpV0::Const {
            constant: ConstIndex(read_u32(reader)?),
            next: instruction(reader.bytes_exact::<16>()?),
        }),
        (1, 3) => Ok(OpV0::Load {
            slot: decode_slot(reader)?,
            next: instruction(reader.bytes_exact::<16>()?),
        }),
        (2, 3) => Ok(OpV0::Store {
            slot: decode_slot(reader)?,
            next: instruction(reader.bytes_exact::<16>()?),
        }),
        (3, 3) => Ok(OpV0::Unary {
            op: UnaryOpV0::from_u64(reader.unsigned()?)
                .ok_or(DecodeError::Schema("unary opcode"))?,
            next: instruction(reader.bytes_exact::<16>()?),
        }),
        (4, 3) => Ok(OpV0::Binary {
            op: BinaryOpV0::from_u64(reader.unsigned()?)
                .ok_or(DecodeError::Schema("binary opcode"))?,
            next: instruction(reader.bytes_exact::<16>()?),
        }),
        (5, 2) => Ok(OpV0::Jump {
            target: instruction(reader.bytes_exact::<16>()?),
        }),
        (6, 3) => Ok(OpV0::JumpIfFalse {
            if_true: instruction(reader.bytes_exact::<16>()?),
            if_false: instruction(reader.bytes_exact::<16>()?),
        }),
        (7, 4) => Ok(OpV0::Call {
            flow: FlowId::from_bytes(reader.bytes_exact::<16>()?),
            argument_count: read_u16(reader)?,
            return_to: instruction(reader.bytes_exact::<16>()?),
        }),
        (8, 2) => Ok(OpV0::Return {
            value: decode_return_mode(reader)?,
        }),
        (9, 4) => Ok(OpV0::Say {
            speaker: reader.optional(|reader| read_u32(reader).map(ConstIndex))?,
            text: ConstIndex(read_u32(reader)?),
            next: instruction(reader.bytes_exact::<16>()?),
        }),
        (10, 3) => {
            let prompt = reader.optional(|reader| read_u32(reader).map(ConstIndex))?;
            let choices_len = bounded_array(
                reader,
                limits.program.max_choices_per_instruction,
                "instruction choices",
            )?;
            let capacity = usize::try_from(choices_len).map_err(|_| DecodeError::LengthOverflow)?;
            let mut choices = Vec::with_capacity(capacity);
            for _ in 0..choices_len {
                expect_array(reader, 4, "choice arm")?;
                choices.push(ChoiceArmV0 {
                    id: ChoiceId::from_bytes(reader.bytes_exact::<16>()?),
                    label: ConstIndex(read_u32(reader)?),
                    visible_if: reader.optional(decode_slot)?,
                    target: instruction(reader.bytes_exact::<16>()?),
                });
            }
            Ok(OpV0::Choice { prompt, choices })
        }
        (11, 2) => Ok(OpV0::Finish {
            value: decode_return_mode(reader)?,
        }),
        _ => Err(DecodeError::Schema("instruction opcode or field count")),
    }
}

fn decode_return_mode(reader: &mut CborReader<'_>) -> Result<ReturnModeV0, DecodeError> {
    ReturnModeV0::from_u64(reader.unsigned()?).ok_or(DecodeError::Schema("return mode"))
}

fn decode_kind(reader: &mut CborReader<'_>) -> Result<ValueKindV0, DecodeError> {
    ValueKindV0::from_u64(reader.unsigned()?).ok_or(DecodeError::Schema("value kind"))
}

fn expect_map(reader: &mut CborReader<'_>, length: u64) -> Result<(), DecodeError> {
    if reader.map_len()? != length {
        return Err(DecodeError::Schema("map field count"));
    }
    expect_key(reader, 0)
}

fn expect_key(reader: &mut CborReader<'_>, key: u64) -> Result<(), DecodeError> {
    if reader.unsigned()? == key {
        Ok(())
    } else {
        Err(DecodeError::NonCanonical("map key order or unknown field"))
    }
}

fn expect_array(
    reader: &mut CborReader<'_>,
    length: u64,
    schema: &'static str,
) -> Result<(), DecodeError> {
    if reader.array_len()? == length {
        Ok(())
    } else {
        Err(DecodeError::Schema(schema))
    }
}

fn bounded_array(
    reader: &mut CborReader<'_>,
    maximum: u64,
    label: &'static str,
) -> Result<u64, DecodeError> {
    let length = reader.array_len()?;
    if length > maximum {
        Err(DecodeError::Limit(label))
    } else {
        Ok(length)
    }
}

fn read_u16(reader: &mut CborReader<'_>) -> Result<u16, DecodeError> {
    u16::try_from(reader.unsigned()?).map_err(|_| DecodeError::IntegerOverflow)
}

fn read_u32(reader: &mut CborReader<'_>) -> Result<u32, DecodeError> {
    u32::try_from(reader.unsigned()?).map_err(|_| DecodeError::IntegerOverflow)
}

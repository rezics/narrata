//! Checked, deterministic relocation of a safe-point state between exact Program Artifacts.
//!
//! A relocation table is not executable code and is never accepted from a save. Applications
//! construct a trusted descriptor, migrate an already checked source state, and the result is
//! validated again against the exact target Program before it can be persisted.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use thiserror::Error;

use crate::{
    ActionId, CapabilityId, ChoiceId, EventTypeId, FieldId, FlowId, GlobalId, HistoryId,
    InstructionId, LocalId, MigrationId, ProgramArtifactId, RecoveryCheckpointId, StateId, TypeId,
    Value, VariantId,
    effect::{
        RewindPolicy, derive_effect_id, derive_statechart_effect_id, effect_payload_digest,
        effect_request_digest,
    },
    limits::SnapshotLoadLimits,
    program::{CheckedProgram, ContentOperand, OpV0},
    runtime::{
        EffectPathV0, FrameStateV0, PendingChoiceItemV0, PendingContent, PendingEffectV0,
        PendingInteractionV0, RuntimeStateV0, RuntimeStatusV0, VmStateV0, derive_interaction_id,
    },
    scene::SceneState,
    snapshot::{export_snapshot, validate_state},
    statechart::{DeferredStatechartWorkV0, InvokedFlowV0, StatechartStateV0},
    version::SNAPSHOT_SCHEMA_V0,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VersionRange {
    pub minimum: u16,
    pub maximum: u16,
}

impl VersionRange {
    pub const fn contains(self, version: u16) -> bool {
        version >= self.minimum && version <= self.maximum
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RelocationTable {
    /// Program-wide shorthand. Use `instruction_locations` when an ID is reused by multiple Flows.
    pub instructions: BTreeMap<InstructionId, InstructionId>,
    pub instruction_locations: BTreeMap<InstructionLocation, InstructionLocation>,
    pub flows: BTreeMap<FlowId, FlowId>,
    pub globals: BTreeMap<GlobalId, GlobalId>,
    pub locals: BTreeMap<LocalId, LocalId>,
    pub local_locations: BTreeMap<LocalLocation, LocalLocation>,
    pub choices: BTreeMap<ChoiceId, ChoiceId>,
    pub actions: BTreeMap<ActionId, ActionId>,
    pub states: BTreeMap<StateId, StateId>,
    pub histories: BTreeMap<HistoryId, HistoryId>,
    pub events: BTreeMap<EventTypeId, EventTypeId>,
    pub types: BTreeMap<TypeId, TypeId>,
    pub fields: BTreeMap<FieldId, FieldId>,
    pub variants: BTreeMap<VariantId, VariantId>,
    pub capabilities: BTreeMap<CapabilityId, CapabilityId>,
    pub dropped_globals: BTreeSet<GlobalId>,
    pub dropped_locals: BTreeSet<LocalId>,
    pub dropped_local_locations: BTreeSet<LocalLocation>,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct InstructionLocation {
    pub flow: FlowId,
    pub instruction: InstructionId,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct LocalLocation {
    pub flow: FlowId,
    pub local: LocalId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RecoveryPoint {
    pub checkpoint: RecoveryCheckpointId,
    pub target_flow: FlowId,
    pub target_instruction: InstructionId,
    pub preserve_globals: bool,
    pub crosses_external_effect_barrier: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MigrationDescriptor {
    pub id: MigrationId,
    pub from: ProgramArtifactId,
    pub to: ProgramArtifactId,
    pub accepted_snapshot_schemas: VersionRange,
    pub relocations: RelocationTable,
    /// Program-wide shorthand for source Programs whose Instruction IDs are globally unique.
    pub recovery_points: BTreeMap<InstructionId, RecoveryPoint>,
    pub recovery_locations: BTreeMap<InstructionLocation, RecoveryPoint>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MigrationOptions {
    pub allow_lossy_recovery: bool,
    pub allow_effect_rekey: bool,
    pub allow_cross_barrier_recovery: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RelocationRecord {
    pub kind: &'static str,
    pub from: String,
    pub to: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecoveryRecord {
    pub checkpoint: RecoveryCheckpointId,
    pub discarded_frames: usize,
    pub discarded_pending_interaction: bool,
    pub discarded_pending_effect: bool,
    pub globals_preserved: bool,
    pub crossed_external_effect_barrier: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MigrationReport {
    pub migration: Option<MigrationId>,
    pub from: Option<ProgramArtifactId>,
    pub to: Option<ProgramArtifactId>,
    pub relocations: Vec<RelocationRecord>,
    pub recoveries: Vec<RecoveryRecord>,
    pub reset_globals: Vec<GlobalId>,
    pub dropped_globals: Vec<GlobalId>,
    pub dropped_locals: Vec<LocalId>,
    pub rekeyed_pending_effect: bool,
}

impl MigrationReport {
    pub fn is_lossy(&self) -> bool {
        !self.recoveries.is_empty()
            || !self.reset_globals.is_empty()
            || !self.dropped_globals.is_empty()
            || !self.dropped_locals.is_empty()
    }
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum MigrationError {
    #[error("migration descriptor does not match the source and target Artifacts")]
    ArtifactMismatch,
    #[error("migration descriptor does not accept Snapshot schema {0}")]
    SnapshotSchema(u16),
    #[error("relocation table maps more than one source identity to the same target identity")]
    AmbiguousRelocation,
    #[error("unmapped {kind} identity {id}")]
    Unmapped { kind: &'static str, id: String },
    #[error("relocated {kind} identity {id} does not exist in the target Program")]
    MissingTarget { kind: &'static str, id: String },
    #[error("relocated Value has duplicate field {0}")]
    DuplicateField(FieldId),
    #[error("relocated slot Value kind differs from the target declaration")]
    ValueKind,
    #[error("a lossy recovery checkpoint requires explicit confirmation")]
    RecoveryConfirmationRequired,
    #[error("recovery would cross an external Effect barrier without explicit confirmation")]
    BarrierConfirmationRequired,
    #[error("pending external Effect identity would change without explicit confirmation")]
    EffectRekeyConfirmationRequired,
    #[error("recovery checkpoint cannot resume a Statechart invocation")]
    UnsupportedRecoveryState,
    #[error("migration cannot move a state to an older Program format")]
    FormatDowngrade,
    #[error("target Program has no scene instructions but the state has a non-default scene")]
    SceneWithoutTarget,
    #[error("migrated state failed target validation: {0}")]
    TargetValidation(String),
}

pub trait TrustedMigration: Send + Sync {
    fn descriptor(&self) -> &MigrationDescriptor;

    fn migrate_value(&self, value: &Value) -> Result<Value, MigrationError> {
        relocate_value(value, &self.descriptor().relocations)
    }
}

#[derive(Clone, Debug)]
pub struct DeclarativeMigration {
    descriptor: MigrationDescriptor,
}

impl DeclarativeMigration {
    pub fn new(descriptor: MigrationDescriptor) -> Result<Self, MigrationError> {
        validate_descriptor(&descriptor)?;
        Ok(Self { descriptor })
    }
}

impl TrustedMigration for DeclarativeMigration {
    fn descriptor(&self) -> &MigrationDescriptor {
        &self.descriptor
    }
}

/// The source state's Snapshot schema must be one the descriptor accepts. The target Program's
/// format decides the migrated state's schema; formats never move backwards (ADR 0018).
pub fn migrate_checked_state(
    migration: &dyn TrustedMigration,
    source: &RuntimeStateV0,
    source_program: &CheckedProgram,
    target_program: &CheckedProgram,
    options: MigrationOptions,
    limits: &SnapshotLoadLimits,
) -> Result<(RuntimeStateV0, MigrationReport), MigrationError> {
    let descriptor = migration.descriptor();
    validate_descriptor(descriptor)?;
    if descriptor.from != source_program.artifact_id()
        || descriptor.to != target_program.artifact_id()
        || source.program_artifact_id != descriptor.from
        || source_program.artifact().program_id != target_program.artifact().program_id
    {
        return Err(MigrationError::ArtifactMismatch);
    }
    let snapshot_schema = source.snapshot_schema.get();
    if !descriptor
        .accepted_snapshot_schemas
        .contains(snapshot_schema)
    {
        return Err(MigrationError::SnapshotSchema(snapshot_schema));
    }
    if target_program.format_version() < source_program.format_version() {
        return Err(MigrationError::FormatDowngrade);
    }

    let mut report = MigrationReport {
        migration: Some(descriptor.id),
        from: Some(descriptor.from),
        to: Some(descriptor.to),
        ..MigrationReport::default()
    };
    let mut next = source.clone();
    next.program_artifact_id = descriptor.to;
    next.semantics_version = target_program.artifact().semantics_version;
    next.snapshot_schema = target_program.snapshot_schema();
    next.scene = migrate_scene(source.scene.as_ref(), target_program)?;
    next.globals = migrate_globals(migration, source, target_program, &mut report)?;

    if let Some(recovery) = find_recovery(source, descriptor, target_program)? {
        if !options.allow_lossy_recovery {
            return Err(MigrationError::RecoveryConfirmationRequired);
        }
        if recovery.crosses_external_effect_barrier && !options.allow_cross_barrier_recovery {
            return Err(MigrationError::BarrierConfirmationRequired);
        }
        apply_recovery(&mut next, source, target_program, recovery, &mut report)?;
    } else {
        migrate_status(migration, &mut next, target_program, options, &mut report)?;
        migrate_statechart(migration, &mut next, target_program, &mut report)?;
    }

    validate_state(&next, target_program, limits)
        .map_err(|error| MigrationError::TargetValidation(error.to_string()))?;
    export_snapshot(&next).map_err(|error| MigrationError::TargetValidation(error.to_string()))?;
    Ok((next, report))
}

/// Schema 0 states always carry a scene; schema 1 states carry one exactly when the target
/// Program uses `ReconcileScene`, and only a default scene may be dropped.
fn migrate_scene(
    source: Option<&SceneState>,
    target: &CheckedProgram,
) -> Result<Option<SceneState>, MigrationError> {
    if target.snapshot_schema() == SNAPSHOT_SCHEMA_V0 || target.uses_scene() {
        return Ok(Some(source.cloned().unwrap_or_default()));
    }
    match source {
        Some(scene) if *scene != SceneState::default() => Err(MigrationError::SceneWithoutTarget),
        _ => Ok(None),
    }
}

fn validate_descriptor(descriptor: &MigrationDescriptor) -> Result<(), MigrationError> {
    if descriptor.accepted_snapshot_schemas.minimum > descriptor.accepted_snapshot_schemas.maximum
        || !injective(&descriptor.relocations.instructions)
        || !injective(&descriptor.relocations.instruction_locations)
        || !injective(&descriptor.relocations.flows)
        || !injective(&descriptor.relocations.globals)
        || !injective(&descriptor.relocations.locals)
        || !injective(&descriptor.relocations.local_locations)
        || !injective(&descriptor.relocations.choices)
        || !injective(&descriptor.relocations.actions)
        || !injective(&descriptor.relocations.states)
        || !injective(&descriptor.relocations.histories)
        || !injective(&descriptor.relocations.events)
        || !injective(&descriptor.relocations.types)
        || !injective(&descriptor.relocations.fields)
        || !injective(&descriptor.relocations.variants)
        || !injective(&descriptor.relocations.capabilities)
    {
        return Err(MigrationError::AmbiguousRelocation);
    }
    Ok(())
}

fn injective<K: Ord, V: Ord>(mapping: &BTreeMap<K, V>) -> bool {
    let values = mapping.values().collect::<BTreeSet<_>>();
    values.len() == mapping.len()
}

fn migrate_globals(
    migration: &dyn TrustedMigration,
    source: &RuntimeStateV0,
    target: &CheckedProgram,
    report: &mut MigrationReport,
) -> Result<BTreeMap<GlobalId, Value>, MigrationError> {
    let table = &migration.descriptor().relocations;
    let mut values = BTreeMap::new();
    for (source_id, value) in &source.globals {
        let target_id = table.globals.get(source_id).copied().unwrap_or(*source_id);
        let Some(declaration) = target.global(target_id) else {
            if table.dropped_globals.contains(source_id) {
                report.dropped_globals.push(*source_id);
                continue;
            }
            return Err(MigrationError::Unmapped {
                kind: "global",
                id: source_id.to_string(),
            });
        };
        record(report, "global", *source_id, target_id);
        let migrated = migration.migrate_value(value)?;
        if migrated.kind() != declaration.kind || values.insert(target_id, migrated).is_some() {
            return Err(MigrationError::ValueKind);
        }
    }
    for declaration in &target.artifact().globals {
        if let std::collections::btree_map::Entry::Vacant(entry) = values.entry(declaration.id) {
            entry.insert(declaration.default.clone());
            report.reset_globals.push(declaration.id);
        }
    }
    Ok(values)
}

fn find_recovery(
    source: &RuntimeStateV0,
    descriptor: &MigrationDescriptor,
    target: &CheckedProgram,
) -> Result<Option<RecoveryPoint>, MigrationError> {
    let frames = state_frames(source);
    for frame in frames {
        let target_flow = map_flow(frame.flow, target, &descriptor.relocations)?;
        let target_instruction = relocated_instruction(
            frame.flow,
            target_flow,
            frame.instruction,
            &descriptor.relocations,
        )?;
        if target
            .instruction(target_flow, target_instruction)
            .is_none()
        {
            let location = InstructionLocation {
                flow: frame.flow,
                instruction: frame.instruction,
            };
            return descriptor
                .recovery_locations
                .get(&location)
                .or_else(|| descriptor.recovery_points.get(&frame.instruction))
                .copied()
                .map(Some)
                .ok_or_else(|| MigrationError::Unmapped {
                    kind: "continuation",
                    id: frame.instruction.to_string(),
                });
        }
    }
    Ok(None)
}

fn apply_recovery(
    next: &mut RuntimeStateV0,
    source: &RuntimeStateV0,
    target: &CheckedProgram,
    recovery: RecoveryPoint,
    report: &mut MigrationReport,
) -> Result<(), MigrationError> {
    if next.statechart.is_some() {
        return Err(MigrationError::UnsupportedRecoveryState);
    }
    let flow = target
        .flow(recovery.target_flow)
        .ok_or_else(|| MigrationError::MissingTarget {
            kind: "recovery Flow",
            id: recovery.target_flow.to_string(),
        })?;
    if target
        .instruction(recovery.target_flow, recovery.target_instruction)
        .is_none()
    {
        return Err(MigrationError::MissingTarget {
            kind: "recovery instruction",
            id: recovery.target_instruction.to_string(),
        });
    }
    if !recovery.preserve_globals {
        next.globals = target
            .artifact()
            .globals
            .iter()
            .map(|declaration| (declaration.id, declaration.default.clone()))
            .collect();
    }
    let locals = flow
        .parameters
        .iter()
        .chain(&flow.locals)
        .map(|declaration| (declaration.id, declaration.default.clone()))
        .collect();
    next.status = RuntimeStatusV0::Ready {
        vm: VmStateV0 {
            frames: vec![FrameStateV0 {
                flow: recovery.target_flow,
                instruction: recovery.target_instruction,
                return_to: None,
                locals,
                evaluation_stack: Vec::new(),
            }],
        },
    };
    report.recoveries.push(RecoveryRecord {
        checkpoint: recovery.checkpoint,
        discarded_frames: state_frames(source).len(),
        discarded_pending_interaction: source.pending().is_some(),
        discarded_pending_effect: source.pending_effect().is_some(),
        globals_preserved: recovery.preserve_globals,
        crossed_external_effect_barrier: recovery.crosses_external_effect_barrier,
    });
    Ok(())
}

fn migrate_status(
    migration: &dyn TrustedMigration,
    state: &mut RuntimeStateV0,
    target: &CheckedProgram,
    options: MigrationOptions,
    report: &mut MigrationReport,
) -> Result<(), MigrationError> {
    let source_status = state.status.clone();
    state.status = match source_status {
        RuntimeStatusV0::Ready { vm } => RuntimeStatusV0::Ready {
            vm: migrate_vm(migration, vm, target, report)?,
        },
        RuntimeStatusV0::Awaiting { vm, pending } => {
            let vm = migrate_vm(migration, vm, target, report)?;
            let pending = migrate_pending_interaction(
                migration.descriptor(),
                state,
                &vm,
                pending,
                target,
                report,
            )?;
            RuntimeStatusV0::Awaiting { vm, pending }
        }
        RuntimeStatusV0::AwaitingEffect { vm, pending } => {
            let source_flow = vm.frames.last().map(|frame| frame.flow);
            let vm = migrate_vm(migration, vm, target, report)?;
            let target_flow = vm.frames.last().map(|frame| frame.flow);
            let pending = migrate_pending_effect(
                migration,
                pending,
                source_flow,
                target_flow,
                target,
                options,
                report,
            )?;
            RuntimeStatusV0::AwaitingEffect { vm, pending }
        }
        RuntimeStatusV0::AwaitingStatechartEffect { pending } => {
            RuntimeStatusV0::AwaitingStatechartEffect {
                pending: migrate_pending_effect(
                    migration, pending, None, None, target, options, report,
                )?,
            }
        }
        RuntimeStatusV0::Finished {
            result,
            final_frames,
        } => RuntimeStatusV0::Finished {
            result: migration.migrate_value(&result)?,
            final_frames: migrate_frames(migration, final_frames, target, report)?,
        },
        RuntimeStatusV0::StatechartStable => RuntimeStatusV0::StatechartStable,
        RuntimeStatusV0::StatechartFinished => RuntimeStatusV0::StatechartFinished,
    };
    Ok(())
}

fn migrate_vm(
    migration: &dyn TrustedMigration,
    vm: VmStateV0,
    target: &CheckedProgram,
    report: &mut MigrationReport,
) -> Result<VmStateV0, MigrationError> {
    Ok(VmStateV0 {
        frames: migrate_frames(migration, vm.frames, target, report)?,
    })
}

fn migrate_frames(
    migration: &dyn TrustedMigration,
    frames: Vec<FrameStateV0>,
    target: &CheckedProgram,
    report: &mut MigrationReport,
) -> Result<Vec<FrameStateV0>, MigrationError> {
    let table = &migration.descriptor().relocations;
    let source_flows = frames.iter().map(|frame| frame.flow).collect::<Vec<_>>();
    let mut migrated = Vec::with_capacity(frames.len());
    for (index, frame) in frames.into_iter().enumerate() {
        let source_flow = frame.flow;
        let flow_id = map_flow(source_flow, target, table)?;
        let instruction = map_instruction(frame.instruction, source_flow, flow_id, target, table)?;
        let return_to = match frame.return_to {
            Some(return_to) => {
                let caller_source = source_flows.get(index.saturating_sub(1)).copied().ok_or(
                    MigrationError::Unmapped {
                        kind: "caller frame",
                        id: return_to.to_string(),
                    },
                )?;
                let caller = map_flow(caller_source, target, table)?;
                Some(map_instruction(
                    return_to,
                    caller_source,
                    caller,
                    target,
                    table,
                )?)
            }
            None => None,
        };
        let target_flow = target
            .flow(flow_id)
            .ok_or_else(|| MigrationError::MissingTarget {
                kind: "flow",
                id: flow_id.to_string(),
            })?;
        let mut locals = BTreeMap::new();
        for (source_id, value) in frame.locals {
            let source_location = LocalLocation {
                flow: source_flow,
                local: source_id,
            };
            let target_id = if let Some(location) = table.local_locations.get(&source_location) {
                if location.flow != flow_id {
                    return Err(MigrationError::Unmapped {
                        kind: "local Flow",
                        id: source_id.to_string(),
                    });
                }
                location.local
            } else {
                table.locals.get(&source_id).copied().unwrap_or(source_id)
            };
            let declaration = target_flow
                .parameters
                .iter()
                .chain(&target_flow.locals)
                .find(|declaration| declaration.id == target_id);
            let Some(declaration) = declaration else {
                if table.dropped_local_locations.contains(&source_location)
                    || table.dropped_locals.contains(&source_id)
                {
                    report.dropped_locals.push(source_id);
                    continue;
                }
                return Err(MigrationError::Unmapped {
                    kind: "local",
                    id: source_id.to_string(),
                });
            };
            record(report, "local", source_id, target_id);
            let value = migration.migrate_value(&value)?;
            if value.kind() != declaration.kind || locals.insert(target_id, value).is_some() {
                return Err(MigrationError::ValueKind);
            }
        }
        for declaration in target_flow.parameters.iter().chain(&target_flow.locals) {
            locals
                .entry(declaration.id)
                .or_insert_with(|| declaration.default.clone());
        }
        record(report, "flow", frame.flow, flow_id);
        record(report, "instruction", frame.instruction, instruction);
        migrated.push(FrameStateV0 {
            flow: flow_id,
            instruction,
            return_to,
            locals,
            evaluation_stack: frame
                .evaluation_stack
                .iter()
                .map(|value| migration.migrate_value(value))
                .collect::<Result<_, _>>()?,
        });
    }
    Ok(migrated)
}

fn migrate_pending_interaction(
    descriptor: &MigrationDescriptor,
    state: &RuntimeStateV0,
    vm: &VmStateV0,
    pending: PendingInteractionV0,
    target: &CheckedProgram,
    report: &mut MigrationReport,
) -> Result<PendingInteractionV0, MigrationError> {
    let frame = vm.frames.last().ok_or_else(|| MigrationError::Unmapped {
        kind: "pending interaction frame",
        id: "<missing>".to_owned(),
    })?;
    let source_flow = state_frames(state)
        .last()
        .map(|frame| frame.flow)
        .ok_or_else(|| MigrationError::Unmapped {
            kind: "pending interaction source frame",
            id: "<missing>".to_owned(),
        })?;
    match pending {
        PendingInteractionV0::Say {
            origin_instruction,
            origin_parent_state,
            origin_input_digest,
            occurrence,
            ..
        } => {
            let origin = map_instruction(
                origin_instruction,
                source_flow,
                frame.flow,
                target,
                &descriptor.relocations,
            )?;
            let OpV0::Say {
                speaker,
                text,
                next,
            } = target
                .instruction(frame.flow, origin)
                .map(|record| &record.op)
                .ok_or_else(|| MigrationError::MissingTarget {
                    kind: "Say",
                    id: origin.to_string(),
                })?
            else {
                return Err(MigrationError::MissingTarget {
                    kind: "Say",
                    id: origin.to_string(),
                });
            };
            record(report, "interaction", origin_instruction, origin);
            Ok(PendingInteractionV0::Say {
                interaction_id: derive_interaction_id(
                    state.execution_id,
                    origin_parent_state,
                    origin_input_digest,
                    origin,
                    occurrence,
                    0,
                ),
                origin_instruction: origin,
                origin_parent_state,
                origin_input_digest,
                occurrence,
                speaker: speaker
                    .map(|operand| target_content(target, operand))
                    .transpose()?,
                text: target_content(target, *text)?,
                resume_to: *next,
            })
        }
        PendingInteractionV0::Choice {
            origin_instruction,
            origin_parent_state,
            origin_input_digest,
            occurrence,
            offered,
            ..
        } => {
            let origin = map_instruction(
                origin_instruction,
                source_flow,
                frame.flow,
                target,
                &descriptor.relocations,
            )?;
            let OpV0::Choice { prompt, choices } = target
                .instruction(frame.flow, origin)
                .map(|record| &record.op)
                .ok_or_else(|| MigrationError::MissingTarget {
                    kind: "Choice",
                    id: origin.to_string(),
                })?
            else {
                return Err(MigrationError::MissingTarget {
                    kind: "Choice",
                    id: origin.to_string(),
                });
            };
            let offered_ids = offered
                .into_iter()
                .map(|item| {
                    descriptor
                        .relocations
                        .choices
                        .get(&item.id)
                        .copied()
                        .unwrap_or(item.id)
                })
                .collect::<BTreeSet<_>>();
            let mut migrated_offered = Vec::new();
            for choice in choices {
                if offered_ids.contains(&choice.id) {
                    migrated_offered.push(PendingChoiceItemV0 {
                        id: choice.id,
                        label: target_content(target, choice.label)?,
                        target: choice.target,
                    });
                }
            }
            if migrated_offered.len() != offered_ids.len() {
                return Err(MigrationError::Unmapped {
                    kind: "choice",
                    id: "pending offered set".to_owned(),
                });
            }
            record(report, "interaction", origin_instruction, origin);
            Ok(PendingInteractionV0::Choice {
                interaction_id: derive_interaction_id(
                    state.execution_id,
                    origin_parent_state,
                    origin_input_digest,
                    origin,
                    occurrence,
                    1,
                ),
                origin_instruction: origin,
                origin_parent_state,
                origin_input_digest,
                occurrence,
                prompt: prompt
                    .map(|operand| target_content(target, operand))
                    .transpose()?,
                offered: migrated_offered,
            })
        }
    }
}

fn migrate_pending_effect(
    migration: &dyn TrustedMigration,
    mut pending: PendingEffectV0,
    source_flow: Option<FlowId>,
    target_flow: Option<FlowId>,
    target: &CheckedProgram,
    options: MigrationOptions,
    report: &mut MigrationReport,
) -> Result<PendingEffectV0, MigrationError> {
    let original_effect = pending.request.id;
    let table = &migration.descriptor().relocations;
    pending.request.capability = table
        .capabilities
        .get(&pending.request.capability)
        .cloned()
        .unwrap_or_else(|| pending.request.capability.clone());
    let capability = target
        .capability(&pending.request.capability)
        .ok_or_else(|| MigrationError::MissingTarget {
            kind: "capability",
            id: pending.request.capability.to_string(),
        })?;
    pending.request.capability_version = capability.version;
    pending.request.delivery = capability.delivery;
    pending.request.rewind = relocate_rewind(&capability.rewind, table);
    pending.request.payload = migration.migrate_value(&pending.request.payload)?;
    if !capability.request_schema.accepts(&pending.request.payload) {
        return Err(MigrationError::ValueKind);
    }
    let payload = effect_payload_digest(&pending.request.payload);
    let request = effect_request_digest(
        &pending.request.capability,
        pending.request.capability_version,
        payload,
        pending.request.delivery,
        &pending.request.rewind,
    );
    pending.request.payload_digest = payload;
    pending.request.request_digest = request;
    pending.path = match pending.path {
        EffectPathV0::Flow { origin, resume_to } => {
            let source_flow = source_flow.ok_or_else(|| MigrationError::Unmapped {
                kind: "Effect source Flow",
                id: origin.to_string(),
            })?;
            let flow = target_flow.ok_or_else(|| MigrationError::Unmapped {
                kind: "Effect instruction",
                id: origin.to_string(),
            })?;
            let origin = map_instruction(origin, source_flow, flow, target, table)?;
            let resume_to = map_instruction(resume_to, source_flow, flow, target, table)?;
            pending.request.id = derive_effect_id(
                pending.request.execution,
                pending.origin_parent_commit,
                pending.origin_input_digest,
                origin,
                pending.occurrence,
                request,
            );
            EffectPathV0::Flow { origin, resume_to }
        }
        EffectPathV0::Statechart {
            site,
            response_to,
            response_event,
        } => {
            let site = map_action(site, target, table)?;
            let response_to = response_to
                .map(|id| map_global(id, target, table))
                .transpose()?;
            let response_event = response_event
                .map(|id| map_event(id, target, table))
                .transpose()?;
            pending.request.id = derive_statechart_effect_id(
                pending.request.execution,
                pending.origin_parent_commit,
                pending.origin_input_digest,
                site,
                pending.occurrence,
                request,
            );
            EffectPathV0::Statechart {
                site,
                response_to,
                response_event,
            }
        }
    };
    if pending.request.id != original_effect {
        if !options.allow_effect_rekey {
            return Err(MigrationError::EffectRekeyConfirmationRequired);
        }
        report.rekeyed_pending_effect = true;
    }
    Ok(pending)
}

fn migrate_statechart(
    migration: &dyn TrustedMigration,
    state: &mut RuntimeStateV0,
    target: &CheckedProgram,
    report: &mut MigrationReport,
) -> Result<(), MigrationError> {
    let Some(source) = state.statechart.take() else {
        return Ok(());
    };
    let table = &migration.descriptor().relocations;
    let active = source
        .active
        .into_iter()
        .map(|id| map_state(id, target, table))
        .collect::<Result<_, _>>()?;
    let completed = source
        .completed
        .into_iter()
        .map(|id| map_state(id, target, table))
        .collect::<Result<_, _>>()?;
    let mut history = BTreeMap::new();
    for (id, values) in source.history {
        let target_id = map_history(id, target, table)?;
        let values = values
            .into_iter()
            .map(|state| map_state(state, target, table))
            .collect::<Result<Vec<_>, _>>()?;
        if history.insert(target_id, values).is_some() {
            return Err(MigrationError::AmbiguousRelocation);
        }
    }
    let internal_queue = map_events(source.internal_queue, target, table)?;
    let deferred_events = map_events(source.deferred_events, target, table)?;
    let mut deferred_work = VecDeque::new();
    for work in source.deferred_work {
        deferred_work.push_back(match work {
            DeferredStatechartWorkV0::Flow {
                site,
                owner,
                flow,
                result_to,
                done_event,
            } => DeferredStatechartWorkV0::Flow {
                site: map_action(site, target, table)?,
                owner: owner.map(|id| map_state(id, target, table)).transpose()?,
                flow: map_flow(flow, target, table)?,
                result_to: result_to
                    .map(|id| map_global(id, target, table))
                    .transpose()?,
                done_event: done_event
                    .map(|id| map_event(id, target, table))
                    .transpose()?,
            },
            DeferredStatechartWorkV0::Effect {
                site,
                owner,
                capability,
                payload,
                response_to,
                response_event,
            } => DeferredStatechartWorkV0::Effect {
                site: map_action(site, target, table)?,
                owner: owner.map(|id| map_state(id, target, table)).transpose()?,
                capability: table
                    .capabilities
                    .get(&capability)
                    .cloned()
                    .unwrap_or(capability),
                payload: migration.migrate_value(&payload)?,
                response_to: response_to
                    .map(|id| map_global(id, target, table))
                    .transpose()?,
                response_event: response_event
                    .map(|id| map_event(id, target, table))
                    .transpose()?,
            },
        });
    }
    let invocation = source
        .invocation
        .map(|invocation| migrate_invocation(invocation, target, table))
        .transpose()?;
    state.statechart = Some(StatechartStateV0 {
        active,
        history,
        completed,
        internal_queue,
        deferred_events,
        deferred_work,
        invocation,
    });
    for (from, to) in &table.states {
        record(report, "state", *from, *to);
    }
    Ok(())
}

fn migrate_invocation(
    invocation: InvokedFlowV0,
    target: &CheckedProgram,
    table: &RelocationTable,
) -> Result<InvokedFlowV0, MigrationError> {
    Ok(InvokedFlowV0 {
        site: map_action(invocation.site, target, table)?,
        flow: map_flow(invocation.flow, target, table)?,
        result_to: invocation
            .result_to
            .map(|id| map_global(id, target, table))
            .transpose()?,
        done_event: invocation
            .done_event
            .map(|id| map_event(id, target, table))
            .transpose()?,
        deferred_events: map_events(invocation.deferred_events, target, table)?,
    })
}

fn map_events(
    values: VecDeque<EventTypeId>,
    target: &CheckedProgram,
    table: &RelocationTable,
) -> Result<VecDeque<EventTypeId>, MigrationError> {
    values
        .into_iter()
        .map(|id| map_event(id, target, table))
        .collect()
}

fn relocate_value(value: &Value, table: &RelocationTable) -> Result<Value, MigrationError> {
    match value {
        Value::List(values) => Ok(Value::List(
            values
                .iter()
                .map(|value| relocate_value(value, table))
                .collect::<Result<_, _>>()?,
        )),
        Value::Record(fields) => {
            let mut migrated = BTreeMap::new();
            for (id, value) in fields {
                let id = table.fields.get(id).copied().unwrap_or(*id);
                if migrated.insert(id, relocate_value(value, table)?).is_some() {
                    return Err(MigrationError::DuplicateField(id));
                }
            }
            Ok(Value::Record(migrated))
        }
        Value::Variant {
            type_id,
            variant_id,
            payload,
        } => Ok(Value::Variant {
            type_id: table.types.get(type_id).copied().unwrap_or(*type_id),
            variant_id: table
                .variants
                .get(variant_id)
                .copied()
                .unwrap_or(*variant_id),
            payload: Box::new(relocate_value(payload, table)?),
        }),
        other => Ok(other.clone()),
    }
}

fn map_flow(
    id: FlowId,
    target: &CheckedProgram,
    table: &RelocationTable,
) -> Result<FlowId, MigrationError> {
    let mapped = table.flows.get(&id).copied().unwrap_or(id);
    target
        .flow(mapped)
        .map(|_| mapped)
        .ok_or_else(|| MigrationError::MissingTarget {
            kind: "flow",
            id: mapped.to_string(),
        })
}

fn map_instruction(
    id: InstructionId,
    source_flow: FlowId,
    target_flow: FlowId,
    target: &CheckedProgram,
    table: &RelocationTable,
) -> Result<InstructionId, MigrationError> {
    let mapped = relocated_instruction(source_flow, target_flow, id, table)?;
    target
        .instruction(target_flow, mapped)
        .map(|_| mapped)
        .ok_or_else(|| MigrationError::MissingTarget {
            kind: "instruction",
            id: mapped.to_string(),
        })
}

fn relocated_instruction(
    source_flow: FlowId,
    target_flow: FlowId,
    id: InstructionId,
    table: &RelocationTable,
) -> Result<InstructionId, MigrationError> {
    let source = InstructionLocation {
        flow: source_flow,
        instruction: id,
    };
    if let Some(location) = table.instruction_locations.get(&source) {
        if location.flow != target_flow {
            return Err(MigrationError::Unmapped {
                kind: "instruction Flow",
                id: id.to_string(),
            });
        }
        Ok(location.instruction)
    } else {
        Ok(table.instructions.get(&id).copied().unwrap_or(id))
    }
}

fn map_global(
    id: GlobalId,
    target: &CheckedProgram,
    table: &RelocationTable,
) -> Result<GlobalId, MigrationError> {
    let mapped = table.globals.get(&id).copied().unwrap_or(id);
    target
        .global(mapped)
        .map(|_| mapped)
        .ok_or_else(|| MigrationError::MissingTarget {
            kind: "global",
            id: mapped.to_string(),
        })
}

fn map_state(
    id: StateId,
    target: &CheckedProgram,
    table: &RelocationTable,
) -> Result<StateId, MigrationError> {
    let mapped = table.states.get(&id).copied().unwrap_or(id);
    target
        .state(mapped)
        .map(|_| mapped)
        .ok_or_else(|| MigrationError::MissingTarget {
            kind: "state",
            id: mapped.to_string(),
        })
}

fn map_history(
    id: HistoryId,
    target: &CheckedProgram,
    table: &RelocationTable,
) -> Result<HistoryId, MigrationError> {
    let mapped = table.histories.get(&id).copied().unwrap_or(id);
    target
        .history(mapped)
        .map(|_| mapped)
        .ok_or_else(|| MigrationError::MissingTarget {
            kind: "history",
            id: mapped.to_string(),
        })
}

fn map_event(
    id: EventTypeId,
    target: &CheckedProgram,
    table: &RelocationTable,
) -> Result<EventTypeId, MigrationError> {
    let mapped = table.events.get(&id).copied().unwrap_or(id);
    target
        .statechart()
        .filter(|chart| chart.events.binary_search(&mapped).is_ok())
        .map(|_| mapped)
        .ok_or_else(|| MigrationError::MissingTarget {
            kind: "event",
            id: mapped.to_string(),
        })
}

fn map_action(
    id: ActionId,
    target: &CheckedProgram,
    table: &RelocationTable,
) -> Result<ActionId, MigrationError> {
    let mapped = table.actions.get(&id).copied().unwrap_or(id);
    target
        .statechart_action(mapped)
        .map(|_| mapped)
        .ok_or_else(|| MigrationError::MissingTarget {
            kind: "action",
            id: mapped.to_string(),
        })
}

fn relocate_rewind(policy: &RewindPolicy, table: &RelocationTable) -> RewindPolicy {
    match policy {
        RewindPolicy::Compensatable { capability } => RewindPolicy::Compensatable {
            capability: table
                .capabilities
                .get(capability)
                .cloned()
                .unwrap_or_else(|| capability.clone()),
        },
        other => other.clone(),
    }
}

fn state_frames(state: &RuntimeStateV0) -> &[FrameStateV0] {
    match &state.status {
        RuntimeStatusV0::Ready { vm }
        | RuntimeStatusV0::Awaiting { vm, .. }
        | RuntimeStatusV0::AwaitingEffect { vm, .. } => &vm.frames,
        RuntimeStatusV0::Finished { final_frames, .. } => final_frames,
        RuntimeStatusV0::StatechartStable
        | RuntimeStatusV0::AwaitingStatechartEffect { .. }
        | RuntimeStatusV0::StatechartFinished => &[],
    }
}

/// The pending record of a target operand: its text in format 0, its content index in format 1.
fn target_content(
    program: &CheckedProgram,
    operand: ContentOperand,
) -> Result<PendingContent, MigrationError> {
    match operand {
        ContentOperand::Constant(index) => match program.constant(index) {
            Some(Value::String(value)) => Ok(PendingContent::LegacyText(value.clone())),
            _ => Err(MigrationError::ValueKind),
        },
        ContentOperand::Content(index) => program
            .content(index)
            .map(|_| PendingContent::Content(index))
            .ok_or_else(|| MigrationError::MissingTarget {
                kind: "content entry",
                id: index.0.to_string(),
            }),
    }
}

fn record(
    report: &mut MigrationReport,
    kind: &'static str,
    from: impl ToString,
    to: impl ToString,
) {
    let from = from.to_string();
    let to = to.to_string();
    if from != to {
        report.relocations.push(RelocationRecord { kind, from, to });
    }
}

//! Trusted migration path selection and atomic persistence.

use std::sync::Arc;

use narrata_core::{
    CheckedProgram, CommitId, MigrationId, ProgramArtifactId, RuntimeStateV0, SnapshotId,
    codec::ObjectKind,
    limits::SnapshotLoadLimits,
    migration::{MigrationOptions, MigrationReport, TrustedMigration, migrate_checked_state},
    program::encode_program_artifact,
    snapshot::export_snapshot,
};
use thiserror::Error;

use crate::{
    COMMIT_SCHEMA_V1, CheckedObject, CommitCauseV1, CommitTransaction, CommitV1, CoordinatorError,
    RefKey, RefMutation, RefRevision, SaveStore, StoreError, load_commit,
};

pub type MigrationPath = narrata_history::migration::MigrationPath<MigrationId, ProgramArtifactId>;
pub type MigrationRegistryError =
    narrata_history::migration::MigrationRegistryError<MigrationId, ProgramArtifactId>;

#[derive(Clone, Debug, Default)]
pub struct ProgramRegistry {
    programs: narrata_history::migration::ArtifactRegistry<ProgramArtifactId, Arc<CheckedProgram>>,
}
impl ProgramRegistry {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn register(&mut self, program: Arc<CheckedProgram>) -> bool {
        self.programs.register(program.artifact_id(), program)
    }
    pub fn get(&self, id: ProgramArtifactId) -> Option<&Arc<CheckedProgram>> {
        self.programs.get(id)
    }
}
#[derive(Default)]
pub struct MigrationRegistry {
    migrations: narrata_history::migration::MigrationRegistry<
        MigrationId,
        ProgramArtifactId,
        Arc<dyn TrustedMigration>,
    >,
}
impl MigrationRegistry {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn register(
        &mut self,
        migration: Arc<dyn TrustedMigration>,
    ) -> Result<(), MigrationRegistryError> {
        let descriptor = migration.descriptor();
        self.migrations
            .register(descriptor.id, descriptor.from, descriptor.to, migration)
    }
    pub fn get(&self, id: MigrationId) -> Option<&Arc<dyn TrustedMigration>> {
        self.migrations.get(id)
    }
    pub fn inspect(
        &self,
        from: ProgramArtifactId,
        to: ProgramArtifactId,
        explicit: Option<&[MigrationId]>,
    ) -> Result<MigrationPath, MigrationRegistryError> {
        self.migrations.inspect(from, to, explicit)
    }
}

#[derive(Clone, Debug)]
pub struct MigrationDryRun {
    pub source_commit: CommitId,
    pub source_artifact: ProgramArtifactId,
    pub target_artifact: ProgramArtifactId,
    pub path: MigrationPath,
    pub reports: Vec<MigrationReport>,
    pub final_state: RuntimeStateV0,
}

#[derive(Clone, Debug)]
pub struct AppliedMigration {
    pub source_commit: CommitId,
    pub commit: CommitId,
    pub commits: Vec<CommitId>,
    pub path: MigrationPath,
    pub reports: Vec<MigrationReport>,
}

#[derive(Debug, Error)]
pub enum MigrationStoreError {
    #[error(transparent)]
    Registry(#[from] MigrationRegistryError),
    #[error(transparent)]
    Coordinator(#[from] CoordinatorError),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("Program Artifact {0} is not registered")]
    MissingProgram(ProgramArtifactId),
    #[error("migration failed: {0}")]
    Migration(#[from] narrata_core::MigrationError),
    #[error("snapshot object construction failed: {0}")]
    Snapshot(String),
    #[error("migration path ended at a different target Artifact")]
    WrongTarget,
}

#[allow(clippy::too_many_arguments)]
pub fn dry_run_migration(
    store: &impl SaveStore,
    registry: &MigrationRegistry,
    programs: &ProgramRegistry,
    source_commit: CommitId,
    target: ProgramArtifactId,
    explicit_path: Option<&[MigrationId]>,
    options: MigrationOptions,
    limits: &SnapshotLoadLimits,
) -> Result<MigrationDryRun, MigrationStoreError> {
    let source_record = crate::read_commit(store, source_commit)?;
    let source_program = programs
        .get(source_record.program)
        .ok_or(MigrationStoreError::MissingProgram(source_record.program))?;
    let loaded = load_commit(store, source_commit, source_program)?;
    let path = registry.inspect(source_record.program, target, explicit_path)?;
    let (final_state, reports) = execute_path(
        registry,
        programs,
        path.ids.as_slice(),
        loaded.state,
        source_program.clone(),
        options,
        limits,
    )?;
    if final_state.program_artifact_id != target {
        return Err(MigrationStoreError::WrongTarget);
    }
    Ok(MigrationDryRun {
        source_commit,
        source_artifact: source_record.program,
        target_artifact: target,
        path,
        reports,
        final_state,
    })
}

#[allow(clippy::too_many_arguments)]
pub fn apply_migration(
    store: &mut impl SaveStore,
    registry: &MigrationRegistry,
    programs: &ProgramRegistry,
    source_commit: CommitId,
    target: ProgramArtifactId,
    explicit_path: Option<&[MigrationId]>,
    options: MigrationOptions,
    target_ref: RefKey,
    expected_ref: Option<RefRevision>,
    observed_at: u64,
    limits: &SnapshotLoadLimits,
) -> Result<AppliedMigration, MigrationStoreError> {
    let source_record = crate::read_commit(store, source_commit)?;
    let source_program = programs
        .get(source_record.program)
        .ok_or(MigrationStoreError::MissingProgram(source_record.program))?;
    let loaded = load_commit(store, source_commit, source_program)?;
    let path = registry.inspect(source_record.program, target, explicit_path)?;

    let mut current_parent = source_commit;
    let mut objects = Vec::new();
    let mut commits = Vec::new();
    let (current_state, reports) = narrata_history::migration::run_path(
        &registry.migrations,
        &path,
        loaded.state,
        |artifact| {
            programs
                .get(artifact)
                .cloned()
                .ok_or(MigrationStoreError::MissingProgram(artifact))
        },
        |migration, source_program, target_program, state| {
            migrate_checked_state(
                migration.as_ref(),
                state,
                source_program,
                target_program,
                options,
                limits,
            )
            .map_err(MigrationStoreError::from)
        },
        |id, to, target_program, next_state, _report| {
            let snapshot_bytes = export_snapshot(next_state)
                .map_err(|error| MigrationStoreError::Snapshot(error.to_string()))?;
            let snapshot_object = CheckedObject::from_bytes(
                &snapshot_bytes,
                ObjectKind::Snapshot,
                next_state.snapshot_schema.get(),
                limits.decode.max_envelope_bytes,
            )
            .map_err(|error| MigrationStoreError::Snapshot(error.to_string()))?;
            let snapshot = SnapshotId::from_bytes(*snapshot_object.id().as_bytes());
            let program_bytes = encode_program_artifact(target_program.artifact());
            let program_object = CheckedObject::from_bytes(
                &program_bytes,
                ObjectKind::Program,
                target_program.format_version().get(),
                limits.decode.max_envelope_bytes,
            )
            .map_err(|error| MigrationStoreError::Snapshot(error.to_string()))?;
            let commit = CommitV1 {
                parent: Some(current_parent),
                execution: source_record.execution,
                program: to,
                snapshot,
                cause: CommitCauseV1::Migration(id),
                ledger_fence: source_record.ledger_fence,
                turn: next_state.turn,
            };
            let commit_object = commit.to_object();
            debug_assert_eq!(commit_object.schema(), COMMIT_SCHEMA_V1);
            let commit_id = CommitId::from_bytes(*commit_object.id().as_bytes());
            objects.extend([program_object, snapshot_object, commit_object]);
            commits.push(commit_id);
            current_parent = commit_id;
            Ok(())
        },
    )?;
    if current_state.program_artifact_id != target {
        return Err(MigrationStoreError::WrongTarget);
    }
    let final_commit = commits
        .last()
        .copied()
        .ok_or(MigrationStoreError::WrongTarget)?;
    store.commit_atomic(CommitTransaction {
        objects,
        refs: vec![RefMutation {
            key: target_ref,
            expected: expected_ref,
            next: Some(final_commit),
        }],
        observed_at,
        ..CommitTransaction::default()
    })?;
    Ok(AppliedMigration {
        source_commit,
        commit: final_commit,
        commits,
        path,
        reports,
    })
}

#[allow(clippy::too_many_arguments)]
fn execute_path(
    registry: &MigrationRegistry,
    programs: &ProgramRegistry,
    ids: &[MigrationId],
    state: RuntimeStateV0,
    program: Arc<CheckedProgram>,
    options: MigrationOptions,
    limits: &SnapshotLoadLimits,
) -> Result<(RuntimeStateV0, Vec<MigrationReport>), MigrationStoreError> {
    let target = ids
        .last()
        .and_then(|id| registry.get(*id))
        .map(|m| m.descriptor().to)
        .ok_or(MigrationStoreError::WrongTarget)?;
    let path = registry.inspect(program.artifact_id(), target, Some(ids))?;
    narrata_history::migration::run_path(
        &registry.migrations,
        &path,
        state,
        |artifact| {
            programs
                .get(artifact)
                .cloned()
                .ok_or(MigrationStoreError::MissingProgram(artifact))
        },
        |migration, source, target, state| {
            migrate_checked_state(migration.as_ref(), state, source, target, options, limits)
                .map_err(MigrationStoreError::from)
        },
        |_, _, _, _, _| Ok(()),
    )
}

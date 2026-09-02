//! Trusted migration path selection and atomic persistence.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

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

const MAX_MIGRATION_PATH_LENGTH: usize = 64;
const MAX_DISCOVERED_PATHS: usize = 2;

#[derive(Clone, Debug, Default)]
pub struct ProgramRegistry {
    programs: BTreeMap<ProgramArtifactId, Arc<CheckedProgram>>,
}

impl ProgramRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, program: Arc<CheckedProgram>) -> bool {
        self.programs
            .insert(program.artifact_id(), program)
            .is_none()
    }

    pub fn get(&self, id: ProgramArtifactId) -> Option<&Arc<CheckedProgram>> {
        self.programs.get(&id)
    }
}

#[derive(Default)]
pub struct MigrationRegistry {
    migrations: BTreeMap<MigrationId, Arc<dyn TrustedMigration>>,
    outgoing: BTreeMap<ProgramArtifactId, Vec<MigrationId>>,
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
        if descriptor.from == descriptor.to {
            return Err(MigrationRegistryError::SelfEdge(descriptor.id));
        }
        if self.migrations.contains_key(&descriptor.id) {
            return Err(MigrationRegistryError::Duplicate(descriptor.id));
        }
        self.outgoing
            .entry(descriptor.from)
            .or_default()
            .push(descriptor.id);
        if let Some(values) = self.outgoing.get_mut(&descriptor.from) {
            values.sort_unstable();
        }
        self.migrations.insert(descriptor.id, migration);
        Ok(())
    }

    pub fn get(&self, id: MigrationId) -> Option<&Arc<dyn TrustedMigration>> {
        self.migrations.get(&id)
    }

    pub fn inspect(
        &self,
        from: ProgramArtifactId,
        to: ProgramArtifactId,
        explicit: Option<&[MigrationId]>,
    ) -> Result<MigrationPath, MigrationRegistryError> {
        if let Some(ids) = explicit {
            return self.validate_explicit(from, to, ids);
        }
        let mut paths = Vec::new();
        let mut visited = BTreeSet::from([from]);
        let mut current = Vec::new();
        self.discover(from, to, &mut visited, &mut current, &mut paths);
        match paths.len() {
            0 => Err(MigrationRegistryError::NoPath { from, to }),
            1 => paths
                .pop()
                .ok_or(MigrationRegistryError::NoPath { from, to }),
            _ => Err(MigrationRegistryError::Ambiguous { from, to }),
        }
    }

    fn validate_explicit(
        &self,
        from: ProgramArtifactId,
        to: ProgramArtifactId,
        ids: &[MigrationId],
    ) -> Result<MigrationPath, MigrationRegistryError> {
        if ids.len() > MAX_MIGRATION_PATH_LENGTH {
            return Err(MigrationRegistryError::PathLimit);
        }
        let mut artifacts = vec![from];
        let mut current = from;
        for id in ids {
            let migration = self
                .migrations
                .get(id)
                .ok_or(MigrationRegistryError::Unknown(*id))?;
            let descriptor = migration.descriptor();
            if descriptor.from != current {
                return Err(MigrationRegistryError::Disconnected(*id));
            }
            current = descriptor.to;
            artifacts.push(current);
        }
        if current != to || ids.is_empty() {
            return Err(MigrationRegistryError::NoPath { from, to });
        }
        Ok(MigrationPath {
            ids: ids.to_vec(),
            artifacts,
        })
    }

    fn discover(
        &self,
        current_artifact: ProgramArtifactId,
        target: ProgramArtifactId,
        visited: &mut BTreeSet<ProgramArtifactId>,
        current_path: &mut Vec<MigrationId>,
        paths: &mut Vec<MigrationPath>,
    ) {
        if paths.len() >= MAX_DISCOVERED_PATHS || current_path.len() >= MAX_MIGRATION_PATH_LENGTH {
            return;
        }
        let Some(edges) = self.outgoing.get(&current_artifact) else {
            return;
        };
        for id in edges {
            let Some(migration) = self.migrations.get(id) else {
                continue;
            };
            let next = migration.descriptor().to;
            if !visited.insert(next) {
                continue;
            }
            current_path.push(*id);
            if next == target {
                let mut artifacts =
                    vec![migration_path_start(self, current_path).unwrap_or(current_artifact)];
                let mut cursor = *artifacts.first().unwrap_or(&current_artifact);
                for migration_id in current_path.iter() {
                    let Some(step) = self.migrations.get(migration_id) else {
                        break;
                    };
                    if step.descriptor().from != cursor {
                        break;
                    }
                    cursor = step.descriptor().to;
                    artifacts.push(cursor);
                }
                paths.push(MigrationPath {
                    ids: current_path.clone(),
                    artifacts,
                });
            } else {
                self.discover(next, target, visited, current_path, paths);
            }
            current_path.pop();
            visited.remove(&next);
            if paths.len() >= MAX_DISCOVERED_PATHS {
                return;
            }
        }
    }
}

fn migration_path_start(
    registry: &MigrationRegistry,
    ids: &[MigrationId],
) -> Option<ProgramArtifactId> {
    ids.first()
        .and_then(|id| registry.migrations.get(id))
        .map(|migration| migration.descriptor().from)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MigrationPath {
    pub ids: Vec<MigrationId>,
    pub artifacts: Vec<ProgramArtifactId>,
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum MigrationRegistryError {
    #[error("migration {0} is already registered")]
    Duplicate(MigrationId),
    #[error("migration {0} has identical source and target Artifacts")]
    SelfEdge(MigrationId),
    #[error("migration {0} is not registered")]
    Unknown(MigrationId),
    #[error("migration {0} does not continue the explicitly selected path")]
    Disconnected(MigrationId),
    #[error("no migration path exists from {from} to {to}")]
    NoPath {
        from: ProgramArtifactId,
        to: ProgramArtifactId,
    },
    #[error("multiple migration paths exist from {from} to {to}; select one explicitly")]
    Ambiguous {
        from: ProgramArtifactId,
        to: ProgramArtifactId,
    },
    #[error("migration path exceeds the configured maximum length")]
    PathLimit,
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

    let mut current_state = loaded.state;
    let mut current_program = source_program.clone();
    let mut current_parent = source_commit;
    let mut objects = Vec::new();
    let mut commits = Vec::new();
    let mut reports = Vec::new();
    for id in &path.ids {
        let migration = registry
            .get(*id)
            .ok_or(MigrationRegistryError::Unknown(*id))?;
        let descriptor = migration.descriptor();
        if descriptor.from != current_program.artifact_id() {
            return Err(MigrationRegistryError::Disconnected(*id).into());
        }
        let target_program = programs
            .get(descriptor.to)
            .ok_or(MigrationStoreError::MissingProgram(descriptor.to))?;
        let (next_state, report) = migrate_checked_state(
            migration.as_ref(),
            &current_state,
            &current_program,
            target_program,
            0,
            options,
            limits,
        )?;
        let snapshot_bytes = export_snapshot(&next_state)
            .map_err(|error| MigrationStoreError::Snapshot(error.to_string()))?;
        let snapshot_object = CheckedObject::from_bytes(
            &snapshot_bytes,
            ObjectKind::Snapshot,
            0,
            limits.decode.max_envelope_bytes,
        )
        .map_err(|error| MigrationStoreError::Snapshot(error.to_string()))?;
        let snapshot = SnapshotId::from_bytes(*snapshot_object.id().as_bytes());
        let program_bytes = encode_program_artifact(target_program.artifact());
        let program_object = CheckedObject::from_bytes(
            &program_bytes,
            ObjectKind::Program,
            0,
            limits.decode.max_envelope_bytes,
        )
        .map_err(|error| MigrationStoreError::Snapshot(error.to_string()))?;
        let commit = CommitV1 {
            parent: Some(current_parent),
            execution: source_record.execution,
            program: descriptor.to,
            snapshot,
            cause: CommitCauseV1::Migration(*id),
            ledger_fence: source_record.ledger_fence,
            turn: next_state.turn,
        };
        let commit_object = commit.to_object();
        debug_assert_eq!(commit_object.schema(), COMMIT_SCHEMA_V1);
        let commit_id = CommitId::from_bytes(*commit_object.id().as_bytes());
        objects.extend([program_object, snapshot_object, commit_object]);
        commits.push(commit_id);
        reports.push(report);
        current_state = next_state;
        current_program = target_program.clone();
        current_parent = commit_id;
    }
    if current_state.program_artifact_id != target {
        return Err(MigrationStoreError::WrongTarget);
    }
    let final_commit = commits
        .last()
        .copied()
        .ok_or(MigrationStoreError::WrongTarget)?;
    store.commit(CommitTransaction {
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
    mut state: RuntimeStateV0,
    mut program: Arc<CheckedProgram>,
    options: MigrationOptions,
    limits: &SnapshotLoadLimits,
) -> Result<(RuntimeStateV0, Vec<MigrationReport>), MigrationStoreError> {
    let mut reports = Vec::with_capacity(ids.len());
    for id in ids {
        let migration = registry
            .get(*id)
            .ok_or(MigrationRegistryError::Unknown(*id))?;
        if migration.descriptor().from != program.artifact_id() {
            return Err(MigrationRegistryError::Disconnected(*id).into());
        }
        let target =
            programs
                .get(migration.descriptor().to)
                .ok_or(MigrationStoreError::MissingProgram(
                    migration.descriptor().to,
                ))?;
        let (next, report) = migrate_checked_state(
            migration.as_ref(),
            &state,
            &program,
            target,
            0,
            options,
            limits,
        )?;
        state = next;
        program = target.clone();
        reports.push(report);
    }
    Ok((state, reports))
}

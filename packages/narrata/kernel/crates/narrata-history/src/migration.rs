//! Trusted artifact graph selection and coordinated migration persistence (ADR 0020).
use crate::{
    ArtifactId, Commit, Domain, History, HistoryError, Object, ObjectId, RefKey, RefMutation,
    RefRevision, Registry, Transaction,
};
use narrata_kernel::codec::{CborWriter, DecodeError};
use narrata_storage::StorageBackend;
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

narrata_kernel::derived_id!(MigrationId, "migration:");
pub const MIGRATION_INPUT_KIND: u16 = 12;
pub const MIGRATION_INPUT_SCHEMA: u16 = 1;
const MAX_MIGRATION_PATH_LENGTH: usize = 64;
const MAX_DISCOVERED_PATHS: usize = 2;

/// Loaded, trusted artifacts; executable code is never imported from a save.
#[derive(Clone, Debug)]
pub struct ArtifactRegistry<A, V> {
    artifacts: BTreeMap<A, V>,
}
impl<A, V> Default for ArtifactRegistry<A, V> {
    fn default() -> Self {
        Self {
            artifacts: BTreeMap::new(),
        }
    }
}
impl<A: Ord, V> ArtifactRegistry<A, V> {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn register(&mut self, id: A, value: V) -> bool {
        self.artifacts.insert(id, value).is_none()
    }
    pub fn get(&self, id: A) -> Option<&V> {
        self.artifacts.get(&id)
    }
}
struct Registered<I, A, M> {
    id: I,
    from: A,
    to: A,
    value: M,
}
impl<I, A, M> Default for MigrationRegistry<I, A, M> {
    fn default() -> Self {
        Self {
            migrations: BTreeMap::new(),
            outgoing: BTreeMap::new(),
        }
    }
}
pub struct MigrationRegistry<I, A, M> {
    migrations: BTreeMap<I, Registered<I, A, M>>,
    outgoing: BTreeMap<A, Vec<I>>,
}

impl<I: Copy + Ord, A: Copy + Ord, M> MigrationRegistry<I, A, M> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(
        &mut self,
        id: I,
        from: A,
        to: A,
        migration: M,
    ) -> Result<(), MigrationRegistryError<I, A>> {
        let descriptor = Registered {
            id,
            from,
            to,
            value: migration,
        };
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
        self.migrations.insert(descriptor.id, descriptor);
        Ok(())
    }

    pub fn get(&self, id: I) -> Option<&M> {
        self.migrations.get(&id).map(|entry| &entry.value)
    }

    pub fn inspect(
        &self,
        from: A,
        to: A,
        explicit: Option<&[I]>,
    ) -> Result<MigrationPath<I, A>, MigrationRegistryError<I, A>> {
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
        from: A,
        to: A,
        ids: &[I],
    ) -> Result<MigrationPath<I, A>, MigrationRegistryError<I, A>> {
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
            let descriptor = migration;
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
        current_artifact: A,
        target: A,
        visited: &mut BTreeSet<A>,
        current_path: &mut Vec<I>,
        paths: &mut Vec<MigrationPath<I, A>>,
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
            let next = migration.to;
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
                    if step.from != cursor {
                        break;
                    }
                    cursor = step.to;
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

fn migration_path_start<I: Copy + Ord, A: Copy + Ord, M>(
    registry: &MigrationRegistry<I, A, M>,
    ids: &[I],
) -> Option<A> {
    ids.first()
        .and_then(|id| registry.migrations.get(id))
        .map(|migration| migration.from)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MigrationPath<I, A> {
    pub ids: Vec<I>,
    pub artifacts: Vec<A>,
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum MigrationRegistryError<I, A> {
    #[error("migration {0} is already registered")]
    Duplicate(I),
    #[error("migration {0} has identical source and target Artifacts")]
    SelfEdge(I),
    #[error("migration {0} is not registered")]
    Unknown(I),
    #[error("migration {0} does not continue the explicitly selected path")]
    Disconnected(I),
    #[error("no migration path exists from {from} to {to}")]
    NoPath { from: A, to: A },
    #[error("multiple migration paths exist from {from} to {to}; select one explicitly")]
    Ambiguous { from: A, to: A },
    #[error("migration path exceeds the configured maximum length")]
    PathLimit,
}

/// Runs the selected chain once. State semantics and staging belong to the caller; discovery,
/// link checks and ordering are shared by generic and legacy domains.
pub fn run_path<I: Copy + Ord, A: Copy + Ord, M, P, S, Report, E>(
    registry: &MigrationRegistry<I, A, M>,
    path: &MigrationPath<I, A>,
    mut state: S,
    mut resolve: impl FnMut(A) -> Result<P, E>,
    mut step: impl FnMut(&M, &P, &P, &S) -> Result<(S, Report), E>,
    mut stage: impl FnMut(I, A, &P, &S, &Report) -> Result<(), E>,
) -> Result<(S, Vec<Report>), E>
where
    E: From<MigrationRegistryError<I, A>>,
{
    let Some(&start) = path.artifacts.first() else {
        return Err(MigrationRegistryError::PathLimit.into());
    };
    let Some(&target) = path.artifacts.last() else {
        return Err(MigrationRegistryError::PathLimit.into());
    };
    let checked = registry.validate_explicit(start, target, &path.ids)?;
    if checked.artifacts != path.artifacts {
        return Err(MigrationRegistryError::NoPath {
            from: start,
            to: target,
        }
        .into());
    }
    let mut current = resolve(start)?;
    let mut reports = Vec::with_capacity(path.ids.len());
    for id in &path.ids {
        let migration = registry
            .migrations
            .get(id)
            .ok_or(MigrationRegistryError::Unknown(*id))?;
        let next = resolve(migration.to)?;
        let (next_state, report) = step(&migration.value, &current, &next, &state)?;
        stage(*id, migration.to, &next, &next_state, &report)?;
        reports.push(report);
        state = next_state;
        current = next;
    }
    Ok((state, reports))
}

/// A distinct input allows an artifact-changing edge without weakening normal transitions.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MigrationInput {
    pub id: MigrationId,
    pub from: ArtifactId,
    pub to: ArtifactId,
}
impl MigrationInput {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = CborWriter::new();
        w.map(3);
        w.unsigned(0);
        w.bytes(self.id.as_bytes());
        w.unsigned(1);
        w.bytes(self.from.as_bytes());
        w.unsigned(2);
        w.bytes(self.to.as_bytes());
        w.into_bytes()
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, HistoryError> {
        crate::layout::decode(
            "migration input",
            bytes,
            |r| {
                crate::layout::expect_map(r, 3)?;
                crate::layout::key(r, 0)?;
                let id = MigrationId::from_bytes(r.bytes_exact::<32>()?);
                crate::layout::key(r, 1)?;
                let from = ArtifactId::from_bytes(r.bytes_exact::<32>()?);
                crate::layout::key(r, 2)?;
                let to = ArtifactId::from_bytes(r.bytes_exact::<32>()?);
                if from == to {
                    return Err(DecodeError::Schema("migration self edge"));
                }
                Ok(Self { id, from, to })
            },
            Self::encode,
        )
    }
    pub fn to_object(self) -> Object {
        Object::new(MIGRATION_INPUT_KIND, MIGRATION_INPUT_SCHEMA, &self.encode())
    }
    pub fn from_object(object: &Object) -> Result<Self, HistoryError> {
        if object.kind() != MIGRATION_INPUT_KIND || object.schema() != MIGRATION_INPUT_SCHEMA {
            return Err(HistoryError::ObjectKind(object.id()));
        }
        Self::decode(object.payload())
            .map_err(|e| HistoryError::Corrupt(object.id(), e.to_string()))
    }
}

#[derive(Clone, Debug)]
pub struct MigrationRequest {
    pub source: ObjectId,
    pub target: ArtifactId,
    pub explicit_path: Option<Vec<MigrationId>>,
    pub target_ref: RefKey,
    pub expected_ref: Option<RefRevision>,
    pub observed_at: u64,
}
#[derive(Clone, Debug)]
pub struct MigrationDryRun<S, Report> {
    pub source: ObjectId,
    pub commit: ObjectId,
    pub commits: Vec<ObjectId>,
    pub path: MigrationPath<MigrationId, ArtifactId>,
    pub state: S,
    pub reports: Vec<Report>,
    transaction: Transaction,
}
#[derive(Debug, Error)]
pub enum MigrationError<E, S> {
    #[error(transparent)]
    History(E),
    #[error(transparent)]
    Registry(MigrationRegistryError<MigrationId, ArtifactId>),
    #[error("artifact {0} is not registered")]
    MissingArtifact(ArtifactId),
    #[error("migration step failed")]
    Step(S),
}
impl<E, S> From<MigrationRegistryError<MigrationId, ArtifactId>> for MigrationError<E, S> {
    fn from(error: MigrationRegistryError<MigrationId, ArtifactId>) -> Self {
        Self::Registry(error)
    }
}
impl<B: StorageBackend, R: Registry> History<B, R> {
    pub fn dry_run_migration<D: Domain, M, Report, S>(
        &self,
        artifacts: &ArtifactRegistry<ArtifactId, D>,
        migrations: &MigrationRegistry<MigrationId, ArtifactId, M>,
        request: &MigrationRequest,
        step: impl FnMut(&M, &D, &D, &D::State) -> Result<(D::State, Report), S>,
    ) -> Result<MigrationDryRun<D::State, Report>, MigrationError<R::Error, S>> {
        let fail = |e: HistoryError| MigrationError::History(e.into());
        let source = Commit::from_object(
            &self
                .reader()
                .require(request.source, Some(D::COMMIT_KIND))
                .map_err(fail)?,
            D::COMMIT_KIND,
        )
        .map_err(fail)?;
        let domain = artifacts
            .get(source.artifact)
            .ok_or(MigrationError::MissingArtifact(source.artifact))?;
        let loaded = self
            .load(domain, request.source)
            .map_err(MigrationError::History)?;
        let path = migrations.inspect(
            source.artifact,
            request.target,
            request.explicit_path.as_deref(),
        )?;
        let mut parent = request.source;
        let mut from = source.artifact;
        let mut depth = source.depth;
        let mut objects = Vec::new();
        let mut commits = Vec::new();
        let mut step = step;
        let (state, reports) = run_path(
            migrations,
            &path,
            loaded.state,
            |id| {
                let domain = artifacts
                    .get(id)
                    .ok_or(MigrationError::MissingArtifact(id))?;
                if domain.artifact_id() != id {
                    return Err(fail(HistoryError::InvalidGraph(
                        "artifact registry identity mismatch",
                    )));
                }
                Ok(domain)
            },
            |migration, source, target, state| {
                let (next, report) =
                    step(migration, source, target, state).map_err(MigrationError::Step)?;
                let (_, next) = crate::session::checked_state(*target, &next).map_err(fail)?;
                Ok((next, report))
            },
            |id, to, domain, state, _report| {
                if domain.artifact_id() != to {
                    return Err(fail(HistoryError::InvalidGraph(
                        "artifact registry identity mismatch",
                    )));
                }
                let (state_object, _) =
                    crate::session::checked_state(*domain, state).map_err(fail)?;
                let input = MigrationInput { id, from, to }.to_object();
                depth = depth
                    .checked_add(1)
                    .ok_or(HistoryError::Limit("commit depth"))
                    .map_err(fail)?;
                let commit = Commit {
                    artifact: to,
                    parent: Some(parent),
                    input: Some(input.id()),
                    state: state_object.id(),
                    depth,
                }
                .to_object(D::COMMIT_KIND);
                parent = commit.id();
                from = to;
                commits.push(parent);
                objects.extend([state_object, input, commit]);
                Ok(())
            },
        )?;
        let transaction = Transaction {
            objects,
            refs: vec![RefMutation {
                key: request.target_ref.clone(),
                expected: request.expected_ref,
                next: Some(parent),
            }],
            observed_at: request.observed_at,
            ..Transaction::default()
        };
        // Dry-run performs the same full graph/CAS/limit planning as application, without writes.
        self.validate_atomic(&transaction)
            .map_err(MigrationError::History)?;
        let target = artifacts
            .get(request.target)
            .ok_or(MigrationError::MissingArtifact(request.target))?;
        let (_, state) = crate::session::checked_state(target, &state).map_err(fail)?;
        Ok(MigrationDryRun {
            source: request.source,
            commit: parent,
            commits,
            path,
            state,
            reports,
            transaction,
        })
    }
    pub fn apply_migration<D: Domain, M, Report, S>(
        &mut self,
        artifacts: &ArtifactRegistry<ArtifactId, D>,
        migrations: &MigrationRegistry<MigrationId, ArtifactId, M>,
        request: &MigrationRequest,
        step: impl FnMut(&M, &D, &D, &D::State) -> Result<(D::State, Report), S>,
    ) -> Result<MigrationDryRun<D::State, Report>, MigrationError<R::Error, S>> {
        let prepared = self.dry_run_migration(artifacts, migrations, request, step)?;
        self.write_atomic(&prepared.transaction, |_| Ok(Vec::new()))
            .map_err(MigrationError::History)?;
        Ok(prepared)
    }
}

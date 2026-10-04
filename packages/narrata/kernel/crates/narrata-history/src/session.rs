//! Sessions of a registered domain: create, advance, check out, save and load, export and
//! import, all through the one engine (ADR 0015).
//!
//! Restoring never replays. Loading reads one Ref, one commit and one state, checks the commit's
//! digest and artifact and decodes the state against the artifact; ancestors are read and
//! checked only when something visits them. That proves the state is well formed for the
//! artifact, not that play could reach it: [`History::verify_path`] replays a path on request.

use std::collections::BTreeSet;

use narrata_kernel::codec::sha256;
use narrata_storage::StorageBackend;
use thiserror::Error;

use crate::{
    BranchId, BundleError, BundleLimits, CheckpointBundle, Commit, Domain, History, HistoryError,
    Object, ObjectId, RefKey, RefMutation, RefName, RefNamespace, RefRevision, RefScope, RefValue,
    Registry, Transaction, bundle, object_id, scan_all,
};

/// A commit read back and checked, with its decoded state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Loaded<S> {
    pub commit: ObjectId,
    pub header: Commit,
    pub state: S,
}

/// The Ref an import wrote and the root it loaded.
pub type Imported<S> = (RefValue, Loaded<S>);

/// The commit an advance moved to.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Advanced<S> {
    pub commit: ObjectId,
    pub depth: u64,
    pub state: S,
    /// The transition was already recorded, so the step did not run.
    pub reused: bool,
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum AdvanceError<E, S> {
    #[error(transparent)]
    History(E),
    /// The caller's step refused the input.
    #[error("the step refused the input")]
    Step(S),
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum AuditError<E, S> {
    #[error(transparent)]
    History(E),
    #[error("the step refused a recorded input")]
    Step(S),
    /// Replaying the recorded input after the parent produced another state than the one the
    /// commit records.
    #[error("replaying commit {commit} produced state {replayed}, not the recorded {recorded}")]
    Diverged {
        commit: ObjectId,
        recorded: ObjectId,
        replayed: ObjectId,
    },
}

/// Encodes a state and keeps it only if it decodes back to the same bytes, so every stored
/// state loads again and the state handed back is the decoded one.
fn checked_state<D: Domain>(
    domain: &D,
    state: &D::State,
) -> Result<(Object, D::State), HistoryError> {
    let object = Object::new(D::STATE_KIND, D::STATE_SCHEMA, &domain.encode_state(state));
    let decoded = domain
        .decode_state(object.payload())
        .map_err(|error| HistoryError::Corrupt(object.id(), error.to_string()))?;
    if domain.encode_state(&decoded) != object.payload() {
        return Err(HistoryError::Corrupt(
            object.id(),
            "state does not re-encode to the same bytes".to_owned(),
        ));
    }
    Ok((object, decoded))
}

fn checked_input<D: Domain>(
    domain: &D,
    input: &D::Input,
) -> Result<(Object, D::Input), HistoryError> {
    let object = Object::new(D::INPUT_KIND, D::INPUT_SCHEMA, &domain.encode_input(input));
    let decoded = decode_input(domain, &object)?;
    if domain.encode_input(&decoded) != object.payload() {
        return Err(HistoryError::Corrupt(
            object.id(),
            "input does not re-encode to the same bytes".to_owned(),
        ));
    }
    Ok((object, decoded))
}

fn decode_state<D: Domain>(domain: &D, object: &Object) -> Result<D::State, HistoryError> {
    if object.kind() != D::STATE_KIND || object.schema() != D::STATE_SCHEMA {
        return Err(HistoryError::ObjectKind(object.id()));
    }
    domain
        .decode_state(object.payload())
        .map_err(|error| HistoryError::Corrupt(object.id(), error.to_string()))
}

fn decode_input<D: Domain>(domain: &D, object: &Object) -> Result<D::Input, HistoryError> {
    if object.kind() != D::INPUT_KIND || object.schema() != D::INPUT_SCHEMA {
        return Err(HistoryError::ObjectKind(object.id()));
    }
    domain
        .decode_input(object.payload())
        .map_err(|error| HistoryError::Corrupt(object.id(), error.to_string()))
}

/// Decodes a commit of domain `D` and checks that it names the domain's artifact.
fn header<D: Domain>(domain: &D, object: &Object) -> Result<Commit, HistoryError> {
    let header = Commit::from_object(object, D::COMMIT_KIND)?;
    let expected = domain.artifact_id();
    if header.artifact != expected {
        return Err(HistoryError::ArtifactMismatch {
            commit: object.id(),
            expected,
            actual: header.artifact,
        });
    }
    Ok(header)
}

/// A branch of session `name` whose head is `commit`.
fn branch_at<B: StorageBackend, R: Registry>(
    history: &History<B, R>,
    name: &RefName,
    commit: ObjectId,
) -> Result<Option<BranchId>, R::Error> {
    let branches = scan_all(
        |after| {
            history.scan_refs(
                &RefScope::Owner(RefNamespace::Branch, name.clone()),
                after,
                BRANCH_PAGE,
            )
        },
        |(key, _): &(RefKey, RefValue)| key.clone(),
    )?;
    Ok(branches
        .iter()
        .find(|(_, value)| value.commit == commit)
        .and_then(|(key, _)| key.branch_id()))
}

/// Branch Refs read per scan when looking for the branch at a commit.
const BRANCH_PAGE: u32 = 256;

/// The branch that starts at `commit`; the same fork always gets the same branch.
fn branch_for(commit: ObjectId) -> BranchId {
    let digest = sha256(&[b"narrata-branch\0".as_slice(), commit.as_bytes()].concat());
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    BranchId::from_bytes(bytes)
}

impl<B: StorageBackend, R: Registry> History<B, R> {
    /// Loads `commit` without replaying anything: one commit and one state are read, checked
    /// and decoded, whatever the depth of history behind them.
    pub fn load<D: Domain>(
        &self,
        domain: &D,
        commit: ObjectId,
    ) -> Result<Loaded<D::State>, R::Error> {
        let reader = self.reader();
        let object = reader.require(commit, Some(D::COMMIT_KIND))?;
        let header = header(domain, &object)?;
        let state = decode_state(domain, &reader.require(header.state, Some(D::STATE_KIND))?)?;
        Ok(Loaded {
            commit,
            header,
            state,
        })
    }

    /// Replays the path from the root to `commit` with `step` and compares every state with
    /// the recorded one. Reads the whole path; an audit, not part of loading.
    pub fn verify_path<D: Domain, S>(
        &self,
        domain: &D,
        commit: ObjectId,
        mut step: impl FnMut(&D, &D::State, &D::Input) -> Result<D::State, S>,
    ) -> Result<(), AuditError<R::Error, S>> {
        let reader = self.reader();
        let history = |error: HistoryError| AuditError::History(error.into());
        let mut path = Vec::new();
        let mut current = Some(commit);
        while let Some(id) = current {
            let object = reader.require(id, Some(D::COMMIT_KIND)).map_err(history)?;
            let header = header(domain, &object).map_err(history)?;
            current = header.parent;
            path.push((id, header));
        }
        path.reverse();
        let Some((_, root)) = path.first() else {
            return Ok(());
        };
        let mut state = decode_state(
            domain,
            &reader
                .require(root.state, Some(D::STATE_KIND))
                .map_err(history)?,
        )
        .map_err(history)?;
        for (id, header) in path.iter().skip(1) {
            let input_id = header
                .input
                .ok_or(HistoryError::InvalidGraph("commit without input"))
                .map_err(history)?;
            let input = decode_input(
                domain,
                &reader
                    .require(input_id, Some(D::INPUT_KIND))
                    .map_err(history)?,
            )
            .map_err(history)?;
            let next = step(domain, &state, &input).map_err(AuditError::Step)?;
            let replayed = object_id(D::STATE_KIND, D::STATE_SCHEMA, &domain.encode_state(&next));
            if replayed != header.state {
                return Err(AuditError::Diverged {
                    commit: *id,
                    recorded: header.state,
                    replayed,
                });
            }
            state = next;
        }
        Ok(())
    }

    /// A checkpoint bundle of `root` and its closure, without what the receiver already has.
    pub fn export(
        &self,
        root: ObjectId,
        receiver_has: &BTreeSet<ObjectId>,
    ) -> Result<CheckpointBundle, BundleError<R::Error>> {
        CheckpointBundle::export(self, self.registry(), root, receiver_has)
    }

    /// Imports checkpoint bytes whose root is a commit of domain `D`, pointing `target` at it.
    /// Tampered bytes fail their digests; the root's artifact and state are checked against
    /// `domain` before anything is written, and the engine validates every carried object.
    pub fn import<D: Domain>(
        &mut self,
        domain: &D,
        bytes: &[u8],
        limits: BundleLimits,
        target: RefKey,
        expected: Option<RefRevision>,
        observed_at: u64,
    ) -> Result<Imported<D::State>, BundleError<R::Error>> {
        let bundle =
            CheckpointBundle::from_bytes(bytes, limits, |kind| bundle::registered(self, kind))?;
        let available = bundle.check_closure(self, self.registry())?;
        let root = bundle.manifest.root;
        let checked = |id: ObjectId| {
            available
                .get(&id)
                .ok_or(BundleError::<R::Error>::MissingObject(id))
        };
        let history = |error: HistoryError| BundleError::Store(error.into());
        let root_object = checked(root)?;
        let header = header(domain, root_object).map_err(history)?;
        let state = decode_state(domain, checked(header.state)?).map_err(history)?;
        let value = bundle.write(self, target, expected, observed_at)?;
        Ok((
            value,
            Loaded {
                commit: root,
                header,
                state,
            },
        ))
    }
}

/// One player's position in a domain's commit graph.
///
/// A session is the Ref `active/<name>/cursor` and the branch heads `branches/<name>/<branch>`.
/// Every commit the session creates stays reachable from one of them, so GC keeps it: advancing
/// from the head of the selected branch moves that branch, and advancing from anywhere else
/// starts a branch at the new commit. Opening a session or checking out a branch head selects
/// that branch; checking out any other commit keeps the selection.
#[derive(Clone, Debug)]
pub struct Session<D> {
    domain: D,
    name: RefName,
    cursor: RefValue,
    branch: Option<BranchId>,
}

impl<D: Domain> Session<D> {
    /// Stores the root commit for `root` and starts the session `name` at it.
    pub fn create<B: StorageBackend, R: Registry>(
        history: &mut History<B, R>,
        domain: D,
        name: RefName,
        root: &D::State,
        observed_at: u64,
    ) -> Result<(Self, Loaded<D::State>), R::Error> {
        let (state_object, state) = checked_state(&domain, root)?;
        let header = Commit::root(domain.artifact_id(), state_object.id());
        let commit = header.to_object(D::COMMIT_KIND);
        let id = commit.id();
        let branch = branch_for(id);
        let written = history.write(
            &Transaction {
                objects: vec![state_object, commit],
                refs: vec![
                    RefMutation {
                        key: RefKey::cursor(name.clone()),
                        expected: None,
                        next: Some(id),
                    },
                    RefMutation {
                        key: RefKey::branch(name.clone(), branch),
                        expected: None,
                        next: Some(id),
                    },
                ],
                observed_at,
                ..Transaction::default()
            },
            |_| Ok(Vec::new()),
        )?;
        let session = Self {
            domain,
            name,
            cursor: RefValue {
                revision: written.root_revision()?,
                commit: id,
            },
            branch: Some(branch),
        };
        Ok((
            session,
            Loaded {
                commit: id,
                header,
                state,
            },
        ))
    }

    /// Reopens the session `name` at its cursor, selecting the branch whose head the cursor is.
    pub fn open<B: StorageBackend, R: Registry>(
        history: &History<B, R>,
        domain: D,
        name: RefName,
    ) -> Result<(Self, Loaded<D::State>), R::Error> {
        let key = RefKey::cursor(name.clone());
        let cursor = history
            .read_ref(&key)?
            .ok_or(HistoryError::MissingRef(key))?;
        let loaded = history.load(&domain, cursor.commit)?;
        let branch = branch_at(history, &name, cursor.commit)?;
        Ok((
            Self {
                domain,
                name,
                cursor,
                branch,
            },
            loaded,
        ))
    }

    pub fn domain(&self) -> &D {
        &self.domain
    }

    pub fn name(&self) -> &RefName {
        &self.name
    }

    /// The commit the cursor names.
    pub fn head(&self) -> ObjectId {
        self.cursor.commit
    }

    pub fn branch(&self) -> Option<BranchId> {
        self.branch
    }

    /// Reads the cursor and requires it at `expected`.
    fn cursor<B: StorageBackend, R: Registry>(
        &self,
        history: &History<B, R>,
        expected: ObjectId,
    ) -> Result<RefValue, R::Error> {
        let key = RefKey::cursor(self.name.clone());
        let cursor = history
            .read_ref(&key)?
            .ok_or(HistoryError::MissingRef(key))?;
        if cursor.commit == expected {
            Ok(cursor)
        } else {
            Err(HistoryError::HeadMoved {
                expected,
                actual: Some(cursor.commit),
            }
            .into())
        }
    }

    /// Applies `input` at `expected`, the head the caller saw. A transition already recorded for
    /// that commit and input is reused without running `step`; otherwise `step` computes the
    /// next state from the decoded parent state and input. A different commit for a recorded
    /// transition is reported as [`HistoryError::Nondeterministic`].
    pub fn advance<B: StorageBackend, R: Registry, S>(
        &mut self,
        history: &mut History<B, R>,
        expected: ObjectId,
        input: &D::Input,
        step: impl FnOnce(&D, &D::State, &D::Input) -> Result<D::State, S>,
        observed_at: u64,
    ) -> Result<Advanced<D::State>, AdvanceError<R::Error, S>> {
        let fail = |error: HistoryError| AdvanceError::History(error.into());
        let (input_object, input) = checked_input(&self.domain, input).map_err(fail)?;
        let cursor = self
            .cursor(history, expected)
            .map_err(AdvanceError::History)?;
        let recorded = history
            .transition(expected, input_object.id())
            .map_err(AdvanceError::History)?;
        let (child, objects) = match recorded {
            Some(child) => {
                let loaded = history
                    .load(&self.domain, child)
                    .map_err(AdvanceError::History)?;
                if loaded.header.parent != Some(expected)
                    || loaded.header.input != Some(input_object.id())
                {
                    return Err(fail(HistoryError::CorruptStore(format!(
                        "transition index entry names commit {child} of another parent or input"
                    ))));
                }
                (loaded, Vec::new())
            }
            None => {
                let parent = history
                    .load(&self.domain, expected)
                    .map_err(AdvanceError::History)?;
                let next = step(&self.domain, &parent.state, &input).map_err(AdvanceError::Step)?;
                let (state_object, state) = checked_state(&self.domain, &next).map_err(fail)?;
                let header = Commit {
                    artifact: parent.header.artifact,
                    parent: Some(expected),
                    input: Some(input_object.id()),
                    state: state_object.id(),
                    depth: parent
                        .header
                        .depth
                        .checked_add(1)
                        .ok_or(HistoryError::Limit("commit depth"))
                        .map_err(fail)?,
                };
                let commit = header.to_object(D::COMMIT_KIND);
                let loaded = Loaded {
                    commit: commit.id(),
                    header,
                    state,
                };
                (loaded, vec![input_object, state_object, commit])
            }
        };
        let reused = objects.is_empty();
        let (branch_ref, branch) = self
            .branch_step(history, expected, child.commit, reused)
            .map_err(AdvanceError::History)?;
        let mut refs = vec![RefMutation {
            key: RefKey::cursor(self.name.clone()),
            expected: Some(cursor.revision),
            next: Some(child.commit),
        }];
        refs.extend(branch_ref);
        let written = history
            .write(
                &Transaction {
                    objects,
                    refs,
                    observed_at,
                    ..Transaction::default()
                },
                |_| Ok(Vec::new()),
            )
            .map_err(AdvanceError::History)?;
        self.cursor = RefValue {
            revision: written.root_revision().map_err(fail)?,
            commit: child.commit,
        };
        self.branch = branch;
        Ok(Advanced {
            commit: child.commit,
            depth: child.header.depth,
            state: child.state,
            reused,
        })
    }

    /// The branch Ref an advance from `parent` to `child` writes, and the branch selected after.
    fn branch_step<B: StorageBackend, R: Registry>(
        &self,
        history: &History<B, R>,
        parent: ObjectId,
        child: ObjectId,
        reused: bool,
    ) -> Result<(Option<RefMutation>, Option<BranchId>), R::Error> {
        if let Some(branch) = self.branch {
            let key = RefKey::branch(self.name.clone(), branch);
            if let Some(head) = history.read_ref(&key)?
                && head.commit == parent
            {
                let mutation = RefMutation {
                    key,
                    expected: Some(head.revision),
                    next: Some(child),
                };
                return Ok((Some(mutation), Some(branch)));
            }
        }
        if reused {
            // The commit existed before this advance; whatever kept it keeps it.
            return Ok((None, self.branch));
        }
        let branch = branch_for(child);
        let key = RefKey::branch(self.name.clone(), branch);
        Ok(match history.read_ref(&key)? {
            Some(head) if head.commit == child => (None, Some(branch)),
            Some(_) => (None, self.branch),
            None => (
                Some(RefMutation {
                    key,
                    expected: None,
                    next: Some(child),
                }),
                Some(branch),
            ),
        })
    }

    /// Moves the cursor from `expected` to `target`, a commit of the same domain and artifact,
    /// loading it without replay.
    pub fn checkout<B: StorageBackend, R: Registry>(
        &mut self,
        history: &mut History<B, R>,
        expected: ObjectId,
        target: ObjectId,
        observed_at: u64,
    ) -> Result<Loaded<D::State>, R::Error> {
        let cursor = self.cursor(history, expected)?;
        let loaded = history.load(&self.domain, target)?;
        let branch = branch_at(history, &self.name, target)?;
        let written = history.write(
            &Transaction {
                refs: vec![RefMutation {
                    key: RefKey::cursor(self.name.clone()),
                    expected: Some(cursor.revision),
                    next: Some(target),
                }],
                observed_at,
                ..Transaction::default()
            },
            |_| Ok(Vec::new()),
        )?;
        self.cursor = RefValue {
            revision: written.root_revision()?,
            commit: target,
        };
        if branch.is_some() {
            self.branch = branch;
        }
        Ok(loaded)
    }

    /// Points the save slot `slot` at the cursor; `expected` is the slot's current revision, or
    /// `None` for a new slot.
    pub fn save<B: StorageBackend, R: Registry>(
        &self,
        history: &mut History<B, R>,
        slot: RefKey,
        expected: Option<RefRevision>,
        observed_at: u64,
    ) -> Result<RefValue, R::Error> {
        let commit = self.cursor.commit;
        let written = history.write(
            &Transaction {
                refs: vec![RefMutation {
                    key: slot,
                    expected,
                    next: Some(commit),
                }],
                observed_at,
                ..Transaction::default()
            },
            |_| Ok(Vec::new()),
        )?;
        Ok(RefValue {
            revision: written.root_revision()?,
            commit,
        })
    }

    /// Moves the cursor to the commit the save slot `slot` names.
    pub fn load_save<B: StorageBackend, R: Registry>(
        &mut self,
        history: &mut History<B, R>,
        slot: &RefKey,
        observed_at: u64,
    ) -> Result<Loaded<D::State>, R::Error> {
        let target = history
            .read_ref(slot)?
            .ok_or_else(|| HistoryError::MissingRef(slot.clone()))?;
        self.checkout(history, self.cursor.commit, target.commit, observed_at)
    }
}

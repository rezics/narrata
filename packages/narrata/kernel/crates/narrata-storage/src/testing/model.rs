//! Random batch sequences against an independent model of the contract.

// The suite reports failures by panicking, like any test.
#![allow(clippy::panic)]

use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    ops::Bound,
};

use proptest::{
    prelude::*,
    test_runner::{Config, TestCaseError, TestRunner},
};

use super::conformance::{A, B, Harness, collect_digests, collect_keys, digest, never_issued};
use crate::{
    Applied, Batch, Expect, KeyAction, KeyEntry, KeyOp, KeyPage, KeySpace, KeyValue, ObjectDigest,
    ObjectPage, Revision, StorageBackend, StorageError,
};

const DEFAULT_CASES: u32 = 64;

#[derive(Clone, Debug)]
enum Step {
    Apply(Vec<Op>),
    ScanKeys {
        space: bool,
        prefix: Vec<u8>,
        after: Option<Vec<u8>>,
        limit: u32,
    },
    ScanObjects {
        after: Option<u8>,
        limit: u32,
    },
}

#[derive(Clone, Debug)]
enum Op {
    PutObject {
        id: u8,
        fill: u8,
    },
    DeleteObject {
        id: u8,
    },
    Key {
        space: bool,
        key: Vec<u8>,
        expect: Pick,
        action: Action,
    },
}

/// How a key operation chooses its precondition, resolved against the model at run time.
#[derive(Clone, Copy, Debug)]
enum Pick {
    Any,
    Absent,
    /// The key's current revision, or `Absent` when the key does not exist.
    Current,
    /// Some revision the backend issued earlier.
    Issued(u8),
    Never,
}

#[derive(Clone, Debug)]
enum Action {
    Put(u8),
    Delete,
    Check,
}

fn space(second: bool) -> KeySpace {
    if second { B } else { A }
}

fn key() -> impl Strategy<Value = Vec<u8>> {
    prop::collection::vec(prop::sample::select(vec![0_u8, 1, 0xFF]), 0..3)
}

fn op() -> impl Strategy<Value = Op> {
    let pick = prop_oneof![
        1 => Just(Pick::Any),
        1 => Just(Pick::Absent),
        3 => Just(Pick::Current),
        1 => any::<u8>().prop_map(Pick::Issued),
        1 => Just(Pick::Never),
    ];
    let action = prop_oneof![
        3 => any::<u8>().prop_map(Action::Put),
        1 => Just(Action::Delete),
        1 => Just(Action::Check),
    ];
    prop_oneof![
        1 => (0_u8..6, any::<u8>()).prop_map(|(id, fill)| Op::PutObject { id, fill }),
        1 => (0_u8..6).prop_map(|id| Op::DeleteObject { id }),
        4 => (any::<bool>(), key(), pick, action).prop_map(|(space, key, expect, action)| {
            Op::Key { space, key, expect, action }
        }),
    ]
}

fn step() -> impl Strategy<Value = Step> {
    prop_oneof![
        4 => prop::collection::vec(op(), 0..5).prop_map(Step::Apply),
        1 => (any::<bool>(), key(), prop::option::of(key()), 1_u32..4).prop_map(
            |(space, prefix, after, limit)| Step::ScanKeys { space, prefix, after, limit }
        ),
        1 => (prop::option::of(0_u8..8), 1_u32..4)
            .prop_map(|(after, limit)| Step::ScanObjects { after, limit }),
    ]
}

/// Runs random steps against fresh stores and compares every result and the full state after
/// each step with a model that shares no code with the backends.
pub fn random_batches_match_model<H: Harness>(harness: &mut H) {
    let mut config = Config {
        failure_persistence: None,
        ..Config::default()
    };
    if std::env::var_os("PROPTEST_CASES").is_none() {
        config.cases = DEFAULT_CASES;
    }
    let harness = RefCell::new(harness);
    let mut runner = TestRunner::new(config);
    let result = runner.run(&prop::collection::vec(step(), 1..24), |steps| {
        let mut backend = harness.borrow_mut().create();
        let mut model = Model::default();
        for step in &steps {
            model.step(&mut backend, step)?;
        }
        Ok(())
    });
    if let Err(error) = result {
        panic!("{error}");
    }
}

/// The model's prediction for a batch it accepts. The backend chooses revision values; the
/// model only predicts whether one is issued and checks that it exceeds all earlier ones.
#[derive(Debug)]
struct Verdict {
    new_revision: bool,
    objects_inserted: u64,
    objects_deleted: u64,
}

#[derive(Default)]
struct Model {
    objects: BTreeMap<ObjectDigest, Vec<u8>>,
    keys: BTreeMap<(KeySpace, Vec<u8>), KeyValue>,
    issued: Vec<Revision>,
}

fn fail(message: String) -> TestCaseError {
    TestCaseError::fail(message)
}

impl Model {
    fn step(
        &mut self,
        backend: &mut impl StorageBackend,
        step: &Step,
    ) -> Result<(), TestCaseError> {
        match step {
            Step::Apply(ops) => {
                let batch = self.batch(ops);
                let expected = self.expect(&batch);
                let actual = backend.apply(&batch);
                self.compare(&batch, expected, actual)?;
            }
            Step::ScanKeys {
                space: second,
                prefix,
                after,
                limit,
            } => {
                let expected = self.key_page(space(*second), prefix, after.as_deref(), *limit);
                let actual = backend
                    .scan_keys(space(*second), prefix, after.as_deref(), *limit)
                    .map_err(|error| fail(format!("key scan failed: {error}")))?;
                prop_assert_eq!(actual, expected);
            }
            Step::ScanObjects { after, limit } => {
                let after = after.map(|id| digest(u64::from(id)));
                let expected = self.object_page(after.as_ref(), *limit);
                let actual = backend
                    .scan_objects(after.as_ref(), *limit)
                    .map_err(|error| fail(format!("object scan failed: {error}")))?;
                prop_assert_eq!(actual, expected);
            }
        }
        self.compare_state(backend)
    }

    fn batch(&self, ops: &[Op]) -> Batch {
        let mut batch = Batch::new();
        for op in ops {
            match op {
                Op::PutObject { id, fill } => {
                    let bytes = vec![*fill; usize::from(fill % 5)];
                    batch = batch.put_object(digest(u64::from(*id)), bytes);
                }
                Op::DeleteObject { id } => batch = batch.delete_object(digest(u64::from(*id))),
                Op::Key {
                    space: second,
                    key,
                    expect,
                    action,
                } => {
                    let space = space(*second);
                    let expect = match expect {
                        Pick::Any => Expect::Any,
                        Pick::Absent => Expect::Absent,
                        Pick::Current => self
                            .keys
                            .get(&(space, key.clone()))
                            .map_or(Expect::Absent, |value| Expect::Revision(value.revision)),
                        Pick::Issued(index) if !self.issued.is_empty() => {
                            Expect::Revision(self.issued[usize::from(*index) % self.issued.len()])
                        }
                        Pick::Issued(_) | Pick::Never => Expect::Revision(never_issued()),
                    };
                    let action = match action {
                        Action::Put(value) => KeyAction::Put(vec![*value]),
                        Action::Delete => KeyAction::Delete,
                        Action::Check => KeyAction::Check,
                    };
                    batch.keys.push(KeyOp {
                        space,
                        key: key.clone(),
                        expect,
                        action,
                    });
                }
            }
        }
        batch
    }

    /// What the batch must do, or the error it must fail with.
    fn expect(&self, batch: &Batch) -> Result<Verdict, StorageError> {
        let mut named = BTreeSet::new();
        let objects = batch
            .put_objects
            .iter()
            .map(|(digest, _)| digest)
            .chain(&batch.delete_objects);
        for digest in objects {
            if !named.insert(*digest) {
                return Err(StorageError::Invalid("object named twice"));
            }
        }
        let mut keys = BTreeSet::new();
        for op in &batch.keys {
            if !keys.insert((op.space, op.key.clone()))
                || (op.action == KeyAction::Check && op.expect == Expect::Any)
            {
                return Err(StorageError::Invalid("malformed key operation"));
            }
        }
        for (index, op) in batch.keys.iter().enumerate() {
            let actual = self.keys.get(&(op.space, op.key.clone()));
            let holds = match op.expect {
                Expect::Any => true,
                Expect::Absent => actual.is_none(),
                Expect::Revision(revision) => actual.map(|value| value.revision) == Some(revision),
            };
            if !holds {
                return Err(crate::Conflict {
                    index,
                    space: op.space,
                    key: op.key.clone(),
                    expected: op.expect,
                    actual: actual.cloned(),
                }
                .into());
            }
        }
        Ok(Verdict {
            new_revision: batch
                .keys
                .iter()
                .any(|op| matches!(op.action, KeyAction::Put(_))),
            objects_inserted: batch
                .put_objects
                .iter()
                .filter(|(digest, _)| !self.objects.contains_key(digest))
                .count() as u64,
            objects_deleted: batch
                .delete_objects
                .iter()
                .filter(|digest| self.objects.contains_key(digest))
                .count() as u64,
        })
    }

    fn compare(
        &mut self,
        batch: &Batch,
        expected: Result<Verdict, StorageError>,
        actual: Result<Applied, StorageError>,
    ) -> Result<(), TestCaseError> {
        match (expected, actual) {
            (Err(StorageError::Invalid(_)), Err(StorageError::Invalid(_))) => Ok(()),
            (Err(expected @ StorageError::Conflict(_)), Err(actual)) => {
                prop_assert_eq!(actual, expected);
                Ok(())
            }
            (Ok(expected), Ok(actual)) => {
                prop_assert_eq!(actual.objects_inserted, expected.objects_inserted);
                prop_assert_eq!(actual.objects_deleted, expected.objects_deleted);
                prop_assert_eq!(actual.revision.is_some(), expected.new_revision);
                if let Some(revision) = actual.revision {
                    if let Some(last) = self.issued.last() {
                        prop_assert!(revision > *last, "revision {revision:?} after {last:?}");
                    }
                    self.issued.push(revision);
                }
                self.apply(batch, actual.revision);
                Ok(())
            }
            (expected, actual) => Err(fail(format!("expected {expected:?}, got {actual:?}"))),
        }
    }

    fn apply(&mut self, batch: &Batch, revision: Option<Revision>) {
        for (digest, bytes) in &batch.put_objects {
            self.objects
                .entry(*digest)
                .or_insert_with(|| bytes.to_vec());
        }
        for digest in &batch.delete_objects {
            self.objects.remove(digest);
        }
        for op in &batch.keys {
            let key = (op.space, op.key.clone());
            match (&op.action, revision) {
                (KeyAction::Put(value), Some(revision)) => {
                    self.keys.insert(
                        key,
                        KeyValue {
                            value: value.clone(),
                            revision,
                        },
                    );
                }
                (KeyAction::Delete, _) => {
                    self.keys.remove(&key);
                }
                (KeyAction::Put(_), None) | (KeyAction::Check, _) => {}
            }
        }
    }

    fn key_page(
        &self,
        space: KeySpace,
        prefix: &[u8],
        after: Option<&[u8]>,
        limit: u32,
    ) -> KeyPage {
        let mut matching = self
            .keys
            .iter()
            .filter(|((candidate, key), _)| {
                *candidate == space
                    && key.starts_with(prefix)
                    && after.is_none_or(|after| key.as_slice() > after)
            })
            .map(|((_, key), value)| KeyEntry {
                key: key.clone(),
                value: value.value.clone(),
                revision: value.revision,
            });
        let entries = matching.by_ref().take(limit as usize).collect();
        KeyPage {
            entries,
            more: matching.next().is_some(),
        }
    }

    fn object_page(&self, after: Option<&ObjectDigest>, limit: u32) -> ObjectPage {
        let lower = after.map_or(Bound::Unbounded, Bound::Excluded);
        let mut digests = self
            .objects
            .range((lower, Bound::Unbounded))
            .map(|(digest, _)| *digest);
        let page = digests.by_ref().take(limit as usize).collect();
        ObjectPage {
            digests: page,
            more: digests.next().is_some(),
        }
    }

    fn compare_state(&self, backend: &impl StorageBackend) -> Result<(), TestCaseError> {
        for space in [A, B] {
            let expected = self.key_page(space, b"", None, u32::MAX).entries;
            prop_assert_eq!(collect_keys(backend, space, b"", None, 3), expected);
        }
        let digests = collect_digests(backend, 4);
        prop_assert_eq!(&digests, &self.objects.keys().copied().collect::<Vec<_>>());
        let stored = backend
            .get_objects(&digests)
            .map_err(|error| fail(format!("object read failed: {error}")))?
            .into_iter()
            .map(|bytes| bytes.map(|bytes| bytes.to_vec()))
            .collect::<Vec<_>>();
        let expected = self.objects.values().cloned().map(Some).collect::<Vec<_>>();
        prop_assert_eq!(stored, expected);
        Ok(())
    }
}

use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use super::Primitive;
use crate::{
    Applied, Batch, Capabilities, KeyPage, KeySpace, KeyValue, ObjectDigest, ObjectPage,
    StorageBackend, StorageError,
};

/// What happens to a call the plan selects.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Fault {
    /// The call fails with this error without reaching the backend, so it has no effect.
    Before(StorageError),
    /// The call reaches the backend, then reports an outcome-unknown I/O error: a batch has
    /// been applied but the caller is told it failed.
    After,
}

/// Selects a call by its 1-based position.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Trigger {
    /// The n-th call of any primitive.
    Call(u64),
    /// The n-th call of one primitive.
    Nth(Primitive, u64),
}

/// A deterministic schedule of faults.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct FaultPlan {
    faults: BTreeMap<Trigger, Fault>,
}

impl FaultPlan {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn at(mut self, trigger: Trigger, fault: Fault) -> Self {
        self.faults.insert(trigger, fault);
        self
    }
}

/// Wraps a backend and fails the calls a [`FaultPlan`] selects.
///
/// Sweeping `Trigger::Call(n)` over every call an operation makes, with both fault kinds,
/// checks that the operation never publishes partial state and recovers from unknown outcomes.
#[derive(Debug)]
pub struct FaultInjecting<B> {
    inner: B,
    plan: FaultPlan,
    calls: Counters,
}

#[derive(Debug, Default)]
struct Counters {
    total: AtomicU64,
    by_primitive: [AtomicU64; 5],
}

impl<B> FaultInjecting<B> {
    pub fn new(inner: B, plan: FaultPlan) -> Self {
        Self {
            inner,
            plan,
            calls: Counters::default(),
        }
    }

    /// Replaces the plan and restarts call numbering.
    pub fn set_plan(&mut self, plan: FaultPlan) {
        self.plan = plan;
        self.calls = Counters::default();
    }

    /// Calls made so far, faulted or not.
    pub fn calls(&self) -> u64 {
        self.calls.total.load(Ordering::SeqCst)
    }

    pub fn inner(&self) -> &B {
        &self.inner
    }

    pub fn into_inner(self) -> B {
        self.inner
    }
}

fn intercept<T>(
    plan: &FaultPlan,
    calls: &Counters,
    primitive: Primitive,
    call: impl FnOnce() -> Result<T, StorageError>,
) -> Result<T, StorageError> {
    let total = calls.total.fetch_add(1, Ordering::SeqCst) + 1;
    let nth = calls.by_primitive[primitive.index()].fetch_add(1, Ordering::SeqCst) + 1;
    let fault = plan
        .faults
        .get(&Trigger::Call(total))
        .or_else(|| plan.faults.get(&Trigger::Nth(primitive, nth)));
    match fault {
        None => call(),
        Some(Fault::Before(error)) => Err(error.clone()),
        Some(Fault::After) => {
            // The result is discarded on purpose: the caller must not learn whether it applied.
            let _ = call();
            Err(StorageError::Io(format!(
                "injected fault after {primitive:?} call {total}"
            )))
        }
    }
}

impl<B: StorageBackend> StorageBackend for FaultInjecting<B> {
    fn capabilities(&self) -> Capabilities {
        self.inner.capabilities()
    }

    fn get_objects(
        &self,
        digests: &[ObjectDigest],
    ) -> Result<Vec<Option<Arc<[u8]>>>, StorageError> {
        intercept(&self.plan, &self.calls, Primitive::GetObjects, || {
            self.inner.get_objects(digests)
        })
    }

    fn scan_objects(
        &self,
        after: Option<&ObjectDigest>,
        limit: u32,
    ) -> Result<ObjectPage, StorageError> {
        intercept(&self.plan, &self.calls, Primitive::ScanObjects, || {
            self.inner.scan_objects(after, limit)
        })
    }

    fn read_key(&self, space: KeySpace, key: &[u8]) -> Result<Option<KeyValue>, StorageError> {
        intercept(&self.plan, &self.calls, Primitive::ReadKey, || {
            self.inner.read_key(space, key)
        })
    }

    fn scan_keys(
        &self,
        space: KeySpace,
        prefix: &[u8],
        after: Option<&[u8]>,
        limit: u32,
    ) -> Result<KeyPage, StorageError> {
        intercept(&self.plan, &self.calls, Primitive::ScanKeys, || {
            self.inner.scan_keys(space, prefix, after, limit)
        })
    }

    fn apply(&mut self, batch: &Batch) -> Result<Applied, StorageError> {
        let inner = &mut self.inner;
        intercept(&self.plan, &self.calls, Primitive::Apply, || {
            inner.apply(batch)
        })
    }
}

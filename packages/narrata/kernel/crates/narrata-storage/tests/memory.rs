#![allow(clippy::unwrap_used)]

use narrata_storage::{
    MemoryBackend,
    testing::{Counting, FaultInjecting, FaultPlan, conformance::Harness},
};

#[derive(Default)]
struct Memory;

impl Harness for Memory {
    type Backend = MemoryBackend;

    fn create(&mut self) -> MemoryBackend {
        MemoryBackend::new()
    }

    fn connect(&mut self, backend: &MemoryBackend) -> Option<MemoryBackend> {
        Some(backend.share())
    }
}

narrata_storage::conformance_tests!(Memory);

/// The wrappers must be transparent when they inject nothing.
mod wrapped {
    use super::*;

    #[derive(Default)]
    struct Wrapped;

    type Backend = Counting<FaultInjecting<MemoryBackend>>;

    impl Harness for Wrapped {
        type Backend = Backend;

        fn create(&mut self) -> Backend {
            Counting::new(FaultInjecting::new(MemoryBackend::new(), FaultPlan::new()))
        }

        fn connect(&mut self, backend: &Backend) -> Option<Backend> {
            let shared = backend.inner().inner().share();
            Some(Counting::new(FaultInjecting::new(shared, FaultPlan::new())))
        }
    }

    narrata_storage::conformance_tests!(Wrapped);
}

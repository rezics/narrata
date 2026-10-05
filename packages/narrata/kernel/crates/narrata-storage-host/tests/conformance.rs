//! The storage contract holds for a cache whose host answers every miss and persists every
//! batch: once with a cache that keeps what it loaded, once with one that loads every read.

use narrata_storage::testing::conformance::Harness;
use narrata_storage_host::{
    StoreId,
    testing::{Driven, MemoryHost},
};

fn host() -> MemoryHost {
    MemoryHost::new(StoreId::from_bytes([7; 16]))
}

mod warm {
    use super::*;

    struct Warm;

    impl Harness for Warm {
        type Backend = Driven;

        fn create(&mut self) -> Driven {
            Driven::warm(host())
        }

        fn reopen(&mut self, backend: Driven) -> Option<Driven> {
            Some(Driven::warm(backend.into_host()))
        }
    }

    narrata_storage::conformance_tests!(Warm);
}

mod cold {
    use super::*;

    struct Cold;

    impl Harness for Cold {
        type Backend = Driven;

        fn create(&mut self) -> Driven {
            Driven::cold(host())
        }

        fn reopen(&mut self, backend: Driven) -> Option<Driven> {
            Some(Driven::cold(backend.into_host()))
        }
    }

    narrata_storage::conformance_tests!(Cold);
}

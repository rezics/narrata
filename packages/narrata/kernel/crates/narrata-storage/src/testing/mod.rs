//! Test support for backends and for the engine built on them.
//!
//! [`conformance`] runs the contract against any backend through a [`conformance::Harness`];
//! [`FaultInjecting`] fails chosen calls before or after they reach the backend; [`Counting`]
//! records calls and returned items so tests can bound the work an operation does.

pub mod conformance;
mod counting;
mod faults;
mod model;

pub use counting::{Counting, Counts};
pub use faults::{Fault, FaultInjecting, FaultPlan, Trigger};

/// The backend calls the wrappers observe; `capabilities` is neither counted nor faulted.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Primitive {
    GetObjects,
    ScanObjects,
    ReadKey,
    ScanKeys,
    Apply,
}

impl Primitive {
    pub const ALL: [Self; 5] = [
        Self::GetObjects,
        Self::ScanObjects,
        Self::ReadKey,
        Self::ScanKeys,
        Self::Apply,
    ];

    const fn index(self) -> usize {
        match self {
            Self::GetObjects => 0,
            Self::ScanObjects => 1,
            Self::ReadKey => 2,
            Self::ScanKeys => 3,
            Self::Apply => 4,
        }
    }
}

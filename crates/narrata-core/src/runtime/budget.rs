use std::num::NonZeroU64;

use thiserror::Error;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SliceBudget(Option<NonZeroU64>);

impl SliceBudget {
    pub fn new(instructions: u64) -> Result<Self, SliceBudgetError> {
        NonZeroU64::new(instructions)
            .map(|value| Self(Some(value)))
            .ok_or(SliceBudgetError)
    }

    pub const fn unlimited() -> Self {
        Self(None)
    }

    pub(crate) fn instructions(self) -> Option<u64> {
        self.0.map(NonZeroU64::get)
    }
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
#[error("slice budget must be greater than zero")]
pub struct SliceBudgetError;

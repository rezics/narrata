//! Strong identities with textual namespaces and fixed binary representations.

mod authored;
mod derived;

pub use authored::*;
pub use derived::*;
pub use narrata_kernel::{authored_id, derived_id, identity::IdParseError};

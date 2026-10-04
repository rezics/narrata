//! Domain-independent binary primitives shared by Narrata's narrative domains.
//!
//! These codecs preserve the ADR 0003 byte profile. Schema meaning and the set of
//! supported object kind codes belong to the domain using them.

pub mod codec;
pub mod content;
pub mod identity;

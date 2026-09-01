//! Independent version axes. A change on one wire contract does not silently version the rest.

macro_rules! version_type {
    ($name:ident, $current:ident) => {
        #[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
        pub struct $name(u16);

        impl $name {
            pub const fn new(value: u16) -> Self {
                Self(value)
            }

            pub const fn get(self) -> u16 {
                self.0
            }
        }

        pub const $current: $name = $name::new(0);
    };
}

version_type!(ProgramFormatVersion, PROGRAM_FORMAT_V0);
version_type!(SemanticsVersion, SEMANTICS_V0);
version_type!(SnapshotSchemaVersion, SNAPSHOT_SCHEMA_V0);
version_type!(ReceiptSchemaVersion, RECEIPT_SCHEMA_V0);
version_type!(EnvelopeVersion, ENVELOPE_V0);

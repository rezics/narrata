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
/// Program format 1 keeps reader text out of the Program (ADR 0018).
pub const PROGRAM_FORMAT_V1: ProgramFormatVersion = ProgramFormatVersion::new(1);
/// Snapshot schema 1 records content-table indices instead of text (ADR 0018).
pub const SNAPSHOT_SCHEMA_V1: SnapshotSchemaVersion = SnapshotSchemaVersion::new(1);

impl ProgramFormatVersion {
    /// The only Snapshot schema a state of this Program format may use.
    pub const fn snapshot_schema(self) -> Option<SnapshotSchemaVersion> {
        match self.0 {
            0 => Some(SNAPSHOT_SCHEMA_V0),
            1 => Some(SNAPSHOT_SCHEMA_V1),
            _ => None,
        }
    }
}
version_type!(ReceiptSchemaVersion, RECEIPT_SCHEMA_V0);
version_type!(EnvelopeVersion, ENVELOPE_V0);
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ProtocolVersion(u16);

impl ProtocolVersion {
    pub const fn new(value: u16) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u16 {
        self.0
    }
}

pub const PROTOCOL_V1: ProtocolVersion = ProtocolVersion::new(1);
/// Results carry content references instead of text (ADR 0018).
pub const PROTOCOL_V2: ProtocolVersion = ProtocolVersion::new(2);

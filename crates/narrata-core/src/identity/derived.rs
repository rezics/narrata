use std::{fmt, str::FromStr};

use super::{IdParseError, text};

macro_rules! derived_id {
    ($name:ident, $prefix:literal) => {
        #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name([u8; 32]);

        impl $name {
            pub const fn from_bytes(bytes: [u8; 32]) -> Self {
                Self(bytes)
            }

            pub const fn as_bytes(&self) -> &[u8; 32] {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(formatter, concat!($prefix, "{}"), hex::encode(self.0))
            }
        }

        impl FromStr for $name {
            type Err = IdParseError;

            fn from_str(input: &str) -> Result<Self, Self::Err> {
                text::parse(input, $prefix).map(Self)
            }
        }
    };
}

derived_id!(ProgramArtifactId, "artifact:");
derived_id!(InteractionId, "interaction:");
derived_id!(StateDigest, "state:");
derived_id!(InputPayloadDigest, "input-payload:");
derived_id!(ReceiptDigest, "receipt:");
derived_id!(ObjectId, "object:");
derived_id!(SnapshotId, "snapshot:");
derived_id!(ReceiptId, "stored-receipt:");
derived_id!(CommitId, "commit:");
derived_id!(TimelineCatalogEventId, "catalog-event:");
derived_id!(CheckpointBundleManifestId, "checkpoint-manifest:");
derived_id!(TimelineArchiveManifestId, "timeline-manifest:");
derived_id!(EffectId, "effect:");
derived_id!(EffectPayloadDigest, "effect-payload:");
derived_id!(EffectRequestDigest, "effect-request:");
derived_id!(DiagnosticId, "diagnostic:");
derived_id!(HostSnapshotDigest, "host-snapshot:");
derived_id!(ContentLockId, "content-lock:");
derived_id!(CompoundSaveManifestId, "compound-save:");
derived_id!(HostTimelineManifestId, "host-timeline:");
derived_id!(BuildProvenanceId, "build-provenance:");

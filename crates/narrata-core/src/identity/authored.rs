use std::{fmt, str::FromStr};

use super::{IdParseError, text};

macro_rules! authored_id {
    ($name:ident, $prefix:literal) => {
        #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name([u8; 16]);

        impl $name {
            pub const fn from_bytes(bytes: [u8; 16]) -> Self {
                Self(bytes)
            }

            pub const fn from_u128(value: u128) -> Self {
                Self(value.to_be_bytes())
            }

            pub const fn as_bytes(&self) -> &[u8; 16] {
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

authored_id!(ProgramId, "program:");
authored_id!(ExecutionId, "execution:");
authored_id!(InputId, "input:");
authored_id!(FlowId, "flow:");
authored_id!(InstructionId, "instruction:");
authored_id!(ChoiceId, "choice:");
authored_id!(GlobalId, "global:");
authored_id!(LocalId, "local:");
authored_id!(TypeId, "type:");
authored_id!(VariantId, "variant:");
authored_id!(FieldId, "field:");
authored_id!(EntityId, "entity:");
authored_id!(LayerId, "layer:");
authored_id!(ActorId, "actor:");
authored_id!(AudioChannelId, "audio-channel:");
authored_id!(StateId, "state:");
authored_id!(RegionId, "region:");
authored_id!(TransitionId, "transition:");
authored_id!(EventTypeId, "event-type:");
authored_id!(HistoryId, "history:");
authored_id!(ActionId, "action:");
authored_id!(MigrationId, "migration:");
authored_id!(RecoveryCheckpointId, "recovery-checkpoint:");
authored_id!(SourceDocumentId, "source-document:");
authored_id!(CheckpointId, "checkpoint:");
authored_id!(ContentOccurrenceId, "content-occurrence:");

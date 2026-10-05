pub use narrata_kernel::content::{
    AnchorId, ContentError, ContentKey, ContentRef, ProviderId, Segment,
};

use crate::codec::{CborReader, CborWriter, DecodeError};

/// One entry of the Program format 1 content table (ADR 0018). The reference is structure and
/// part of the Program identity; the text it resolves to is not.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ContentEntryV1 {
    /// A short text: a speaker, a prompt or an option label.
    Ref(ContentRef),
    /// Blocks of one content unit: the body of a `Say`.
    Segment(Segment),
}

impl ContentEntryV1 {
    /// Canonical form: `[0, ContentRef]` or `[1, Segment]`.
    pub(crate) fn encode(&self, writer: &mut CborWriter) {
        writer.array(2);
        match self {
            Self::Ref(reference) => {
                writer.unsigned(0);
                reference.encode(writer);
            }
            Self::Segment(segment) => {
                writer.unsigned(1);
                segment.encode(writer);
            }
        }
    }

    pub(crate) fn decode(reader: &mut CborReader<'_>) -> Result<Self, DecodeError> {
        if reader.array_len()? != 2 {
            return Err(DecodeError::Schema("content entry"));
        }
        match reader.unsigned()? {
            0 => Ok(Self::Ref(ContentRef::decode(reader)?)),
            1 => Ok(Self::Segment(Segment::decode(reader)?)),
            _ => Err(DecodeError::Schema("content entry tag")),
        }
    }

    pub fn kind_name(&self) -> &'static str {
        match self {
            Self::Ref(_) => "reference",
            Self::Segment(_) => "segment",
        }
    }
}

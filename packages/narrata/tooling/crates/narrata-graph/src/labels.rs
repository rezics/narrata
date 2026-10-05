//! ADR 0016 label schema 1. Title references are separate from geometry identities.

use std::collections::BTreeMap;

use narrata_kernel::{
    codec::{CborReader, CborWriter, DecodeError, decode_checked},
    content::ContentRef,
};

use crate::{
    ClusterId, Id, MAX_NODES,
    wire::{self, EncodedObject, LABEL_KIND},
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LabelTable {
    pub cluster: ClusterId,
    /// Only nodes with a title have a row. The host resolves every reference.
    pub titles: BTreeMap<Id, ContentRef>,
}

fn payload(table: &LabelTable) -> Vec<u8> {
    let mut writer = CborWriter::new();
    writer.map(2);
    writer.unsigned(0);
    writer.bytes(&table.cluster.0);
    writer.unsigned(1);
    writer.array(table.titles.len() as u64);
    for (id, title) in &table.titles {
        writer.array(2);
        writer.bytes(&id.0);
        title.encode(&mut writer);
    }
    writer.into_bytes()
}

pub fn encode_labels(table: &LabelTable) -> Result<EncodedObject, DecodeError> {
    if table.titles.len() > MAX_NODES {
        return Err(DecodeError::Limit("label rows"));
    }
    wire::object(LABEL_KIND, payload(table))
}

/// Checks the envelope, requested identity, canonical field set, sorted unique node IDs,
/// content references and the requested cluster. It does not infer node membership.
pub fn decode_labels(
    bytes: &[u8],
    id: &[u8; 32],
    cluster: ClusterId,
) -> Result<LabelTable, DecodeError> {
    let bytes = wire::payload(bytes, id, LABEL_KIND)?;
    let table = decode_checked(bytes, &wire::limits(), read, payload)?;
    if table.cluster != cluster {
        return Err(DecodeError::Schema("label cluster"));
    }
    Ok(table)
}

fn read(reader: &mut CborReader<'_>) -> Result<LabelTable, DecodeError> {
    if reader.map_len()? != 2 || reader.unsigned()? != 0 {
        return Err(DecodeError::Schema("label fields"));
    }
    let cluster = ClusterId(reader.bytes_exact()?);
    if reader.unsigned()? != 1 {
        return Err(DecodeError::Schema("label fields"));
    }
    let count = reader.array_len()?;
    if count > MAX_NODES as u64 {
        return Err(DecodeError::Limit("label rows"));
    }
    let mut titles = BTreeMap::new();
    let mut previous = None;
    for _ in 0..count {
        if reader.array_len()? != 2 {
            return Err(DecodeError::Schema("label row"));
        }
        let id = Id(reader.bytes_exact()?);
        if previous.is_some_and(|previous| previous >= id) {
            return Err(DecodeError::NonCanonical("label ID order"));
        }
        previous = Some(id);
        titles.insert(id, ContentRef::decode(reader)?);
    }
    Ok(LabelTable { cluster, titles })
}

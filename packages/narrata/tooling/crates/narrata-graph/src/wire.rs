//! ADR 0016 schema 1. Map keys and positional rows below are the wire contract.

use std::collections::BTreeSet;

use narrata_kernel::codec::{
    CborReader, CborWriter, DecodeError, DecodeLimits, EnvelopeLimits, decode_checked,
    decode_envelope, encode_envelope, object_id,
};

use crate::{
    ClusterId, Columns, DetailLevel, MAX_EDGES, MAX_NODES, Publication, TILE_ELEMENT_LIMIT, Tile,
};

pub const INDEX_KIND: u16 = 0x0120;
pub const WORK_KIND: u16 = 0x0121;
pub const CLUSTER_KIND: u16 = 0x0122;
pub const NODE_KIND: u16 = 0x0123;
/// Per-cluster title references; see `crate::LabelTable`.
pub const LABEL_KIND: u16 = 0x0124;
pub const SCHEMA_VERSION: u16 = 1;
pub const MAX_OBJECT_BYTES: u64 = 512 * 1024 * 1024;
const MAX_TILES: usize = 2 * MAX_NODES + MAX_EDGES + 1;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EncodedObject {
    pub id: [u8; 32],
    pub bytes: Vec<u8>,
}

impl EncodedObject {
    pub fn filename(&self) -> String {
        format!("{}.cbor", hex::encode(self.id))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Bounds {
    pub min_x: i64,
    pub min_y: i64,
    pub max_x: i64,
    pub max_y: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClusterIndex {
    pub id: ClusterId,
    pub origin_x: i64,
    pub origin_y: i64,
    /// Indices into `TileIndex::tiles`, including the shared work tile.
    pub tiles: Vec<u32>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TileEntry {
    pub level: DetailLevel,
    pub cluster: Option<u32>,
    pub partition: Option<u32>,
    pub ordinal: u32,
    pub object_id: [u8; 32],
    /// Work uses overview coordinates; other levels use world coordinates.
    pub bounds: Option<Bounds>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TileIndex {
    clusters: Vec<ClusterIndex>,
    tiles: Vec<TileEntry>,
}

impl TileIndex {
    pub fn clusters(&self) -> &[ClusterIndex] {
        &self.clusters
    }
    pub fn tiles(&self) -> &[TileEntry] {
        &self.tiles
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EncodedGraph {
    pub index: EncodedObject,
    /// Same order as the decoded index's tile entries.
    pub tiles: Vec<EncodedObject>,
}

fn kind(level: DetailLevel) -> u16 {
    match level {
        DetailLevel::Work => WORK_KIND,
        DetailLevel::Cluster => CLUSTER_KIND,
        DetailLevel::Node => NODE_KIND,
    }
}

fn level(code: u64) -> Result<DetailLevel, DecodeError> {
    match code {
        0 => Ok(DetailLevel::Work),
        1 => Ok(DetailLevel::Cluster),
        2 => Ok(DetailLevel::Node),
        _ => Err(DecodeError::Schema("detail level")),
    }
}

fn level_code(level: DetailLevel) -> u64 {
    match level {
        DetailLevel::Work => 0,
        DetailLevel::Cluster => 1,
        DetailLevel::Node => 2,
    }
}

pub(crate) fn limits() -> DecodeLimits {
    DecodeLimits {
        max_payload_bytes: MAX_OBJECT_BYTES,
        max_depth: 8,
        max_string_bytes: MAX_OBJECT_BYTES,
        max_collection_items: MAX_TILES as u64,
        max_total_items: MAX_TILES as u64 * 24,
    }
}

pub(crate) fn object(kind: u16, payload: Vec<u8>) -> Result<EncodedObject, DecodeError> {
    if payload.len() as u64 + 56 > MAX_OBJECT_BYTES {
        return Err(DecodeError::Limit("graph object bytes"));
    }
    Ok(EncodedObject {
        id: object_id(kind, SCHEMA_VERSION, &payload),
        bytes: encode_envelope(kind, SCHEMA_VERSION, &payload),
    })
}

pub(crate) fn payload<'a>(
    bytes: &'a [u8],
    id: &[u8; 32],
    kind: u16,
) -> Result<&'a [u8], DecodeError> {
    let envelope = decode_envelope(
        bytes,
        kind,
        SCHEMA_VERSION,
        &EnvelopeLimits {
            max_envelope_bytes: MAX_OBJECT_BYTES,
            max_payload_bytes: MAX_OBJECT_BYTES - 56,
        },
    )?;
    if object_id(kind, SCHEMA_VERSION, envelope.payload) != *id {
        return Err(DecodeError::Schema("graph object identity"));
    }
    Ok(envelope.payload)
}

/// Encodes all geometry and its index. Summary JSON is encoded separately so an
/// adapter can attach the program artifact ID without changing geometry IDs.
pub fn encode_publication(publication: &Publication) -> Result<EncodedGraph, DecodeError> {
    let mut index = TileIndex {
        clusters: publication
            .layout
            .clusters
            .iter()
            .map(|cluster| ClusterIndex {
                id: cluster.id,
                origin_x: cluster.origin_x,
                origin_y: cluster.origin_y,
                tiles: vec![0],
            })
            .collect(),
        tiles: Vec::new(),
    };
    let mut tiles = Vec::new();
    for tile in &publication.tiles {
        validate_tile(tile, index.clusters.len())?;
        validate_work_ids(tile, &index.clusters)?;
        let encoded = object(kind(tile.level), tile_payload(tile))?;
        let tile_index =
            u32::try_from(index.tiles.len()).map_err(|_| DecodeError::LengthOverflow)?;
        if let Some(cluster) = tile.cluster {
            index.clusters[cluster as usize].tiles.push(tile_index);
        }
        index.tiles.push(TileEntry {
            level: tile.level,
            cluster: tile.cluster,
            partition: tile.partition,
            ordinal: tile.ordinal,
            object_id: encoded.id,
            bounds: bounds(tile, &index.clusters)?,
        });
        tiles.push(encoded);
    }
    Ok(EncodedGraph {
        index: encode_index(&index)?,
        tiles,
    })
}

pub fn encode_index(index: &TileIndex) -> Result<EncodedObject, DecodeError> {
    validate_index(index)?;
    object(INDEX_KIND, index_payload(index))
}

pub fn decode_index(bytes: &[u8], expected_id: &[u8; 32]) -> Result<TileIndex, DecodeError> {
    decode_checked(
        payload(bytes, expected_id, INDEX_KIND)?,
        &limits(),
        read_index,
        index_payload,
    )
}

/// The index supplies the expected kind, identity, frame table and metadata.
/// A checked index alone does not imply that its referenced objects exist.
pub fn decode_tile(bytes: &[u8], index: &TileIndex, tile_index: u32) -> Result<Tile, DecodeError> {
    // The private index fields preserve its checked construction. Avoid an
    // O(all index rows) validation for every lazy tile fetch.
    let entry = index
        .tiles
        .get(tile_index as usize)
        .ok_or(DecodeError::Schema("tile index"))?;
    let tile = decode_checked(
        payload(bytes, &entry.object_id, kind(entry.level))?,
        &limits(),
        |reader| read_tile(reader, entry.level, index.clusters.len()),
        tile_payload,
    )?;
    if (tile.cluster, tile.partition, tile.ordinal)
        != (entry.cluster, entry.partition, entry.ordinal)
        || bounds(&tile, &index.clusters)? != entry.bounds
    {
        return Err(DecodeError::Schema("tile index metadata or bounds"));
    }
    validate_work_ids(&tile, &index.clusters)?;
    Ok(tile)
}

fn validate_work_ids(tile: &Tile, clusters: &[ClusterIndex]) -> Result<(), DecodeError> {
    if tile.level == DetailLevel::Work
        && tile
            .columns
            .ids
            .iter()
            .zip(clusters)
            .any(|(id, cluster)| *id != cluster.id.0)
    {
        return Err(DecodeError::Schema("work cluster identity"));
    }
    Ok(())
}

fn shape(
    level: DetailLevel,
    cluster: Option<u32>,
    partition: Option<u32>,
    ordinal: u32,
    clusters: usize,
) -> Result<(), DecodeError> {
    let valid = match level {
        DetailLevel::Work => cluster.is_none() && partition.is_none() && ordinal == 0,
        DetailLevel::Cluster => cluster.is_some() && partition.is_none() && ordinal == 0,
        DetailLevel::Node => {
            cluster.is_some() && partition.is_some_and(|p| (p as usize) < MAX_NODES)
        }
    };
    if !valid || cluster.is_some_and(|c| c as usize >= clusters) {
        return Err(DecodeError::Schema("tile ownership"));
    }
    Ok(())
}

fn validate_index(index: &TileIndex) -> Result<(), DecodeError> {
    if index.clusters.len() > MAX_NODES || index.tiles.len() > MAX_TILES {
        return Err(DecodeError::Limit("graph index rows"));
    }
    if index.tiles.len() < index.clusters.len() + 1
        || index.tiles[0].level != DetailLevel::Work
        || index
            .clusters
            .windows(2)
            .any(|pair| pair[0].id >= pair[1].id)
    {
        return Err(DecodeError::Schema("index order or work tile"));
    }
    let mut mapping = vec![vec![0_u32]; index.clusters.len()];
    let mut identities = BTreeSet::new();
    let mut previous = None;
    let mut partition = None;
    let mut partition_cluster = None;
    let mut ordinal = 0;
    for (row, tile) in index.tiles.iter().enumerate() {
        shape(
            tile.level,
            tile.cluster,
            tile.partition,
            tile.ordinal,
            index.clusters.len(),
        )?;
        let key = (tile.level, tile.cluster, tile.partition, tile.ordinal);
        if previous.is_some_and(|previous| previous >= key) || !identities.insert(tile.object_id) {
            return Err(DecodeError::Schema("tile order or duplicate identity"));
        }
        previous = Some(key);
        if tile
            .bounds
            .is_some_and(|b| b.min_x > b.max_x || b.min_y > b.max_y)
            || (tile.bounds.is_none() && !(row == 0 && index.clusters.is_empty()))
        {
            return Err(DecodeError::Schema("tile bounds"));
        }
        if row > 0
            && row <= index.clusters.len()
            && (tile.level != DetailLevel::Cluster || tile.cluster != Some((row - 1) as u32))
        {
            return Err(DecodeError::Schema("cluster tile coverage"));
        }
        if row > index.clusters.len() && tile.level != DetailLevel::Node {
            return Err(DecodeError::Schema("node tile coverage"));
        }
        if tile.level == DetailLevel::Node {
            if tile.partition != partition {
                if tile.partition != Some(partition.map_or(0, |p| p + 1)) || tile.ordinal != 0 {
                    return Err(DecodeError::Schema("partition sequence"));
                }
                partition = tile.partition;
                partition_cluster = tile.cluster;
            } else if tile.ordinal != ordinal + 1 || tile.cluster != partition_cluster {
                return Err(DecodeError::Schema("tile ordinal sequence"));
            }
            ordinal = tile.ordinal;
        }
        if let Some(cluster) = tile.cluster {
            mapping[cluster as usize].push(row as u32);
        }
    }
    for (cluster, expected) in index.clusters.iter().zip(mapping) {
        if cluster.tiles != expected
            || !cluster
                .tiles
                .iter()
                .any(|row| index.tiles[*row as usize].level == DetailLevel::Node)
        {
            return Err(DecodeError::Schema("cluster tile mapping"));
        }
    }
    Ok(())
}

fn validate_tile(tile: &Tile, clusters: usize) -> Result<(), DecodeError> {
    shape(
        tile.level,
        tile.cluster,
        tile.partition,
        tile.ordinal,
        clusters,
    )?;
    let c = &tile.columns;
    let nodes = c.ids.len();
    let edges = c.sources.len();
    if nodes > MAX_NODES
        || edges > MAX_EDGES
        || (tile.level == DetailLevel::Node && c.element_count() > TILE_ELEMENT_LIMIT)
    {
        return Err(DecodeError::Limit("tile elements"));
    }
    if [
        c.x.len(),
        c.y.len(),
        c.clusters.len(),
        c.kinds.len(),
        c.importance.len(),
        c.node_flags.len(),
        c.author_order.len(),
        c.has_author_order.len(),
    ]
    .iter()
    .any(|len| *len != nodes)
        || [
            c.targets.len(),
            c.edge_kind_masks.len(),
            c.edge_kind_counts.len(),
            c.edge_importance.len(),
            c.edge_flags.len(),
        ]
        .iter()
        .any(|len| *len != edges)
    {
        return Err(DecodeError::Schema("column row counts"));
    }
    if (nodes == 0 && (tile.level != DetailLevel::Work || clusters != 0))
        || (tile.level == DetailLevel::Work && nodes != clusters)
        || c.ids.iter().collect::<BTreeSet<_>>().len() != nodes
    {
        return Err(DecodeError::Schema("tile node identities"));
    }
    for row in 0..nodes {
        if c.clusters[row] as usize >= clusters
            || c.has_author_order[row] > 1
            || (c.has_author_order[row] == 0 && c.author_order[row] != 0)
            || c.node_flags[row] & !63 != 0
        {
            return Err(DecodeError::Schema("node columns"));
        }
        let valid = match tile.level {
            DetailLevel::Work => c.kinds[row] == 6 && c.clusters[row] as usize == row,
            DetailLevel::Cluster => c.kinds[row] == 7 && Some(c.clusters[row]) == tile.cluster,
            DetailLevel::Node => {
                c.kinds[row] <= 5
                    && (Some(c.clusters[row]) == tile.cluster || c.node_flags[row] & 32 != 0)
                    && (c.kinds[row] != 5 || c.node_flags[row] & 4 != 0)
            }
        };
        if !valid
            || (tile.level != DetailLevel::Node
                && (c.node_flags[row] != 0 || c.has_author_order[row] != 0))
        {
            return Err(DecodeError::Schema("node kind or level"));
        }
    }
    let mut total_edges = 0_u64;
    for row in 0..edges {
        let source = c.sources[row] as usize;
        let target = c.targets[row] as usize;
        if source >= nodes || target >= nodes {
            return Err(DecodeError::Schema("edge endpoint index"));
        }
        let mask = c.edge_kind_counts[row]
            .iter()
            .enumerate()
            .fold(0, |mask, (kind, count)| {
                mask | (u8::from(*count > 0) << kind)
            });
        total_edges += c.edge_kind_counts[row]
            .iter()
            .map(|count| u64::from(*count))
            .sum::<u64>();
        if mask == 0
            || mask != c.edge_kind_masks[row]
            || c.edge_flags[row] & !3 != 0
            || c.edge_importance[row] != c.importance[source].max(c.importance[target])
        {
            return Err(DecodeError::Schema("edge columns"));
        }
    }
    if total_edges > MAX_EDGES as u64 {
        return Err(DecodeError::Limit("edge multiplicity"));
    }
    Ok(())
}

fn bounds(tile: &Tile, clusters: &[ClusterIndex]) -> Result<Option<Bounds>, DecodeError> {
    let mut bounds: Option<Bounds> = None;
    for row in 0..tile.columns.ids.len() {
        let mut x = i64::from(tile.columns.x[row]);
        let mut y = i64::from(tile.columns.y[row]);
        if tile.level != DetailLevel::Work {
            let cluster = clusters
                .get(tile.columns.clusters[row] as usize)
                .ok_or(DecodeError::Schema("cluster index"))?;
            x = x
                .checked_add(cluster.origin_x)
                .ok_or(DecodeError::IntegerOverflow)?;
            y = y
                .checked_add(cluster.origin_y)
                .ok_or(DecodeError::IntegerOverflow)?;
        }
        bounds = Some(match bounds {
            None => Bounds {
                min_x: x,
                min_y: y,
                max_x: x,
                max_y: y,
            },
            Some(b) => Bounds {
                min_x: b.min_x.min(x),
                min_y: b.min_y.min(y),
                max_x: b.max_x.max(x),
                max_y: b.max_y.max(y),
            },
        });
    }
    Ok(bounds)
}

fn option(writer: &mut CborWriter, value: Option<u32>) {
    match value {
        Some(value) => writer.unsigned(u64::from(value)),
        None => writer.null(),
    }
}

fn field(reader: &mut CborReader<'_>, key: u64) -> Result<(), DecodeError> {
    if reader.unsigned()? != key {
        return Err(DecodeError::Schema("field set"));
    }
    Ok(())
}

fn length(actual: u64, expected: u64) -> Result<(), DecodeError> {
    if actual != expected {
        return Err(DecodeError::Schema("container length"));
    }
    Ok(())
}

fn u32_value(reader: &mut CborReader<'_>) -> Result<u32, DecodeError> {
    u32::try_from(reader.unsigned()?).map_err(|_| DecodeError::IntegerOverflow)
}

fn row_count(reader: &mut CborReader<'_>, max: usize) -> Result<usize, DecodeError> {
    let value = reader.unsigned()?;
    if value > max as u64 {
        return Err(DecodeError::Limit("row count"));
    }
    usize::try_from(value).map_err(|_| DecodeError::LengthOverflow)
}

fn array_count(reader: &mut CborReader<'_>, max: usize) -> Result<usize, DecodeError> {
    let value = reader.array_len()?;
    if value > max as u64 {
        return Err(DecodeError::Limit("array rows"));
    }
    usize::try_from(value).map_err(|_| DecodeError::LengthOverflow)
}

fn index_payload(index: &TileIndex) -> Vec<u8> {
    let mut w = CborWriter::new();
    w.map(2);
    w.unsigned(0);
    w.array(index.clusters.len() as u64);
    for cluster in &index.clusters {
        w.array(4);
        w.bytes(&cluster.id.0);
        w.signed(cluster.origin_x);
        w.signed(cluster.origin_y);
        w.array(cluster.tiles.len() as u64);
        for tile in &cluster.tiles {
            w.unsigned(u64::from(*tile));
        }
    }
    w.unsigned(1);
    w.array(index.tiles.len() as u64);
    for tile in &index.tiles {
        w.array(6);
        w.unsigned(level_code(tile.level));
        option(&mut w, tile.cluster);
        option(&mut w, tile.partition);
        w.unsigned(u64::from(tile.ordinal));
        w.bytes(&tile.object_id);
        match tile.bounds {
            None => w.null(),
            Some(b) => {
                w.array(4);
                for v in [b.min_x, b.min_y, b.max_x, b.max_y] {
                    w.signed(v);
                }
            }
        }
    }
    w.into_bytes()
}

fn read_index(r: &mut CborReader<'_>) -> Result<TileIndex, DecodeError> {
    length(r.map_len()?, 2)?;
    field(r, 0)?;
    let mut clusters = Vec::new();
    let mut mapping_rows = 0;
    for _ in 0..array_count(r, MAX_NODES)? {
        length(r.array_len()?, 4)?;
        let id = ClusterId(r.bytes_exact()?);
        let origin_x = r.signed()?;
        let origin_y = r.signed()?;
        let mut tiles = Vec::new();
        let count = array_count(r, MAX_TILES)?;
        mapping_rows += count;
        if mapping_rows > MAX_TILES + MAX_NODES {
            return Err(DecodeError::Limit("cluster mappings"));
        }
        for _ in 0..count {
            tiles.push(u32_value(r)?);
        }
        clusters.push(ClusterIndex {
            id,
            origin_x,
            origin_y,
            tiles,
        });
    }
    field(r, 1)?;
    let mut tiles = Vec::new();
    for _ in 0..array_count(r, MAX_TILES)? {
        length(r.array_len()?, 6)?;
        tiles.push(TileEntry {
            level: level(r.unsigned()?)?,
            cluster: r.optional(u32_value)?,
            partition: r.optional(u32_value)?,
            ordinal: u32_value(r)?,
            object_id: r.bytes_exact()?,
            bounds: r.optional(|r| {
                length(r.array_len()?, 4)?;
                Ok(Bounds {
                    min_x: r.signed()?,
                    min_y: r.signed()?,
                    max_x: r.signed()?,
                    max_y: r.signed()?,
                })
            })?,
        });
    }
    let index = TileIndex { clusters, tiles };
    validate_index(&index)?;
    Ok(index)
}

fn packed<T, const N: usize>(values: &[T], bytes: impl Fn(&T) -> [u8; N]) -> Vec<u8> {
    values.iter().flat_map(bytes).collect()
}

fn tile_payload(tile: &Tile) -> Vec<u8> {
    let c = &tile.columns;
    let mut w = CborWriter::new();
    w.map(6);
    w.unsigned(0);
    option(&mut w, tile.cluster);
    w.unsigned(1);
    option(&mut w, tile.partition);
    w.unsigned(2);
    w.unsigned(u64::from(tile.ordinal));
    w.unsigned(3);
    w.unsigned(c.ids.len() as u64);
    w.unsigned(4);
    w.unsigned(c.sources.len() as u64);
    w.unsigned(5);
    w.map(15);
    // 0..9 node columns; 9..15 edge columns. All integers are little endian.
    w.unsigned(0);
    w.bytes(&packed(&c.ids, |id| *id));
    w.unsigned(1);
    w.bytes(&packed(&c.x, |v| v.to_le_bytes()));
    w.unsigned(2);
    w.bytes(&packed(&c.y, |v| v.to_le_bytes()));
    w.unsigned(3);
    w.bytes(&packed(&c.clusters, |v| v.to_le_bytes()));
    w.unsigned(4);
    w.bytes(&c.kinds);
    w.unsigned(5);
    w.bytes(&c.importance);
    w.unsigned(6);
    w.bytes(&packed(&c.node_flags, |v| v.to_le_bytes()));
    w.unsigned(7);
    w.bytes(&packed(&c.author_order, |v| v.to_le_bytes()));
    w.unsigned(8);
    w.bytes(&c.has_author_order);
    w.unsigned(9);
    w.bytes(&packed(&c.sources, |v| v.to_le_bytes()));
    w.unsigned(10);
    w.bytes(&packed(&c.targets, |v| v.to_le_bytes()));
    w.unsigned(11);
    w.bytes(&c.edge_kind_masks);
    w.unsigned(12);
    w.bytes(
        &c.edge_kind_counts
            .iter()
            .flat_map(|counts| counts.iter().flat_map(|v| v.to_le_bytes()))
            .collect::<Vec<_>>(),
    );
    w.unsigned(13);
    w.bytes(&c.edge_importance);
    w.unsigned(14);
    w.bytes(&c.edge_flags);
    w.into_bytes()
}

fn column<T, const N: usize>(
    r: &mut CborReader<'_>,
    key: u64,
    rows: usize,
    value: impl Fn([u8; N]) -> T,
) -> Result<Vec<T>, DecodeError> {
    field(r, key)?;
    let size = rows.checked_mul(N).ok_or(DecodeError::LengthOverflow)?;
    let bytes = r.bytes(size as u64)?;
    if bytes.len() != size {
        return Err(DecodeError::Schema("column byte length"));
    }
    Ok(bytes
        .as_chunks::<N>()
        .0
        .iter()
        .copied()
        .map(value)
        .collect())
}

fn read_tile(
    r: &mut CborReader<'_>,
    level: DetailLevel,
    clusters: usize,
) -> Result<Tile, DecodeError> {
    length(r.map_len()?, 6)?;
    field(r, 0)?;
    let cluster = r.optional(u32_value)?;
    field(r, 1)?;
    let partition = r.optional(u32_value)?;
    field(r, 2)?;
    let ordinal = u32_value(r)?;
    field(r, 3)?;
    let nodes = row_count(r, MAX_NODES)?;
    field(r, 4)?;
    let edges = row_count(r, MAX_EDGES)?;
    if level == DetailLevel::Node && nodes + edges > TILE_ELEMENT_LIMIT {
        return Err(DecodeError::Limit("tile elements"));
    }
    field(r, 5)?;
    length(r.map_len()?, 15)?;
    let columns = Columns {
        ids: column(r, 0, nodes, |v: [u8; 16]| v)?,
        x: column(r, 1, nodes, i32::from_le_bytes)?,
        y: column(r, 2, nodes, i32::from_le_bytes)?,
        clusters: column(r, 3, nodes, u32::from_le_bytes)?,
        kinds: column(r, 4, nodes, |v: [u8; 1]| v[0])?,
        importance: column(r, 5, nodes, |v: [u8; 1]| v[0])?,
        node_flags: column(r, 6, nodes, u32::from_le_bytes)?,
        author_order: column(r, 7, nodes, u64::from_le_bytes)?,
        has_author_order: column(r, 8, nodes, |v: [u8; 1]| v[0])?,
        sources: column(r, 9, edges, u32::from_le_bytes)?,
        targets: column(r, 10, edges, u32::from_le_bytes)?,
        edge_kind_masks: column(r, 11, edges, |v: [u8; 1]| v[0])?,
        edge_kind_counts: column(r, 12, edges, |v: [u8; 20]| {
            std::array::from_fn(|i| {
                u32::from_le_bytes([v[i * 4], v[i * 4 + 1], v[i * 4 + 2], v[i * 4 + 3]])
            })
        })?,
        edge_importance: column(r, 13, edges, |v: [u8; 1]| v[0])?,
        edge_flags: column(r, 14, edges, |v: [u8; 1]| v[0])?,
    };
    let tile = Tile {
        level,
        cluster,
        partition,
        ordinal,
        columns,
    };
    validate_tile(&tile, clusters)?;
    Ok(tile)
}

#[cfg(test)]
#[path = "../tests/support/mod.rs"]
mod test_support;

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use proptest::prelude::*;

    use super::test_support as support;

    fn prepared() -> (Publication, EncodedGraph, TileIndex) {
        let publication = crate::prepare(support::compatibility_graph()).unwrap();
        let objects = encode_publication(&publication).unwrap();
        let index = decode_index(&objects.index.bytes, &objects.index.id).unwrap();
        (publication, objects, index)
    }

    fn replaced_tile(tile: &Tile, index: &mut TileIndex, row: usize) -> Vec<u8> {
        let object = object(kind(tile.level), tile_payload(tile)).unwrap();
        index.tiles[row].object_id = object.id;
        object.bytes
    }

    #[test]
    fn checked_columns_reject_invalid_rows_even_with_recomputed_identity() {
        let (publication, _, index) = prepared();
        let row = publication
            .tiles
            .iter()
            .position(|tile| tile.level == DetailLevel::Node)
            .unwrap();
        for mutation in 0..13 {
            let mut tile = publication.tiles[row].clone();
            match mutation {
                0 => {
                    tile.columns.x.pop();
                }
                1 => tile.columns.targets[0] = tile.columns.ids.len() as u32,
                2 => tile.columns.clusters[0] = index.clusters.len() as u32,
                3 => tile.columns.kinds[0] = 255,
                4 => tile.columns.node_flags[0] = 64,
                5 => tile.columns.edge_kind_masks[0] ^= 1,
                6 => tile.columns.edge_flags[0] = 4,
                7 => tile.columns.has_author_order[0] = 2,
                8 => {
                    tile.columns.has_author_order[0] = 0;
                    tile.columns.author_order[0] = 1;
                }
                9 => tile.columns.ids[1] = tile.columns.ids[0],
                10 => tile.columns.edge_kind_counts[0] = [u32::MAX; 5],
                11 => tile.partition = None,
                _ => tile.columns.edge_importance[0] ^= 1,
            }
            let mut changed_index = index.clone();
            let bytes = replaced_tile(&tile, &mut changed_index, row);
            assert!(
                decode_tile(&bytes, &changed_index, row as u32).is_err(),
                "mutation {mutation}"
            );
        }
    }

    #[test]
    fn kind_schema_identity_metadata_bounds_and_limits_are_checked() {
        let (publication, objects, index) = prepared();
        let row = publication
            .tiles
            .iter()
            .position(|tile| tile.level == DetailLevel::Node)
            .unwrap();
        let raw = tile_payload(&publication.tiles[row]);
        for (kind, schema) in [(LABEL_KIND, 1), (NODE_KIND, 0), (NODE_KIND, 2)] {
            assert!(decode_tile(&encode_envelope(kind, schema, &raw), &index, row as u32).is_err());
        }
        assert!(decode_index(&objects.index.bytes, &[0; 32]).is_err());
        assert!(decode_tile(&objects.tiles[row].bytes, &index, u32::MAX).is_err());
        let mut changed = index.clone();
        changed.tiles[row].bounds.as_mut().unwrap().max_x += 1;
        assert!(decode_tile(&objects.tiles[row].bytes, &changed, row as u32).is_err());
        let mut tile = publication.tiles[row].clone();
        tile.ordinal += 1;
        let bytes = replaced_tile(&tile, &mut index.clone(), row);
        let mut changed = index.clone();
        changed.tiles[row].object_id = object_id(NODE_KIND, 1, &tile_payload(&tile));
        assert!(decode_tile(&bytes, &changed, row as u32).is_err());
        let mut r = CborReader::new(&[0x1a, 0xff, 0xff, 0xff, 0xff]);
        assert!(row_count(&mut r, MAX_NODES).is_err());
        let mut tile = publication.tiles[row].clone();
        tile.columns.ids.resize(TILE_ELEMENT_LIMIT + 1, [0; 16]);
        assert!(validate_tile(&tile, index.clusters.len()).is_err());
    }

    #[test]
    fn malformed_index_payloads_are_rejected_after_checksum_verification() {
        let (_, _, index) = prepared();
        for mutation in 0..9 {
            let mut changed = index.clone();
            match mutation {
                0 => changed.clusters[0].tiles.push(u32::MAX),
                1 => changed.tiles[1].cluster = Some(u32::MAX),
                2 => changed.tiles[1].level = DetailLevel::Work,
                3 => changed.clusters.swap(0, 1),
                4 => changed.tiles[1].object_id = changed.tiles[0].object_id,
                5 => changed.tiles[1].bounds.as_mut().unwrap().min_x = i64::MAX,
                6 => changed.tiles[1].bounds = None,
                7 => changed.tiles.last_mut().unwrap().ordinal = u32::MAX,
                _ => changed.tiles.clear(),
            }
            let object = object(INDEX_KIND, index_payload(&changed)).unwrap();
            assert!(
                decode_index(&object.bytes, &object.id).is_err(),
                "mutation {mutation}"
            );
        }
    }

    #[test]
    fn noncanonical_unknown_duplicate_and_trailing_fields_are_rejected() {
        let (_, objects, _) = prepared();
        let raw = payload(&objects.index.bytes, &objects.index.id, INDEX_KIND).unwrap();
        let mut nonminimal = raw.to_vec();
        nonminimal.splice(1..2, [0x18, 0]);
        let mut unknown = raw.to_vec();
        unknown[1] = 2;
        let mut duplicate = raw.to_vec();
        duplicate[0] = 0xa3;
        duplicate.extend_from_slice(&[1, 0x80]);
        let mut trailing = raw.to_vec();
        trailing.push(0);
        let mut indefinite = raw.to_vec();
        indefinite[0] = 0xbf;
        indefinite.push(0xff);
        for raw in [nonminimal, unknown, duplicate, trailing, indefinite] {
            let object = object(INDEX_KIND, raw).unwrap();
            assert!(decode_index(&object.bytes, &object.id).is_err());
        }
    }

    #[test]
    fn typed_columns_use_little_endian_including_signed_extremes() {
        let (publication, _, mut index) = prepared();
        let row = publication
            .tiles
            .iter()
            .position(|tile| tile.level == DetailLevel::Node)
            .unwrap();
        let mut tile = publication.tiles[row].clone();
        tile.columns.x[0] = i32::MIN;
        tile.columns.y[0] = i32::MAX;
        tile.columns.author_order[0] = u64::MAX;
        tile.columns.has_author_order[0] = 1;
        let raw = tile_payload(&tile);
        let mut r = CborReader::new(&raw);
        length(r.map_len().unwrap(), 6).unwrap();
        for key in 0..3 {
            field(&mut r, key).unwrap();
            r.optional(u32_value).unwrap();
        }
        for key in 3..5 {
            field(&mut r, key).unwrap();
            r.unsigned().unwrap();
        }
        field(&mut r, 5).unwrap();
        length(r.map_len().unwrap(), 15).unwrap();
        field(&mut r, 0).unwrap();
        assert_eq!(
            r.bytes(MAX_OBJECT_BYTES).unwrap(),
            packed(&tile.columns.ids, |id| *id)
        );
        field(&mut r, 1).unwrap();
        assert_eq!(
            &r.bytes(MAX_OBJECT_BYTES).unwrap()[..4],
            &i32::MIN.to_le_bytes()
        );
        field(&mut r, 2).unwrap();
        assert_eq!(
            &r.bytes(MAX_OBJECT_BYTES).unwrap()[..4],
            &i32::MAX.to_le_bytes()
        );
        let bytes = replaced_tile(&tile, &mut index, row);
        index.tiles[row].bounds = bounds(&tile, &index.clusters).unwrap();
        assert_eq!(decode_tile(&bytes, &index, row as u32).unwrap(), tile);
    }

    proptest! {
        #[test]
        fn any_single_byte_tile_tampering_is_rejected(row in 0_usize..13, offset in any::<usize>(), mask in 1_u8..=255) {
            let (_, objects, index) = prepared();
            let row = row % objects.tiles.len();
            let mut bytes = objects.tiles[row].bytes.clone();
            let offset = offset % bytes.len();
            bytes[offset] ^= mask;
            prop_assert!(decode_tile(&bytes, &index, row as u32).is_err());
        }

        #[test]
        fn any_single_byte_index_tampering_is_rejected(offset in any::<usize>(), mask in 1_u8..=255) {
            let (_, objects, _) = prepared();
            let mut bytes = objects.index.bytes.clone();
            let offset = offset % bytes.len();
            bytes[offset] ^= mask;
            prop_assert!(decode_index(&bytes, &objects.index.id).is_err());
        }

        #[test]
        fn out_of_range_endpoints_are_rejected_with_a_valid_checksum(endpoint in MAX_NODES as u32..=u32::MAX) {
            let (publication, _, mut index) = prepared();
            let row = publication.tiles.iter().position(|tile| tile.level == DetailLevel::Node).unwrap();
            let mut tile = publication.tiles[row].clone();
            tile.columns.targets[0] = endpoint;
            let bytes = replaced_tile(&tile, &mut index, row);
            prop_assert!(decode_tile(&bytes, &index, row as u32).is_err());
        }

        #[test]
        fn arbitrary_input_never_panics(bytes in prop::collection::vec(any::<u8>(), 0..2048)) {
            let (_, objects, index) = prepared();
            let _ = decode_index(&bytes, &objects.index.id);
            let _ = decode_tile(&bytes, &index, 0);
            let _ = crate::SemanticSummary::decode_json(&bytes);
        }
    }
}

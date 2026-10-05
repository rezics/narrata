//! Minting authored IDs (ADR 0013 §3). `compile` never mints; this tool writes UUIDv7 values
//! into the sources once, and from then on they are part of the work.

use std::{
    collections::BTreeSet,
    time::{SystemTime, UNIX_EPOCH},
};

use narrata_nodes::{
    AuthoredId, ChoicePointId, Error, ExecutionId, NodeId, OptionId, PackageSource, Result,
    SourcePlan, lower_builtin,
};

/// A version 7 UUID: 48 bits of Unix milliseconds, then random bits.
pub fn uuid_v7() -> Result<[u8; 16]> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| Error::new("clock", "ids", "the system clock is before 1970"))?
        .as_millis() as u64;
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes[6..]).map_err(|e| Error::new("random", "ids", e.to_string()))?;
    bytes[..6].copy_from_slice(&millis.to_be_bytes()[2..]);
    bytes[6] = 0x70 | (bytes[6] & 0x0f);
    bytes[8] = 0x80 | (bytes[8] & 0x3f);
    Ok(bytes)
}

pub fn execution_id() -> Result<ExecutionId> {
    Ok(ExecutionId::from_bytes(uuid_v7()?))
}

/// Mints IDs that are not in `taken`, and records them there.
pub struct Minter {
    taken: BTreeSet<[u8; 16]>,
    pub minted: usize,
}

impl Minter {
    pub fn new(taken: impl IntoIterator<Item = AuthoredId>) -> Self {
        Self {
            taken: taken.into_iter().map(|id| bytes(&id)).collect(),
            minted: 0,
        }
    }

    fn next(&mut self) -> Result<[u8; 16]> {
        loop {
            let id = uuid_v7()?;
            if self.taken.insert(id) {
                self.minted += 1;
                return Ok(id);
            }
        }
    }

    pub fn node(&mut self) -> Result<NodeId> {
        self.next().map(NodeId::from_bytes)
    }

    pub fn choice_point(&mut self) -> Result<ChoicePointId> {
        self.next().map(ChoicePointId::from_bytes)
    }

    pub fn option(&mut self) -> Result<OptionId> {
        self.next().map(OptionId::from_bytes)
    }
}

fn bytes(id: &AuthoredId) -> [u8; 16] {
    *match id {
        AuthoredId::Node(id) => id.as_bytes(),
        AuthoredId::ChoicePoint(id) => id.as_bytes(),
        AuthoredId::Option(id) => id.as_bytes(),
    }
}

/// Every ID a package already uses or has deleted.
pub fn package_ids(package: &PackageSource) -> Vec<AuthoredId> {
    let mut ids = package.tombstones.clone();
    for graph in package.graphs.values() {
        for node in graph.nodes.values() {
            ids.extend(node.id.map(AuthoredId::Node));
            if node.type_id == "narrata.passage"
                && let Ok(SourcePlan::Passage(passage)) = lower_builtin("passage", &node.data)
            {
                for point in &passage.choice_points {
                    ids.extend(point.id.map(AuthoredId::ChoicePoint));
                    ids.extend(
                        point
                            .options
                            .iter()
                            .filter_map(|option| option.id.map(AuthoredId::Option)),
                    );
                }
            }
        }
    }
    ids
}

/// Fills every missing node, choice point and option ID of a package. Choice points live in
/// the data of `narrata.passage` nodes; other node types carry only the node ID.
pub fn fill(package: &mut PackageSource, minter: &mut Minter) -> Result<()> {
    for graph in package.graphs.values_mut() {
        for node in graph.nodes.values_mut() {
            if node.id.is_none() {
                node.id = Some(minter.node()?);
            }
            if node.type_id != "narrata.passage" {
                continue;
            }
            let Some(points) = node
                .data
                .get_mut("choice_points")
                .and_then(serde_json::Value::as_array_mut)
            else {
                continue;
            };
            for point in points
                .iter_mut()
                .filter_map(serde_json::Value::as_object_mut)
            {
                if !point.contains_key("id") {
                    point.insert("id".into(), serde_json::json!(minter.choice_point()?));
                }
                let Some(options) = point
                    .get_mut("options")
                    .and_then(serde_json::Value::as_array_mut)
                else {
                    continue;
                };
                for option in options
                    .iter_mut()
                    .filter_map(serde_json::Value::as_object_mut)
                {
                    if !option.contains_key("id") {
                        option.insert("id".into(), serde_json::json!(minter.option()?));
                    }
                }
            }
        }
    }
    Ok(())
}

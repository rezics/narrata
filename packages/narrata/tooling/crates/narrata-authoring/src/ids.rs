//! UUIDv7 minting with caller-supplied clock and cryptographic randomness. No platform
//! capabilities are needed for checking, compilation or an unchanged edit.

use std::collections::BTreeSet;

use narrata_nodes::{
    AuthoredId, ChoicePointId, Error, NodeId, OptionId, PackageSource, ProjectSource, Result,
    SourcePlan, lower_builtin,
};

use crate::{CheckedRecord, split_source};

/// Clock returns Unix milliseconds; random fills the requested bytes using a secure source.
pub fn uuid_v7(
    clock: &mut impl FnMut() -> Result<u64>,
    random: &mut impl FnMut(&mut [u8]) -> Result<()>,
) -> Result<[u8; 16]> {
    let millis = clock()?;
    if millis >= 1 << 48 {
        return Err(Error::new(
            "clock",
            "ids",
            "UUIDv7 timestamp exceeds 48 bits",
        ));
    }
    let mut bytes = [0_u8; 16];
    random(&mut bytes[6..])?;
    bytes[..6].copy_from_slice(&millis.to_be_bytes()[2..]);
    bytes[6] = 0x70 | (bytes[6] & 0x0f);
    bytes[8] = 0x80 | (bytes[8] & 0x3f);
    Ok(bytes)
}

/// Occupancy is shared across authored kinds as well as live/deleted identities.
pub struct Minter<C, R> {
    taken: BTreeSet<[u8; 16]>,
    clock: C,
    random: R,
    pub minted: usize,
    pub allocations: Vec<AuthoredId>,
}

impl<C: FnMut() -> Result<u64>, R: FnMut(&mut [u8]) -> Result<()>> Minter<C, R> {
    pub fn new(taken: impl IntoIterator<Item = AuthoredId>, clock: C, random: R) -> Self {
        Self {
            taken: taken.into_iter().map(|id| bytes(&id)).collect(),
            clock,
            random,
            minted: 0,
            allocations: Vec::new(),
        }
    }

    fn next(&mut self) -> Result<[u8; 16]> {
        // A broken platform source must fail rather than freeze an editor indefinitely.
        for _ in 0..1024 {
            let id = uuid_v7(&mut self.clock, &mut self.random)?;
            if self.taken.insert(id) {
                self.minted += 1;
                return Ok(id);
            }
        }
        Err(Error::new(
            "random",
            "ids",
            "UUID collision retry budget exhausted",
        ))
    }

    pub fn node(&mut self) -> Result<NodeId> {
        let id = NodeId::from_bytes(self.next()?);
        self.allocations.push(AuthoredId::Node(id));
        Ok(id)
    }

    pub fn choice_point(&mut self) -> Result<ChoicePointId> {
        let id = ChoicePointId::from_bytes(self.next()?);
        self.allocations.push(AuthoredId::ChoicePoint(id));
        Ok(id)
    }

    pub fn option(&mut self) -> Result<OptionId> {
        let id = OptionId::from_bytes(self.next()?);
        self.allocations.push(AuthoredId::Option(id));
        Ok(id)
    }
}

fn bytes(id: &AuthoredId) -> [u8; 16] {
    *match id {
        AuthoredId::Node(id) => id.as_bytes(),
        AuthoredId::ChoicePoint(id) => id.as_bytes(),
        AuthoredId::Option(id) => id.as_bytes(),
    }
}

/// Every existing live or deleted ID. Invalid passage edits still reserve any well-formed
/// IDs they contain; fixing a schema error must never permit a collision with that edit.
pub fn package_ids(package: &PackageSource) -> Vec<AuthoredId> {
    let mut ids = package.tombstones.clone();
    for graph in package.graphs.values() {
        for node in graph.nodes.values() {
            ids.extend(node.id.map(AuthoredId::Node));
            if node.type_id == "narrata.passage"
                && let Some(points) = node
                    .data
                    .get("choice_points")
                    .and_then(serde_json::Value::as_array)
            {
                for point in points {
                    ids.extend(
                        point
                            .get("id")
                            .cloned()
                            .and_then(|v| serde_json::from_value::<ChoicePointId>(v).ok())
                            .map(AuthoredId::ChoicePoint),
                    );
                    if let Some(options) =
                        point.get("options").and_then(serde_json::Value::as_array)
                    {
                        ids.extend(options.iter().filter_map(|option| {
                            option
                                .get("id")
                                .cloned()
                                .and_then(|v| serde_json::from_value::<OptionId>(v).ok())
                                .map(AuthoredId::Option)
                        }));
                    }
                }
            }
        }
    }
    ids
}

/// Fills missing IDs without rewriting node data or replacing existing IDs.
pub fn fill<C: FnMut() -> Result<u64>, R: FnMut(&mut [u8]) -> Result<()>>(
    package: &mut PackageSource,
    minter: &mut Minter<C, R>,
) -> Result<()> {
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

#[derive(Clone, Debug)]
pub struct MintedEdit {
    pub source: ProjectSource,
    pub records: Vec<CheckedRecord>,
    pub minted: Vec<AuthoredId>,
}

/// An unminted R2 edit is an interface input, never a persisted draft record. Returning a
/// fresh value keeps platform failures from partially changing the caller's edit.
pub fn mint_ids(
    edit: &ProjectSource,
    occupied: impl IntoIterator<Item = AuthoredId>,
    clock: impl FnMut() -> Result<u64>,
    random: impl FnMut(&mut [u8]) -> Result<()>,
) -> Result<MintedEdit> {
    let taken = occupied
        .into_iter()
        .chain(edit.packages.values().flat_map(package_ids));
    let mut minter = Minter::new(taken, clock, random);
    let mut source = crate::records::normalize_source(edit)?;
    for package in source.packages.values_mut() {
        fill(package, &mut minter)?;
        // Validate built-in payloads before offering records for persistence.
        for graph in package.graphs.values() {
            for node in graph
                .nodes
                .values()
                .filter(|node| node.type_id == "narrata.passage")
            {
                let _: SourcePlan = lower_builtin("passage", &node.data)?;
            }
        }
    }
    let records = split_source(&source).into_result()?;
    Ok(MintedEdit {
        source,
        records,
        minted: minter.allocations,
    })
}

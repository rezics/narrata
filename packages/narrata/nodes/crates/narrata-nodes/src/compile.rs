//! Source to artifact (ADR 0013 §3, §8). `compile` never mints IDs: a missing, malformed or
//! repeated ID is an error. Aliases become IDs and move to the name table; every graph
//! becomes its own chunk.

use std::collections::{BTreeMap, BTreeSet};

use narrata_kernel::codec::digest_bytes;

use crate::{
    AuthoredId, ChoicePointId, CompositionLock, Diagnostic, Error, FORMAT_VERSION,
    MAX_SOURCE_BYTES, NodeId, NodeRegistry, OptionId, PackageLock, Program, ProjectSource, Result,
    SourcePlan,
    analysis::{Analysis, analyze},
    check::{check_chunk, check_manifest, name, sorted_names},
    expr::check_scalar,
    plan::{
        ChoicePoint, Chunk, Ending, Graph, GraphEntry, GraphHeader, GraphNames, GraphRef, Manifest,
        NameTable, OptionPlan, Outcome, PackageInstance, Passage, Plan, ProductHeader,
        SharedVariable, Signature, TombstoneSet,
    },
    source::{GraphSource, OutcomeSource, PackageSource},
    wire::{self, Pack},
};

pub struct Compilation {
    pub program: Program,
    /// The single-file pack, including the name table.
    pub pack: Vec<u8>,
    pub names: NameTable,
    pub lock: CompositionLock,
    pub analysis: Analysis,
    pub diagnostics: Vec<Diagnostic>,
}

impl std::fmt::Debug for Compilation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Compilation")
            .field("artifact_id", &self.program.artifact_id())
            .finish_non_exhaustive()
    }
}

/// Canonical JSON: object keys sorted, no insignificant whitespace.
pub fn canonical_json(value: &impl serde::Serialize) -> Result<Vec<u8>> {
    fn sort(value: serde_json::Value) -> serde_json::Value {
        match value {
            serde_json::Value::Object(map) => {
                let ordered: BTreeMap<_, _> = map
                    .into_iter()
                    .map(|(key, value)| (key, sort(value)))
                    .collect();
                serde_json::Value::Object(ordered.into_iter().collect())
            }
            serde_json::Value::Array(items) => {
                serde_json::Value::Array(items.into_iter().map(sort).collect())
            }
            value => value,
        }
    }
    let value =
        serde_json::to_value(value).map_err(|e| Error::new("encoding", "$", e.to_string()))?;
    serde_json::to_vec(&sort(value)).map_err(|e| Error::new("encoding", "$", e.to_string()))
}

/// The lock's source digest of a package: aliases and text references included.
pub fn package_digest(package: &PackageSource) -> Result<String> {
    let bytes = canonical_json(package)?;
    if bytes.len() > MAX_SOURCE_BYTES {
        return Err(Error::new(
            "limit",
            "packages",
            "package source exceeds 16 MiB",
        ));
    }
    Ok(hex::encode(digest_bytes(
        "narrata.nodes.package-source",
        1,
        &bytes,
    )))
}

fn bounded_data(data: &serde_json::Value, path: &str) -> Result<()> {
    let mut pending = vec![(0, data)];
    let mut count = 0;
    while let Some((depth, value)) = pending.pop() {
        count += 1;
        if depth > 64 || count > 1_000_000 {
            return Err(Error::new(
                "limit",
                path,
                "node configuration is too deep or too large",
            ));
        }
        match value {
            serde_json::Value::Array(items) => pending.extend(items.iter().map(|v| (depth + 1, v))),
            serde_json::Value::Object(fields) => {
                pending.extend(fields.values().map(|v| (depth + 1, v)))
            }
            _ => {}
        }
    }
    Ok(())
}

struct Lowered {
    graphs: BTreeMap<GraphRef, (GraphHeader, BTreeMap<NodeId, Plan>)>,
    names: BTreeMap<GraphRef, GraphNames>,
    tombstones: TombstoneSet,
    node_types: BTreeMap<String, String>,
}

pub fn compile(source: &ProjectSource, registry: &NodeRegistry) -> Result<Compilation> {
    let manifest_source = &source.manifest;
    if manifest_source.format_version != FORMAT_VERSION {
        return Err(Error::new(
            "version",
            "format_version",
            "unsupported project format",
        ));
    }
    if source.packages.is_empty() || source.packages.keys().ne(manifest_source.packages.keys()) {
        return Err(Error::new(
            "reference",
            "packages",
            "the project lists 1 or more package instances, each with a source",
        ));
    }
    let product_source = &manifest_source.product;
    name(&product_source.id, "product.id")?;
    for label in product_source.shared_labels.keys() {
        if !product_source.shared.contains_key(label) {
            return Err(Error::new(
                "reference",
                "product.shared_labels",
                format!("unknown shared variable {label}"),
            ));
        }
    }
    for (key, value) in &product_source.shared {
        name(key, "product.shared")?;
        check_scalar(value, "product.shared")?;
    }
    let lowered = lower(source, registry)?;
    let mut bindings = BTreeMap::new();
    for binding in &product_source.bindings {
        if bindings
            .insert(binding.from.clone(), binding.to.clone())
            .is_some()
        {
            return Err(Error::new(
                "duplicate",
                "product.bindings",
                "import port has multiple providers",
            ));
        }
    }
    let mut packages = BTreeMap::new();
    let mut graphs = BTreeMap::new();
    let mut chunks = Vec::new();
    let mut chunk_envelopes = Vec::new();
    for (index, (key, (header, nodes))) in lowered.graphs.iter().enumerate() {
        let package = source
            .packages
            .get(&key.package)
            .ok_or_else(|| Error::new("reference", key.label(), "missing package source"))?;
        packages.insert(
            key.package.clone(),
            PackageInstance {
                id: package.id.clone(),
                version: package.version.clone(),
            },
        );
        graphs.insert(
            key.clone(),
            GraphEntry {
                signature: header.signature(),
                exported: package.exports.contains(&key.graph),
                chunk: index as u32,
            },
        );
        let chunk = Chunk {
            graphs: BTreeMap::from([(
                key.clone(),
                Graph {
                    header: header.clone(),
                    nodes: nodes.clone(),
                },
            )]),
        };
        let (id, envelope) = wire::seal(wire::KIND_CHUNK, &wire::encode_chunk(&chunk));
        if envelope.len() > crate::MAX_CHUNK_BYTES + 56 {
            return Err(Error::new("limit", key.label(), "chunk exceeds 4 MiB"));
        }
        chunks.push((chunk, id));
        chunk_envelopes.push(envelope);
    }
    let (tombstones_id, tombstones_envelope) = wire::seal(
        wire::KIND_TOMBSTONES,
        &wire::encode_tombstones(&lowered.tombstones),
    );
    let manifest = Manifest {
        product: ProductHeader {
            id: product_source.id.clone(),
            title: product_source.title.clone(),
            entry: product_source.entry.clone(),
            arguments: product_source.arguments.clone(),
            shared: product_source
                .shared
                .iter()
                .map(|(key, value)| {
                    (
                        key.clone(),
                        SharedVariable {
                            value: value.clone(),
                            label: product_source.shared_labels.get(key).cloned(),
                        },
                    )
                })
                .collect(),
            endings: product_source
                .endings
                .iter()
                .map(|(key, ending)| {
                    (
                        key.clone(),
                        Ending {
                            title: ending.title.clone(),
                            body: ending.body.clone(),
                        },
                    )
                })
                .collect(),
            bindings,
        },
        packages,
        graphs,
        chunks: chunks.iter().map(|(_, id)| *id).collect(),
        node_types: lowered.node_types.clone(),
        tombstones: tombstones_id,
    };
    check_manifest(&manifest)?;
    let provisional = NameTable {
        artifact: crate::ArtifactId::from_bytes([0; 32]),
        migrated_from_r1: manifest_source.migrated_from_r1,
        graphs: lowered.names.clone(),
    };
    for (index, (chunk, _)) in chunks.iter().enumerate() {
        check_chunk(
            &manifest,
            index as u32,
            chunk,
            &lowered.tombstones,
            Some(&provisional),
        )?;
    }
    let (manifest_id, manifest_envelope) =
        wire::seal(wire::KIND_MANIFEST, &wire::encode_manifest(&manifest));
    if manifest_envelope.len() > crate::MAX_MANIFEST_BYTES + 56 {
        return Err(Error::new("limit", "manifest", "manifest exceeds 4 MiB"));
    }
    let artifact_id = crate::ArtifactId::from_bytes(*manifest_id.as_bytes());
    let names = NameTable {
        artifact: artifact_id,
        ..provisional
    };
    let pack = Pack {
        manifest: manifest_envelope,
        chunks: chunk_envelopes,
        tombstones: tombstones_envelope,
        names: Some(wire::seal(wire::KIND_NAMES, &wire::encode_names(&names)).1),
    }
    .encode();
    // Reopening the pack runs every checked decoder over what was just encoded.
    let (program, reopened) = Program::from_pack(&pack)?;
    if reopened.as_ref() != Some(&names) || program.artifact_id() != artifact_id {
        return Err(Error::new(
            "state",
            "pack",
            "pack does not reopen to its sources",
        ));
    }
    program.verify_artifact()?;
    let analysis = analyze(&program, Some(&names))?;
    let lock = CompositionLock {
        format_version: FORMAT_VERSION,
        artifact_id,
        packages: source
            .packages
            .iter()
            .map(|(alias, package)| {
                Ok((
                    alias.clone(),
                    PackageLock {
                        id: package.id.clone(),
                        version: package.version.clone(),
                        digest: package_digest(package)?,
                    },
                ))
            })
            .collect::<Result<_>>()?,
        node_types: lowered.node_types,
    };
    let diagnostics = analysis.diagnostics.clone();
    Ok(Compilation {
        program,
        pack,
        names,
        lock,
        analysis,
        diagnostics,
    })
}

fn lower(source: &ProjectSource, registry: &NodeRegistry) -> Result<Lowered> {
    let mut lowered = Lowered {
        graphs: BTreeMap::new(),
        names: BTreeMap::new(),
        tombstones: TombstoneSet::default(),
        node_types: BTreeMap::new(),
    };
    let mut seen = BTreeMap::<AuthoredId, String>::new();
    let mut claim = |id: AuthoredId, path: &str| -> Result<()> {
        if let Some(first) = seen.insert(id, path.to_owned()) {
            return Err(Error::new(
                "duplicate",
                path,
                format!("{id} is already used at {first}"),
            ));
        }
        Ok(())
    };
    for (alias, package) in &source.packages {
        name(alias, "packages")?;
        let path = format!("packages.{alias}");
        if package.id.is_empty()
            || package.id.len() > 128
            || package.version.is_empty()
            || package.version.len() > 64
        {
            return Err(Error::new(
                "identifier",
                &path,
                "package ID and version must be non-empty and bounded",
            ));
        }
        let mut exports = package.exports.clone();
        exports.sort();
        sorted_names(&exports, &format!("{path}.exports"))?;
        for export in &exports {
            if !package.graphs.contains_key(export) {
                return Err(Error::new(
                    "reference",
                    &path,
                    format!("export {export} does not exist"),
                ));
            }
        }
        let mut own = BTreeSet::new();
        for tombstone in &package.tombstones {
            if !own.insert(*tombstone) {
                return Err(Error::new(
                    "duplicate",
                    format!("{path}.tombstones"),
                    format!("duplicate tombstone {tombstone}"),
                ));
            }
            lowered.tombstones.insert(*tombstone);
        }
        for (graph_name, graph) in &package.graphs {
            name(graph_name, &format!("{path}.graphs"))?;
            let key = GraphRef {
                package: alias.clone(),
                graph: graph_name.clone(),
            };
            let graph_path = format!("{path}.graphs.{graph_name}");
            let (header, nodes, names) = lower_graph(
                graph,
                registry,
                &graph_path,
                &mut claim,
                &mut lowered.node_types,
            )?;
            lowered.graphs.insert(key.clone(), (header, nodes));
            lowered.names.insert(key, names);
        }
    }
    for (id, path) in &seen {
        if lowered.tombstones.contains(id) {
            return Err(Error::new(
                "tombstone",
                path,
                format!("{id} is listed as deleted"),
            ));
        }
    }
    Ok(lowered)
}

type Claim<'a> = dyn FnMut(AuthoredId, &str) -> Result<()> + 'a;

fn lower_graph(
    graph: &GraphSource,
    registry: &NodeRegistry,
    path: &str,
    claim: &mut Claim<'_>,
    node_types: &mut BTreeMap<String, String>,
) -> Result<(GraphHeader, BTreeMap<NodeId, Plan>, GraphNames)> {
    let mut names = GraphNames::default();
    let mut ids = BTreeMap::new();
    for (alias, node) in &graph.nodes {
        name(alias, &format!("{path}.nodes"))?;
        let node_path = format!("{path}.nodes.{alias}");
        let id = node.id.ok_or_else(|| {
            Error::new(
                "missing_id",
                &node_path,
                "node has no id; run `narrata-book ids` to mint one",
            )
        })?;
        claim(AuthoredId::Node(id), &node_path)?;
        ids.insert(alias.clone(), id);
        names.nodes.insert(id, alias.clone());
    }
    let resolve = |alias: &str, at: &str| -> Result<NodeId> {
        ids.get(alias).copied().ok_or_else(|| {
            Error::new(
                "reference",
                at,
                format!("target node {alias} does not exist"),
            )
        })
    };
    let mut outcomes = graph.outcomes.clone();
    outcomes.sort();
    sorted_names(&outcomes, path)?;
    let mut imports = BTreeMap::new();
    for (port, import) in &graph.imports {
        let mut import_outcomes = import.outcomes.clone();
        import_outcomes.sort();
        sorted_names(&import_outcomes, path)?;
        imports.insert(
            port.clone(),
            Signature {
                parameters: import.parameters.clone(),
                outcomes: import_outcomes,
            },
        );
    }
    for key in graph.parameters.keys().chain(graph.shared.keys()) {
        name(key, path)?;
    }
    let header = GraphHeader {
        title: graph.title.clone(),
        parameters: graph.parameters.clone(),
        locals: graph.locals.clone(),
        shared: graph.shared.clone(),
        imports,
        outcomes,
        entry: resolve(&graph.entry, path)?,
    };
    let mut nodes = BTreeMap::new();
    for (alias, node) in &graph.nodes {
        let node_path = format!("{path}.nodes.{alias}");
        bounded_data(&node.data, &node_path)?;
        let (plan, revision) = registry.lower(&node.type_id, &node.data, &node_path)?;
        node_types.insert(node.type_id.clone(), revision);
        let plan = match plan {
            SourcePlan::Passage(passage) => {
                let mut point_keys = BTreeSet::new();
                let mut choice_points = Vec::new();
                for (index, point) in passage.choice_points.iter().enumerate() {
                    let point_path = format!("{node_path}.choice_points[{index}]");
                    name(&point.key, &point_path)?;
                    if !point_keys.insert(point.key.clone()) {
                        return Err(Error::new(
                            "duplicate",
                            &point_path,
                            format!("duplicate choice point key {}", point.key),
                        ));
                    }
                    let id: ChoicePointId = point.id.ok_or_else(|| {
                        Error::new("missing_id", &point_path, "choice point has no id")
                    })?;
                    claim(AuthoredId::ChoicePoint(id), &point_path)?;
                    names.choice_points.insert(id, point.key.clone());
                    let mut option_keys = BTreeSet::new();
                    let mut options = Vec::new();
                    for (option_index, option) in point.options.iter().enumerate() {
                        let option_path = format!("{point_path}.options[{option_index}]");
                        name(&option.key, &option_path)?;
                        if !option_keys.insert(option.key.clone()) {
                            return Err(Error::new(
                                "duplicate",
                                &option_path,
                                format!("duplicate option key {}", option.key),
                            ));
                        }
                        let id: OptionId = option.id.ok_or_else(|| {
                            Error::new("missing_id", &option_path, "option has no id")
                        })?;
                        claim(AuthoredId::Option(id), &option_path)?;
                        names.options.insert(id, option.key.clone());
                        options.push(OptionPlan {
                            id,
                            label: option.label.clone(),
                            visible_if: option.visible_if.clone(),
                            enabled_if: option.enabled_if.clone(),
                            reason: option.reason.clone(),
                            effects: option.effects.clone(),
                            outcome: match &option.outcome {
                                OutcomeSource::Local { reply, rejoin } => Outcome::Local {
                                    reply: reply.clone(),
                                    rejoin: rejoin.clone(),
                                },
                                OutcomeSource::Branch { target } => Outcome::Branch {
                                    target: resolve(target, &option_path)?,
                                },
                            },
                        });
                    }
                    choice_points.push(ChoicePoint {
                        id,
                        placement: point.placement.clone(),
                        min: point.min,
                        max: point.max,
                        options,
                        proposals: point.proposals,
                    });
                }
                Plan::Passage(Passage {
                    title: passage.title,
                    body: passage.body,
                    args: passage.args,
                    choice_points,
                    next: passage
                        .next
                        .as_deref()
                        .map(|next| resolve(next, &node_path))
                        .transpose()?,
                })
            }
            SourcePlan::Branch {
                condition,
                when_true,
                when_false,
            } => Plan::Branch {
                condition,
                when_true: resolve(&when_true, &node_path)?,
                when_false: resolve(&when_false, &node_path)?,
            },
            SourcePlan::Mutate { assignments, next } => Plan::Mutate {
                assignments,
                next: resolve(&next, &node_path)?,
            },
            SourcePlan::Call {
                target,
                arguments,
                on_return,
            } => Plan::Call {
                target,
                arguments,
                on_return: on_return
                    .iter()
                    .map(|(outcome, alias)| Ok((outcome.clone(), resolve(alias, &node_path)?)))
                    .collect::<Result<_>>()?,
            },
            SourcePlan::Return { outcome } => Plan::Return { outcome },
        };
        let id = ids
            .get(alias)
            .copied()
            .ok_or_else(|| Error::new("state", &node_path, "missing node id"))?;
        nodes.insert(id, plan);
    }
    Ok((header, nodes, names))
}

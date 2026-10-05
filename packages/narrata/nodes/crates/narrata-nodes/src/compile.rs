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
    check::{check_chunk, check_manifest, collect_names, name},
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

pub fn check_node_data(data: &serde_json::Value, path: &str) -> Result<()> {
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
    check_source_header(source, &mut crate::check::Checks::first())?;
    let product_source = &manifest_source.product;
    let lowered = lower(source, registry)?;
    let bindings = collect_bindings(product_source, &mut crate::check::Checks::first())?;
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
        check_chunk_size(envelope.len(), &key.label())?;
        chunks.push((chunk, id));
        chunk_envelopes.push(envelope);
    }
    let (tombstones_id, tombstones_envelope) = wire::seal(
        wire::KIND_TOMBSTONES,
        &wire::encode_tombstones(&lowered.tombstones),
    );
    let manifest = Manifest {
        product: product_header(product_source, bindings),
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
    check_manifest_size(manifest_envelope.len())?;
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
    lower_collect(source, registry, &mut crate::check::Checks::first())
}

fn claim_id(
    seen: &mut BTreeMap<AuthoredId, String>,
    id: AuthoredId,
    path: &str,
    checks: &mut crate::check::Checks,
) -> Result<()> {
    if let Some(first) = seen.insert(id, path.to_owned()) {
        checks.error(Error::new(
            "duplicate",
            path,
            format!("{id} is already used at {first}"),
        ))?;
    }
    Ok(())
}
fn lower_collect(
    source: &ProjectSource,
    registry: &NodeRegistry,
    checks: &mut crate::check::Checks,
) -> Result<Lowered> {
    let mut lowered = Lowered {
        graphs: BTreeMap::new(),
        names: BTreeMap::new(),
        tombstones: TombstoneSet::default(),
        node_types: BTreeMap::new(),
    };
    let mut seen = BTreeMap::<AuthoredId, String>::new();
    for (alias, package) in &source.packages {
        checks.check(name(alias, "packages"))?;
        let path = format!("packages.{alias}");
        checks.check(crate::check::check_package_identity(
            &package.id,
            &package.version,
            &path,
        ))?;
        let mut exports = package.exports.clone();
        exports.sort();
        collect_names(&exports, &format!("{path}.exports"), checks)?;
        for export in &exports {
            if !package.graphs.contains_key(export) {
                checks.error(Error::new(
                    "reference",
                    &path,
                    format!("export {export} does not exist"),
                ))?;
            }
        }
        let mut own = BTreeSet::new();
        for tombstone in &package.tombstones {
            if !own.insert(*tombstone) {
                checks.error(Error::new(
                    "duplicate",
                    format!("{path}.tombstones"),
                    format!("duplicate tombstone {tombstone}"),
                ))?;
            }
            lowered.tombstones.insert(*tombstone);
        }
        for (graph_name, graph) in &package.graphs {
            checks.check(name(graph_name, &format!("{path}.graphs")))?;
            let key = GraphRef {
                package: alias.clone(),
                graph: graph_name.clone(),
            };
            let graph_path = format!("{path}.graphs.{graph_name}");
            let (header, nodes, names) = lower_graph(
                graph,
                registry,
                &graph_path,
                &mut seen,
                &mut lowered.node_types,
                checks,
            )?;
            lowered.graphs.insert(key.clone(), (header, nodes));
            lowered.names.insert(key, names);
        }
    }
    for (id, path) in &seen {
        if lowered.tombstones.contains(id) {
            checks.error(Error::new(
                "tombstone",
                path,
                format!("{id} is listed as deleted"),
            ))?;
        }
    }
    Ok(lowered)
}

fn lower_graph(
    graph: &GraphSource,
    registry: &NodeRegistry,
    path: &str,
    seen: &mut BTreeMap<AuthoredId, String>,
    node_types: &mut BTreeMap<String, String>,
    checks: &mut crate::check::Checks,
) -> Result<(GraphHeader, BTreeMap<NodeId, Plan>, GraphNames)> {
    let mut names = GraphNames::default();
    let mut ids = BTreeMap::new();
    for (alias, node) in &graph.nodes {
        checks.check(name(alias, &format!("{path}.nodes")))?;
        let node_path = format!("{path}.nodes.{alias}");
        let Some(id) = checks.check(node.id.ok_or_else(|| {
            Error::new(
                "missing_id",
                &node_path,
                "node has no id; run `narrata-book ids` to mint one",
            )
        }))?
        else {
            checks.skip(&node_path, "node identity-dependent checks require an ID")?;
            continue;
        };
        claim_id(seen, AuthoredId::Node(id), &node_path, checks)?;
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
    collect_names(&outcomes, path, checks)?;
    let mut imports = BTreeMap::new();
    for (port, import) in &graph.imports {
        let mut import_outcomes = import.outcomes.clone();
        import_outcomes.sort();
        collect_names(&import_outcomes, path, checks)?;
        imports.insert(
            port.clone(),
            Signature {
                parameters: import.parameters.clone(),
                outcomes: import_outcomes,
            },
        );
    }
    for key in graph.parameters.keys().chain(graph.shared.keys()) {
        checks.check(name(key, path))?;
    }
    let header = GraphHeader {
        title: graph.title.clone(),
        parameters: graph.parameters.clone(),
        locals: graph.locals.clone(),
        shared: graph.shared.clone(),
        imports,
        outcomes,
        entry: checks
            .check(resolve(&graph.entry, &checks.path(path, "entry")))?
            .unwrap_or_else(|| NodeId::from_bytes([0; 16])),
    };
    let mut nodes = BTreeMap::new();
    for (alias, node) in &graph.nodes {
        let node_path = format!("{path}.nodes.{alias}");
        if checks
            .check(check_node_data(&node.data, &node_path))?
            .is_none()
        {
            checks.skip(&node_path, "lowering requires bounded data")?;
            continue;
        }
        let Some((plan, revision)) =
            checks.check(registry.lower(&node.type_id, &node.data, &node_path))?
        else {
            checks.skip(&node_path, "node semantics require a valid payload")?;
            continue;
        };
        node_types.insert(node.type_id.clone(), revision);
        let plan = match plan {
            SourcePlan::Passage(passage) => {
                let mut point_keys = BTreeSet::new();
                let mut choice_points = Vec::new();
                for (index, point) in passage.choice_points.iter().enumerate() {
                    let point_path = format!("{node_path}.choice_points[{index}]");
                    checks.check(name(&point.key, &point_path))?;
                    if !point_keys.insert(point.key.clone()) {
                        checks.error(Error::new(
                            "duplicate",
                            &point_path,
                            format!("duplicate choice point key {}", point.key),
                        ))?;
                    }
                    let id: ChoicePointId = checks
                        .check(point.id.ok_or_else(|| {
                            Error::new("missing_id", &point_path, "choice point has no id")
                        }))?
                        .unwrap_or_else(|| ChoicePointId::from_bytes([0; 16]));
                    claim_id(seen, AuthoredId::ChoicePoint(id), &point_path, checks)?;
                    names.choice_points.insert(id, point.key.clone());
                    let mut option_keys = BTreeSet::new();
                    let mut options = Vec::new();
                    for (option_index, option) in point.options.iter().enumerate() {
                        let option_path = format!("{point_path}.options[{option_index}]");
                        checks.check(name(&option.key, &option_path))?;
                        if !option_keys.insert(option.key.clone()) {
                            checks.error(Error::new(
                                "duplicate",
                                &option_path,
                                format!("duplicate option key {}", option.key),
                            ))?;
                        }
                        let id: OptionId = checks
                            .check(option.id.ok_or_else(|| {
                                Error::new("missing_id", &option_path, "option has no id")
                            }))?
                            .unwrap_or_else(|| OptionId::from_bytes([0; 16]));
                        claim_id(seen, AuthoredId::Option(id), &option_path, checks)?;
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
                                    target: checks
                                        .check(resolve(
                                            target,
                                            &checks.path(&option_path, "outcome.target"),
                                        ))?
                                        .unwrap_or_else(|| NodeId::from_bytes([0; 16])),
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
                        .map(|next| {
                            checks
                                .check(resolve(next, &checks.path(&node_path, "next")))
                                .map(|id| id.unwrap_or_else(|| NodeId::from_bytes([0; 16])))
                        })
                        .transpose()?,
                })
            }
            SourcePlan::Branch {
                condition,
                when_true,
                when_false,
            } => Plan::Branch {
                condition,
                when_true: checks
                    .check(resolve(&when_true, &checks.path(&node_path, "when_true")))?
                    .unwrap_or_else(|| NodeId::from_bytes([0; 16])),
                when_false: checks
                    .check(resolve(&when_false, &checks.path(&node_path, "when_false")))?
                    .unwrap_or_else(|| NodeId::from_bytes([0; 16])),
            },
            SourcePlan::Mutate { assignments, next } => Plan::Mutate {
                assignments,
                next: checks
                    .check(resolve(&next, &checks.path(&node_path, "next")))?
                    .unwrap_or_else(|| NodeId::from_bytes([0; 16])),
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
                    .map(|(outcome, alias)| {
                        Ok((
                            outcome.clone(),
                            checks
                                .check(resolve(
                                    alias,
                                    &checks.path(&node_path, &format!("on_return.{outcome}")),
                                ))?
                                .unwrap_or_else(|| NodeId::from_bytes([0; 16])),
                        ))
                    })
                    .collect::<Result<_>>()?,
            },
            SourcePlan::Return { outcome } => Plan::Return { outcome },
        };
        let Some(id) = ids.get(alias).copied() else {
            continue;
        };
        nodes.insert(id, plan);
    }
    Ok((header, nodes, names))
}

fn check_source_header(source: &ProjectSource, checks: &mut crate::check::Checks) -> Result<()> {
    let manifest_source = &source.manifest;
    check_source_version(manifest_source.format_version, checks)?;
    if source.packages.is_empty() || source.packages.keys().ne(manifest_source.packages.keys()) {
        checks.error(Error::new(
            "reference",
            "packages",
            "the project lists 1 or more package instances, each with a source",
        ))?;
    }
    check_product_source(&manifest_source.product, checks)?;
    Ok(())
}

#[derive(Clone, Debug)]
pub struct SourceValidation {
    pub complete: bool,
    pub errors: Vec<Error>,
    pub skipped: Vec<Error>,
}

/// Bounded, collecting validation over the same lowering and semantic rules as `compile`.
/// Partial plans are internal only: no artifact can escape this diagnostic path.
pub fn validate_source(
    source: &ProjectSource,
    registry: &NodeRegistry,
    max_checks: usize,
    max_diagnostics: usize,
) -> SourceValidation {
    fn collect(
        source: &ProjectSource,
        registry: &NodeRegistry,
        checks: &mut crate::check::Checks,
    ) -> Result<()> {
        check_source_header(source, checks)?;
        let lowered = lower_collect(source, registry, checks)?;
        let product = &source.manifest.product;
        let bindings = collect_bindings(product, checks)?;
        let manifest = Manifest {
            product: product_header(product, bindings),
            packages: source
                .packages
                .iter()
                .map(|(alias, package)| {
                    (
                        alias.clone(),
                        PackageInstance {
                            id: package.id.clone(),
                            version: package.version.clone(),
                        },
                    )
                })
                .collect(),
            graphs: lowered
                .graphs
                .iter()
                .enumerate()
                .map(|(index, (key, (header, _)))| {
                    (
                        key.clone(),
                        GraphEntry {
                            signature: header.signature(),
                            exported: source
                                .packages
                                .get(&key.package)
                                .is_some_and(|package| package.exports.contains(&key.graph)),
                            chunk: index as u32,
                        },
                    )
                })
                .collect(),
            chunks: vec![crate::ObjectId::from_bytes([0; 32]); lowered.graphs.len()],
            node_types: lowered.node_types,
            tombstones: crate::ObjectId::from_bytes([0; 32]),
        };
        crate::check::collect_manifest(&manifest, checks)?;
        checks.check(check_manifest_size(
            wire::encode_manifest(&manifest).len().saturating_add(56),
        ))?;
        let names = NameTable {
            artifact: crate::ArtifactId::from_bytes([0; 32]),
            migrated_from_r1: source.manifest.migrated_from_r1,
            graphs: lowered.names,
        };
        let mut points = BTreeSet::new();
        let mut options = BTreeSet::new();
        for (key, (header, nodes)) in lowered.graphs {
            checks.tick(&key.label())?;
            let source_graph = &source.packages[&key.package].graphs[&key.graph];
            // All source identities remain valid reference targets even if their own payload
            // could not lower. The zero placeholder denotes a reference already diagnosed
            // during partial lowering; it is never serialized or compiled.
            let available = source_graph
                .nodes
                .values()
                .filter_map(|node| node.id)
                .chain([NodeId::from_bytes([0; 16])])
                .collect();
            checks.available = Some(available);
            checks.graph_size = Some(source_graph.nodes.len());
            let graph = Graph { header, nodes };
            crate::check::collect_graph(
                &manifest,
                &key,
                &graph,
                &lowered.tombstones,
                Some(&names),
                &mut points,
                &mut options,
                checks,
            )?;
            let chunk = Chunk {
                graphs: BTreeMap::from([(key.clone(), graph)]),
            };
            checks.check(check_chunk_size(
                wire::encode_chunk(&chunk).len().saturating_add(56),
                &key.label(),
            ))?;
        }
        for (alias, package) in &source.packages {
            checks.tick("packages")?;
            checks.check(package_digest(package).map_err(|error| {
                Error::new(&error.code, format!("packages.{alias}"), error.message)
            }))?;
        }
        Ok(())
    }
    let mut checks = crate::check::Checks::collecting(max_checks, max_diagnostics);
    if let Err(error) = collect(source, registry, &mut checks) {
        checks.errors.push(error);
    }
    checks
        .errors
        .sort_by(|a, b| (&a.path, &a.code, &a.message).cmp(&(&b.path, &b.code, &b.message)));
    checks.errors.dedup();
    SourceValidation {
        complete: checks.complete,
        errors: checks.errors,
        skipped: checks.skipped,
    }
}

fn collect_bindings(
    product_source: &crate::source::ProductSource,
    checks: &mut crate::check::Checks,
) -> Result<BTreeMap<crate::plan::ImportRef, GraphRef>> {
    let mut bindings = BTreeMap::new();
    for binding in &product_source.bindings {
        if bindings
            .insert(binding.from.clone(), binding.to.clone())
            .is_some()
        {
            checks.error(Error::new(
                "duplicate",
                "product.bindings",
                "import port has multiple providers",
            ))?;
        }
    }
    Ok(bindings)
}

fn product_header(
    product_source: &crate::source::ProductSource,
    bindings: BTreeMap<crate::plan::ImportRef, GraphRef>,
) -> ProductHeader {
    ProductHeader {
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
    }
}

fn check_source_version(version: u16, checks: &mut crate::check::Checks) -> Result<()> {
    if version != FORMAT_VERSION {
        checks.error(Error::new(
            "version",
            "format_version",
            "unsupported project format",
        ))?;
    }
    Ok(())
}

fn check_chunk_size(bytes: usize, path: &str) -> Result<()> {
    if bytes > crate::MAX_CHUNK_BYTES + 56 {
        Err(Error::new("limit", path, "chunk exceeds 4 MiB"))
    } else {
        Ok(())
    }
}

fn check_manifest_size(bytes: usize) -> Result<()> {
    if bytes > crate::MAX_MANIFEST_BYTES + 56 {
        Err(Error::new("limit", "manifest", "manifest exceeds 4 MiB"))
    } else {
        Ok(())
    }
}

fn check_product_source(
    product_source: &crate::source::ProductSource,
    checks: &mut crate::check::Checks,
) -> Result<()> {
    checks.check(name(&product_source.id, "product.id"))?;
    for label in product_source.shared_labels.keys() {
        if !product_source.shared.contains_key(label) {
            checks.error(Error::new(
                "reference",
                "product.shared_labels",
                format!("unknown shared variable {label}"),
            ))?;
        }
    }
    for (key, value) in &product_source.shared {
        checks.check(name(key, "product.shared"))?;
        checks.check(check_scalar(value, "product.shared"))?;
    }
    Ok(())
}

pub fn validate_manifest_source(manifest: &crate::ProjectManifest) -> Vec<Error> {
    let mut checks = crate::check::Checks::collecting(usize::MAX, usize::MAX);
    let result = (|| {
        check_source_version(manifest.format_version, &mut checks)?;
        check_product_source(&manifest.product, &mut checks)
    })();
    if let Err(error) = result {
        checks.errors.push(error);
    }
    checks.errors
}

use std::collections::{BTreeMap, BTreeSet};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    Assignment, Bundle, CallTarget, CompositionLock, Diagnostic, Error, FORMAT_VERSION, Graph,
    GraphRef, ImportRef, MAX_DOCUMENT_BYTES, MAX_NODES, NodePlan, NodeRegistry, PackageLock,
    Result, ScalarType, Scope, expr,
};

#[derive(Clone, Debug, Serialize)]
pub(crate) struct CompiledGraph {
    pub source: Graph,
    pub nodes: BTreeMap<String, NodePlan>,
}

/// Constructed only by [`compile`]. All graph references, expressions and call contracts are checked.
#[derive(Clone, Debug)]
pub struct CheckedProduct {
    pub(crate) source: Bundle,
    pub(crate) graphs: BTreeMap<GraphRef, CompiledGraph>,
    pub(crate) bindings: BTreeMap<ImportRef, GraphRef>,
    lock: CompositionLock,
    analysis: Vec<GraphAnalysis>,
}

#[derive(Clone, Debug)]
pub struct Compilation {
    pub product: CheckedProduct,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NodeAddress {
    pub package: String,
    pub graph: String,
    pub node: String,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EdgeAnalysis {
    pub kind: String,
    pub label: String,
    pub target: NodeAddress,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NodeAnalysis {
    pub id: String,
    pub type_id: String,
    pub label: String,
    pub edges: Vec<EdgeAnalysis>,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GraphAnalysis {
    pub reference: GraphRef,
    pub title: String,
    pub entry: String,
    pub nodes: Vec<NodeAnalysis>,
}

impl CheckedProduct {
    pub fn artifact_id(&self) -> &str {
        &self.lock.artifact_id
    }
    pub fn title(&self) -> &str {
        &self.source.product.title
    }
    pub fn source(&self) -> &Bundle {
        &self.source
    }
    pub fn lock(&self) -> &CompositionLock {
        &self.lock
    }
    pub fn analysis(&self) -> &[GraphAnalysis] {
        &self.analysis
    }

    pub(crate) fn graph(&self, key: &GraphRef) -> Result<&CompiledGraph> {
        self.graphs
            .get(key)
            .ok_or_else(|| Error::new("state", key.label(), "missing checked graph"))
    }

    pub(crate) fn target(&self, key: &GraphRef, target: &CallTarget) -> Result<GraphRef> {
        resolve_target(key, target, &self.bindings)
    }
}

pub(crate) fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 80
        && name
            .bytes()
            .all(|v| v.is_ascii_alphanumeric() || matches!(v, b'_' | b'-'))
}

fn name(value: &str, path: &str) -> Result<()> {
    if valid_name(value) {
        Ok(())
    } else {
        Err(Error::new(
            "identifier",
            path,
            "use 1..80 ASCII letters, digits, underscores or hyphens",
        ))
    }
}

fn unique(values: &[String], path: &str) -> Result<()> {
    let mut seen = BTreeSet::new();
    for value in values {
        name(value, path)?;
        if !seen.insert(value) {
            return Err(Error::new(
                "duplicate",
                path,
                format!("duplicate identifier {value}"),
            ));
        }
    }
    Ok(())
}

pub(crate) fn canonical_bytes(value: &impl Serialize) -> Result<Vec<u8>> {
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

pub(crate) fn digest(domain: &str, value: &impl Serialize) -> Result<String> {
    let mut hash = Sha256::new();
    hash.update(domain.as_bytes());
    hash.update([0]);
    hash.update(canonical_bytes(value)?);
    Ok(hex::encode(hash.finalize()))
}

pub fn compile(mut source: Bundle, registry: &NodeRegistry) -> Result<Compilation> {
    // Also bound callers constructing Bundle values directly, before recursive serialization.
    for package in source.packages.values() {
        for graph in package.graphs.values() {
            for node in graph.nodes.values() {
                let mut pending = vec![(0, &node.data)];
                let mut count = 0;
                while let Some((depth, value)) = pending.pop() {
                    count += 1;
                    if depth > 64 || count > 100_000 {
                        return Err(Error::new(
                            "limit",
                            "node.data",
                            "node configuration is too deep or too large",
                        ));
                    }
                    match value {
                        serde_json::Value::Array(items) => {
                            pending.extend(items.iter().map(|v| (depth + 1, v)))
                        }
                        serde_json::Value::Object(fields) => {
                            pending.extend(fields.values().map(|v| (depth + 1, v)))
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    if source.format_version != FORMAT_VERSION {
        return Err(Error::new(
            "version",
            "format_version",
            "unsupported node bundle format",
        ));
    }
    if canonical_bytes(&source)?.len() > MAX_DOCUMENT_BYTES {
        return Err(Error::new("limit", "$", "bundle exceeds 4 MiB"));
    }
    if source.packages.is_empty() || source.packages.len() > 64 {
        return Err(Error::new(
            "limit",
            "packages",
            "expected 1..64 package instances",
        ));
    }
    name(&source.product.id, "product.id")?;
    expr::check_text(&source.product.title, "product.title")?;
    if source.product.shared.len() > 256 {
        return Err(Error::new(
            "limit",
            "product.shared",
            "too many shared variables",
        ));
    }
    for (key, value) in &source.product.shared {
        name(key, "product.shared")?;
        expr::check_scalar(value, "product.shared")?;
    }
    for (key, label) in &source.product.shared_labels {
        if !source.product.shared.contains_key(key) {
            return Err(Error::new(
                "reference",
                "product.shared_labels",
                format!("unknown shared variable {key}"),
            ));
        }
        expr::check_text(label, "product.shared_labels")?;
    }
    let mut graphs = BTreeMap::new();
    let mut node_count = 0;
    let mut node_types = BTreeMap::new();
    for (alias, package) in &mut source.packages {
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
        unique(&package.exports, &format!("{path}.exports"))?;
        package.exports.sort();
        for export in &package.exports {
            if !package.graphs.contains_key(export) {
                return Err(Error::new(
                    "reference",
                    &path,
                    format!("export {export} does not exist"),
                ));
            }
        }
        for (id, content) in &package.content {
            name(id, &format!("{path}.content"))?;
            expr::check_text(&content.title, &path)?;
            if content.paragraphs.len() > 256 {
                return Err(Error::new("limit", &path, "too many content paragraphs"));
            }
            for text in &content.paragraphs {
                expr::check_text(text, &path)?;
            }
        }
        if package.graphs.len() > 256 {
            return Err(Error::new("limit", &path, "too many graphs"));
        }
        for (id, graph) in &mut package.graphs {
            name(id, &format!("{path}.graphs"))?;
            let graph_path = format!("{path}.graphs.{id}");
            expr::check_text(&graph.title, &graph_path)?;
            for key in graph
                .parameters
                .keys()
                .chain(graph.locals.keys())
                .chain(graph.shared.keys())
                .chain(graph.imports.keys())
            {
                name(key, &graph_path)?;
            }
            if graph.parameters.len() + graph.locals.len() + graph.shared.len() > 256 {
                return Err(Error::new("limit", &graph_path, "too many variables"));
            }
            for value in graph.locals.values() {
                expr::check_scalar(value, &graph_path)?;
            }
            for (key, kind) in &graph.shared {
                let actual = source.product.shared.get(key).ok_or_else(|| {
                    Error::new(
                        "reference",
                        &graph_path,
                        format!("product does not provide shared variable {key}"),
                    )
                })?;
                expr::expect(actual.kind(), *kind, &graph_path)?;
            }
            unique(&graph.outcomes, &graph_path)?;
            if graph.outcomes.is_empty() {
                return Err(Error::new(
                    "contract",
                    &graph_path,
                    "graph needs at least one declared outcome",
                ));
            }
            graph.outcomes.sort();
            for signature in graph.imports.values_mut() {
                unique(&signature.outcomes, &graph_path)?;
                signature.outcomes.sort();
                for key in signature.parameters.keys() {
                    name(key, &graph_path)?;
                }
            }
            if !graph.nodes.contains_key(&graph.entry) {
                return Err(Error::new(
                    "reference",
                    &graph_path,
                    "entry node does not exist",
                ));
            }
            let mut plans = BTreeMap::new();
            for (node_id, definition) in &graph.nodes {
                node_count += 1;
                if node_count > MAX_NODES {
                    return Err(Error::new("limit", "packages", "bundle exceeds 4096 nodes"));
                }
                name(node_id, &graph_path)?;
                let node_path = format!("{graph_path}.nodes.{node_id}");
                let (plan, revision) =
                    registry.lower(&definition.type_id, &definition.data, &node_path)?;
                node_types.insert(definition.type_id.clone(), revision);
                plans.insert(node_id.clone(), plan);
            }
            graphs.insert(
                GraphRef {
                    package: alias.clone(),
                    graph: id.clone(),
                },
                CompiledGraph {
                    source: graph.clone(),
                    nodes: plans,
                },
            );
        }
    }
    let mut bindings = BTreeMap::new();
    for binding in &source.product.bindings {
        let from = GraphRef {
            package: binding.from.package.clone(),
            graph: binding.from.graph.clone(),
        };
        let owner = graphs.get(&from).ok_or_else(|| {
            Error::new(
                "binding",
                "product.bindings",
                format!("unknown import owner {}", from.label()),
            )
        })?;
        let signature = owner
            .source
            .imports
            .get(&binding.from.port)
            .ok_or_else(|| {
                Error::new("binding", "product.bindings", "import port is not declared")
            })?;
        let target = graphs.get(&binding.to).ok_or_else(|| {
            Error::new("binding", "product.bindings", "target graph does not exist")
        })?;
        require_export(&source, &binding.to)?;
        if signature.parameters != target.source.parameters
            || signature.outcomes != target.source.outcomes
        {
            return Err(Error::new(
                "contract",
                "product.bindings",
                format!(
                    "import {:?} does not match {} parameters and outcomes",
                    binding.from,
                    binding.to.label()
                ),
            ));
        }
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
    let mut diagnostics = Vec::new();
    for (key, graph) in &graphs {
        for port in graph.source.imports.keys() {
            if !bindings.contains_key(&ImportRef {
                package: key.package.clone(),
                graph: key.graph.clone(),
                port: port.clone(),
            }) {
                return Err(Error::new(
                    "binding",
                    key.label(),
                    format!("missing provider for {port}"),
                ));
            }
        }
        for (node_id, plan) in &graph.nodes {
            let path = format!(
                "packages.{}.graphs.{}.nodes.{node_id}",
                key.package, key.graph
            );
            check_plan(&source, &graphs, &bindings, key, &graph.source, plan, &path)?;
        }
        let mut reachable = BTreeSet::new();
        let mut queue = vec![graph.source.entry.clone()];
        while let Some(id) = queue.pop() {
            if !reachable.insert(id.clone()) {
                continue;
            }
            if let Some(plan) = graph.nodes.get(&id) {
                queue.extend(local_edges(plan).into_iter().map(|(_, _, target)| target));
            }
        }
        for id in graph.nodes.keys().filter(|id| !reachable.contains(*id)) {
            diagnostics.push(Diagnostic {
                code: "structurally_unreachable".into(),
                path: format!("{}.nodes.{id}", key.label()),
                message: "No structural path from this graph's entry (conditions are not solved)."
                    .into(),
            });
        }
    }
    require_export(&source, &source.product.entry)?;
    let entry = graphs
        .get(&source.product.entry)
        .ok_or_else(|| Error::new("reference", "product.entry", "entry graph does not exist"))?;
    if source
        .product
        .arguments
        .keys()
        .ne(entry.source.parameters.keys())
    {
        return Err(Error::new(
            "contract",
            "product.arguments",
            "entry arguments do not match parameters",
        ));
    }
    for (key, value) in &source.product.arguments {
        let kind = entry
            .source
            .parameters
            .get(key)
            .ok_or_else(|| Error::new("contract", key, "missing entry parameter"))?;
        expr::check_scalar(value, key)?;
        expr::expect(value.kind(), *kind, key)?;
    }
    source.product.bindings.sort_by(|a, b| a.from.cmp(&b.from));
    let graph_fingerprints: Vec<_> = graphs.iter().collect();
    let artifact_id = digest(
        "NARRATA-NODES-PRODUCT-1",
        &(&source, &node_types, graph_fingerprints),
    )?;
    let packages = source
        .packages
        .iter()
        .map(|(alias, package)| {
            Ok((
                alias.clone(),
                PackageLock {
                    id: package.id.clone(),
                    version: package.version.clone(),
                    digest: digest("NARRATA-NODES-PACKAGE-1", package)?,
                },
            ))
        })
        .collect::<Result<_>>()?;
    let mut product = CheckedProduct {
        source,
        graphs,
        bindings,
        lock: CompositionLock {
            format_version: FORMAT_VERSION,
            artifact_id,
            packages,
            node_types,
        },
        analysis: Vec::new(),
    };
    product.analysis = analyze(&product)?;
    Ok(Compilation {
        product,
        diagnostics,
    })
}

fn require_export(source: &Bundle, target: &GraphRef) -> Result<()> {
    let package = source.packages.get(&target.package).ok_or_else(|| {
        Error::new(
            "reference",
            target.label(),
            "package instance does not exist",
        )
    })?;
    if !package.exports.contains(&target.graph) {
        return Err(Error::new(
            "contract",
            target.label(),
            "graph is not exported",
        ));
    }
    Ok(())
}

fn resolve_target(
    key: &GraphRef,
    target: &CallTarget,
    bindings: &BTreeMap<ImportRef, GraphRef>,
) -> Result<GraphRef> {
    match target {
        CallTarget::Local { graph } => Ok(GraphRef {
            package: key.package.clone(),
            graph: graph.clone(),
        }),
        CallTarget::Import { port } => bindings
            .get(&ImportRef {
                package: key.package.clone(),
                graph: key.graph.clone(),
                port: port.clone(),
            })
            .cloned()
            .ok_or_else(|| Error::new("binding", key.label(), format!("unbound import {port}"))),
    }
}

fn assignments(values: &[Assignment], graph: &Graph, path: &str) -> Result<()> {
    for assignment in values {
        if assignment.target.scope == Scope::Parameter {
            return Err(Error::new(
                "readonly",
                path,
                "parameters are immutable bindings",
            ));
        }
        let expected = expr::variable_type(
            graph,
            assignment.target.scope,
            &assignment.target.name,
            path,
        )?;
        expr::expect(
            expr::check(&assignment.value, graph, 0, path)?,
            expected,
            path,
        )?;
    }
    Ok(())
}

fn check_plan(
    source: &Bundle,
    graphs: &BTreeMap<GraphRef, CompiledGraph>,
    bindings: &BTreeMap<ImportRef, GraphRef>,
    key: &GraphRef,
    graph: &Graph,
    plan: &NodePlan,
    path: &str,
) -> Result<()> {
    for (_, _, target) in local_edges(plan) {
        if !graph.nodes.contains_key(&target) {
            return Err(Error::new(
                "reference",
                path,
                format!("target node {target} does not exist"),
            ));
        }
    }
    if let NodePlan::Content { content, .. } | NodePlan::Decision { content, .. } = plan {
        let text = source
            .packages
            .get(&key.package)
            .and_then(|p| p.content.get(content))
            .ok_or_else(|| {
                Error::new("content", path, format!("unknown local content {content}"))
            })?;
        expr::check_template(&text.title, graph, path)?;
        for paragraph in &text.paragraphs {
            expr::check_template(paragraph, graph, path)?;
        }
    }
    match plan {
        NodePlan::Content { label, .. } => expr::check_template(label, graph, path)?,
        NodePlan::Decision { choices, .. } => {
            if choices.is_empty() || choices.len() > 128 {
                return Err(Error::new("limit", path, "expected 1..128 choices"));
            }
            unique(
                &choices.iter().map(|c| c.id.clone()).collect::<Vec<_>>(),
                path,
            )?;
            for choice in choices {
                expr::check_template(&choice.label, graph, path)?;
                if let Some(reason) = &choice.disabled_reason {
                    expr::check_template(reason, graph, path)?;
                }
                for condition in [choice.visible_if.as_ref(), choice.enabled_if.as_ref()]
                    .into_iter()
                    .flatten()
                {
                    expr::expect(
                        expr::check(condition, graph, 0, path)?,
                        ScalarType::Bool,
                        path,
                    )?;
                }
                assignments(&choice.assignments, graph, path)?;
            }
        }
        NodePlan::Branch { condition, .. } => expr::expect(
            expr::check(condition, graph, 0, path)?,
            ScalarType::Bool,
            path,
        )?,
        NodePlan::Mutate {
            assignments: values,
            ..
        } => assignments(values, graph, path)?,
        NodePlan::Call {
            target,
            arguments,
            on_return,
        } => {
            let target = resolve_target(key, target, bindings)?;
            let callee = graphs.get(&target).ok_or_else(|| {
                Error::new(
                    "reference",
                    path,
                    format!("unknown graph {}", target.label()),
                )
            })?;
            if arguments.keys().ne(callee.source.parameters.keys())
                || on_return.keys().ne(callee.source.outcomes.iter())
            {
                return Err(Error::new(
                    "contract",
                    path,
                    "call must bind every parameter and every outcome exactly once",
                ));
            }
            for (name, value) in arguments {
                let expected = callee
                    .source
                    .parameters
                    .get(name)
                    .ok_or_else(|| Error::new("contract", path, "undeclared argument"))?;
                expr::expect(expr::check(value, graph, 0, path)?, *expected, path)?;
            }
        }
        NodePlan::Return { outcome } => {
            if !graph.outcomes.contains(outcome) {
                return Err(Error::new(
                    "contract",
                    path,
                    format!("undeclared outcome {outcome}"),
                ));
            }
        }
    }
    Ok(())
}

fn local_edges(plan: &NodePlan) -> Vec<(String, String, String)> {
    match plan {
        NodePlan::Content { next, .. } | NodePlan::Mutate { next, .. } => {
            vec![("next".into(), String::new(), next.clone())]
        }
        NodePlan::Decision { choices, .. } => choices
            .iter()
            .map(|c| ("choice".into(), c.label.clone(), c.target.clone()))
            .collect(),
        NodePlan::Branch {
            when_true,
            when_false,
            ..
        } => vec![
            ("condition".into(), "true".into(), when_true.clone()),
            ("condition".into(), "false".into(), when_false.clone()),
        ],
        NodePlan::Call { on_return, .. } => on_return
            .iter()
            .map(|(outcome, target)| ("return".into(), outcome.clone(), target.clone()))
            .collect(),
        NodePlan::Return { .. } => Vec::new(),
    }
}

fn analyze(product: &CheckedProduct) -> Result<Vec<GraphAnalysis>> {
    product
        .graphs
        .iter()
        .map(|(key, graph)| {
            let nodes = graph
                .nodes
                .iter()
                .map(|(id, plan)| {
                    let type_id = graph
                        .source
                        .nodes
                        .get(id)
                        .map(|n| n.type_id.clone())
                        .ok_or_else(|| Error::new("state", id, "missing node definition"))?;
                    let mut edges = local_edges(plan)
                        .into_iter()
                        .map(|(kind, label, node)| EdgeAnalysis {
                            kind,
                            label,
                            target: NodeAddress {
                                package: key.package.clone(),
                                graph: key.graph.clone(),
                                node,
                            },
                        })
                        .collect::<Vec<_>>();
                    let label = match plan {
                        NodePlan::Content { content, .. } | NodePlan::Decision { content, .. } => {
                            product
                                .source
                                .packages
                                .get(&key.package)
                                .and_then(|p| p.content.get(content))
                                .map(|c| c.title.clone())
                                .unwrap_or_else(|| id.clone())
                        }
                        NodePlan::Call { target, .. } => {
                            let target = product.target(key, target)?;
                            let callee = product.graph(&target)?;
                            edges.push(EdgeAnalysis {
                                kind: "call".into(),
                                label: "call".into(),
                                target: NodeAddress {
                                    package: target.package,
                                    graph: target.graph,
                                    node: callee.source.entry.clone(),
                                },
                            });
                            format!("调用 {}", callee.source.title)
                        }
                        NodePlan::Return { outcome } => format!("返回 {outcome}"),
                        NodePlan::Branch { .. } => "条件判断".into(),
                        NodePlan::Mutate { .. } => "更新状态".into(),
                    };
                    Ok(NodeAnalysis {
                        id: id.clone(),
                        type_id,
                        label,
                        edges,
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            Ok(GraphAnalysis {
                reference: key.clone(),
                title: graph.source.title.clone(),
                entry: graph.source.entry.clone(),
                nodes,
            })
        })
        .collect()
}

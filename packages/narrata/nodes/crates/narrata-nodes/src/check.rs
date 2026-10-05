//! Structural rules shared by `compile` and the checked decoding of a manifest or chunk, so
//! that a decoded chunk proves the same invariants the compiler established (ADR 0013 §8).

use std::collections::BTreeSet;

use crate::{
    Error, MAX_ARGS, MAX_ASSIGNMENTS, MAX_CHOICE_POINTS, MAX_GRAPH_NODES, MAX_OPTIONS, MAX_SHARED,
    MAX_VARIABLES, Result, ScalarType,
    expr::{Declarations, check_scalar, expect},
    plan::{
        CallTarget, ChoicePoint, Chunk, Graph, GraphRef, ImportRef, Manifest, NameTable, Outcome,
        Passage, Plan, Signature, TombstoneSet, node_path,
    },
    registry::valid_type_id,
    valid_name,
};

/// One rule implementation serves fail-fast decoding and bounded editor validation.
pub(crate) struct Checks {
    pub errors: Vec<Error>,
    pub skipped: Vec<Error>,
    pub complete: bool,
    first: bool,
    work: usize,
    diagnostics: usize,
    pub available: Option<BTreeSet<crate::NodeId>>,
    pub graph_size: Option<usize>,
}

impl Checks {
    pub fn is_first(&self) -> bool {
        self.first
    }
    pub fn first() -> Self {
        Self {
            errors: Vec::new(),
            skipped: Vec::new(),
            complete: true,
            first: true,
            work: usize::MAX,
            diagnostics: usize::MAX,
            available: None,
            graph_size: None,
        }
    }
    pub fn collecting(work: usize, diagnostics: usize) -> Self {
        Self {
            errors: Vec::new(),
            skipped: Vec::new(),
            complete: true,
            first: false,
            work,
            diagnostics,
            available: None,
            graph_size: None,
        }
    }
    pub fn tick(&mut self, path: &str) -> Result<()> {
        if self.work == 0 {
            self.complete = false;
            return Err(Error::new(
                "limit",
                path,
                "validation work budget exhausted",
            ));
        }
        self.work -= 1;
        Ok(())
    }
    pub fn check<T>(&mut self, value: Result<T>) -> Result<Option<T>> {
        self.tick("validation")?;
        match value {
            Ok(value) => Ok(Some(value)),
            Err(error) => {
                self.error(error)?;
                Ok(None)
            }
        }
    }
    pub fn error(&mut self, error: Error) -> Result<()> {
        if self.first {
            return Err(error);
        }
        if self.errors.len() + self.skipped.len() >= self.diagnostics {
            self.complete = false;
            return Err(Error::new(
                "limit",
                "validation",
                "validation diagnostic budget exhausted",
            ));
        }
        self.errors.push(error);
        Ok(())
    }
    pub fn skip(&mut self, path: &str, message: &str) -> Result<()> {
        self.tick(path)?;
        if self.errors.len() + self.skipped.len() >= self.diagnostics {
            self.complete = false;
            return Err(Error::new(
                "limit",
                path,
                "validation diagnostic budget exhausted",
            ));
        }
        self.skipped
            .push(Error::new("checks_skipped", path, message));
        Ok(())
    }
    pub fn path(&self, path: &str, field: &str) -> String {
        if self.first {
            path.into()
        } else {
            format!("{path}.{field}")
        }
    }
}

pub(crate) fn name(value: &str, path: &str) -> Result<()> {
    if valid_name(value) {
        Ok(())
    } else {
        Err(Error::new(
            "identifier",
            path,
            format!("{value:?}: use 1..80 ASCII letters, digits, underscores or hyphens"),
        ))
    }
}

pub(crate) fn collect_names(values: &[String], path: &str, checks: &mut Checks) -> Result<()> {
    for value in values {
        checks.check(name(value, path))?;
    }
    if values.windows(2).any(|pair| pair[0] >= pair[1]) {
        checks.error(Error::new("duplicate", path, "names must be unique"))?;
    }
    Ok(())
}

pub fn check_package_identity(id: &str, version: &str, path: &str) -> Result<()> {
    if id.is_empty() || id.len() > 128 || version.is_empty() || version.len() > 64 {
        Err(Error::new(
            "identifier",
            path,
            "package ID and version must be non-empty and bounded",
        ))
    } else {
        Ok(())
    }
}

fn collect_signature(value: &Signature, path: &str, checks: &mut Checks) -> Result<()> {
    if value.parameters.len() > MAX_VARIABLES {
        checks.error(Error::new("limit", path, "too many parameters"))?;
    }
    for key in value.parameters.keys() {
        checks.check(name(key, path))?;
    }
    collect_names(&value.outcomes, path, checks)?;
    if value.outcomes.is_empty() {
        checks.error(Error::new(
            "contract",
            path,
            "graph needs at least one declared outcome",
        ))?;
    }
    Ok(())
}

pub(crate) fn check_manifest(manifest: &Manifest) -> Result<()> {
    collect_manifest(manifest, &mut Checks::first())
}
pub(crate) fn collect_manifest(manifest: &Manifest, checks: &mut Checks) -> Result<()> {
    let product = &manifest.product;
    checks.check(name(&product.id, "product.id"))?;
    if manifest.packages.is_empty() {
        checks.error(Error::new(
            "limit",
            "packages",
            "expected package instances",
        ))?;
    }
    for (alias, package) in &manifest.packages {
        checks.check(name(alias, "packages"))?;
        checks.check(check_package_identity(
            &package.id,
            &package.version,
            &format!("packages.{alias}"),
        ))?;
    }
    for (type_id, revision) in &manifest.node_types {
        if !valid_type_id(type_id) || revision.is_empty() || revision.len() > 64 {
            checks.error(Error::new(
                "identifier",
                "node_types",
                "invalid type identifier or semantic revision",
            ))?;
        }
    }
    let mut chunk_packages = vec![None; manifest.chunks.len()];
    for (key, entry) in &manifest.graphs {
        let path = format!("packages.{}.graphs.{}", key.package, key.graph);
        checks.check(name(&key.graph, &path))?;
        if !manifest.packages.contains_key(&key.package) {
            checks.error(Error::new(
                "reference",
                &path,
                "package instance does not exist",
            ))?;
        }
        collect_signature(&entry.signature, &path, checks)?;
        let Some(slot) = checks.check(
            chunk_packages
                .get_mut(entry.chunk as usize)
                .ok_or_else(|| Error::new("reference", &path, "chunk index out of range")),
        )?
        else {
            continue;
        };
        match slot {
            Some(package) if package != &key.package => {
                checks.error(Error::new(
                    "chunk",
                    &path,
                    "a chunk holds graphs of one package instance",
                ))?;
            }
            _ => *slot = Some(key.package.clone()),
        }
    }
    if chunk_packages.iter().any(Option::is_none) {
        checks.error(Error::new(
            "chunk",
            "chunks",
            "every chunk must hold a graph",
        ))?;
    }
    let entry =
        checks.check(manifest.graphs.get(&product.entry).ok_or_else(|| {
            Error::new("reference", "product.entry", "entry graph does not exist")
        }))?;
    if let Some(entry) = entry {
        if !entry.exported {
            checks.error(Error::new(
                "contract",
                product.entry.label(),
                "graph is not exported",
            ))?;
        }
        if product
            .arguments
            .keys()
            .ne(entry.signature.parameters.keys())
        {
            checks.error(Error::new(
                "contract",
                "product.arguments",
                "entry arguments do not match parameters",
            ))?;
        }
        for (key, value) in &product.arguments {
            checks.check(check_scalar(value, "product.arguments"))?;
            if let Some(kind) = entry.signature.parameters.get(key) {
                checks.check(expect(
                    value.kind(),
                    *kind,
                    &format!("product.arguments.{key}"),
                ))?;
            }
        }
    }
    if entry.is_none() {
        for (key, value) in &product.arguments {
            checks.check(check_scalar(value, &format!("product.arguments.{key}")))?;
        }
    }
    if product.shared.len() > MAX_SHARED {
        checks.error(Error::new(
            "limit",
            "product.shared",
            "too many shared variables",
        ))?;
    }
    for (key, variable) in &product.shared {
        checks.check(name(key, "product.shared"))?;
        checks.check(check_scalar(&variable.value, "product.shared"))?;
    }
    if let Some(entry) = entry {
        for outcome in product.endings.keys() {
            if !entry.signature.outcomes.contains(outcome) {
                checks.error(Error::new(
                    "reference",
                    "product.endings",
                    format!("{outcome} is not an outcome of the entry graph"),
                ))?;
            }
        }
    } else {
        checks.skip(
            "product.entry",
            "arguments and endings require an entry graph",
        )?;
    }
    for (from, to) in &product.bindings {
        if !manifest.graphs.contains_key(&from.owner()) {
            checks.error(Error::new(
                "binding",
                "product.bindings",
                format!("unknown import owner {}", from.owner().label()),
            ))?;
        }
        let Some(target) = checks.check(manifest.graphs.get(to).ok_or_else(|| {
            Error::new("binding", "product.bindings", "target graph does not exist")
        }))?
        else {
            continue;
        };
        if !target.exported {
            checks.error(Error::new("contract", to.label(), "graph is not exported"))?;
        }
    }
    Ok(())
}

/// The graphs a manifest assigns to chunk `index`.
pub(crate) fn chunk_graphs(manifest: &Manifest, index: u32) -> BTreeSet<&GraphRef> {
    manifest
        .graphs
        .iter()
        .filter(|(_, entry)| entry.chunk == index)
        .map(|(key, _)| key)
        .collect()
}

pub(crate) fn check_chunk(
    manifest: &Manifest,
    index: u32,
    chunk: &Chunk,
    tombstones: &TombstoneSet,
    names: Option<&NameTable>,
) -> Result<()> {
    if chunk.graphs.keys().collect::<BTreeSet<_>>() != chunk_graphs(manifest, index) {
        return Err(Error::new(
            "chunk",
            format!("chunks[{index}]"),
            "chunk graphs differ from the manifest",
        ));
    }
    let mut points = BTreeSet::new();
    let mut options = BTreeSet::new();
    for (key, graph) in &chunk.graphs {
        collect_graph(
            manifest,
            key,
            graph,
            tombstones,
            names,
            &mut points,
            &mut options,
            &mut Checks::first(),
        )?;
    }
    Ok(())
}

#[expect(
    clippy::too_many_arguments,
    reason = "the shared rule needs decoder context and the collecting sink"
)]
pub(crate) fn collect_graph(
    manifest: &Manifest,
    key: &GraphRef,
    graph: &Graph,
    tombstones: &TombstoneSet,
    names: Option<&NameTable>,
    points: &mut BTreeSet<crate::ChoicePointId>,
    options: &mut BTreeSet<crate::OptionId>,
    checks: &mut Checks,
) -> Result<()> {
    let path = format!("packages.{}.graphs.{}", key.package, key.graph);
    let header = &graph.header;
    let entry = manifest
        .graphs
        .get(key)
        .ok_or_else(|| Error::new("chunk", &path, "graph is not in the manifest"))?;
    if header.signature() != entry.signature {
        checks.error(Error::new(
            "chunk",
            &path,
            "graph header differs from its manifest signature",
        ))?;
    }
    collect_graph_locals(header, &path, checks)?;
    for (shared, kind) in &header.shared {
        let Some(declared) =
            checks.check(manifest.product.shared.get(shared).ok_or_else(|| {
                Error::new(
                    "reference",
                    &path,
                    format!("product does not provide shared variable {shared}"),
                )
            }))?
        else {
            checks.skip(&path, "dependent checks require a valid declaration")?;
            continue;
        };
        checks.check(expect(declared.value.kind(), *kind, &path))?;
    }
    for (port, import) in &header.imports {
        checks.check(name(port, &path))?;
        collect_signature(import, &path, checks)?;
        let reference = ImportRef {
            package: key.package.clone(),
            graph: key.graph.clone(),
            port: port.clone(),
        };
        let Some(target) =
            checks.check(manifest.product.bindings.get(&reference).ok_or_else(|| {
                Error::new(
                    "binding",
                    key.label(),
                    format!("missing provider for {port}"),
                )
            }))?
        else {
            checks.skip(&path, "dependent checks require a valid declaration")?;
            continue;
        };
        let Some(provided) =
            checks.check(manifest.graphs.get(target).ok_or_else(|| {
                Error::new("binding", key.label(), "target graph does not exist")
            }))?
        else {
            checks.skip(&path, "import signature checks require a provider")?;
            continue;
        };
        if &provided.signature != import {
            checks.error(Error::new(
                "contract",
                "product.bindings",
                format!(
                    "import {port} of {} does not match {} parameters and outcomes",
                    key.label(),
                    target.label()
                ),
            ))?;
        }
    }
    for from in manifest.product.bindings.keys() {
        if &from.owner() == key && !header.imports.contains_key(&from.port) {
            checks.error(Error::new(
                "binding",
                "product.bindings",
                "import port is not declared",
            ))?;
        }
    }
    let graph_size = checks.graph_size.unwrap_or(graph.nodes.len());
    if graph_size == 0 || graph_size > MAX_GRAPH_NODES {
        checks.error(Error::new("limit", &path, "a graph holds 1..4096 nodes"))?;
    }
    let available = checks.available.clone();
    if !available.as_ref().map_or_else(
        || graph.nodes.contains_key(&header.entry),
        |ids| ids.contains(&header.entry),
    ) {
        checks.error(Error::new("reference", &path, "entry node does not exist"))?;
    }
    let declarations = header.declarations();
    for (id, plan) in &graph.nodes {
        let path = node_path(names, key, id);
        if tombstones.nodes.contains(id) {
            checks.error(Error::new("tombstone", &path, format!("{id} is deleted")))?;
        }
        let target = |node: &crate::NodeId| -> Result<()> {
            if available
                .as_ref()
                .map_or_else(|| graph.nodes.contains_key(node), |ids| ids.contains(node))
            {
                Ok(())
            } else {
                Err(Error::new(
                    "reference",
                    &path,
                    format!("target node {node} does not exist"),
                ))
            }
        };
        match plan {
            Plan::Passage(passage) => {
                collect_passage(
                    &declarations,
                    passage,
                    &target,
                    tombstones,
                    points,
                    options,
                    &path,
                    checks,
                )?;
            }
            Plan::Branch {
                condition,
                when_true,
                when_false,
            } => {
                let at = checks.path(&path, "condition");
                collect_expected(&declarations, condition, ScalarType::Bool, &at, checks)?;
                checks.check(target(when_true))?;
                checks.check(target(when_false))?;
            }
            Plan::Mutate { assignments, next } => {
                if assignments.len() > MAX_ASSIGNMENTS {
                    checks.error(Error::new("limit", &path, "too many assignments"))?;
                }
                for (index, assignment) in assignments.iter().enumerate() {
                    declarations.collect_assignment(
                        assignment,
                        &checks.path(&path, &format!("assignments[{index}]")),
                        checks,
                    )?;
                }
                checks.check(target(next))?;
            }
            Plan::Call {
                target: call,
                arguments,
                on_return,
            } => {
                let callee = checks.check(match call {
                    CallTarget::Local { graph } => manifest
                        .graphs
                        .get(&GraphRef {
                            package: key.package.clone(),
                            graph: graph.clone(),
                        })
                        .map(|entry| &entry.signature)
                        .ok_or_else(|| {
                            Error::new("reference", &path, format!("unknown graph {graph}"))
                        }),
                    CallTarget::Import { port } => header.imports.get(port).ok_or_else(|| {
                        Error::new("reference", &path, format!("unknown import {port}"))
                    }),
                })?;
                if let Some(callee) = callee {
                    if arguments.keys().ne(callee.parameters.keys())
                        || on_return.keys().ne(callee.outcomes.iter())
                    {
                        checks.error(Error::new(
                            "contract",
                            &path,
                            "call must bind every parameter and every outcome exactly once",
                        ))?;
                    }
                    for (argument, value) in arguments {
                        let at = checks.path(&path, &format!("arguments.{argument}"));
                        if let Some(kind) = callee.parameters.get(argument) {
                            collect_expected(&declarations, value, *kind, &at, checks)?;
                        } else {
                            declarations.collect(value, &at, checks)?;
                        }
                    }
                } else {
                    checks.skip(&path, "call contract checks require a valid callee")?;
                    for (argument, value) in arguments {
                        declarations.collect(
                            value,
                            &checks.path(&path, &format!("arguments.{argument}")),
                            checks,
                        )?;
                    }
                }
                for node in on_return.values() {
                    checks.check(target(node))?;
                }
            }
            Plan::Return { outcome } => {
                if !header.outcomes.contains(outcome) {
                    checks.error(Error::new(
                        "contract",
                        &path,
                        format!("undeclared outcome {outcome}"),
                    ))?;
                }
            }
        }
    }
    Ok(())
}

#[expect(
    clippy::too_many_arguments,
    reason = "the shared rule needs decoder context and the collecting sink"
)]
fn collect_passage(
    declarations: &Declarations<'_>,
    passage: &Passage,
    target: &dyn Fn(&crate::NodeId) -> Result<()>,
    tombstones: &TombstoneSet,
    points: &mut BTreeSet<crate::ChoicePointId>,
    options: &mut BTreeSet<crate::OptionId>,
    path: &str,
    checks: &mut Checks,
) -> Result<()> {
    if passage.args.len() > MAX_ARGS {
        checks.error(Error::new(
            "limit",
            path,
            "a passage has at most 256 arguments",
        ))?;
    }
    for (argument, value) in &passage.args {
        checks.check(name(argument, path))?;
        declarations.collect(
            value,
            &checks.path(path, &format!("args.{argument}")),
            checks,
        )?;
    }
    if passage.choice_points.len() > MAX_CHOICE_POINTS {
        checks.error(Error::new(
            "limit",
            path,
            "a passage has at most 64 choice points",
        ))?;
    }
    let count = passage.choice_points.len();
    for (index, point) in passage.choice_points.iter().enumerate() {
        let path = format!("{path}.choice_points[{index}]");
        if point.placement.is_none() && index + 1 < count {
            checks.error(Error::new(
                "placement",
                &path,
                "only the last choice point may omit its placement",
            ))?;
        }
        if tombstones.choice_points.contains(&point.id) {
            checks.error(Error::new(
                "tombstone",
                &path,
                format!("{} is deleted", point.id),
            ))?;
        }
        if !points.insert(point.id) {
            checks.error(Error::new(
                "duplicate",
                &path,
                format!("duplicate {}", point.id),
            ))?;
        }
        collect_choice_point(
            declarations,
            passage,
            point,
            target,
            tombstones,
            options,
            &path,
            checks,
        )?;
    }
    checks.check(match (passage.reaches_end(), &passage.next) {
        (true, None) => Err(Error::new(
            "contract",
            path,
            "the passage can run to its end but has no next node",
        )),
        (false, Some(_)) => Err(Error::new(
            "contract",
            path,
            "next is unreachable: the last choice point only branches",
        )),
        (_, Some(next)) => target(next),
        (false, None) => Ok(()),
    })?;
    Ok(())
}

#[expect(
    clippy::too_many_arguments,
    reason = "the shared rule needs decoder context and the collecting sink"
)]
fn collect_choice_point(
    declarations: &Declarations<'_>,
    passage: &Passage,
    point: &ChoicePoint,
    target: &dyn Fn(&crate::NodeId) -> Result<()>,
    tombstones: &TombstoneSet,
    options: &mut BTreeSet<crate::OptionId>,
    path: &str,
    checks: &mut Checks,
) -> Result<()> {
    let count = point.options.len();
    let mut rejoins = point.options.iter().map(|option| match &option.outcome {
        Outcome::Local { rejoin, .. } => Some(rejoin),
        Outcome::Branch { .. } => None,
    });
    let shared = rejoins
        .next()
        .flatten()
        .is_some_and(|first| rejoins.all(|rejoin| rejoin == Some(first)));
    choice_cardinality(count, point.min, point.max, shared, path, checks)?;
    for (index, option) in point.options.iter().enumerate() {
        let path = format!("{path}.options[{index}]");
        if tombstones.options.contains(&option.id) {
            checks.error(Error::new(
                "tombstone",
                &path,
                format!("{} is deleted", option.id),
            ))?;
        }
        if !options.insert(option.id) {
            checks.error(Error::new(
                "duplicate",
                &path,
                format!("duplicate {}", option.id),
            ))?;
        }
        checks.check(option_label(
            count,
            option.label.is_some(),
            &checks.path(&path, "label"),
        ))?;
        for (field, condition) in [
            ("visible_if", &option.visible_if),
            ("enabled_if", &option.enabled_if),
        ]
        .into_iter()
        .filter_map(|(field, condition)| condition.as_ref().map(|condition| (field, condition)))
        {
            let at = checks.path(&path, field);
            collect_expected(declarations, condition, ScalarType::Bool, &at, checks)?;
        }
        if option.effects.len() > MAX_ASSIGNMENTS {
            checks.error(Error::new("limit", &path, "too many effects"))?;
        }
        for (index, effect) in option.effects.iter().enumerate() {
            declarations.collect_assignment(
                effect,
                &checks.path(&path, &format!("effects[{index}]")),
                checks,
            )?;
        }
        match &option.outcome {
            Outcome::Branch { target: node } => {
                checks.check(target(node))?;
            }
            Outcome::Local { reply, .. } => {
                let Some(body) = checks.check(passage.body.as_ref().ok_or_else(|| {
                    Error::new("local", &path, "local outcomes need a passage body")
                }))?
                else {
                    checks.skip(&path, "dependent checks require a valid declaration")?;
                    continue;
                };
                if reply.as_ref().is_some_and(|reply| reply.unit != body.unit) {
                    checks.error(Error::new(
                        "local",
                        &path,
                        "a reply must be in the passage body's content unit",
                    ))?;
                }
            }
        }
    }
    Ok(())
}

pub(crate) fn check_passage(
    declarations: &Declarations<'_>,
    passage: &Passage,
    target: &dyn Fn(&crate::NodeId) -> Result<()>,
    tombstones: &TombstoneSet,
    points: &mut BTreeSet<crate::ChoicePointId>,
    options: &mut BTreeSet<crate::OptionId>,
    path: &str,
) -> Result<()> {
    collect_passage(
        declarations,
        passage,
        target,
        tombstones,
        points,
        options,
        path,
        &mut Checks::first(),
    )
}

fn choice_cardinality(
    count: usize,
    min: u16,
    max: u16,
    shared: bool,
    path: &str,
    checks: &mut Checks,
) -> Result<()> {
    if count == 0 || count > MAX_OPTIONS {
        checks.error(Error::new("limit", path, "expected 1..128 options"))?;
    }
    if max == 0 || usize::from(max) > count || min > max {
        checks.error(Error::new(
            "cardinality",
            path,
            "require 1 <= max <= options and 0 <= min <= max",
        ))?;
    }
    if (max > 1 || min == 0) && !shared {
        checks.error(Error::new(
            "cardinality",
            path,
            "multiple selection or min = 0 needs local options with one shared rejoin",
        ))?;
    }
    Ok(())
}

/// Local source constraints that do not need a graph or product declaration environment.
pub fn validate_choice_source(point: &crate::source::ChoicePointSource) -> Vec<Error> {
    fn collect(point: &crate::source::ChoicePointSource, checks: &mut Checks) -> Result<()> {
        checks.check(name(&point.key, "/payload/key"))?;
        if point.id.is_none() {
            checks.error(Error::new(
                "missing_id",
                "/payload/id",
                "persisted choice point needs an ID",
            ))?;
        }
        let mut rejoins = point.options.iter().map(|option| match &option.outcome {
            crate::source::OutcomeSource::Local { rejoin, .. } => Some(rejoin),
            crate::source::OutcomeSource::Branch { .. } => None,
        });
        let shared = rejoins
            .next()
            .flatten()
            .is_some_and(|first| rejoins.all(|rejoin| rejoin == Some(first)));
        choice_cardinality(
            point.options.len(),
            point.min,
            point.max,
            shared,
            "/payload",
            checks,
        )?;
        let mut keys = BTreeSet::new();
        let mut ids = BTreeSet::new();
        for (index, option) in point.options.iter().enumerate() {
            let path = format!("/payload/options/{index}");
            checks.check(name(&option.key, &format!("{path}/key")))?;
            if !keys.insert(&option.key) {
                checks.error(Error::new(
                    "duplicate",
                    format!("{path}/key"),
                    "duplicate option key",
                ))?;
            }
            match option.id {
                None => checks.error(Error::new(
                    "missing_id",
                    format!("{path}/id"),
                    "persisted option needs an ID",
                ))?,
                Some(id) if !ids.insert(id) => checks.error(Error::new(
                    "duplicate",
                    format!("{path}/id"),
                    "duplicate option ID",
                ))?,
                _ => {}
            }
            checks.check(option_label(
                point.options.len(),
                option.label.is_some(),
                &format!("{path}/label"),
            ))?;
            if option.effects.len() > MAX_ASSIGNMENTS {
                checks.error(Error::new(
                    "limit",
                    format!("{path}/effects"),
                    "too many effects",
                ))?;
            }
        }
        Ok(())
    }
    let mut checks = Checks::collecting(usize::MAX, usize::MAX);
    if let Err(error) = collect(point, &mut checks) {
        checks.errors.push(error);
    }
    checks.errors
}

fn option_label(count: usize, present: bool, path: &str) -> Result<()> {
    if count > 1 && !present {
        Err(Error::new(
            "label",
            path,
            "every option of a choice point with two or more options needs a label",
        ))
    } else {
        Ok(())
    }
}

fn collect_expected(
    declarations: &Declarations<'_>,
    expression: &crate::Expr,
    expected: ScalarType,
    path: &str,
    checks: &mut Checks,
) -> Result<()> {
    if let Some(actual) = declarations.collect(expression, path, checks)? {
        checks.check(expect(actual, expected, path))?;
    }
    Ok(())
}

fn collect_graph_locals(
    header: &crate::plan::GraphHeader,
    path: &str,
    checks: &mut Checks,
) -> Result<()> {
    if header.locals.len() > MAX_VARIABLES
        || header.imports.len() > MAX_VARIABLES
        || header.shared.len() > MAX_SHARED
    {
        checks.error(Error::new("limit", path, "too many variables"))?;
    }
    for (local, value) in &header.locals {
        checks.check(name(local, path))?;
        checks.check(check_scalar(value, path))?;
    }
    Ok(())
}

pub fn validate_graph_source(source: &crate::source::GraphSource) -> Vec<Error> {
    fn collect(source: &crate::source::GraphSource, checks: &mut Checks) -> Result<()> {
        let mut outcomes = source.outcomes.clone();
        outcomes.sort();
        let mut imports = source.imports.clone();
        for import in imports.values_mut() {
            import.outcomes.sort();
        }
        let header = crate::plan::GraphHeader {
            title: source.title.clone(),
            parameters: source.parameters.clone(),
            locals: source.locals.clone(),
            shared: source.shared.clone(),
            imports,
            outcomes,
            entry: crate::NodeId::from_bytes([0; 16]),
        };
        collect_signature(&header.signature(), "", checks)?;
        collect_graph_locals(&header, "", checks)?;
        for shared in source.shared.keys() {
            checks.check(name(shared, "shared"))?;
        }
        for (port, import) in &header.imports {
            checks.check(name(port, "imports"))?;
            collect_signature(import, &format!("imports.{port}"), checks)?;
        }
        Ok(())
    }
    let mut checks = Checks::collecting(usize::MAX, usize::MAX);
    if let Err(error) = collect(source, &mut checks) {
        checks.errors.push(error);
    }
    checks.errors
}

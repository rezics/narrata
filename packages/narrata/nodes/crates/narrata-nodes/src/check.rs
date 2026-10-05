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

pub(crate) fn sorted_names(values: &[String], path: &str) -> Result<()> {
    for value in values {
        name(value, path)?;
    }
    if values.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err(Error::new("duplicate", path, "names must be unique"));
    }
    Ok(())
}

fn signature(value: &Signature, path: &str) -> Result<()> {
    if value.parameters.len() > MAX_VARIABLES {
        return Err(Error::new("limit", path, "too many parameters"));
    }
    for key in value.parameters.keys() {
        name(key, path)?;
    }
    sorted_names(&value.outcomes, path)?;
    if value.outcomes.is_empty() {
        return Err(Error::new(
            "contract",
            path,
            "graph needs at least one declared outcome",
        ));
    }
    Ok(())
}

pub(crate) fn check_manifest(manifest: &Manifest) -> Result<()> {
    let product = &manifest.product;
    name(&product.id, "product.id")?;
    if manifest.packages.is_empty() {
        return Err(Error::new(
            "limit",
            "packages",
            "expected package instances",
        ));
    }
    for (alias, package) in &manifest.packages {
        name(alias, "packages")?;
        if package.id.is_empty()
            || package.id.len() > 128
            || package.version.is_empty()
            || package.version.len() > 64
        {
            return Err(Error::new(
                "identifier",
                format!("packages.{alias}"),
                "package ID and version must be non-empty and bounded",
            ));
        }
    }
    for (type_id, revision) in &manifest.node_types {
        if !valid_type_id(type_id) || revision.is_empty() || revision.len() > 64 {
            return Err(Error::new(
                "identifier",
                "node_types",
                "invalid type identifier or semantic revision",
            ));
        }
    }
    let mut chunk_packages = vec![None; manifest.chunks.len()];
    for (key, entry) in &manifest.graphs {
        let path = format!("packages.{}.graphs.{}", key.package, key.graph);
        name(&key.graph, &path)?;
        if !manifest.packages.contains_key(&key.package) {
            return Err(Error::new(
                "reference",
                &path,
                "package instance does not exist",
            ));
        }
        signature(&entry.signature, &path)?;
        let slot = chunk_packages
            .get_mut(entry.chunk as usize)
            .ok_or_else(|| Error::new("reference", &path, "chunk index out of range"))?;
        match slot {
            Some(package) if package != &key.package => {
                return Err(Error::new(
                    "chunk",
                    &path,
                    "a chunk holds graphs of one package instance",
                ));
            }
            _ => *slot = Some(key.package.clone()),
        }
    }
    if chunk_packages.iter().any(Option::is_none) {
        return Err(Error::new(
            "chunk",
            "chunks",
            "every chunk must hold a graph",
        ));
    }
    let entry = manifest
        .graphs
        .get(&product.entry)
        .ok_or_else(|| Error::new("reference", "product.entry", "entry graph does not exist"))?;
    if !entry.exported {
        return Err(Error::new(
            "contract",
            product.entry.label(),
            "graph is not exported",
        ));
    }
    if product
        .arguments
        .keys()
        .ne(entry.signature.parameters.keys())
    {
        return Err(Error::new(
            "contract",
            "product.arguments",
            "entry arguments do not match parameters",
        ));
    }
    for (key, value) in &product.arguments {
        check_scalar(value, "product.arguments")?;
        if let Some(kind) = entry.signature.parameters.get(key) {
            expect(value.kind(), *kind, &format!("product.arguments.{key}"))?;
        }
    }
    if product.shared.len() > MAX_SHARED {
        return Err(Error::new(
            "limit",
            "product.shared",
            "too many shared variables",
        ));
    }
    for (key, variable) in &product.shared {
        name(key, "product.shared")?;
        check_scalar(&variable.value, "product.shared")?;
    }
    for outcome in product.endings.keys() {
        if !entry.signature.outcomes.contains(outcome) {
            return Err(Error::new(
                "reference",
                "product.endings",
                format!("{outcome} is not an outcome of the entry graph"),
            ));
        }
    }
    for (from, to) in &product.bindings {
        if !manifest.graphs.contains_key(&from.owner()) {
            return Err(Error::new(
                "binding",
                "product.bindings",
                format!("unknown import owner {}", from.owner().label()),
            ));
        }
        let target = manifest.graphs.get(to).ok_or_else(|| {
            Error::new("binding", "product.bindings", "target graph does not exist")
        })?;
        if !target.exported {
            return Err(Error::new("contract", to.label(), "graph is not exported"));
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
        check_graph(
            manifest,
            key,
            graph,
            tombstones,
            names,
            &mut points,
            &mut options,
        )?;
    }
    Ok(())
}

fn check_graph(
    manifest: &Manifest,
    key: &GraphRef,
    graph: &Graph,
    tombstones: &TombstoneSet,
    names: Option<&NameTable>,
    points: &mut BTreeSet<crate::ChoicePointId>,
    options: &mut BTreeSet<crate::OptionId>,
) -> Result<()> {
    let path = format!("packages.{}.graphs.{}", key.package, key.graph);
    let header = &graph.header;
    let entry = manifest
        .graphs
        .get(key)
        .ok_or_else(|| Error::new("chunk", &path, "graph is not in the manifest"))?;
    if header.signature() != entry.signature {
        return Err(Error::new(
            "chunk",
            &path,
            "graph header differs from its manifest signature",
        ));
    }
    if header.locals.len() > MAX_VARIABLES
        || header.imports.len() > MAX_VARIABLES
        || header.shared.len() > MAX_SHARED
    {
        return Err(Error::new("limit", &path, "too many variables"));
    }
    for (local, value) in &header.locals {
        name(local, &path)?;
        check_scalar(value, &path)?;
    }
    for (shared, kind) in &header.shared {
        let declared = manifest.product.shared.get(shared).ok_or_else(|| {
            Error::new(
                "reference",
                &path,
                format!("product does not provide shared variable {shared}"),
            )
        })?;
        expect(declared.value.kind(), *kind, &path)?;
    }
    for (port, import) in &header.imports {
        name(port, &path)?;
        signature(import, &path)?;
        let reference = ImportRef {
            package: key.package.clone(),
            graph: key.graph.clone(),
            port: port.clone(),
        };
        let target = manifest.product.bindings.get(&reference).ok_or_else(|| {
            Error::new(
                "binding",
                key.label(),
                format!("missing provider for {port}"),
            )
        })?;
        let provided = manifest
            .graphs
            .get(target)
            .ok_or_else(|| Error::new("binding", key.label(), "target graph does not exist"))?;
        if &provided.signature != import {
            return Err(Error::new(
                "contract",
                "product.bindings",
                format!(
                    "import {port} of {} does not match {} parameters and outcomes",
                    key.label(),
                    target.label()
                ),
            ));
        }
    }
    for from in manifest.product.bindings.keys() {
        if &from.owner() == key && !header.imports.contains_key(&from.port) {
            return Err(Error::new(
                "binding",
                "product.bindings",
                "import port is not declared",
            ));
        }
    }
    if graph.nodes.is_empty() || graph.nodes.len() > MAX_GRAPH_NODES {
        return Err(Error::new("limit", &path, "a graph holds 1..4096 nodes"));
    }
    if !graph.nodes.contains_key(&header.entry) {
        return Err(Error::new("reference", &path, "entry node does not exist"));
    }
    let declarations = header.declarations();
    for (id, plan) in &graph.nodes {
        let path = node_path(names, key, id);
        if tombstones.nodes.contains(id) {
            return Err(Error::new("tombstone", &path, format!("{id} is deleted")));
        }
        let target = |node: &crate::NodeId| -> Result<()> {
            if graph.nodes.contains_key(node) {
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
                check_passage(
                    &declarations,
                    passage,
                    &target,
                    tombstones,
                    points,
                    options,
                    &path,
                )?;
            }
            Plan::Branch {
                condition,
                when_true,
                when_false,
            } => {
                expect(
                    declarations.check(condition, &path)?,
                    ScalarType::Bool,
                    &path,
                )?;
                target(when_true)?;
                target(when_false)?;
            }
            Plan::Mutate { assignments, next } => {
                if assignments.len() > MAX_ASSIGNMENTS {
                    return Err(Error::new("limit", &path, "too many assignments"));
                }
                declarations.check_assignments(assignments, &path)?;
                target(next)?;
            }
            Plan::Call {
                target: call,
                arguments,
                on_return,
            } => {
                let callee = match call {
                    CallTarget::Local { graph } => manifest
                        .graphs
                        .get(&GraphRef {
                            package: key.package.clone(),
                            graph: graph.clone(),
                        })
                        .map(|entry| &entry.signature)
                        .ok_or_else(|| {
                            Error::new("reference", &path, format!("unknown graph {graph}"))
                        })?,
                    CallTarget::Import { port } => header.imports.get(port).ok_or_else(|| {
                        Error::new("reference", &path, format!("unknown import {port}"))
                    })?,
                };
                if arguments.keys().ne(callee.parameters.keys())
                    || on_return.keys().ne(callee.outcomes.iter())
                {
                    return Err(Error::new(
                        "contract",
                        &path,
                        "call must bind every parameter and every outcome exactly once",
                    ));
                }
                for (argument, value) in arguments {
                    if let Some(kind) = callee.parameters.get(argument) {
                        expect(declarations.check(value, &path)?, *kind, &path)?;
                    }
                }
                for node in on_return.values() {
                    target(node)?;
                }
            }
            Plan::Return { outcome } => {
                if !header.outcomes.contains(outcome) {
                    return Err(Error::new(
                        "contract",
                        &path,
                        format!("undeclared outcome {outcome}"),
                    ));
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
    if passage.args.len() > MAX_ARGS {
        return Err(Error::new(
            "limit",
            path,
            "a passage has at most 256 arguments",
        ));
    }
    for (argument, value) in &passage.args {
        name(argument, path)?;
        declarations.check(value, path)?;
    }
    if passage.choice_points.len() > MAX_CHOICE_POINTS {
        return Err(Error::new(
            "limit",
            path,
            "a passage has at most 64 choice points",
        ));
    }
    let count = passage.choice_points.len();
    for (index, point) in passage.choice_points.iter().enumerate() {
        let path = format!("{path}.choice_points[{index}]");
        if point.placement.is_none() && index + 1 < count {
            return Err(Error::new(
                "placement",
                &path,
                "only the last choice point may omit its placement",
            ));
        }
        if tombstones.choice_points.contains(&point.id) {
            return Err(Error::new(
                "tombstone",
                &path,
                format!("{} is deleted", point.id),
            ));
        }
        if !points.insert(point.id) {
            return Err(Error::new(
                "duplicate",
                &path,
                format!("duplicate {}", point.id),
            ));
        }
        check_choice_point(
            declarations,
            passage,
            point,
            target,
            tombstones,
            options,
            &path,
        )?;
    }
    match (passage.reaches_end(), &passage.next) {
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
    }
}

fn check_choice_point(
    declarations: &Declarations<'_>,
    passage: &Passage,
    point: &ChoicePoint,
    target: &dyn Fn(&crate::NodeId) -> Result<()>,
    tombstones: &TombstoneSet,
    options: &mut BTreeSet<crate::OptionId>,
    path: &str,
) -> Result<()> {
    let count = point.options.len();
    if count == 0 || count > MAX_OPTIONS {
        return Err(Error::new("limit", path, "expected 1..128 options"));
    }
    if point.max == 0 || usize::from(point.max) > count || point.min > point.max {
        return Err(Error::new(
            "cardinality",
            path,
            "require 1 <= max <= options and 0 <= min <= max",
        ));
    }
    if point.max > 1 || point.min == 0 {
        let mut rejoins = point.options.iter().map(|option| match &option.outcome {
            Outcome::Local { rejoin, .. } => Ok(rejoin),
            Outcome::Branch { .. } => Err(()),
        });
        let first = rejoins.next().transpose();
        let shared = first
            .ok()
            .flatten()
            .is_some_and(|first| rejoins.all(|rejoin| rejoin.is_ok_and(|rejoin| rejoin == first)));
        if !shared {
            return Err(Error::new(
                "cardinality",
                path,
                "multiple selection or min = 0 needs local options with one shared rejoin",
            ));
        }
    }
    for (index, option) in point.options.iter().enumerate() {
        let path = format!("{path}.options[{index}]");
        if tombstones.options.contains(&option.id) {
            return Err(Error::new(
                "tombstone",
                &path,
                format!("{} is deleted", option.id),
            ));
        }
        if !options.insert(option.id) {
            return Err(Error::new(
                "duplicate",
                &path,
                format!("duplicate {}", option.id),
            ));
        }
        if count > 1 && option.label.is_none() {
            return Err(Error::new(
                "label",
                &path,
                "every option of a choice point with two or more options needs a label",
            ));
        }
        for condition in [&option.visible_if, &option.enabled_if]
            .into_iter()
            .flatten()
        {
            expect(
                declarations.check(condition, &path)?,
                ScalarType::Bool,
                &path,
            )?;
        }
        if option.effects.len() > MAX_ASSIGNMENTS {
            return Err(Error::new("limit", &path, "too many effects"));
        }
        declarations.check_assignments(&option.effects, &path)?;
        match &option.outcome {
            Outcome::Branch { target: node } => target(node)?,
            Outcome::Local { reply, .. } => {
                let body = passage.body.as_ref().ok_or_else(|| {
                    Error::new("local", &path, "local outcomes need a passage body")
                })?;
                if reply.as_ref().is_some_and(|reply| reply.unit != body.unit) {
                    return Err(Error::new(
                        "local",
                        &path,
                        "a reply must be in the passage body's content unit",
                    ));
                }
            }
        }
    }
    Ok(())
}

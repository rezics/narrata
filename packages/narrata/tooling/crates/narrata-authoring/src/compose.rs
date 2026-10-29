//! Pure composition and ID continuity. The host atomically persists the returned tombstone
//! records and lock; no hidden source or filesystem mutation takes place here.

use std::collections::{BTreeMap, BTreeSet};

use narrata_nodes::{
    AuthoredId, Compilation, CompositionLock, Error, NodeRegistry, ObjectCompilation, Owner,
    Program, ProjectSource, Result, compile_objects,
};

use crate::records::{CheckedRecord, CheckedRecords, DraftRecord, locate_diagnostics};
use crate::validate::{
    Collector, Diagnostic, Report, ValidationBudget, validate_records_with_budget, warning,
};

#[derive(Clone, Copy, Debug)]
pub enum ComposeMode<'a> {
    Update,
    Locked(&'a CompositionLock),
}

#[derive(Debug)]
pub struct ComposedSource {
    pub compilation: Compilation,
    pub appended: Vec<(String, AuthoredId)>,
    pub tombstones: Vec<DraftRecord>,
    pub compared: bool,
}

#[derive(Debug)]
pub struct ComposedObjects {
    pub compilation: ObjectCompilation,
    pub appended: Vec<(String, AuthoredId)>,
    pub tombstones: Vec<DraftRecord>,
    pub compared: bool,
}

fn package_of(owners: &BTreeMap<AuthoredId, Owner>, id: &AuthoredId) -> Option<String> {
    let mut current = *id;
    for _ in 0..3 {
        match owners.get(&current)? {
            Owner::Graph(graph) => return Some(graph.package.clone()),
            Owner::Node(node) => current = AuthoredId::Node(*node),
            Owner::ChoicePoint(point) => current = AuthoredId::ChoicePoint(*point),
        }
    }
    None
}

pub fn compose_source(
    source: &ProjectSource,
    previous: Option<&Program>,
    mode: ComposeMode<'_>,
) -> Result<ComposedSource> {
    compose_with_registry(source, previous, mode, &NodeRegistry::gamebook())
}

pub fn compose_with_registry(
    source: &ProjectSource,
    previous: Option<&Program>,
    mode: ComposeMode<'_>,
    registry: &NodeRegistry,
) -> Result<ComposedSource> {
    let composed = compose_objects_with_registry(source, previous, mode, registry)?;
    Ok(ComposedSource {
        compilation: composed.compilation.into_packed()?,
        appended: composed.appended,
        tombstones: composed.tombstones,
        compared: composed.compared,
    })
}

pub fn compose_objects_with_registry(
    source: &ProjectSource,
    previous: Option<&Program>,
    mode: ComposeMode<'_>,
    registry: &NodeRegistry,
) -> Result<ComposedObjects> {
    let mut compilation = compile_objects(source, registry)?;
    let mut appended = Vec::new();
    if let Some(old) = previous {
        old.verify_artifact()?;
        let old_owners = old.owners()?;
        let new_owners = compilation.program.owners()?;
        let declared: BTreeSet<_> = source
            .packages
            .values()
            .flat_map(|package| package.tombstones.iter().copied())
            .collect();
        if let Some(removed) = old.tombstones()?.iter().find(|id| !declared.contains(id)) {
            return Err(Error::new(
                "tombstone_removed",
                removed.to_string(),
                "tombstones only grow; restore the deleted tombstone",
            ));
        }
        for (id, owner) in &old_owners {
            match new_owners.get(id) {
                Some(new_owner) if new_owner != owner => {
                    return Err(Error::new(
                        "owner_changed",
                        id.to_string(),
                        "a moved or copied node, choice point or option needs a new ID",
                    ));
                }
                Some(_) => {}
                None if declared.contains(id) => {}
                None => {
                    let alias = package_of(&old_owners, id).filter(|alias| source.packages.contains_key(alias)).ok_or_else(|| Error::new("tombstone", id.to_string(), "the package that owned this deleted ID is no longer in the project"))?;
                    appended.push((alias, *id));
                }
            }
        }
    }
    if !appended.is_empty() {
        if matches!(mode, ComposeMode::Locked(_)) {
            return Err(Error::new(
                "tombstones_needed",
                appended[0].1.to_string(),
                "deleted IDs need tombstones; compose without --locked to append them",
            ));
        }
        let mut updated = source.clone();
        for (alias, id) in &appended {
            if let Some(package) = updated.packages.get_mut(alias) {
                package.tombstones.push(*id);
            }
        }
        compilation = compile_objects(&updated, registry)?;
    }
    if let ComposeMode::Locked(expected) = mode
        && expected != &compilation.lock
    {
        return Err(Error::new(
            "lock_mismatch",
            "lock",
            "package sources, node semantics or the artifact differ from the lock",
        ));
    }
    let tombstones = appended
        .iter()
        .map(|(package, payload)| DraftRecord::Tombstone {
            format_version: crate::records::DraftVersion,
            package: package.clone(),
            payload: *payload,
        })
        .collect();
    Ok(ComposedObjects {
        compilation,
        appended,
        tombstones,
        compared: previous.is_some(),
    })
}

/// A checked record set and its program objects, before optional packing or publication.
#[derive(Debug)]
pub struct ComposedRecords {
    pub records: CheckedRecords,
    pub composed: ComposedObjects,
}

/// Preview compilation of persisted records. Record, assembly and semantic diagnostics are
/// collected first; only a fully checked set is composed, once, into program objects. No
/// single-file pack is built, so the 64 MiB pack budget does not bound the work.
pub fn compose_records(
    records: &[&[u8]],
    previous: Option<&Program>,
    mode: ComposeMode<'_>,
) -> Report<ComposedRecords> {
    compose_records_with_budget(
        records,
        previous,
        mode,
        &NodeRegistry::gamebook(),
        ValidationBudget::default(),
    )
}

pub fn compose_records_with_budget(
    records: &[&[u8]],
    previous: Option<&Program>,
    mode: ComposeMode<'_>,
    registry: &NodeRegistry,
    budget: ValidationBudget,
) -> Report<ComposedRecords> {
    let report = validate_records_with_budget(records, registry, budget);
    let mut out = Collector::new(budget.max_diagnostics);
    out.complete = report.complete;
    out.diagnostics = report.diagnostics;
    let Some(checked) = report.value else {
        return out.finish(None);
    };
    let value = match compose_objects_with_registry(checked.source(), previous, mode, registry) {
        Ok(composed) => {
            for diagnostic in &composed.compilation.diagnostics {
                out.push(warning(diagnostic));
            }
            Some(composed)
        }
        Err(error) => {
            out.error(error, None);
            None
        }
    };
    // Identity paths are not source paths; keep them from falling back to the project record.
    let (mut identified, mut located): (Vec<_>, Vec<_>) = std::mem::take(&mut out.diagnostics)
        .into_iter()
        .partition(|diagnostic| identity(diagnostic).is_some());
    locate_diagnostics(checked.records(), &mut located);
    for diagnostic in &mut identified {
        locate_identity(checked.records(), diagnostic);
    }
    out.diagnostics = located;
    out.diagnostics.extend(identified);
    out.finish(value.map(|composed| ComposedRecords {
        records: checked,
        composed,
    }))
}

/// Continuity errors name an authored ID rather than a source path. Locate the record that
/// still holds it; a deleted ID keeps only its identity.
fn identity(diagnostic: &Diagnostic) -> Option<AuthoredId> {
    if diagnostic.location.record.is_some() {
        return None;
    }
    diagnostic.location.source_path.as_deref()?.parse().ok()
}

fn locate_identity(records: &[CheckedRecord], diagnostic: &mut Diagnostic) {
    let Some(id) = identity(diagnostic) else {
        return;
    };
    let location = &mut diagnostic.location;
    let holder = records.iter().find(|checked| match (checked.record(), id) {
        (DraftRecord::Node { payload, .. }, AuthoredId::Node(id)) => payload.id == Some(id),
        (DraftRecord::ChoicePoint { payload, .. }, AuthoredId::ChoicePoint(id)) => {
            payload.id == Some(id)
        }
        (DraftRecord::ChoicePoint { payload, .. }, AuthoredId::Option(id)) => {
            payload.options.iter().any(|option| option.id == Some(id))
        }
        _ => false,
    });
    let pointer = holder
        .and_then(|checked| match (checked.record(), id) {
            (DraftRecord::ChoicePoint { payload, .. }, AuthoredId::Option(id)) => payload
                .options
                .iter()
                .position(|option| option.id == Some(id))
                .map(|index| format!("/payload/options/{index}")),
            _ => None,
        })
        .unwrap_or_else(|| "/payload/id".into());
    let source_path = location.source_path.take();
    if let Some(checked) = holder {
        *location = checked.record().location(&pointer);
    }
    location.source_path = source_path;
    match id {
        AuthoredId::Node(id) => location.node = Some(id),
        AuthoredId::ChoicePoint(id) => location.choice_point = Some(id),
        AuthoredId::Option(id) => location.option = Some(id),
    }
}

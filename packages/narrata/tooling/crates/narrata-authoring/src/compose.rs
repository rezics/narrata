//! Pure composition and ID continuity. The host atomically persists the returned tombstone
//! records and lock; no hidden source or filesystem mutation takes place here.

use std::collections::{BTreeMap, BTreeSet};

use narrata_nodes::{
    AuthoredId, Compilation, CompositionLock, Error, NodeRegistry, Owner, Program, ProjectSource,
    Result, compile,
};

use crate::records::DraftRecord;

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
    let mut compilation = compile(source, registry)?;
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
        compilation = compile(&updated, registry)?;
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
    Ok(ComposedSource {
        compilation,
        appended,
        tombstones,
        compared: previous.is_some(),
    })
}

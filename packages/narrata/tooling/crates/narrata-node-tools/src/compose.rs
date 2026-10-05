//! Composition with ID continuity (ADR 0013 §3): compared with the previous artifact, vanished
//! IDs gain tombstones, tombstones never disappear and a live ID never changes owner.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use narrata_nodes::{
    AuthoredId, Compilation, CompositionLock, Error, NodeRegistry, Owner, Program, Result, compile,
    parse_json,
};

use crate::files::{ProjectFiles, pretty, read_text, write_text};

/// The publication entry point for a filesystem project. Outlines are exported by content
/// providers, for example `narrata-book outline`. Missing providers are explicitly skipped.
/// Outline defects remain diagnostics and never alter the artifact or success status.
pub fn compose_published(
    path: &Path,
    output: &Path,
    locked: bool,
    outline_paths: &[PathBuf],
) -> Result<Composed> {
    let mut project = crate::load_project(path)?;
    let previous = if output.exists() {
        Some(crate::read_bytes(output)?)
    } else {
        None
    };
    let mut composed = compose(&mut project, previous.as_deref(), locked)?;
    let compilation = &mut composed.compilation;
    if locked {
        verify_lock(&compilation.lock, &project)?;
    }
    let publication = crate::publish::publish(&compilation.program, Some(&compilation.names))?;
    let outlines = outline_paths
        .iter()
        .map(|path| parse_json(&read_text(path)?))
        .collect::<Result<Vec<_>>>()?;
    compilation.analysis = crate::publish::publication_analysis(
        compilation.analysis.clone(),
        &compilation.program,
        Some(&compilation.names),
        &publication,
        &outlines,
    )?;
    compilation.diagnostics = compilation.analysis.diagnostics.clone();
    crate::write_bytes(output, &compilation.pack)?;
    if !locked {
        write_text(&project.lock_path(), &pretty(&compilation.lock)?)?;
    }
    crate::publish::write_publication(output, &publication)?;
    write_text(
        &output.with_extension("analysis.json"),
        &pretty(&compilation.analysis)?,
    )?;
    Ok(composed)
}

#[derive(Debug)]
pub struct Composed {
    pub compilation: Compilation,
    /// Tombstones appended to package sources, by package alias.
    pub appended: Vec<(String, AuthoredId)>,
    /// Whether a previous artifact was compared.
    pub compared: bool,
}

/// The package that owned `id` in `owners`.
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

/// Compiles the project. With a previous artifact, appends tombstones for vanished IDs to the
/// package sources (and rewrites those files) unless `locked`, in which case needing one fails.
pub fn compose(
    project: &mut ProjectFiles,
    previous: Option<&[u8]>,
    locked: bool,
) -> Result<Composed> {
    let registry = NodeRegistry::gamebook();
    let compilation = compile(&project.source, &registry)?;
    let Some(previous) = previous else {
        return Ok(Composed {
            compilation,
            appended: Vec::new(),
            compared: false,
        });
    };
    let (old, _) = Program::from_pack(previous)?;
    let old_owners = old.owners()?;
    let new_owners = compilation.program.owners()?;
    let declared: BTreeSet<AuthoredId> = project
        .source
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
    let mut appended = Vec::new();
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
                let alias = package_of(&old_owners, id)
                    .filter(|alias| project.source.packages.contains_key(alias))
                    .ok_or_else(|| {
                        Error::new(
                            "tombstone",
                            id.to_string(),
                            "the package that owned this deleted ID is no longer in the project",
                        )
                    })?;
                appended.push((alias, *id));
            }
        }
    }
    if appended.is_empty() {
        return Ok(Composed {
            compilation,
            appended,
            compared: true,
        });
    }
    if locked {
        return Err(Error::new(
            "tombstones_needed",
            appended[0].1.to_string(),
            "deleted IDs need tombstones; compose without --locked to append them",
        ));
    }
    for (alias, id) in &appended {
        if let Some(package) = project.source.packages.get_mut(alias) {
            package.tombstones.push(*id);
        }
    }
    let changed: BTreeSet<&String> = appended.iter().map(|(alias, _)| alias).collect();
    for alias in changed {
        if let (Some(package), Some(path)) = (
            project.source.packages.get(alias),
            project.package_paths.get(alias),
        ) {
            write_text(path, &pretty(package)?)?;
        }
    }
    Ok(Composed {
        compilation: compile(&project.source, &registry)?,
        appended,
        compared: true,
    })
}

pub fn verify_lock(lock: &CompositionLock, project: &ProjectFiles) -> Result<()> {
    let path = project.lock_path();
    let recorded: CompositionLock = parse_json(&read_text(&path)?)?;
    if &recorded != lock {
        return Err(Error::new(
            "lock_mismatch",
            path.display().to_string(),
            "package sources, node semantics or the artifact differ from the lock",
        ));
    }
    Ok(())
}

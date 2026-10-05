//! Composition with ID continuity (ADR 0013 §3): compared with the previous artifact, vanished
//! IDs gain tombstones, tombstones never disappear and a live ID never changes owner.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use narrata_nodes::{AuthoredId, Compilation, CompositionLock, Error, Program, Result, parse_json};

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

/// Reads the baseline and expected lock, delegates continuity to the pure authoring core,
/// and writes source files only after composition has succeeded.
pub fn compose(
    project: &mut ProjectFiles,
    previous: Option<&[u8]>,
    locked: bool,
) -> Result<Composed> {
    let previous = previous
        .map(Program::from_pack)
        .transpose()?
        .map(|(program, _)| program);
    let expected = if locked {
        Some(parse_json::<CompositionLock>(&read_text(
            &project.lock_path(),
        )?)?)
    } else {
        None
    };
    let mode = expected.as_ref().map_or(
        narrata_authoring::ComposeMode::Update,
        narrata_authoring::ComposeMode::Locked,
    );
    let composed = narrata_authoring::compose_source(&project.source, previous.as_ref(), mode)
        .map_err(|error| {
            if error.code == "lock_mismatch" {
                Error::new(
                    &error.code,
                    project.lock_path().display().to_string(),
                    error.message,
                )
            } else {
                error
            }
        })?;
    for (alias, id) in &composed.appended {
        if let Some(package) = project.source.packages.get_mut(alias) {
            package.tombstones.push(*id);
        }
    }
    let changed: BTreeSet<&String> = composed.appended.iter().map(|(alias, _)| alias).collect();
    for alias in changed {
        if let (Some(package), Some(path)) = (
            project.source.packages.get(alias),
            project.package_paths.get(alias),
        ) {
            write_text(path, &pretty(package)?)?;
        }
    }
    Ok(Composed {
        compilation: composed.compilation,
        appended: composed.appended,
        compared: composed.compared,
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

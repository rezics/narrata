//! The file boundary: bounded reads, atomic single-file writes and the project source closure.
//! The node engine itself never reads files.

use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use narrata_nodes::{
    Error, MAX_PACK_BYTES, MAX_SOURCE_BYTES, PackageSource, ProjectManifest, ProjectSource, Result,
    parse_json,
};

pub(crate) fn io_error(path: &Path, error: std::io::Error) -> Error {
    Error::new("io", path.display().to_string(), error.to_string())
}

fn read_limited(path: &Path, limit: usize) -> Result<Vec<u8>> {
    let mut value = Vec::new();
    File::open(path)
        .map_err(|e| io_error(path, e))?
        .take(limit as u64 + 1)
        .read_to_end(&mut value)
        .map_err(|e| io_error(path, e))?;
    if value.len() > limit {
        return Err(Error::new(
            "limit",
            path.display().to_string(),
            format!("file exceeds {} MiB", limit / (1024 * 1024)),
        ));
    }
    Ok(value)
}

/// Reads a source document of at most 16 MiB.
pub fn read_text(path: &Path) -> Result<String> {
    String::from_utf8(read_limited(path, MAX_SOURCE_BYTES)?)
        .map_err(|_| Error::new("encoding", path.display().to_string(), "file is not UTF-8"))
}

/// Reads checkpoint text under the session budget, independently of the source budget.
pub(crate) fn read_session(path: &Path) -> Result<String> {
    String::from_utf8(read_limited(path, narrata_nodes::MAX_EXPORT_BYTES)?)
        .map_err(|_| Error::new("encoding", path.display().to_string(), "file is not UTF-8"))
}

/// Reads a pack of at most 64 MiB.
pub fn read_bytes(path: &Path) -> Result<Vec<u8>> {
    read_limited(path, MAX_PACK_BYTES)
}

static TEMP_ID: AtomicU64 = AtomicU64::new(0);

/// Atomically replaces one file. Several outputs are individually published files, not a
/// transaction.
pub fn write_bytes(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|e| io_error(parent, e))?;
    let file_name = path
        .file_name()
        .ok_or_else(|| {
            Error::new(
                "path",
                path.display().to_string(),
                "output needs a filename",
            )
        })?
        .to_string_lossy();
    let temp = parent.join(format!(
        ".{file_name}.{}.{}.tmp",
        std::process::id(),
        TEMP_ID.fetch_add(1, Ordering::Relaxed)
    ));
    let mut owns_temp = false;
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .map_err(|e| io_error(&temp, e))?;
        owns_temp = true;
        file.write_all(bytes).map_err(|e| io_error(&temp, e))?;
        file.sync_all().map_err(|e| io_error(&temp, e))?;
        drop(file);
        fs::rename(&temp, path).map_err(|e| io_error(path, e))
    })();
    if result.is_err() && owns_temp {
        let _ = fs::remove_file(&temp);
    }
    result
}

pub fn write_text(path: &Path, text: &str) -> Result<()> {
    write_bytes(path, text.as_bytes())
}

/// Pretty JSON with a final newline, the form every generated JSON file uses.
pub fn pretty(value: &impl serde::Serialize) -> Result<String> {
    serde_json::to_string_pretty(value)
        .map(|text| text + "\n")
        .map_err(|e| Error::new("encoding", "$", e.to_string()))
}

/// A project on disk: the parsed sources and where each package came from.
#[derive(Clone, Debug)]
pub struct ProjectFiles {
    pub manifest_path: PathBuf,
    pub source: ProjectSource,
    pub package_paths: BTreeMap<String, PathBuf>,
}

impl ProjectFiles {
    pub fn lock_path(&self) -> PathBuf {
        self.manifest_path.with_extension("lock.json")
    }
}

/// Resolves a package path inside the manifest's directory closure: relative, no parent
/// traversal, and no symbolic link that escapes after resolution.
pub(crate) fn package_path(root: &Path, alias: &str, relative: &str) -> Result<PathBuf> {
    let relative_path = Path::new(relative);
    if relative.is_empty()
        || relative.contains(':')
        || relative_path
            .components()
            .any(|c| !matches!(c, Component::Normal(_) | Component::CurDir))
    {
        return Err(Error::new(
            "path",
            alias,
            "package paths must be relative and cannot traverse parent directories",
        ));
    }
    let resolved = root
        .join(relative_path)
        .canonicalize()
        .map_err(|e| io_error(&root.join(relative_path), e))?;
    if !resolved.starts_with(root) {
        return Err(Error::new(
            "path",
            alias,
            "package resolves outside the project source closure",
        ));
    }
    Ok(resolved)
}

pub fn load_project(path: &Path) -> Result<ProjectFiles> {
    let manifest_path = path.canonicalize().map_err(|e| io_error(path, e))?;
    let root = manifest_path.parent().ok_or_else(|| {
        Error::new(
            "path",
            manifest_path.display().to_string(),
            "manifest needs a parent directory",
        )
    })?;
    let manifest: ProjectManifest = parse_json(&read_text(&manifest_path)?)?;
    let mut packages = BTreeMap::new();
    let mut package_paths = BTreeMap::new();
    for (alias, relative) in &manifest.packages {
        let resolved = package_path(root, alias, relative)?;
        let package: PackageSource = parse_json(&read_text(&resolved)?)?;
        packages.insert(alias.clone(), package);
        package_paths.insert(alias.clone(), resolved);
    }
    Ok(ProjectFiles {
        manifest_path,
        source: ProjectSource { manifest, packages },
        package_paths,
    })
}

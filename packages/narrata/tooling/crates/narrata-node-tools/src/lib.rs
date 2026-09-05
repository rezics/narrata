//! Source closure resolution is an I/O boundary; the node engine never reads files.

use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use narrata_nodes::{
    Bundle, CheckedProduct, Compilation, CompositionLock, Error, FORMAT_VERSION,
    MAX_DOCUMENT_BYTES, NarrativePackage, NodePlan, NodeRegistry, ProjectManifest, Result,
    SaveArchive, Session, SessionView, compile, parse_json,
};

pub fn read_text(path: &Path) -> Result<String> {
    let mut value = String::new();
    File::open(path)
        .map_err(|e| io_error(path, e))?
        .take(MAX_DOCUMENT_BYTES as u64 + 1)
        .read_to_string(&mut value)
        .map_err(|e| io_error(path, e))?;
    if value.len() > MAX_DOCUMENT_BYTES {
        return Err(Error::new(
            "limit",
            path.display().to_string(),
            "file exceeds 4 MiB",
        ));
    }
    Ok(value)
}

pub fn load_project(path: &Path) -> Result<Compilation> {
    let path = path.canonicalize().map_err(|e| io_error(path, e))?;
    let root = path.parent().ok_or_else(|| {
        Error::new(
            "path",
            path.display().to_string(),
            "manifest needs a parent directory",
        )
    })?;
    let text = read_text(&path)?;
    let manifest: ProjectManifest = parse_json(&text)?;
    if manifest.format_version != FORMAT_VERSION {
        return Err(Error::new(
            "version",
            "format_version",
            "unsupported project format",
        ));
    }
    let mut total = text.len();
    let mut packages = BTreeMap::new();
    for (alias, relative) in manifest.packages {
        let relative_path = Path::new(&relative);
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
        let text = read_text(&resolved)?;
        total += text.len();
        if total > MAX_DOCUMENT_BYTES {
            return Err(Error::new(
                "limit",
                "packages",
                "source closure exceeds 4 MiB",
            ));
        }
        packages.insert(alias, parse_json::<NarrativePackage>(&text)?);
    }
    compile(
        Bundle {
            format_version: FORMAT_VERSION,
            product: manifest.product,
            packages,
        },
        &NodeRegistry::gamebook(),
    )
}

pub fn load_bundle(path: &Path) -> Result<Compilation> {
    compile(parse_json(&read_text(path)?)?, &NodeRegistry::gamebook())
}

pub fn verify_lock(product: &CheckedProduct, path: &Path) -> Result<()> {
    let lock: CompositionLock = parse_json(&read_text(path)?)?;
    if &lock != product.lock() {
        return Err(Error::new(
            "lock_mismatch",
            path.display().to_string(),
            "selected package bytes, bindings or node semantics differ from the lock",
        ));
    }
    Ok(())
}

static TEMP_ID: AtomicU64 = AtomicU64::new(0);

/// Atomically replace one output file. Multiple compose outputs are individually published files.
pub fn write_text(path: &Path, text: &str) -> Result<()> {
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
        file.write_all(text.as_bytes())
            .map_err(|e| io_error(&temp, e))?;
        file.sync_all().map_err(|e| io_error(&temp, e))?;
        drop(file);
        fs::rename(&temp, path).map_err(|e| io_error(path, e))
    })();
    if result.is_err() && owns_temp {
        let _ = fs::remove_file(&temp);
    }
    result
}

fn io_error(path: &Path, e: std::io::Error) -> Error {
    Error::new("io", path.display().to_string(), e.to_string())
}

fn options(
    args: &[String],
    allowed: &[&str],
) -> std::result::Result<BTreeMap<String, String>, String> {
    let mut out = BTreeMap::new();
    let mut i = 0;
    while i < args.len() {
        let key = &args[i];
        if !allowed.contains(&key.as_str()) {
            return Err(format!("unknown option {key}"));
        }
        let value = if key == "--locked" {
            "true".into()
        } else {
            i += 1;
            args.get(i)
                .cloned()
                .ok_or_else(|| format!("{key} requires a value"))?
        };
        if out.insert(key.clone(), value).is_some() {
            return Err(format!("duplicate option {key}"));
        }
        i += 1;
    }
    Ok(out)
}

pub fn run(args: &[String]) -> std::result::Result<(), String> {
    let Some(command) = args.first() else {
        return Err(usage().into());
    };
    if ["--help", "-h", "help"].contains(&command.as_str()) {
        println!("{}", usage());
        return Ok(());
    }
    if command == "schemas" {
        let flags = options(&args[1..], &["--out"])?;
        let output = flags
            .get("--out")
            .ok_or("schemas requires --out <directory>")?;
        for (name, schema) in [
            ("bundle.schema.json", schemars::schema_for!(Bundle)),
            (
                "project.schema.json",
                schemars::schema_for!(ProjectManifest),
            ),
            ("node-plan.schema.json", schemars::schema_for!(NodePlan)),
            ("view.schema.json", schemars::schema_for!(SessionView)),
            ("save.schema.json", schemars::schema_for!(SaveArchive)),
        ] {
            write_text(
                &Path::new(output).join(name),
                &(serde_json::to_string_pretty(&schema).map_err(|e| e.to_string())? + "\n"),
            )
            .map_err(|e| e.to_string())?;
        }
        return Ok(());
    }
    let path = Path::new(args.get(1).ok_or_else(|| usage().to_owned())?);
    match command.as_str() {
        "compose" => {
            let flags = options(&args[2..], &["--out", "--locked"])?;
            let output = Path::new(
                flags
                    .get("--out")
                    .ok_or("compose requires --out <bundle.json>")?,
            );
            let compilation = load_project(path).map_err(|e| e.to_string())?;
            let lock_path = path.with_extension("lock.json");
            if flags.contains_key("--locked") {
                verify_lock(&compilation.product, &lock_path).map_err(|e| e.to_string())?;
            }
            let source = serde_json::to_string_pretty(compilation.product.source())
                .map_err(|e| e.to_string())?
                + "\n";
            write_text(output, &source).map_err(|e| e.to_string())?;
            if !flags.contains_key("--locked") {
                write_text(
                    &lock_path,
                    &(serde_json::to_string_pretty(compilation.product.lock())
                        .map_err(|e| e.to_string())?
                        + "\n"),
                )
                .map_err(|e| e.to_string())?;
            }
            let analysis = serde_json::json!({"artifact_id":compilation.product.artifact_id(),"graphs":compilation.product.analysis(),"diagnostics":compilation.diagnostics});
            write_text(
                &output.with_extension("analysis.json"),
                &(serde_json::to_string_pretty(&analysis).map_err(|e| e.to_string())? + "\n"),
            )
            .map_err(|e| e.to_string())?;
            println!("{}", compilation.product.artifact_id());
        }
        "validate" | "inspect" => {
            options(&args[2..], &[])?;
            let c = load_bundle(path).map_err(|e| e.to_string())?;
            let out = serde_json::json!({"lock":c.product.lock(),"graphs":c.product.analysis(),"diagnostics":c.diagnostics});
            println!(
                "{}",
                serde_json::to_string_pretty(&out).map_err(|e| e.to_string())?
            );
        }
        "run" => {
            let flags = options(&args[2..], &["--actions", "--load", "--save", "--checkout"])?;
            let product = Arc::new(load_bundle(path).map_err(|e| e.to_string())?.product);
            let mut session = if let Some(load) = flags.get("--load") {
                Session::restore(
                    product,
                    &read_text(Path::new(load)).map_err(|e| e.to_string())?,
                )
            } else {
                Session::new(product)
            }
            .map_err(|e| e.to_string())?;
            if let Some(commit) = flags.get("--checkout") {
                session.checkout(commit).map_err(|e| e.to_string())?;
            }
            if let Some(actions) = flags.get("--actions") {
                for action in actions.split(',').filter(|v| !v.is_empty()) {
                    let cursor = session.cursor().map_err(|e| e.to_string())?.to_owned();
                    session.select(&cursor, action).map_err(|e| e.to_string())?;
                }
            }
            if let Some(save) = flags.get("--save") {
                write_text(Path::new(save), &session.save().map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?;
            }
            println!(
                "{}",
                serde_json::to_string_pretty(&session.view().map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?
            );
        }
        _ => return Err(usage().into()),
    }
    Ok(())
}

pub fn usage() -> &'static str {
    "narrata-book compose <project.json> --out <bundle.json> [--locked]\n\
     narrata-book validate|inspect <bundle.json>\n\
     narrata-book run <bundle.json> [--actions a,b] [--load save.json] [--save save.json] [--checkout commit]\n\
     narrata-book schemas --out <directory>\n\
     These commands are also available as narrata gamebook <command>."
}

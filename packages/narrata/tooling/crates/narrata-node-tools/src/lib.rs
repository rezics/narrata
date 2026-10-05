//! Filesystem tooling and the `narrata-book` CLI for R2 node works. Source closure resolution
//! is an I/O boundary; the node engine never reads files.

#![forbid(unsafe_code)]

#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
mod compose;
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
mod files;
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
pub mod ids;
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
mod play;
pub mod publish;
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
pub mod r1;

#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
pub use native::*;

#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
mod native {
    use super::{files, ids, r1};
    use std::{
        collections::BTreeMap,
        path::{Path, PathBuf},
        sync::Arc,
    };

    use narrata_content_local::{ContentPack, LocalContent};
    use narrata_nodes::{
        AuthoredId, Error, NameTable, Program, Result, Session, analyze, plan::GraphRef,
    };

    pub use super::compose::compose_published;
    pub use super::compose::{Composed, compose, verify_lock};
    pub use super::play::{act, book, new_session, render};
    pub use files::{
        ProjectFiles, load_project, pretty, read_bytes, read_text, write_bytes, write_text,
    };

    /// Command options; a flag may repeat when the command accepts several values.
    fn options(
        args: &[String],
        allowed: &[&str],
        switches: &[&str],
    ) -> std::result::Result<BTreeMap<String, Vec<String>>, String> {
        let mut out = BTreeMap::<String, Vec<String>>::new();
        let mut i = 0;
        while i < args.len() {
            let key = &args[i];
            if switches.contains(&key.as_str()) {
                out.entry(key.clone()).or_default().push("true".into());
            } else if allowed.contains(&key.as_str()) {
                i += 1;
                let value = args
                    .get(i)
                    .cloned()
                    .ok_or_else(|| format!("{key} requires a value"))?;
                out.entry(key.clone()).or_default().push(value);
            } else {
                return Err(format!("unknown option {key}"));
            }
            i += 1;
        }
        for (key, values) in &out {
            if values.len() > 1 && !["--content", "--outline"].contains(&key.as_str()) {
                return Err(format!("duplicate option {key}"));
            }
        }
        Ok(out)
    }

    fn one<'a>(flags: &'a BTreeMap<String, Vec<String>>, key: &str) -> Option<&'a str> {
        flags
            .get(key)
            .and_then(|values| values.first())
            .map(String::as_str)
    }

    fn text(error: Error) -> String {
        error.to_string()
    }

    /// Loads local content packs; the first is the original language.
    pub fn load_content(paths: &[String]) -> Result<LocalContent> {
        let mut content = LocalContent::new();
        for path in paths {
            content.add(ContentPack::parse(&read_text(Path::new(path))?)?)?;
        }
        Ok(content)
    }

    pub fn open_pack(path: &Path) -> Result<(Arc<Program>, Option<NameTable>)> {
        let (program, names) = Program::from_pack(&read_bytes(path)?)?;
        Ok((Arc::new(program), names))
    }

    pub fn run(args: &[String]) -> std::result::Result<(), String> {
        let Some(command) = args.first() else {
            return Err(usage().into());
        };
        if ["--help", "-h", "help"].contains(&command.as_str()) {
            println!("{}", usage());
            return Ok(());
        }
        let rest = &args[1..];
        if command == "schemas" {
            let flags = options(rest, &["--out"], &[])?;
            let output = one(&flags, "--out").ok_or("schemas requires --out <directory>")?;
            for (name, schema) in narrata_nodes::schemas()
                .into_iter()
                .chain(narrata_content_local::schemas())
                .chain(narrata_authoring::schemas())
            {
                write_text(
                    &Path::new(output).join(name),
                    &pretty(&schema).map_err(text)?,
                )
                .map_err(text)?;
            }
            return Ok(());
        }
        let path = Path::new(rest.first().ok_or_else(|| usage().to_owned())?);
        let rest = &rest[1..];
        match command.as_str() {
            "compose" => compose_command(path, rest),
            "ids" => {
                options(rest, &[], &[])?;
                let mut project = load_project(path).map_err(text)?;
                let taken = project.source.packages.values().flat_map(ids::package_ids);
                let mut minter = ids::Minter::new(taken.collect::<Vec<_>>());
                for (alias, package) in &mut project.source.packages {
                    let before = minter.minted;
                    ids::fill(package, &mut minter).map_err(text)?;
                    if minter.minted > before
                        && let Some(path) = project.package_paths.get(alias)
                    {
                        write_text(path, &pretty(package).map_err(text)?).map_err(text)?;
                    }
                }
                println!("minted {} IDs", minter.minted);
                Ok(())
            }
            "inspect" => {
                options(rest, &[], &[])?;
                let (program, names) = open_pack(path).map_err(text)?;
                println!(
                    "{}",
                    pretty(&inspect(&program, names.as_ref()).map_err(text)?).map_err(text)?
                );
                Ok(())
            }
            "outline" => {
                let flags = options(rest, &["--out"], &[])?;
                let pack = ContentPack::parse(&read_text(path).map_err(text)?).map_err(text)?;
                let outline = pretty(&narrata_content_local::outline(&pack)).map_err(text)?;
                match one(&flags, "--out") {
                    Some(out) => write_text(Path::new(out), &outline).map_err(text)?,
                    None => print!("{outline}"),
                }
                Ok(())
            }
            "run" => run_command(path, rest),
            "migrate-r1" => migrate_command(path, rest),
            _ => Err(usage().into()),
        }
    }

    fn compose_command(path: &Path, args: &[String]) -> std::result::Result<(), String> {
        let flags = options(args, &["--out", "--outline"], &["--locked"])?;
        let output =
            Path::new(one(&flags, "--out").ok_or("compose requires --out <story.narpack>")?);
        let locked = flags.contains_key("--locked");
        let outlines: Vec<PathBuf> = flags
            .get("--outline")
            .into_iter()
            .flatten()
            .map(PathBuf::from)
            .collect();
        let composed = compose_published(path, output, locked, &outlines).map_err(text)?;
        if !composed.compared {
            eprintln!(
                "no previous artifact at {}; tombstone checks skipped",
                output.display()
            );
        }
        for (alias, id) in &composed.appended {
            eprintln!("tombstone appended to {alias}: {id}");
        }
        let compilation = &composed.compilation;
        for diagnostic in &compilation.diagnostics {
            eprintln!(
                "{}: {} ({})",
                diagnostic.code, diagnostic.message, diagnostic.path
            );
        }
        println!("{}", compilation.program.artifact_id());
        Ok(())
    }

    fn run_command(path: &Path, args: &[String]) -> std::result::Result<(), String> {
        let flags = options(
            args,
            &[
                "--content",
                "--language",
                "--actions",
                "--load",
                "--save",
                "--checkout",
                "--execution",
            ],
            &[],
        )?;
        let (program, names) = open_pack(path).map_err(text)?;
        let mut session = match one(&flags, "--load") {
            Some(load) => Session::restore(
                program,
                &files::read_session(Path::new(load)).map_err(text)?,
            ),
            None => new_session(program, one(&flags, "--execution")),
        }
        .map_err(text)?;
        if let Some(commit) = one(&flags, "--checkout") {
            let commit = commit
                .parse()
                .map_err(|_| "--checkout expects commit:<64 hex digits>".to_owned())?;
            session.checkout(&commit).map_err(text)?;
        }
        if let Some(actions) = one(&flags, "--actions") {
            act(&mut session, names.as_ref(), actions).map_err(text)?;
        }
        if let Some(save) = one(&flags, "--save") {
            write_text(Path::new(save), &session.export().map_err(text)?).map_err(text)?;
        }
        match flags.get("--content") {
            Some(paths) => {
                let content = load_content(paths).map_err(text)?;
                let languages = one(&flags, "--language")
                    .map(str::to_owned)
                    .into_iter()
                    .collect();
                println!(
                    "{}",
                    render(&session, names.as_ref(), &content, languages).map_err(text)?
                );
            }
            None => println!(
                "{}",
                pretty(&book(&session, names.as_ref()).map_err(text)?).map_err(text)?
            ),
        }
        Ok(())
    }

    /// Converts an R1 project (`migrate-r1 <project.json> --out <directory>`) or migrates an R1
    /// save onto a migrated pack (`migrate-r1 <pack> --save <save.json> --content <pack.json>
    /// --out <export.json>`).
    fn migrate_command(path: &Path, args: &[String]) -> std::result::Result<(), String> {
        let flags = options(
            args,
            &["--out", "--language", "--save", "--content", "--execution"],
            &[],
        )?;
        let output = Path::new(one(&flags, "--out").ok_or("migrate-r1 requires --out")?);
        if let Some(save) = one(&flags, "--save") {
            let (program, names) = open_pack(path).map_err(text)?;
            let names = names.ok_or("the pack has no name table")?;
            let content = load_content(
                flags
                    .get("--content")
                    .ok_or("save migration requires --content <pack.json>")?,
            )
            .map_err(text)?;
            let execution = match one(&flags, "--execution") {
                Some(id) => id
                    .parse()
                    .map_err(|_| "--execution expects execution:<32 hex digits>".to_owned())?,
                None => ids::execution_id().map_err(text)?,
            };
            let ref_text =
                |reference: &narrata_kernel::content::ContentRef| content.text(reference, &[]);
            let session = narrata_nodes::r1::migrate_save(
                program,
                &names,
                &read_text(Path::new(save)).map_err(text)?,
                execution,
                &ref_text,
            )
            .map_err(text)?;
            write_text(output, &session.export().map_err(text)?).map_err(text)?;
            println!("{}", session.cursor().map_err(text)?);
            return Ok(());
        }
        let language = one(&flags, "--language").unwrap_or("zh-Hans");
        let loaded = r1::load(path).map_err(text)?;
        let mut minter = ids::Minter::new(Vec::<AuthoredId>::new());
        let migrated = loaded.migrate(language, &mut minter).map_err(text)?;
        for (alias, relative) in &migrated.manifest.packages {
            if let Some(package) = migrated.packages.get(alias) {
                write_text(&output.join(relative), &pretty(package).map_err(text)?)
                    .map_err(text)?;
            }
        }
        write_text(
            &output.join("project.json"),
            &pretty(&migrated.manifest).map_err(text)?,
        )
        .map_err(text)?;
        write_text(
            &output.join("content").join(format!("{language}.json")),
            &pretty(&migrated.content).map_err(text)?,
        )
        .map_err(text)?;
        println!("{}", hex::encode(loaded.artifact_id().map_err(text)?.0));
        Ok(())
    }

    /// A readable JSON description of a pack: the manifest, and every graph's plans keyed by
    /// alias when the name table is present.
    pub fn inspect(program: &Program, names: Option<&NameTable>) -> Result<serde_json::Value> {
        let manifest = program.manifest();
        let product = &manifest.product;
        let mut graphs = Vec::new();
        for (reference, entry) in &manifest.graphs {
            let graph = program.graph(reference)?;
            let nodes: serde_json::Map<String, serde_json::Value> = graph
            .nodes
            .iter()
            .map(|(id, plan)| {
                let key = names
                    .and_then(|names| names.node(reference, id))
                    .map_or_else(|| id.to_string(), str::to_owned);
                Ok((
                    key,
                    serde_json::json!({"id": id, "plan": serde_json::to_value(plan).map_err(|e| Error::new("encoding", "plan", e.to_string()))?}),
                ))
            })
            .collect::<Result<_>>()?;
            graphs.push(serde_json::json!({
                "reference": reference,
                "exported": entry.exported,
                "chunk": manifest.chunks.get(entry.chunk as usize),
                "parameters": entry.signature.parameters,
                "outcomes": entry.signature.outcomes,
                "entry": graph.header.entry,
                "nodes": nodes,
            }));
        }
        let analysis = analyze(program, names)?;
        Ok(serde_json::json!({
            "artifact_id": program.artifact_id(),
            "migrated_from_r1": names.and_then(|names| names.migrated_from_r1),
            "product": {
                "id": product.id,
                "title": product.title,
                "entry": product.entry,
                "arguments": product.arguments,
                "shared": product.shared.iter().map(|(name, variable)| (name.clone(), serde_json::json!({"value": variable.value, "label": variable.label}))).collect::<serde_json::Map<_, _>>(),
                "endings": product.endings.iter().map(|(outcome, ending)| (outcome.clone(), serde_json::json!({"title": ending.title, "body": ending.body}))).collect::<serde_json::Map<_, _>>(),
                "bindings": product.bindings.iter().map(|(from, to): (_, &GraphRef)| serde_json::json!({"from": from, "to": to})).collect::<Vec<_>>(),
            },
            "packages": manifest.packages.iter().map(|(alias, package)| (alias.clone(), serde_json::json!({"id": package.id, "version": package.version}))).collect::<serde_json::Map<_, _>>(),
            "node_types": manifest.node_types,
            "tombstones": program.tombstones()?.iter().map(|id| id.to_string()).collect::<Vec<_>>(),
            "graphs": graphs,
            "diagnostics": analysis.diagnostics,
        }))
    }

    pub fn usage() -> &'static str {
        "narrata-book compose <project.json> --out <story.narpack> [--locked] [--outline <outline.json>]...\n\
     narrata-book ids <project.json>\n\
     narrata-book inspect <story.narpack>\n\
     narrata-book outline <content.json> [--out <outline.json>]\n\
     narrata-book run <story.narpack> [--content <pack.json>]... [--language <tag>] [--actions a,b+c,~]\n\
     \x20                [--load export.json] [--save export.json] [--checkout commit] [--execution id]\n\
     narrata-book migrate-r1 <r1-project.json> --out <directory> [--language <tag>]\n\
     narrata-book migrate-r1 <story.narpack> --save <r1-save.json> --content <pack.json> --out <export.json> [--execution id]\n\
     narrata-book schemas --out <directory>\n\
     These commands are also available as narrata gamebook <command>."
    }
}

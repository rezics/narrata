//! Compiles a small host-owned VN source into the same immutable pack read by the Web API.

use std::{
    collections::BTreeMap,
    env,
    error::Error,
    fs,
    path::{Path, PathBuf},
};

use narrata_nodes::{NodeRegistry, PackageSource, ProjectManifest, ProjectSource, compile};
use serde_json::from_slice;

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = env::args_os().skip(1);
    let manifest_path = args
        .next()
        .map(PathBuf::from)
        .ok_or("usage: emit_vn_demo <project.json> <package.json> <story.narpack>")?;
    let package_path = args
        .next()
        .map(PathBuf::from)
        .ok_or("usage: emit_vn_demo <project.json> <package.json> <story.narpack>")?;
    let output_path = args
        .next()
        .map(PathBuf::from)
        .ok_or("usage: emit_vn_demo <project.json> <package.json> <story.narpack>")?;
    if args.next().is_some() {
        return Err("usage: emit_vn_demo <project.json> <package.json> <story.narpack>".into());
    }

    let manifest: ProjectManifest = from_slice(&fs::read(&manifest_path)?)?;
    let package: PackageSource = from_slice(&fs::read(&package_path)?)?;
    let alias = manifest.product.entry.package.clone();
    if !manifest.packages.contains_key(&alias) {
        return Err(format!("the product entry package `{alias}` is not declared").into());
    }
    let source = ProjectSource {
        manifest,
        packages: BTreeMap::from([(alias, package)]),
    };
    let compilation = compile(&source, &NodeRegistry::visual_novel())?;
    if let Some(parent) = output_path.parent().filter(|path| *path != Path::new("")) {
        fs::create_dir_all(parent)?;
    }
    fs::write(&output_path, compilation.pack)?;
    println!("{}", compilation.program.artifact_id());
    Ok(())
}

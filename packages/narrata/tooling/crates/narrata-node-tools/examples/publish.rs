//! Publishes a project with provider-exported outlines using the filesystem API.

use std::{error::Error, path::PathBuf};

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args().skip(1).map(PathBuf::from);
    let project = args
        .next()
        .ok_or("supply project.json, output.narpack and optional outline.json paths")?;
    let output = args.next().ok_or("supply an output pack path")?;
    let outlines: Vec<_> = args.collect();
    let composed = narrata_node_tools::compose_published(&project, &output, true, &outlines)?;
    for diagnostic in &composed.compilation.diagnostics {
        eprintln!(
            "{}: {} ({})",
            diagnostic.code, diagnostic.message, diagnostic.path
        );
    }
    println!("{}", composed.compilation.program.artifact_id());
    Ok(())
}

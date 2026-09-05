use std::{collections::BTreeMap, path::PathBuf};

use narrata_nodes::{Bundle, NodePlan, ProjectManifest, SaveArchive, SessionView};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let directory = PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or("provide an output directory")?,
    );
    std::fs::create_dir_all(&directory)?;
    let schemas = BTreeMap::from([
        ("bundle.schema.json", schemars::schema_for!(Bundle)),
        ("node-plan.schema.json", schemars::schema_for!(NodePlan)),
        (
            "project.schema.json",
            schemars::schema_for!(ProjectManifest),
        ),
        ("save.schema.json", schemars::schema_for!(SaveArchive)),
        ("view.schema.json", schemars::schema_for!(SessionView)),
    ]);
    for (name, schema) in schemas {
        std::fs::write(
            directory.join(name),
            format!("{}\n", serde_json::to_string_pretty(&schema)?),
        )?;
    }
    Ok(())
}

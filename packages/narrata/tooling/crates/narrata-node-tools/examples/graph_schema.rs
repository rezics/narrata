use std::{error::Error, path::PathBuf};

fn main() -> Result<(), Box<dyn Error>> {
    let output = PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or("supply a schema output path")?,
    );
    narrata_node_tools::write_text(
        &output,
        &narrata_node_tools::pretty(&narrata_node_tools::publish::graph_files_schema())?,
    )?;
    Ok(())
}

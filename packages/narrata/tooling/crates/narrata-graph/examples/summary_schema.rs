use std::{error::Error, fs};

fn main() -> Result<(), Box<dyn Error>> {
    let output = std::env::args()
        .nth(1)
        .ok_or("supply a schema output path")?;
    let mut bytes = serde_json::to_vec_pretty(&narrata_graph::summary_schema())?;
    bytes.push(b'\n');
    fs::write(output, bytes)?;
    Ok(())
}

//! Measure the existing fixed-seed 100k-choice generator, without resolving any content.
use narrata_authoring::project_relations;
use narrata_node_bench::{WorkSize, generate};
use narrata_nodes::canonical_json;
use serde_json::json;

fn main() -> narrata_nodes::Result<()> {
    let compilation = generate(WorkSize::new(100_000)?)?;
    let started = std::time::Instant::now();
    let projection = project_relations(&compilation.program)?;
    let bytes: usize = projection.parts().iter().map(|p| p.bytes.len()).sum();
    println!(
        "{}",
        String::from_utf8_lossy(&canonical_json(
            &json!({"choices":100000,"counts":projection.index().counts,"row_count":projection.index().row_count,"part_bytes":bytes,"index_bytes":projection.index_object().bytes.len(),"parts":projection.parts().len(),"max_part_bytes":projection.parts().iter().map(|p| p.bytes.len()).max(),"projection_millis":started.elapsed().as_millis()})
        )?)
    );
    Ok(())
}

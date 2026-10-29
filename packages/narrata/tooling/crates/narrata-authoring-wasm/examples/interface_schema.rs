//! Writes the authoring interface JSON Schema; the npm build generates declarations from it.

fn main() -> std::process::ExitCode {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: interface_schema <authoring-interface.schema.json>");
        return std::process::ExitCode::FAILURE;
    };
    let written = narrata_authoring_wasm::interface::schema_text()
        .map_err(|error| error.to_string())
        .and_then(|text| std::fs::write(&path, text).map_err(|error| error.to_string()));
    match written {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{path}: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}

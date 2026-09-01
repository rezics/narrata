pub(crate) fn program(args: &[String]) -> Result<(), String> {
    let path = args.first().ok_or_else(super::usage)?;
    let bytes = super::read_bytes(path)?;
    let checked = narrata_core::load_program(&bytes, &Default::default())
        .map_err(|error| error.to_string())?;
    println!("valid {}", checked.artifact_id());
    Ok(())
}

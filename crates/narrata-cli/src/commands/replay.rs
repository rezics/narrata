pub(crate) fn fixture(args: &[String]) -> Result<(), String> {
    let path = args.first().ok_or_else(super::usage)?;
    let fixture = narrata_testkit::fixture::load_fixture(std::path::Path::new(path))
        .map_err(|error| error.to_string())?;
    let manifest =
        narrata_testkit::fixture::run_fixture(&fixture).map_err(|error| error.to_string())?;
    for (index, (state, receipt)) in manifest
        .state_digests
        .iter()
        .zip(&manifest.receipt_digests)
        .enumerate()
    {
        println!("step {index}: {state} {receipt}");
    }
    Ok(())
}

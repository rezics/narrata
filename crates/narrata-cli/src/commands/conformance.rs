pub(crate) fn directory(args: &[String]) -> Result<(), String> {
    let path = args.first().ok_or_else(super::usage)?;
    let mut fixtures = std::fs::read_dir(path)
        .map_err(|error| format!("cannot read fixture directory {path}: {error}"))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .collect::<Vec<_>>();
    fixtures.sort();
    if fixtures.is_empty() {
        return Err("fixture directory contains no JSON fixtures".to_owned());
    }
    for path in fixtures {
        let fixture = narrata_testkit::fixture::load_fixture(&path)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        narrata_testkit::fixture::run_fixture(&fixture)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        println!("ok {}", path.display());
    }
    Ok(())
}

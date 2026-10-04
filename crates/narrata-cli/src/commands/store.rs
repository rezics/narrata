pub(crate) fn run(command: &str, args: &[String]) -> Result<(), String> {
    match (command, args) {
        ("migrate-v2", [source, target]) => {
            narrata_store_sqlite::migrate_v2(source, target).map_err(|error| error.to_string())?;
            println!("migrated={source} store={target}");
            Ok(())
        }
        ("migrate-v2", _) => {
            Err("usage: narrata store migrate-v2 <schema-v2-store> <new-store>".to_owned())
        }
        _ => Err("unknown store command".to_owned()),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::path::Path;

    use narrata_store::SaveStore;

    use super::run;

    #[test]
    fn migrate_v2_copies_a_schema_v2_store_into_a_new_one() {
        let directory =
            std::env::temp_dir().join(format!("narrata-cli-migrate-v2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        let source = directory.join("save-v2.sqlite3");
        std::fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../fixtures/compat/store-sqlite-v2/save-v2.sqlite3"),
            &source,
        )
        .unwrap();
        let target = directory.join("saves.sqlite3");
        let path = |path: &Path| path.to_str().unwrap().to_owned();

        assert!(narrata_store_sqlite::open(&source).is_err());
        run("migrate-v2", &[path(&source), path(&target)]).unwrap();
        let store = narrata_store_sqlite::open(&target).unwrap();
        assert!(store.integrity_scan().unwrap().is_empty());
        drop(store);
        assert!(run("migrate-v2", &[path(&source), path(&target)]).is_err());
        assert!(run("migrate-v2", &[path(&source)]).is_err());
        std::fs::remove_dir_all(&directory).unwrap();
    }
}

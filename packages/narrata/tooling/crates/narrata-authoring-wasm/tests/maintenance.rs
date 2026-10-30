#![allow(clippy::unwrap_used, clippy::panic)]
use narrata_authoring::publication::publication_from_pack;
use narrata_authoring_wasm::maintenance::{
    Engine, ImportSelection, MaintenanceLimits, MaintenanceRequest, MaintenanceResult,
};
use narrata_storage_host::{
    StoreId,
    testing::{self, MemoryHost},
};
use std::path::Path;

fn limits() -> MaintenanceLimits {
    MaintenanceLimits {
        object_reads: 100_000,
        bytes: 64 * 1024 * 1024,
        pages: 10_000,
        work: 1_000_000,
        page_size: 2,
        loads: 10_000,
        load_bytes: 64 * 1024 * 1024,
    }
}

#[test]
fn migrated_and_proposal_checkpoints_use_the_same_cold_cache_driver_and_exact_shallow_page() {
    for case in [
        "unseen-rule",
        "republished-after-migration",
        "proposal-retained",
        "shallow-history",
    ] {
        let directory = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../../../fixtures/compat/nodes-evolution-v1")
            .join(case);
        let mut engine = Engine::default();
        for file in ["source.narpack", "target.narpack", "ancestor.narpack"] {
            if directory.join(file).exists() {
                let publication =
                    publication_from_pack(&std::fs::read(directory.join(file)).unwrap()).unwrap();
                engine
                    .registry
                    .register_program(publication.shared_program())
                    .unwrap();
            }
        }
        let save = hex::encode(std::fs::read(directory.join("target.checkpoint")).unwrap());
        let checked = engine.inspect_save(&save).unwrap();
        let host = MemoryHost::new(StoreId::from_bytes([9; 16]));
        let cache = engine.cache.share();
        let execution = "execution:0190f2a0000070008000000000000001"
            .parse()
            .unwrap();
        testing::run(&cache, &host, || {
            engine.import(
                &save,
                ImportSelection {
                    execution,
                    artifact: checked.artifact,
                    cursor: checked.cursor,
                    expected_revision: None,
                    now: 100,
                },
                limits(),
            )
        })
        .unwrap();
        engine.refresh();
        let shallow = testing::run(&cache, &host, || {
            engine.export(
                execution,
                checked.artifact,
                checked.cursor,
                Some(0),
                limits(),
            )
        })
        .unwrap();
        assert_eq!(
            engine.inspect_save(&hex::encode(shallow)).unwrap().cursor,
            checked.cursor
        );
        engine.refresh();
        assert!(
            matches!(testing::run(&cache,&host,||engine.run(MaintenanceRequest::Integrity,limits())).unwrap(),MaintenanceResult::Integrity {issues} if issues.is_empty())
        );
        // Invalid exact identities and corrupted bytes never replace the already confirmed root.
        let mut corrupted = hex::decode(&save).unwrap();
        let last = corrupted.len() - 1;
        corrupted[last] ^= 1;
        assert!(
            engine
                .import(
                    &hex::encode(corrupted),
                    ImportSelection {
                        execution,
                        artifact: checked.artifact,
                        cursor: checked.cursor,
                        expected_revision: None,
                        now: 100
                    },
                    limits()
                )
                .is_err()
        );
        engine.refresh();
        let MaintenanceResult::Roots { items, .. } = testing::run(&cache, &host, || {
            engine.run(
                MaintenanceRequest::Roots {
                    after: None,
                    limit: 2,
                },
                limits(),
            )
        })
        .unwrap() else {
            panic!("roots")
        };
        assert_eq!(items[0].commit, checked.cursor);
        // Opening consumes one unit before and after loading the layout. The root scan
        // must not receive a fresh work counter on its next cold retry.
        engine.refresh();
        let before = host
            .load(&narrata_storage_host::LoadRequest::default())
            .store;
        let result = testing::run(&cache, &host, || {
            engine.run(
                MaintenanceRequest::Roots {
                    after: None,
                    limit: 2,
                },
                MaintenanceLimits {
                    work: 3,
                    ..limits()
                },
            )
        });
        assert!(matches!(result, Err(testing::RunError::Failed(error)) if error.code == "limit"));
        assert_eq!(
            host.load(&narrata_storage_host::LoadRequest::default())
                .store,
            before
        );
    }
}

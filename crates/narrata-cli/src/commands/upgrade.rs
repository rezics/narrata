//! `narrata upgrade`: moves format 0 Programs and their saves to the text-free format 1
//! (ADR 0018). The target Program and the migration are derived from the source, so no
//! descriptor file is needed.

use std::{str::FromStr, sync::Arc};

use narrata_core::{
    CheckedProgram, CommitId,
    migration::MigrationOptions,
    program::{encode_program_artifact, load_program},
    upgrade::{
        UNDETERMINED_LANGUAGE, UpgradedProgram, format_upgrade_migration, upgrade_program_v0,
    },
};
use narrata_store::{
    MigrationRegistry, ProgramRegistry, RefKey, RefName, RefRevision, apply_migration,
    dry_run_migration,
};

use super::{
    migrate::{Flags, print_reports},
    read_bytes,
};

pub(crate) fn run(command: &str, args: &[String]) -> Result<(), String> {
    match command {
        "program" => program(args),
        "dry-run" => execute(args, false),
        "apply" => execute(args, true),
        _ => Err("unknown upgrade command; expected program, dry-run, or apply".to_owned()),
    }
}

/// Writes the format 1 Program (hex when the path ends in `.hex`) and its content pack.
fn program(args: &[String]) -> Result<(), String> {
    let source_path = args
        .first()
        .ok_or_else(|| "upgrade program requires a format 0 Program".to_owned())?;
    let flags = Flags::parse(&args[1..])?;
    let source = checked_program(source_path)?;
    let language = flags.one("--language")?.unwrap_or(UNDETERMINED_LANGUAGE);
    let (upgraded, target) = upgrade(&source, language)?;
    let bytes = encode_program_artifact(&upgraded.artifact);
    let program_out = flags.required_one("--program-out")?;
    let program_file = if program_out.ends_with(".hex") {
        format!("{}\n", hex::encode(&bytes)).into_bytes()
    } else {
        bytes
    };
    std::fs::write(program_out, program_file)
        .map_err(|error| format!("cannot write {program_out}: {error}"))?;
    let content_out = flags.required_one("--content-out")?;
    let content = upgraded
        .content
        .to_json()
        .map_err(|error| error.to_string())?;
    std::fs::write(content_out, format!("{content}\n"))
        .map_err(|error| format!("cannot write {content_out}: {error}"))?;
    println!("from={}", source.artifact_id());
    println!("to={}", target.artifact_id());
    println!("content_entries={}", upgraded.content.entries.len());
    Ok(())
}

/// Reports or applies the format upgrade of one Commit in a SQLite store. Applying writes a
/// Migration Commit whose parent is the source Commit and points the save Ref at it.
fn execute(args: &[String], apply: bool) -> Result<(), String> {
    let store_path = args
        .first()
        .ok_or_else(|| "upgrade command requires a SQLite store path".to_owned())?;
    let flags = Flags::parse(&args[1..])?;
    let source = checked_program(flags.required_one("--source")?)?;
    let (_, target) = upgrade(&source, UNDETERMINED_LANGUAGE)?;
    let mut programs = ProgramRegistry::new();
    programs.register(source.clone());
    programs.register(target.clone());
    let mut registry = MigrationRegistry::new();
    registry
        .register(Arc::new(
            format_upgrade_migration(&source, &target).map_err(|error| error.to_string())?,
        ))
        .map_err(|error| error.to_string())?;
    let commit =
        CommitId::from_str(flags.required_one("--commit")?).map_err(|error| error.to_string())?;
    let mut store = narrata_store_sqlite::open(store_path).map_err(|error| error.to_string())?;
    if apply {
        let owner = RefName::new(flags.required_one("--ref-owner")?.to_owned())
            .map_err(|error| error.to_string())?;
        let slot = RefName::new(flags.required_one("--ref-slot")?.to_owned())
            .map_err(|error| error.to_string())?;
        let expected = flags
            .one("--expected-revision")?
            .map(|value| {
                value
                    .parse::<u64>()
                    .ok()
                    .and_then(RefRevision::from_u64)
                    .ok_or_else(|| "--expected-revision must be a positive integer".to_owned())
            })
            .transpose()?;
        let result = apply_migration(
            &mut store,
            &registry,
            &programs,
            commit,
            target.artifact_id(),
            None,
            MigrationOptions::default(),
            RefKey::save(owner, slot),
            expected,
            0,
            &Default::default(),
        )
        .map_err(|error| error.to_string())?;
        println!("commit={}", result.commit);
        println!("program={}", target.artifact_id());
        print_reports(&result.reports);
    } else {
        let result = dry_run_migration(
            &store,
            &registry,
            &programs,
            commit,
            target.artifact_id(),
            None,
            MigrationOptions::default(),
            &Default::default(),
        )
        .map_err(|error| error.to_string())?;
        println!("dry-run target={}", result.target_artifact);
        print_reports(&result.reports);
    }
    Ok(())
}

fn upgrade(
    source: &CheckedProgram,
    language: &str,
) -> Result<(UpgradedProgram, Arc<CheckedProgram>), String> {
    let upgraded = upgrade_program_v0(source, language).map_err(|error| error.to_string())?;
    let target = load_program(
        &encode_program_artifact(&upgraded.artifact),
        &Default::default(),
    )
    .map_err(|error| error.to_string())?;
    Ok((upgraded, target))
}

fn checked_program(path: &str) -> Result<Arc<CheckedProgram>, String> {
    load_program(&read_bytes(path)?, &Default::default()).map_err(|error| error.to_string())
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used)]
mod tests {
    use std::{path::Path, str::FromStr};

    use narrata_content_local::{
        Content, ContentPack, LocalContent, Payload, Resolution, ResolveContext, ResolveItem,
        ResolveRequest,
    };
    use narrata_core::{
        CommitId, load_program,
        runtime::{ContentView, DraftResult, pending_view},
    };
    use narrata_store::{
        BundleLimits, CheckpointBundle, RefKey, RefName, SaveStore, load_commit, scan_all,
    };

    use super::{read_bytes, run};

    fn corpus(name: &str) -> String {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/compat/stage5-v0")
            .join(name)
            .to_str()
            .unwrap()
            .to_owned()
    }

    #[test]
    fn upgrade_writes_a_format_1_program_whose_text_the_local_provider_resolves() {
        let directory = tempfile::tempdir().unwrap();
        let path = |name: &str| directory.path().join(name).to_str().unwrap().to_owned();
        run(
            "program",
            &[
                corpus("program-v0.hex"),
                "--program-out".to_owned(),
                path("program-v1.hex"),
                "--content-out".to_owned(),
                path("content.json"),
                "--language".to_owned(),
                "en".to_owned(),
            ],
        )
        .unwrap();
        let program = load_program(
            &read_bytes(&path("program-v1.hex")).unwrap(),
            &Default::default(),
        )
        .unwrap();
        assert_eq!(program.format_version().get(), 1);
        let pack =
            ContentPack::parse(&std::fs::read_to_string(path("content.json")).unwrap()).unwrap();
        let mut content = LocalContent::new();
        content.add(pack).unwrap();

        let store_path = path("saves.sqlite3");
        let mut store = narrata_store_sqlite::open(&store_path).unwrap();
        let manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(corpus("manifest.json")).unwrap()).unwrap();
        let commit = manifest["commit_id"].as_str().unwrap().to_owned();
        CheckpointBundle::from_bytes(
            &read_bytes(&corpus("checkpoint-bundle-v1.hex")).unwrap(),
            BundleLimits::default(),
        )
        .unwrap()
        .import(
            &mut store,
            RefKey::active(RefName::new("stage5").unwrap()).unwrap(),
            None,
            0,
        )
        .unwrap();
        drop(store);

        let flags = [
            "--source".to_owned(),
            corpus("program-v0.hex"),
            "--commit".to_owned(),
            commit.clone(),
        ];
        run(
            "dry-run",
            &[[store_path.clone()].as_slice(), &flags].concat(),
        )
        .unwrap();
        let unchanged = narrata_store_sqlite::open(&store_path).unwrap();
        let refs = |store: &narrata_store_sqlite::SqliteStore| {
            scan_all(
                |after| store.scan_refs(&narrata_store::RefScope::All, after, 16),
                |(key, _)| key.clone(),
            )
            .unwrap()
            .len()
        };
        assert_eq!(refs(&unchanged), 1, "a dry run writes no Ref");
        drop(unchanged);
        let apply = [
            "--ref-owner".to_owned(),
            "player".to_owned(),
            "--ref-slot".to_owned(),
            "upgraded".to_owned(),
        ];
        run(
            "apply",
            &[[store_path.clone()].as_slice(), &flags, &apply].concat(),
        )
        .unwrap();

        let store = narrata_store_sqlite::open(&store_path).unwrap();
        let save = store
            .read_ref(&RefKey::save(
                RefName::new("player").unwrap(),
                RefName::new("upgraded").unwrap(),
            ))
            .unwrap()
            .unwrap();
        let loaded = load_commit(&store, save.commit, &program).unwrap();
        assert_eq!(
            loaded.commit.parent,
            Some(CommitId::from_str(&commit).unwrap())
        );
        let DraftResult::AwaitSay(view) =
            pending_view(&program, loaded.state.pending().unwrap()).unwrap()
        else {
            panic!("the stage5 corpus waits on a Say");
        };
        let ContentView::Segment(body) = view.text else {
            panic!("format 1 shows a segment");
        };
        let resolved = content
            .resolve(&ResolveRequest {
                context: ResolveContext {
                    languages: vec!["en".to_owned()],
                },
                items: vec![ResolveItem {
                    content: Content::Segment(body),
                    args: Default::default(),
                }],
            })
            .unwrap();
        assert!(matches!(
            &resolved[0],
            Resolution::Ok { payload: Payload::Text { text }, .. } if text == "Hello"
        ));
    }
}

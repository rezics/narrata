use std::{collections::BTreeMap, str::FromStr};

use narrata_core::{CommitId, ExecutionId, codec::ObjectKind, program::load_program};
use narrata_store::{
    BranchId, CommitCauseV1, CommitV1, RefName, SaveStore, SessionCoordinator, TimelineOperationId,
};
use narrata_store_sqlite::SqliteStore;

use super::read_bytes;

pub(crate) fn run(command: &str, args: &[String]) -> Result<(), String> {
    match command {
        "log" => log(args),
        "rewind" => move_cursor(args, false),
        "redo" => move_cursor(args, true),
        "fork" => fork(args),
        "bookmark" => bookmark(args),
        _ => Err("unknown timeline command".to_owned()),
    }
}

fn log(args: &[String]) -> Result<(), String> {
    let (path, flags) = parse(args)?;
    let execution = parse_execution(required(&flags, "--execution")?)?;
    let store = SqliteStore::open(path).map_err(|error| error.to_string())?;
    let mut commits = store
        .list_objects()
        .map_err(|error| error.to_string())?
        .into_iter()
        .filter(|object| object.kind() == ObjectKind::Commit)
        .filter_map(|object| {
            CommitV1::decode(object.payload())
                .ok()
                .filter(|commit| commit.execution == execution)
                .map(|commit| (CommitId::from_bytes(*object.id().as_bytes()), commit))
        })
        .collect::<Vec<_>>();
    commits.sort_by_key(|(id, commit)| (commit.turn, *id));
    for (id, commit) in commits {
        let cause = match commit.cause {
            CommitCauseV1::Genesis => "genesis",
            CommitCauseV1::RuntimeTransition(_) => "transition",
        };
        let parent = commit
            .parent
            .map_or_else(|| "-".to_owned(), |value| short(&value.to_string()));
        println!(
            "turn={} commit={} parent={} cause={}",
            commit.turn.0,
            short(&id.to_string()),
            parent,
            cause
        );
    }
    Ok(())
}

fn move_cursor(args: &[String], redo: bool) -> Result<(), String> {
    let (path, flags) = parse(args)?;
    let mut coordinator = open(path, &flags)?;
    let steps = required(&flags, "--steps")?
        .parse::<u64>()
        .map_err(|_| "--steps must be an unsigned integer".to_owned())?;
    let loaded = if redo {
        coordinator.redo(steps, 0)
    } else {
        coordinator.rewind(steps, 0)
    }
    .map_err(|error| error.to_string())?;
    println!(
        "turn={} commit={}",
        loaded.commit.turn.0,
        short(&loaded.id.to_string())
    );
    Ok(())
}

fn fork(args: &[String]) -> Result<(), String> {
    let (path, flags) = parse(args)?;
    let mut coordinator = open(path, &flags)?;
    let branch =
        BranchId::from_str(required(&flags, "--new-branch")?).map_err(|error| error.to_string())?;
    let operation = TimelineOperationId::from_str(required(&flags, "--operation")?)
        .map_err(|error| error.to_string())?;
    let value = coordinator
        .fork(branch, operation, 0)
        .map_err(|error| error.to_string())?;
    println!(
        "branch={} commit={} revision={}",
        branch,
        short(&value.commit.to_string()),
        value.revision.get()
    );
    Ok(())
}

fn bookmark(args: &[String]) -> Result<(), String> {
    let (path, flags) = parse(args)?;
    let mut coordinator = open(path, &flags)?;
    let owner =
        RefName::new(required(&flags, "--owner")?.to_owned()).map_err(|error| error.to_string())?;
    let name =
        RefName::new(required(&flags, "--name")?.to_owned()).map_err(|error| error.to_string())?;
    let operation = TimelineOperationId::from_str(required(&flags, "--operation")?)
        .map_err(|error| error.to_string())?;
    let value = coordinator
        .bookmark(owner, name, operation, 0)
        .map_err(|error| error.to_string())?;
    println!(
        "commit={} revision={}",
        short(&value.commit.to_string()),
        value.revision.get()
    );
    Ok(())
}

fn open(
    path: &str,
    flags: &BTreeMap<String, String>,
) -> Result<SessionCoordinator<SqliteStore>, String> {
    let execution = parse_execution(required(flags, "--execution")?)?;
    let session = RefName::new(required(flags, "--session")?.to_owned())
        .map_err(|error| error.to_string())?;
    let branch =
        BranchId::from_str(required(flags, "--branch")?).map_err(|error| error.to_string())?;
    let bytes = read_bytes(required(flags, "--program")?)?;
    let program = load_program(&bytes, &Default::default()).map_err(|error| error.to_string())?;
    SessionCoordinator::open(
        SqliteStore::open(path).map_err(|error| error.to_string())?,
        program,
        execution,
        session,
        branch,
    )
    .map_err(|error| error.to_string())
}

fn parse(args: &[String]) -> Result<(&str, BTreeMap<String, String>), String> {
    let path = args
        .first()
        .ok_or_else(|| "timeline command requires a SQLite store path".to_owned())?;
    let mut flags = BTreeMap::new();
    let rest = &args[1..];
    let (pairs, remainder) = rest.as_chunks::<2>();
    if !remainder.is_empty() {
        return Err("timeline flags require a value".to_owned());
    }
    for [key, value] in pairs {
        if !key.starts_with("--") || flags.insert(key.clone(), value.clone()).is_some() {
            return Err("invalid or duplicate timeline flag".to_owned());
        }
    }
    Ok((path, flags))
}

fn required<'a>(flags: &'a BTreeMap<String, String>, key: &str) -> Result<&'a str, String> {
    flags
        .get(key)
        .map(String::as_str)
        .ok_or_else(|| format!("missing {key}"))
}

fn parse_execution(value: &str) -> Result<ExecutionId, String> {
    ExecutionId::from_str(value).map_err(|error| error.to_string())
}

fn short(value: &str) -> String {
    let split = value.find(':').map_or(0, |index| index.saturating_add(1));
    value
        .get(split..split.saturating_add(12).min(value.len()))
        .unwrap_or(value)
        .to_owned()
}

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Display,
    str::FromStr,
    sync::Arc,
};

use narrata_core::{
    ActionId, CapabilityId, ChoiceId, CommitId, EventTypeId, FieldId, FlowId, GlobalId, HistoryId,
    InstructionId, LocalId, MigrationId, ProgramArtifactId, RecoveryCheckpointId, StateId, TypeId,
    VariantId,
    migration::{
        DeclarativeMigration, InstructionLocation, LocalLocation, MigrationDescriptor,
        MigrationOptions, RecoveryPoint, RelocationTable, VersionRange,
    },
    program::load_program,
};
use narrata_store::{
    MigrationRegistry, ProgramRegistry, RefKey, RefName, RefRevision, apply_migration,
    dry_run_migration,
};
use serde::Deserialize;

use super::read_bytes;

pub(crate) fn run(command: &str, args: &[String]) -> Result<(), String> {
    match command {
        "inspect" => inspect(args),
        "dry-run" => execute(args, false),
        "apply" => execute(args, true),
        _ => Err("unknown migrate command; expected inspect, dry-run, or apply".to_owned()),
    }
}

fn inspect(args: &[String]) -> Result<(), String> {
    let source_path = args
        .first()
        .ok_or_else(|| "migrate inspect requires a source Program".to_owned())?;
    let target_path = args
        .get(1)
        .ok_or_else(|| "migrate inspect requires a target Program".to_owned())?;
    let flags = Flags::parse(&args[2..])?;
    let source = checked_program(source_path)?;
    let target = checked_program(target_path)?;
    let registry = migration_registry(&flags)?;
    let explicit = explicit_path(&flags)?;
    let path = registry
        .inspect(
            source.artifact_id(),
            target.artifact_id(),
            explicit.as_deref(),
        )
        .map_err(|error| error.to_string())?;
    println!("from={}", source.artifact_id());
    println!("to={}", target.artifact_id());
    println!(
        "path={}",
        path.ids
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(",")
    );
    Ok(())
}

fn execute(args: &[String], apply: bool) -> Result<(), String> {
    let store_path = args
        .first()
        .ok_or_else(|| "migrate command requires a SQLite store path".to_owned())?;
    let flags = Flags::parse(&args[1..])?;
    let source_path = flags.required_one("--source")?;
    let target_path = flags.required_one("--target")?;
    let source = checked_program(source_path)?;
    let target = checked_program(target_path)?;
    let mut programs = ProgramRegistry::new();
    programs.register(source.clone());
    programs.register(target.clone());
    for path in flags.values("--program") {
        programs.register(checked_program(path)?);
    }
    let registry = migration_registry(&flags)?;
    let explicit = explicit_path(&flags)?;
    let commit =
        CommitId::from_str(flags.required_one("--commit")?).map_err(|error| error.to_string())?;
    let options = MigrationOptions {
        allow_lossy_recovery: flags.switch("--confirm-lossy"),
        allow_effect_rekey: flags.switch("--confirm-effect-rekey"),
        allow_cross_barrier_recovery: flags.switch("--confirm-barrier"),
    };
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
            explicit.as_deref(),
            options,
            RefKey::save(owner, slot),
            expected,
            0,
            &Default::default(),
        )
        .map_err(|error| error.to_string())?;
        println!("commit={}", result.commit);
        print_reports(&result.reports);
    } else {
        let result = dry_run_migration(
            &store,
            &registry,
            &programs,
            commit,
            target.artifact_id(),
            explicit.as_deref(),
            options,
            &Default::default(),
        )
        .map_err(|error| error.to_string())?;
        println!("dry-run target={}", result.target_artifact);
        print_reports(&result.reports);
    }
    Ok(())
}

pub(super) fn print_reports(reports: &[narrata_core::MigrationReport]) {
    for report in reports {
        println!(
            "migration={} relocations={} recoveries={} lossy={} effect_rekey={}",
            report
                .migration
                .map_or_else(|| "-".to_owned(), |value| value.to_string()),
            report.relocations.len(),
            report.recoveries.len(),
            report.is_lossy(),
            report.rekeyed_pending_effect
        );
        for recovery in &report.recoveries {
            println!(
                "  recovery={} frames={} pending_interaction={} pending_effect={} crossed_barrier={}",
                recovery.checkpoint,
                recovery.discarded_frames,
                recovery.discarded_pending_interaction,
                recovery.discarded_pending_effect,
                recovery.crossed_external_effect_barrier
            );
        }
    }
}

fn checked_program(path: &str) -> Result<Arc<narrata_core::CheckedProgram>, String> {
    load_program(&read_bytes(path)?, &Default::default()).map_err(|error| error.to_string())
}

fn migration_registry(flags: &Flags) -> Result<MigrationRegistry, String> {
    let mut registry = MigrationRegistry::new();
    let descriptors = flags.values("--descriptor");
    if descriptors.is_empty() {
        return Err("at least one --descriptor is required".to_owned());
    }
    for path in descriptors {
        let bytes = std::fs::read(path)
            .map_err(|error| format!("cannot read migration descriptor {path}: {error}"))?;
        let wire: DescriptorWire = serde_json::from_slice(&bytes)
            .map_err(|error| format!("invalid migration descriptor {path}: {error}"))?;
        let migration =
            DeclarativeMigration::new(wire.checked()?).map_err(|error| error.to_string())?;
        registry
            .register(Arc::new(migration))
            .map_err(|error| error.to_string())?;
    }
    Ok(registry)
}

fn explicit_path(flags: &Flags) -> Result<Option<Vec<MigrationId>>, String> {
    flags
        .one("--path")?
        .map(|value| {
            value
                .split(',')
                .map(|id| MigrationId::from_str(id).map_err(|error| error.to_string()))
                .collect()
        })
        .transpose()
}

#[derive(Default)]
pub(super) struct Flags {
    values: BTreeMap<String, Vec<String>>,
    switches: BTreeSet<String>,
}

impl Flags {
    pub(super) fn parse(args: &[String]) -> Result<Self, String> {
        let mut parsed = Self::default();
        let mut index = 0;
        while index < args.len() {
            let key = args
                .get(index)
                .ok_or_else(|| "invalid migrate flags".to_owned())?;
            if !key.starts_with("--") {
                return Err(format!("unexpected migrate argument {key}"));
            }
            if matches!(
                key.as_str(),
                "--confirm-lossy" | "--confirm-effect-rekey" | "--confirm-barrier"
            ) {
                if !parsed.switches.insert(key.clone()) {
                    return Err(format!("duplicate flag {key}"));
                }
                index += 1;
                continue;
            }
            let value = args
                .get(index + 1)
                .ok_or_else(|| format!("{key} requires a value"))?;
            parsed
                .values
                .entry(key.clone())
                .or_default()
                .push(value.clone());
            index += 2;
        }
        Ok(parsed)
    }

    fn values(&self, key: &str) -> Vec<&str> {
        self.values
            .get(key)
            .into_iter()
            .flatten()
            .map(String::as_str)
            .collect()
    }

    pub(super) fn one(&self, key: &str) -> Result<Option<&str>, String> {
        let values = self.values(key);
        if values.len() > 1 {
            Err(format!("{key} may only be specified once"))
        } else {
            Ok(values.first().copied())
        }
    }

    pub(super) fn required_one(&self, key: &str) -> Result<&str, String> {
        self.one(key)?.ok_or_else(|| format!("missing {key}"))
    }

    fn switch(&self, key: &str) -> bool {
        self.switches.contains(key)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DescriptorWire {
    id: String,
    from: String,
    to: String,
    snapshot_schema: VersionRangeWire,
    #[serde(default)]
    relocations: RelocationsWire,
    #[serde(default)]
    recovery_points: Vec<RecoveryWire>,
}

impl DescriptorWire {
    fn checked(self) -> Result<MigrationDescriptor, String> {
        let mut recovery_points = BTreeMap::new();
        let mut recovery_locations = BTreeMap::new();
        for recovery in self.recovery_points {
            let source = parse_id::<InstructionId>(&recovery.source_instruction)?;
            let value = RecoveryPoint {
                checkpoint: parse_id::<RecoveryCheckpointId>(&recovery.checkpoint)?,
                target_flow: parse_id::<FlowId>(&recovery.target_flow)?,
                target_instruction: parse_id::<InstructionId>(&recovery.target_instruction)?,
                preserve_globals: recovery.preserve_globals,
                crosses_external_effect_barrier: recovery.crosses_external_effect_barrier,
            };
            if let Some(source_flow) = recovery.source_flow {
                let location = InstructionLocation {
                    flow: parse_id(&source_flow)?,
                    instruction: source,
                };
                if recovery_locations.insert(location, value).is_some() {
                    return Err(format!(
                        "duplicate recovery point for {}/{}",
                        location.flow, location.instruction
                    ));
                }
            } else if recovery_points.insert(source, value).is_some() {
                return Err(format!(
                    "duplicate program-wide recovery point for {source}"
                ));
            }
        }
        Ok(MigrationDescriptor {
            id: parse_id(&self.id)?,
            from: parse_id(&self.from)?,
            to: parse_id(&self.to)?,
            accepted_snapshot_schemas: VersionRange {
                minimum: self.snapshot_schema.minimum,
                maximum: self.snapshot_schema.maximum,
            },
            relocations: self.relocations.checked()?,
            recovery_points,
            recovery_locations,
        })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VersionRangeWire {
    minimum: u16,
    maximum: u16,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct RelocationsWire {
    instructions: BTreeMap<String, String>,
    instruction_locations: Vec<InstructionRelocationWire>,
    flows: BTreeMap<String, String>,
    globals: BTreeMap<String, String>,
    locals: BTreeMap<String, String>,
    local_locations: Vec<LocalRelocationWire>,
    choices: BTreeMap<String, String>,
    actions: BTreeMap<String, String>,
    states: BTreeMap<String, String>,
    histories: BTreeMap<String, String>,
    events: BTreeMap<String, String>,
    types: BTreeMap<String, String>,
    fields: BTreeMap<String, String>,
    variants: BTreeMap<String, String>,
    capabilities: BTreeMap<String, String>,
    dropped_globals: BTreeSet<String>,
    dropped_locals: BTreeSet<String>,
    dropped_local_locations: Vec<LocalLocationWire>,
}

impl RelocationsWire {
    fn checked(self) -> Result<RelocationTable, String> {
        Ok(RelocationTable {
            instructions: parse_map(self.instructions)?,
            instruction_locations: collect_unique_map(
                self.instruction_locations
                    .into_iter()
                    .map(InstructionRelocationWire::checked),
                "instruction location",
            )?,
            flows: parse_map(self.flows)?,
            globals: parse_map(self.globals)?,
            locals: parse_map(self.locals)?,
            local_locations: collect_unique_map(
                self.local_locations
                    .into_iter()
                    .map(LocalRelocationWire::checked),
                "local location",
            )?,
            choices: parse_map(self.choices)?,
            actions: parse_map(self.actions)?,
            states: parse_map(self.states)?,
            histories: parse_map(self.histories)?,
            events: parse_map(self.events)?,
            types: parse_map(self.types)?,
            fields: parse_map(self.fields)?,
            variants: parse_map(self.variants)?,
            capabilities: self
                .capabilities
                .into_iter()
                .map(|(from, to)| {
                    Ok((
                        CapabilityId::new(from).map_err(|error| error.to_string())?,
                        CapabilityId::new(to).map_err(|error| error.to_string())?,
                    ))
                })
                .collect::<Result<_, String>>()?,
            dropped_globals: self
                .dropped_globals
                .into_iter()
                .map(|value| parse_id(&value))
                .collect::<Result<_, _>>()?,
            dropped_locals: self
                .dropped_locals
                .into_iter()
                .map(|value| parse_id(&value))
                .collect::<Result<_, _>>()?,
            dropped_local_locations: self
                .dropped_local_locations
                .into_iter()
                .map(LocalLocationWire::checked)
                .collect::<Result<_, _>>()?,
        })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InstructionRelocationWire {
    source_flow: String,
    source_instruction: String,
    target_flow: String,
    target_instruction: String,
}

impl InstructionRelocationWire {
    fn checked(self) -> Result<(InstructionLocation, InstructionLocation), String> {
        Ok((
            InstructionLocation {
                flow: parse_id(&self.source_flow)?,
                instruction: parse_id(&self.source_instruction)?,
            },
            InstructionLocation {
                flow: parse_id(&self.target_flow)?,
                instruction: parse_id(&self.target_instruction)?,
            },
        ))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LocalLocationWire {
    flow: String,
    local: String,
}

impl LocalLocationWire {
    fn checked(self) -> Result<LocalLocation, String> {
        Ok(LocalLocation {
            flow: parse_id(&self.flow)?,
            local: parse_id(&self.local)?,
        })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LocalRelocationWire {
    source_flow: String,
    source_local: String,
    target_flow: String,
    target_local: String,
}

impl LocalRelocationWire {
    fn checked(self) -> Result<(LocalLocation, LocalLocation), String> {
        Ok((
            LocalLocation {
                flow: parse_id(&self.source_flow)?,
                local: parse_id(&self.source_local)?,
            },
            LocalLocation {
                flow: parse_id(&self.target_flow)?,
                local: parse_id(&self.target_local)?,
            },
        ))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RecoveryWire {
    #[serde(default)]
    source_flow: Option<String>,
    source_instruction: String,
    checkpoint: String,
    target_flow: String,
    target_instruction: String,
    #[serde(default)]
    preserve_globals: bool,
    #[serde(default)]
    crosses_external_effect_barrier: bool,
}

fn parse_map<K, V>(values: BTreeMap<String, String>) -> Result<BTreeMap<K, V>, String>
where
    K: FromStr + Ord,
    K::Err: Display,
    V: FromStr,
    V::Err: Display,
{
    values
        .into_iter()
        .map(|(from, to)| Ok((parse_id(&from)?, parse_id(&to)?)))
        .collect()
}

fn collect_unique_map<K: Ord, V>(
    values: impl IntoIterator<Item = Result<(K, V), String>>,
    label: &str,
) -> Result<BTreeMap<K, V>, String> {
    let mut result = BTreeMap::new();
    for value in values {
        let (from, to) = value?;
        if result.insert(from, to).is_some() {
            return Err(format!("duplicate {label} relocation"));
        }
    }
    Ok(result)
}

fn parse_id<T>(value: &str) -> Result<T, String>
where
    T: FromStr,
    T::Err: Display,
{
    T::from_str(value).map_err(|error| error.to_string())
}

fn _all_relocation_ids_are_parsed(
    _: (
        ActionId,
        ChoiceId,
        EventTypeId,
        FieldId,
        GlobalId,
        HistoryId,
        LocalId,
        StateId,
        TypeId,
        VariantId,
        ProgramArtifactId,
    ),
) {
}

//! Reader for schema v2 save databases, written by the adapter that loaded and rewrote the
//! whole database on every operation. Rows are decoded as strictly as that adapter decoded
//! them; revisions are dropped because the engine assigns new ones (ADR 0014).

use std::path::Path;

use narrata_core::{
    CapabilityId, CapabilityVersion, CommitId, CompoundSaveManifestId, DeliveryPolicy,
    DiagnosticId, EffectId, EffectRequestDigest, ExecutionId, InputId, InputPayloadDigest,
    ObjectId, RewindPolicy, TimelineArchiveManifestId, TimelineCatalogEventId, codec::ObjectKind,
};
use narrata_store::{
    CatalogRefKey, CheckedObject, CompoundSaveRefKey, EffectLedgerEntry, InputRecord, LeaseId,
    LedgerFence, LedgerStatus, Pin, RefKey, RefName, RefNamespace, StoreContents, StoreError,
    TimelineArchiveRefKey, TimelineCoverage,
};
use rusqlite::{Connection, Error as SqlError, ErrorCode, OpenFlags, OptionalExtension};

const SCHEMA_VERSION: i64 = 2;
type RawObjectRow = ([u8; 32], u16, u16, Vec<u8>, u64);

fn open_read_only(path: &Path) -> Result<Connection, StoreError> {
    Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(map_sql)
}

/// The schema version of a v2-era database, or `None` for anything else, including files that
/// do not exist or are not SQLite databases.
pub(crate) fn schema_version(path: &Path) -> Option<i64> {
    if !path.is_file() {
        return None;
    }
    let connection = open_read_only(path).ok()?;
    connection
        .query_row(
            "SELECT value FROM schema_meta WHERE key = 'schema_version'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .ok()
        .flatten()
}

/// Reads a schema v2 database without modifying it.
pub(crate) fn read(path: &Path) -> Result<StoreContents, StoreError> {
    let connection = open_read_only(path)?;
    let version: i64 = connection
        .query_row(
            "SELECT value FROM schema_meta WHERE key = 'schema_version'",
            [],
            |row| row.get(0),
        )
        .map_err(map_sql)?;
    if version != SCHEMA_VERSION {
        return Err(StoreError::CorruptStore(format!(
            "expected save schema v{SCHEMA_VERSION}, found v{version}"
        )));
    }
    read_contents(&connection)
}

fn positive(value: i64, name: &'static str) -> Result<(), StoreError> {
    if value > 0 {
        Ok(())
    } else {
        Err(StoreError::CorruptStore(format!("non-positive {name}")))
    }
}

pub(crate) fn read_contents(connection: &Connection) -> Result<StoreContents, StoreError> {
    let mut state = StoreContents::default();
    for (id, kind, schema, bytes, inserted_at) in raw_objects(connection)? {
        let kind = ObjectKind::from_code(kind)
            .ok_or_else(|| StoreError::CorruptStore("unknown ObjectKind".to_owned()))?;
        let object = CheckedObject::from_bytes(&bytes, kind, schema, bytes.len() as u64)
            .map_err(|error| StoreError::Corrupt(ObjectId::from_bytes(id), error.to_string()))?;
        if object.id().as_bytes() != &id {
            return Err(StoreError::Corrupt(
                ObjectId::from_bytes(id),
                "ObjectId mismatch".to_owned(),
            ));
        }
        state.objects.push((object, inserted_at));
    }

    let mut statement = connection
        .prepare(
            "SELECT namespace, owner, name, revision, commit_id
             FROM refs ORDER BY namespace, owner, name",
        )
        .map_err(map_sql)?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, Vec<u8>>(4)?,
            ))
        })
        .map_err(map_sql)?;
    for row in rows {
        let (namespace, owner, name, revision, commit) = row.map_err(map_sql)?;
        positive(revision, "Ref revision")?;
        let namespace = u8::try_from(namespace)
            .ok()
            .and_then(RefNamespace::from_code)
            .ok_or_else(|| StoreError::CorruptStore("Ref namespace".to_owned()))?;
        state.refs.push((
            RefKey::new(
                namespace,
                RefName::new(owner)
                    .map_err(|_| StoreError::CorruptStore("Ref owner".to_owned()))?,
                RefName::new(name).map_err(|_| StoreError::CorruptStore("Ref name".to_owned()))?,
            ),
            CommitId::from_bytes(blob::<32>(commit, "Ref Commit")?),
        ));
    }

    let mut statement = connection
        .prepare(
            "SELECT timeline, revision, event_id, coverage_kind, baseline, source_manifest
             FROM catalog_heads ORDER BY timeline",
        )
        .map_err(map_sql)?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, Vec<u8>>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, Vec<u8>>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, Vec<u8>>(4)?,
                row.get::<_, Option<Vec<u8>>>(5)?,
            ))
        })
        .map_err(map_sql)?;
    for row in rows {
        let (timeline, revision, event, kind, baseline, source) = row.map_err(map_sql)?;
        positive(revision, "Catalog Head revision")?;
        let timeline = ExecutionId::from_bytes(blob::<16>(timeline, "Catalog timeline")?);
        let baseline = CommitId::from_bytes(blob::<32>(baseline, "Catalog baseline")?);
        let coverage = match (kind, source) {
            (0, None) => TimelineCoverage::FromBaseline { baseline },
            (1, Some(source)) => TimelineCoverage::Imported {
                baseline,
                source: TimelineArchiveManifestId::from_bytes(blob::<32>(
                    source,
                    "Catalog source",
                )?),
            },
            _ => return Err(StoreError::CorruptStore("Catalog coverage".to_owned())),
        };
        state.catalogs.push((
            CatalogRefKey::new(timeline),
            TimelineCatalogEventId::from_bytes(blob::<32>(event, "Catalog event")?),
            coverage,
        ));
    }

    let mut statement = connection
        .prepare(
            "SELECT timeline, name, revision, manifest_id
             FROM archive_refs ORDER BY timeline, name",
        )
        .map_err(map_sql)?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, Vec<u8>>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, Vec<u8>>(3)?,
            ))
        })
        .map_err(map_sql)?;
    for row in rows {
        let (timeline, name, revision, manifest) = row.map_err(map_sql)?;
        positive(revision, "archive revision")?;
        state.archives.push((
            TimelineArchiveRefKey::new(
                ExecutionId::from_bytes(blob::<16>(timeline, "Archive timeline")?),
                RefName::new(name)
                    .map_err(|_| StoreError::CorruptStore("Archive name".to_owned()))?,
            ),
            TimelineArchiveManifestId::from_bytes(blob::<32>(manifest, "Archive manifest")?),
        ));
    }

    let mut statement = connection
        .prepare(
            "SELECT owner, slot, revision, manifest_id
             FROM compound_save_refs ORDER BY owner, slot",
        )
        .map_err(map_sql)?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, Vec<u8>>(3)?,
            ))
        })
        .map_err(map_sql)?;
    for row in rows {
        let (owner, slot, revision, manifest) = row.map_err(map_sql)?;
        positive(revision, "Compound Save revision")?;
        state.compound_saves.push((
            CompoundSaveRefKey::new(
                RefName::new(owner)
                    .map_err(|_| StoreError::CorruptStore("Compound Save owner".to_owned()))?,
                RefName::new(slot)
                    .map_err(|_| StoreError::CorruptStore("Compound Save slot".to_owned()))?,
            ),
            CompoundSaveManifestId::from_bytes(blob::<32>(manifest, "Compound Save manifest")?),
        ));
    }

    let mut statement = connection
        .prepare(
            "SELECT execution, input_id, parent, payload_digest, commit_id
             FROM input_index ORDER BY execution, input_id",
        )
        .map_err(map_sql)?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, Vec<u8>>(0)?,
                row.get::<_, Vec<u8>>(1)?,
                row.get::<_, Vec<u8>>(2)?,
                row.get::<_, Vec<u8>>(3)?,
                row.get::<_, Vec<u8>>(4)?,
            ))
        })
        .map_err(map_sql)?;
    for row in rows {
        let (execution, input, parent, payload, commit) = row.map_err(map_sql)?;
        state.inputs.push(InputRecord {
            execution: ExecutionId::from_bytes(blob::<16>(execution, "Input execution")?),
            input: InputId::from_bytes(blob::<16>(input, "Input ID")?),
            parent: CommitId::from_bytes(blob::<32>(parent, "Input parent")?),
            payload: InputPayloadDigest::from_bytes(blob::<32>(payload, "Input payload")?),
            commit: CommitId::from_bytes(blob::<32>(commit, "Input Commit")?),
        });
    }

    let mut statement = connection
        .prepare("SELECT owner, object_id, expires_at FROM pins ORDER BY owner, object_id")
        .map_err(map_sql)?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Vec<u8>>(1)?,
                row.get::<_, Option<i64>>(2)?,
            ))
        })
        .map_err(map_sql)?;
    for row in rows {
        let (owner, object, expires) = row.map_err(map_sql)?;
        state.pins.push(Pin {
            owner,
            object: ObjectId::from_bytes(blob::<32>(object, "Pin object")?),
            expires_at: expires.map(u64_from_i64).transpose()?,
        });
    }

    let mut statement = connection
        .prepare("SELECT execution, fence FROM ledger_fences ORDER BY execution")
        .map_err(map_sql)?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, i64>(1)?))
        })
        .map_err(map_sql)?;
    for row in rows {
        let (execution, fence) = row.map_err(map_sql)?;
        state.ledger_fences.push((
            ExecutionId::from_bytes(blob::<16>(execution, "ledger fence execution")?),
            LedgerFence::from_u64(u64_from_i64(fence)?),
        ));
    }

    let mut statement = connection
        .prepare(
            "SELECT execution, effect_id, request_digest, capability, capability_version,
                    origin_commit, delivery, rewind, compensation_capability, status,
                    lease_id, expires_at, response_id, diagnostic_id, fence, by_effect,
                    original_fence, compensation_fence
             FROM effect_ledger ORDER BY execution, effect_id",
        )
        .map_err(map_sql)?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, Vec<u8>>(0)?,
                row.get::<_, Vec<u8>>(1)?,
                row.get::<_, Vec<u8>>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, Vec<u8>>(5)?,
                row.get::<_, i64>(6)?,
                row.get::<_, i64>(7)?,
                row.get::<_, Option<String>>(8)?,
                row.get::<_, i64>(9)?,
                row.get::<_, Option<Vec<u8>>>(10)?,
                row.get::<_, Option<i64>>(11)?,
                row.get::<_, Option<Vec<u8>>>(12)?,
                row.get::<_, Option<Vec<u8>>>(13)?,
                row.get::<_, Option<i64>>(14)?,
                row.get::<_, Option<Vec<u8>>>(15)?,
                row.get::<_, Option<i64>>(16)?,
                row.get::<_, Option<i64>>(17)?,
            ))
        })
        .map_err(map_sql)?;
    for row in rows {
        let (
            execution,
            effect,
            request_digest,
            capability,
            capability_version,
            origin_commit,
            delivery,
            rewind,
            compensation_capability,
            status,
            lease,
            expires_at,
            response,
            diagnostic,
            fence,
            by_effect,
            original_fence,
            compensation_fence,
        ) = row.map_err(map_sql)?;
        let execution = ExecutionId::from_bytes(blob::<16>(execution, "ledger execution")?);
        let capability = CapabilityId::new(capability)
            .map_err(|_| StoreError::CorruptStore("ledger capability".to_owned()))?;
        let capability_version = CapabilityVersion::new(
            u16::try_from(capability_version)
                .map_err(|_| StoreError::CorruptStore("ledger capability version".to_owned()))?,
        )
        .ok_or_else(|| StoreError::CorruptStore("zero capability version".to_owned()))?;
        let delivery = decode_delivery(delivery)?;
        let rewind = decode_rewind(rewind, compensation_capability)?;
        let ledger_status = decode_ledger_status(
            status,
            lease,
            expires_at,
            response,
            diagnostic,
            fence,
            by_effect,
            original_fence,
            compensation_fence,
        )?;
        state.effects.push(EffectLedgerEntry {
            execution,
            effect: EffectId::from_bytes(blob::<32>(effect, "ledger Effect ID")?),
            request_digest: EffectRequestDigest::from_bytes(blob::<32>(
                request_digest,
                "ledger request digest",
            )?),
            capability,
            capability_version,
            origin_commit: CommitId::from_bytes(blob::<32>(origin_commit, "ledger origin Commit")?),
            delivery,
            rewind,
            status: ledger_status,
        });
    }
    Ok(state)
}

fn raw_objects(connection: &Connection) -> Result<Vec<RawObjectRow>, StoreError> {
    let mut statement = connection
        .prepare("SELECT id, kind, schema, payload, inserted_at FROM objects ORDER BY id")
        .map_err(map_sql)?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, Vec<u8>>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, Vec<u8>>(3)?,
                row.get::<_, i64>(4)?,
            ))
        })
        .map_err(map_sql)?;
    let mut values = Vec::new();
    for row in rows {
        let (id, kind, schema, bytes, inserted_at) = row.map_err(map_sql)?;
        values.push((
            blob::<32>(id, "Object ID")?,
            u16::try_from(kind).map_err(|_| StoreError::CorruptStore("Object kind".to_owned()))?,
            u16::try_from(schema)
                .map_err(|_| StoreError::CorruptStore("Object schema".to_owned()))?,
            bytes,
            u64_from_i64(inserted_at)?,
        ));
    }
    Ok(values)
}

fn u64_from_i64(value: i64) -> Result<u64, StoreError> {
    u64::try_from(value).map_err(|_| StoreError::CorruptStore("negative integer".to_owned()))
}

fn blob<const N: usize>(value: Vec<u8>, name: &'static str) -> Result<[u8; N], StoreError> {
    value
        .try_into()
        .map_err(|_| StoreError::CorruptStore(format!("{name} length")))
}

fn decode_delivery(value: i64) -> Result<DeliveryPolicy, StoreError> {
    match value {
        0 => Ok(DeliveryPolicy::Reconcile),
        1 => Ok(DeliveryPolicy::RecordedQuery),
        2 => Ok(DeliveryPolicy::AtLeastOnceIdempotent),
        3 => Ok(DeliveryPolicy::HostTransactional),
        _ => Err(StoreError::CorruptStore(
            "ledger delivery policy".to_owned(),
        )),
    }
}

fn decode_rewind(value: i64, capability: Option<String>) -> Result<RewindPolicy, StoreError> {
    match (value, capability) {
        (0, None) => Ok(RewindPolicy::Reapply),
        (1, None) => Ok(RewindPolicy::ReuseRecordedResponse),
        (2, None) => Ok(RewindPolicy::Barrier),
        (3, Some(capability)) => Ok(RewindPolicy::Compensatable {
            capability: CapabilityId::new(capability)
                .map_err(|_| StoreError::CorruptStore("compensation capability".to_owned()))?,
        }),
        _ => Err(StoreError::CorruptStore("ledger rewind policy".to_owned())),
    }
}

#[allow(clippy::too_many_arguments)]
fn decode_ledger_status(
    status: i64,
    lease: Option<Vec<u8>>,
    expires_at: Option<i64>,
    response: Option<Vec<u8>>,
    diagnostic: Option<Vec<u8>>,
    fence: Option<i64>,
    by_effect: Option<Vec<u8>>,
    original_fence: Option<i64>,
    compensation_fence: Option<i64>,
) -> Result<LedgerStatus, StoreError> {
    match status {
        0 if response.is_none()
            && diagnostic.is_none()
            && fence.is_none()
            && by_effect.is_none()
            && original_fence.is_none()
            && compensation_fence.is_none() =>
        {
            Ok(LedgerStatus::Claimed {
                lease: LeaseId::from_bytes(blob::<16>(
                    lease.ok_or_else(|| StoreError::CorruptStore("missing lease".to_owned()))?,
                    "lease ID",
                )?),
                expires_at: u64_from_i64(
                    expires_at.ok_or_else(|| {
                        StoreError::CorruptStore("missing lease expiry".to_owned())
                    })?,
                )?,
            })
        }
        1 if lease.is_none()
            && expires_at.is_none()
            && diagnostic.is_none()
            && by_effect.is_none()
            && original_fence.is_none()
            && compensation_fence.is_none() =>
        {
            Ok(LedgerStatus::Completed {
                response: ObjectId::from_bytes(blob::<32>(
                    response.ok_or_else(|| {
                        StoreError::CorruptStore("missing completed response".to_owned())
                    })?,
                    "completed response",
                )?),
                fence: decode_fence(fence)?,
            })
        }
        2 if lease.is_none()
            && expires_at.is_none()
            && diagnostic.is_none()
            && by_effect.is_none()
            && original_fence.is_none()
            && compensation_fence.is_none() =>
        {
            Ok(LedgerStatus::Rejected {
                response: ObjectId::from_bytes(blob::<32>(
                    response.ok_or_else(|| {
                        StoreError::CorruptStore("missing rejected response".to_owned())
                    })?,
                    "rejected response",
                )?),
                fence: decode_fence(fence)?,
            })
        }
        3 if lease.is_none()
            && expires_at.is_none()
            && response.is_none()
            && fence.is_none()
            && by_effect.is_none()
            && original_fence.is_none()
            && compensation_fence.is_none() =>
        {
            Ok(LedgerStatus::RetryableFailure {
                diagnostic: DiagnosticId::from_bytes(blob::<32>(
                    diagnostic.ok_or_else(|| {
                        StoreError::CorruptStore("missing retry diagnostic".to_owned())
                    })?,
                    "retry diagnostic",
                )?),
            })
        }
        4 if lease.is_none()
            && expires_at.is_none()
            && response.is_none()
            && by_effect.is_none()
            && original_fence.is_none()
            && compensation_fence.is_none() =>
        {
            Ok(LedgerStatus::UnknownOutcome {
                diagnostic: DiagnosticId::from_bytes(blob::<32>(
                    diagnostic.ok_or_else(|| {
                        StoreError::CorruptStore("missing unknown diagnostic".to_owned())
                    })?,
                    "unknown diagnostic",
                )?),
                fence: decode_fence(fence)?,
            })
        }
        5 if lease.is_none() && expires_at.is_none() && diagnostic.is_none() && fence.is_none() => {
            Ok(LedgerStatus::Compensated {
                response: ObjectId::from_bytes(blob::<32>(
                    response.ok_or_else(|| {
                        StoreError::CorruptStore("missing compensated response".to_owned())
                    })?,
                    "compensated response",
                )?),
                original_fence: decode_fence(original_fence)?,
                by_effect: EffectId::from_bytes(blob::<32>(
                    by_effect.ok_or_else(|| {
                        StoreError::CorruptStore("missing compensating Effect".to_owned())
                    })?,
                    "compensating Effect",
                )?),
                compensation_fence: decode_fence(compensation_fence)?,
            })
        }
        _ => Err(StoreError::CorruptStore("ledger status shape".to_owned())),
    }
}

fn decode_fence(value: Option<i64>) -> Result<LedgerFence, StoreError> {
    let value = value.ok_or_else(|| StoreError::CorruptStore("missing fence".to_owned()))?;
    let value = u64_from_i64(value)?;
    if value == 0 {
        Err(StoreError::CorruptStore("zero terminal fence".to_owned()))
    } else {
        Ok(LedgerFence::from_u64(value))
    }
}

fn map_sql(error: SqlError) -> StoreError {
    match &error {
        SqlError::SqliteFailure(code, message) => match code.code {
            ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked => StoreError::Busy,
            ErrorCode::DiskFull => StoreError::Full,
            ErrorCode::DatabaseCorrupt | ErrorCode::NotADatabase => {
                StoreError::CorruptStore(message.clone().unwrap_or_else(|| error.to_string()))
            }
            ErrorCode::SystemIoFailure => StoreError::Io(error.to_string()),
            _ => StoreError::Io(error.to_string()),
        },
        _ => StoreError::Io(error.to_string()),
    }
}

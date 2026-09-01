//! Crash-safe SQLite SaveStore adapter.
//!
//! Every mutation starts an `IMMEDIATE` SQLite transaction, reconstructs the checked in-memory
//! reference model from that transaction's consistent view, applies the same typed transaction,
//! and persists the resulting state before commit. `WAL` plus `synchronous=FULL` is requested for
//! file-backed databases; callers should still verify the guarantees of their filesystem.

#![forbid(unsafe_code)]

use std::{collections::BTreeMap, path::Path, time::Duration};

use narrata_core::{
    CapabilityId, CapabilityVersion, CommitId, DeliveryPolicy, DiagnosticId, EffectId,
    EffectRequestDigest, ExecutionId, InputId, InputPayloadDigest, ObjectId, RewindPolicy,
    TimelineArchiveManifestId, TimelineCatalogEventId, codec::ObjectKind,
};
use narrata_store::{
    ArchiveMutation, CatalogHeadRefValue, CatalogMutation, CatalogRefKey, CheckedObject,
    CommitCauseV1, CommitOutcome, CommitTransaction, CommitV1, CompoundSaveManifestV1,
    CompoundSaveRefKey, CompoundSaveRefValue, EffectClaim, EffectClaimResult, EffectLedgerEntry,
    EffectOutcomeRecord, FaultPoint, GcReport, HostTimelineManifestV1, InputRecord, IntegrityIssue,
    LeaseId, LedgerFence, LedgerStatus, MemoryStore, MemoryStoreState, Pin, RefKey, RefName,
    RefNamespace, RefRevision, RefValue, RetentionPolicy, SaveStore, StoreError,
    TimelineArchiveManifestV1, TimelineArchiveRefKey, TimelineArchiveRefValue,
    TimelineCatalogEventV1, TimelineCoverage,
};
use rusqlite::{
    Connection, Error as SqlError, ErrorCode, Transaction, TransactionBehavior, params,
};

const SCHEMA_VERSION: i64 = 2;
type RawObjectRow = ([u8; 32], u16, u16, Vec<u8>, u64);

pub struct SqliteStore {
    connection: Connection,
    fault: Option<FaultPoint>,
}

impl SqliteStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let connection = Connection::open(path).map_err(map_sql)?;
        Self::initialize(connection)
    }

    pub fn open_in_memory() -> Result<Self, StoreError> {
        let connection = Connection::open_in_memory().map_err(map_sql)?;
        Self::initialize(connection)
    }

    fn initialize(connection: Connection) -> Result<Self, StoreError> {
        connection
            .busy_timeout(Duration::from_secs(5))
            .map_err(map_sql)?;
        connection
            .execute_batch(
                "PRAGMA journal_mode=WAL;
                 PRAGMA synchronous=FULL;
                 PRAGMA foreign_keys=ON;
                 CREATE TABLE IF NOT EXISTS schema_meta (
                   key TEXT PRIMARY KEY,
                   value INTEGER NOT NULL
                 ) STRICT;
                 CREATE TABLE IF NOT EXISTS objects (
                   id BLOB PRIMARY KEY CHECK(length(id) = 32),
                   kind INTEGER NOT NULL,
                   schema INTEGER NOT NULL,
                   payload BLOB NOT NULL,
                   inserted_at INTEGER NOT NULL
                 ) STRICT;
                 CREATE TABLE IF NOT EXISTS object_edges (
                   source BLOB NOT NULL CHECK(length(source) = 32),
                   target BLOB NOT NULL CHECK(length(target) = 32),
                   edge_kind INTEGER NOT NULL,
                   PRIMARY KEY(source, target, edge_kind),
                   FOREIGN KEY(source) REFERENCES objects(id) ON DELETE CASCADE
                 ) STRICT;
                 CREATE TABLE IF NOT EXISTS refs (
                   namespace INTEGER NOT NULL,
                   owner TEXT NOT NULL,
                   name TEXT NOT NULL,
                   revision INTEGER NOT NULL CHECK(revision > 0),
                   commit_id BLOB NOT NULL CHECK(length(commit_id) = 32),
                   metadata BLOB,
                   PRIMARY KEY(namespace, owner, name)
                 ) STRICT;
                 CREATE TABLE IF NOT EXISTS catalog_heads (
                   timeline BLOB PRIMARY KEY CHECK(length(timeline) = 16),
                   revision INTEGER NOT NULL CHECK(revision > 0),
                   event_id BLOB NOT NULL CHECK(length(event_id) = 32),
                   coverage_kind INTEGER NOT NULL,
                   baseline BLOB NOT NULL CHECK(length(baseline) = 32),
                   source_manifest BLOB CHECK(source_manifest IS NULL OR length(source_manifest) = 32),
                   metadata BLOB
                 ) STRICT;
                 CREATE TABLE IF NOT EXISTS archive_refs (
                   timeline BLOB NOT NULL CHECK(length(timeline) = 16),
                   name TEXT NOT NULL,
                   revision INTEGER NOT NULL CHECK(revision > 0),
                   manifest_id BLOB NOT NULL CHECK(length(manifest_id) = 32),
                   metadata BLOB,
                   PRIMARY KEY(timeline, name)
                 ) STRICT;
                 CREATE TABLE IF NOT EXISTS pins (
                   owner TEXT NOT NULL,
                   object_id BLOB NOT NULL CHECK(length(object_id) = 32),
                   expires_at INTEGER,
                   PRIMARY KEY(owner, object_id)
                 ) STRICT;
                 CREATE TABLE IF NOT EXISTS input_index (
                   execution BLOB NOT NULL CHECK(length(execution) = 16),
                   input_id BLOB NOT NULL CHECK(length(input_id) = 16),
                   parent BLOB NOT NULL CHECK(length(parent) = 32),
                   payload_digest BLOB NOT NULL CHECK(length(payload_digest) = 32),
                   commit_id BLOB NOT NULL CHECK(length(commit_id) = 32),
                   PRIMARY KEY(execution, input_id)
                 ) STRICT;
                 CREATE TABLE IF NOT EXISTS ledger_fences (
                   execution BLOB PRIMARY KEY CHECK(length(execution) = 16),
                   fence INTEGER NOT NULL CHECK(fence >= 0)
                 ) STRICT;
                 CREATE TABLE IF NOT EXISTS effect_ledger (
                   execution BLOB NOT NULL CHECK(length(execution) = 16),
                   effect_id BLOB NOT NULL CHECK(length(effect_id) = 32),
                   request_digest BLOB NOT NULL CHECK(length(request_digest) = 32),
                   capability TEXT NOT NULL,
                   capability_version INTEGER NOT NULL CHECK(capability_version > 0),
                   origin_commit BLOB NOT NULL CHECK(length(origin_commit) = 32),
                   delivery INTEGER NOT NULL,
                   rewind INTEGER NOT NULL,
                   compensation_capability TEXT,
                   status INTEGER NOT NULL,
                   lease_id BLOB CHECK(lease_id IS NULL OR length(lease_id) = 16),
                   expires_at INTEGER,
                   response_id BLOB CHECK(response_id IS NULL OR length(response_id) = 32),
                   diagnostic_id BLOB CHECK(diagnostic_id IS NULL OR length(diagnostic_id) = 32),
                   fence INTEGER,
                   by_effect BLOB CHECK(by_effect IS NULL OR length(by_effect) = 32),
                   original_fence INTEGER,
                   compensation_fence INTEGER,
                   PRIMARY KEY(execution, effect_id)
                 ) STRICT;
                 CREATE TABLE IF NOT EXISTS compound_save_refs (
                   owner TEXT NOT NULL,
                   slot TEXT NOT NULL,
                   revision INTEGER NOT NULL CHECK(revision > 0),
                   manifest_id BLOB NOT NULL CHECK(length(manifest_id) = 32),
                   PRIMARY KEY(owner, slot)
                 ) STRICT;
                 INSERT INTO schema_meta(key, value) VALUES('schema_version', 1)
                 ON CONFLICT(key) DO NOTHING;
                 UPDATE schema_meta SET value=2 WHERE key='schema_version' AND value=1;",
            )
            .map_err(map_sql)?;
        let version: i64 = connection
            .query_row(
                "SELECT value FROM schema_meta WHERE key='schema_version'",
                [],
                |row| row.get(0),
            )
            .map_err(map_sql)?;
        if version != SCHEMA_VERSION {
            return Err(StoreError::CorruptStore(
                "unsupported schema version".to_owned(),
            ));
        }
        Ok(Self {
            connection,
            fault: None,
        })
    }

    pub fn inject_fault(&mut self, point: Option<FaultPoint>) {
        self.fault = point;
    }

    pub fn connection(&self) -> &Connection {
        &self.connection
    }

    pub fn journal_mode(&self) -> Result<String, StoreError> {
        self.connection
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .map_err(map_sql)
    }

    pub fn synchronous(&self) -> Result<i64, StoreError> {
        self.connection
            .query_row("PRAGMA synchronous", [], |row| row.get(0))
            .map_err(map_sql)
    }

    fn memory(&self) -> Result<MemoryStore, StoreError> {
        load_memory(&self.connection)
    }
}

impl SaveStore for SqliteStore {
    fn get_object(&self, id: ObjectId) -> Result<Option<CheckedObject>, StoreError> {
        self.memory()?.get_object(id)
    }

    fn list_objects(&self) -> Result<Vec<CheckedObject>, StoreError> {
        self.memory()?.list_objects()
    }

    fn read_ref(&self, key: &RefKey) -> Result<Option<RefValue>, StoreError> {
        self.memory()?.read_ref(key)
    }

    fn list_refs(&self) -> Result<Vec<(RefKey, RefValue)>, StoreError> {
        self.memory()?.list_refs()
    }

    fn read_catalog_head(
        &self,
        key: &CatalogRefKey,
    ) -> Result<Option<CatalogHeadRefValue>, StoreError> {
        self.memory()?.read_catalog_head(key)
    }

    fn list_catalog_heads(&self) -> Result<Vec<(CatalogRefKey, CatalogHeadRefValue)>, StoreError> {
        self.memory()?.list_catalog_heads()
    }

    fn read_timeline_archive(
        &self,
        key: &TimelineArchiveRefKey,
    ) -> Result<Option<TimelineArchiveRefValue>, StoreError> {
        self.memory()?.read_timeline_archive(key)
    }

    fn list_timeline_archives(
        &self,
    ) -> Result<Vec<(TimelineArchiveRefKey, TimelineArchiveRefValue)>, StoreError> {
        self.memory()?.list_timeline_archives()
    }

    fn read_compound_save(
        &self,
        key: &CompoundSaveRefKey,
    ) -> Result<Option<CompoundSaveRefValue>, StoreError> {
        self.memory()?.read_compound_save(key)
    }

    fn list_compound_saves(
        &self,
    ) -> Result<Vec<(CompoundSaveRefKey, CompoundSaveRefValue)>, StoreError> {
        self.memory()?.list_compound_saves()
    }

    fn read_input(
        &self,
        execution: ExecutionId,
        input: InputId,
    ) -> Result<Option<InputRecord>, StoreError> {
        self.memory()?.read_input(execution, input)
    }

    fn list_pins(&self) -> Result<Vec<Pin>, StoreError> {
        self.memory()?.list_pins()
    }

    fn read_effect(
        &self,
        execution: ExecutionId,
        effect: EffectId,
    ) -> Result<Option<EffectLedgerEntry>, StoreError> {
        self.memory()?.read_effect(execution, effect)
    }

    fn list_effects(&self, execution: ExecutionId) -> Result<Vec<EffectLedgerEntry>, StoreError> {
        self.memory()?.list_effects(execution)
    }

    fn current_ledger_fence(&self, execution: ExecutionId) -> Result<LedgerFence, StoreError> {
        self.memory()?.current_ledger_fence(execution)
    }

    fn claim_effect(&mut self, claim: EffectClaim) -> Result<EffectClaimResult, StoreError> {
        let sql_tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_sql)?;
        let mut memory = load_memory(&sql_tx)?;
        memory.inject_fault(self.fault);
        let result = memory.claim_effect(claim)?;
        write_state(&sql_tx, &memory.export_state())?;
        sql_tx.commit().map_err(map_sql)?;
        Ok(result)
    }

    fn renew_effect_lease(
        &mut self,
        execution: ExecutionId,
        effect: EffectId,
        lease: LeaseId,
        now: u64,
        expires_at: u64,
    ) -> Result<EffectLedgerEntry, StoreError> {
        let sql_tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_sql)?;
        let mut memory = load_memory(&sql_tx)?;
        memory.inject_fault(self.fault);
        let result = memory.renew_effect_lease(execution, effect, lease, now, expires_at)?;
        write_state(&sql_tx, &memory.export_state())?;
        sql_tx.commit().map_err(map_sql)?;
        Ok(result)
    }

    fn record_effect_outcome(
        &mut self,
        record: EffectOutcomeRecord,
    ) -> Result<EffectLedgerEntry, StoreError> {
        let sql_tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_sql)?;
        let mut memory = load_memory(&sql_tx)?;
        memory.inject_fault(self.fault);
        let result = memory.record_effect_outcome(record)?;
        write_state(&sql_tx, &memory.export_state())?;
        sql_tx.commit().map_err(map_sql)?;
        Ok(result)
    }

    fn mark_effect_compensated(
        &mut self,
        execution: ExecutionId,
        original: EffectId,
        by_effect: EffectId,
    ) -> Result<EffectLedgerEntry, StoreError> {
        let sql_tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_sql)?;
        let mut memory = load_memory(&sql_tx)?;
        memory.inject_fault(self.fault);
        let result = memory.mark_effect_compensated(execution, original, by_effect)?;
        write_state(&sql_tx, &memory.export_state())?;
        sql_tx.commit().map_err(map_sql)?;
        Ok(result)
    }

    fn commit(&mut self, transaction: CommitTransaction) -> Result<CommitOutcome, StoreError> {
        let sql_tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_sql)?;
        let mut memory = load_memory(&sql_tx)?;
        memory.inject_fault(self.fault);
        let outcome = memory.commit(transaction)?;
        write_state(&sql_tx, &memory.export_state())?;
        sql_tx.commit().map_err(map_sql)?;
        Ok(outcome)
    }

    fn collect(&mut self, policy: RetentionPolicy) -> Result<GcReport, StoreError> {
        let sql_tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_sql)?;
        let mut memory = load_memory(&sql_tx)?;
        memory.inject_fault(self.fault);
        let report = memory.collect(policy)?;
        if !policy.dry_run {
            write_state(&sql_tx, &memory.export_state())?;
        }
        sql_tx.commit().map_err(map_sql)?;
        Ok(report)
    }

    fn integrity_scan(&self) -> Result<Vec<IntegrityIssue>, StoreError> {
        let database_check: String = self
            .connection
            .query_row("PRAGMA integrity_check", [], |row| row.get(0))
            .map_err(map_sql)?;
        if database_check != "ok" {
            return Err(StoreError::CorruptStore(database_check));
        }
        let rows = raw_objects(&self.connection)?;
        let mut issues = Vec::new();
        for (id_bytes, kind_code, schema, bytes, _) in rows {
            let id = ObjectId::from_bytes(id_bytes);
            let Some(kind) = ObjectKind::from_code(kind_code) else {
                issues.push(IntegrityIssue {
                    object: id,
                    diagnostic: "unknown ObjectKind".to_owned(),
                });
                continue;
            };
            match CheckedObject::from_bytes(&bytes, kind, schema, bytes.len() as u64) {
                Ok(object) if object.id() == id => {}
                Ok(_) => issues.push(IntegrityIssue {
                    object: id,
                    diagnostic: "ObjectId mismatch".to_owned(),
                }),
                Err(error) => issues.push(IntegrityIssue {
                    object: id,
                    diagnostic: error.to_string(),
                }),
            }
        }
        Ok(issues)
    }
}

fn load_memory(connection: &Connection) -> Result<MemoryStore, StoreError> {
    let mut state = MemoryStoreState::default();
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
            RefValue {
                revision: revision_from_i64(revision)?,
                commit: CommitId::from_bytes(blob::<32>(commit, "Ref Commit")?),
            },
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
            CatalogHeadRefValue {
                revision: revision_from_i64(revision)?,
                event: TimelineCatalogEventId::from_bytes(blob::<32>(event, "Catalog event")?),
                coverage,
            },
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
        state.archives.push((
            TimelineArchiveRefKey::new(
                ExecutionId::from_bytes(blob::<16>(timeline, "Archive timeline")?),
                RefName::new(name)
                    .map_err(|_| StoreError::CorruptStore("Archive name".to_owned()))?,
            ),
            TimelineArchiveRefValue {
                revision: revision_from_i64(revision)?,
                manifest: TimelineArchiveManifestId::from_bytes(blob::<32>(
                    manifest,
                    "Archive manifest",
                )?),
            },
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
        state.compound_saves.push((
            CompoundSaveRefKey::new(
                RefName::new(owner)
                    .map_err(|_| StoreError::CorruptStore("Compound Save owner".to_owned()))?,
                RefName::new(slot)
                    .map_err(|_| StoreError::CorruptStore("Compound Save slot".to_owned()))?,
            ),
            CompoundSaveRefValue {
                revision: revision_from_i64(revision)?,
                manifest: narrata_core::CompoundSaveManifestId::from_bytes(blob::<32>(
                    manifest,
                    "Compound Save manifest",
                )?),
            },
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
    MemoryStore::from_state(state)
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

fn write_state(transaction: &Transaction<'_>, state: &MemoryStoreState) -> Result<(), StoreError> {
    transaction
        .execute_batch(
            "DELETE FROM object_edges;
             DELETE FROM effect_ledger;
             DELETE FROM ledger_fences;
             DELETE FROM compound_save_refs;
             DELETE FROM refs;
             DELETE FROM catalog_heads;
             DELETE FROM archive_refs;
             DELETE FROM pins;
             DELETE FROM input_index;
             DELETE FROM objects;",
        )
        .map_err(map_sql)?;
    for (object, inserted_at) in &state.objects {
        transaction
            .execute(
                "INSERT INTO objects(id, kind, schema, payload, inserted_at)
                 VALUES(?1, ?2, ?3, ?4, ?5)",
                params![
                    object.id().as_bytes().as_slice(),
                    i64::from(object.kind().code()),
                    i64::from(object.schema()),
                    object.bytes(),
                    to_i64(*inserted_at, "object inserted_at")?,
                ],
            )
            .map_err(map_sql)?;
    }
    for (source, target, kind) in edge_rows(state)? {
        transaction
            .execute(
                "INSERT INTO object_edges(source, target, edge_kind) VALUES(?1, ?2, ?3)",
                params![
                    source.as_bytes().as_slice(),
                    target.as_bytes().as_slice(),
                    kind
                ],
            )
            .map_err(map_sql)?;
    }
    for (key, value) in &state.refs {
        transaction
            .execute(
                "INSERT INTO refs(namespace, owner, name, revision, commit_id, metadata)
                 VALUES(?1, ?2, ?3, ?4, ?5, NULL)",
                params![
                    i64::from(key.namespace() as u8),
                    key.owner().as_str(),
                    key.name().as_str(),
                    to_i64(value.revision.get(), "Ref revision")?,
                    value.commit.as_bytes().as_slice(),
                ],
            )
            .map_err(map_sql)?;
    }
    for (key, value) in &state.catalogs {
        let (coverage_kind, baseline, source) = match value.coverage {
            TimelineCoverage::FromBaseline { baseline } => (0_i64, baseline, None),
            TimelineCoverage::Imported { baseline, source } => (1, baseline, Some(source)),
        };
        transaction
            .execute(
                "INSERT INTO catalog_heads(
                   timeline, revision, event_id, coverage_kind, baseline, source_manifest, metadata
                 ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, NULL)",
                params![
                    key.timeline().as_bytes().as_slice(),
                    to_i64(value.revision.get(), "Catalog revision")?,
                    value.event.as_bytes().as_slice(),
                    coverage_kind,
                    baseline.as_bytes().as_slice(),
                    source.map(|id| id.as_bytes().to_vec()),
                ],
            )
            .map_err(map_sql)?;
    }
    for (key, value) in &state.archives {
        transaction
            .execute(
                "INSERT INTO archive_refs(timeline, name, revision, manifest_id, metadata)
                 VALUES(?1, ?2, ?3, ?4, NULL)",
                params![
                    key.timeline().as_bytes().as_slice(),
                    key.name().as_str(),
                    to_i64(value.revision.get(), "Archive revision")?,
                    value.manifest.as_bytes().as_slice(),
                ],
            )
            .map_err(map_sql)?;
    }
    for input in &state.inputs {
        transaction
            .execute(
                "INSERT INTO input_index(execution, input_id, parent, payload_digest, commit_id)
                 VALUES(?1, ?2, ?3, ?4, ?5)",
                params![
                    input.execution.as_bytes().as_slice(),
                    input.input.as_bytes().as_slice(),
                    input.parent.as_bytes().as_slice(),
                    input.payload.as_bytes().as_slice(),
                    input.commit.as_bytes().as_slice(),
                ],
            )
            .map_err(map_sql)?;
    }
    for pin in &state.pins {
        transaction
            .execute(
                "INSERT INTO pins(owner, object_id, expires_at) VALUES(?1, ?2, ?3)",
                params![
                    pin.owner,
                    pin.object.as_bytes().as_slice(),
                    pin.expires_at
                        .map(|value| to_i64(value, "Pin expiration"))
                        .transpose()?,
                ],
            )
            .map_err(map_sql)?;
    }
    for (execution, fence) in &state.ledger_fences {
        transaction
            .execute(
                "INSERT INTO ledger_fences(execution, fence) VALUES(?1, ?2)",
                params![
                    execution.as_bytes().as_slice(),
                    to_i64(fence.get(), "ledger fence")?,
                ],
            )
            .map_err(map_sql)?;
    }
    for (key, value) in &state.compound_saves {
        transaction
            .execute(
                "INSERT INTO compound_save_refs(owner, slot, revision, manifest_id)
                 VALUES(?1, ?2, ?3, ?4)",
                params![
                    key.owner().as_str(),
                    key.slot().as_str(),
                    to_i64(value.revision.get(), "Compound Save revision")?,
                    value.manifest.as_bytes().as_slice(),
                ],
            )
            .map_err(map_sql)?;
    }
    for entry in &state.effects {
        let (rewind, compensation_capability) = encode_rewind(&entry.rewind);
        let encoded = encode_ledger_status(&entry.status)?;
        transaction
            .execute(
                "INSERT INTO effect_ledger(
                   execution, effect_id, request_digest, capability, capability_version,
                   origin_commit, delivery, rewind, compensation_capability, status,
                   lease_id, expires_at, response_id, diagnostic_id, fence, by_effect,
                   original_fence, compensation_fence
                 ) VALUES(
                   ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14,
                   ?15, ?16, ?17, ?18
                 )",
                params![
                    entry.execution.as_bytes().as_slice(),
                    entry.effect.as_bytes().as_slice(),
                    entry.request_digest.as_bytes().as_slice(),
                    entry.capability.as_str(),
                    i64::from(entry.capability_version.get()),
                    entry.origin_commit.as_bytes().as_slice(),
                    i64::from(entry.delivery as u8),
                    rewind,
                    compensation_capability,
                    encoded.status,
                    encoded.lease,
                    encoded.expires_at,
                    encoded.response,
                    encoded.diagnostic,
                    encoded.fence,
                    encoded.by_effect,
                    encoded.original_fence,
                    encoded.compensation_fence,
                ],
            )
            .map_err(map_sql)?;
    }
    Ok(())
}

fn edge_rows(state: &MemoryStoreState) -> Result<Vec<(ObjectId, ObjectId, i64)>, StoreError> {
    let objects = state
        .objects
        .iter()
        .map(|(object, _)| (object.id(), object))
        .collect::<BTreeMap<_, _>>();
    let mut edges = Vec::new();
    for (object, _) in &state.objects {
        match object.kind() {
            ObjectKind::Commit => {
                let commit = CommitV1::decode(object.payload())
                    .map_err(|error| StoreError::Corrupt(object.id(), error.to_string()))?;
                if let Some(parent) = commit.parent {
                    edges.push((object.id(), object_id(parent.as_bytes()), 0));
                }
                edges.push((object.id(), object_id(commit.snapshot.as_bytes()), 1));
                if let CommitCauseV1::RuntimeTransition(receipt) = commit.cause {
                    edges.push((object.id(), object_id(receipt.as_bytes()), 2));
                }
                let program = objects
                    .iter()
                    .filter(|(_, candidate)| candidate.kind() == ObjectKind::Program)
                    .find_map(|(id, candidate)| {
                        narrata_core::program::load_program(candidate.bytes(), &Default::default())
                            .ok()
                            .filter(|program| program.artifact_id() == commit.program)
                            .map(|_| *id)
                    })
                    .ok_or(StoreError::InvalidGraph("Program edge"))?;
                edges.push((object.id(), program, 3));
            }
            ObjectKind::TimelineCatalogEvent => {
                let event = TimelineCatalogEventV1::decode(object.payload())
                    .map_err(|error| StoreError::Corrupt(object.id(), error.to_string()))?;
                if let Some(previous) = event.previous {
                    edges.push((object.id(), object_id(previous.as_bytes()), 4));
                }
                edges.extend(
                    event
                        .referenced_commits()
                        .iter()
                        .map(|id| (object.id(), object_id(id.as_bytes()), 5)),
                );
            }
            ObjectKind::CheckpointBundleManifest => {
                let manifest = narrata_store::CheckpointBundleManifestV1::decode(object.payload())
                    .map_err(|error| StoreError::Corrupt(object.id(), error.to_string()))?;
                edges.push((object.id(), object_id(manifest.root.as_bytes()), 6));
                edges.extend(
                    manifest
                        .objects
                        .iter()
                        .map(|descriptor| (object.id(), descriptor.id, 7)),
                );
                edges.extend(
                    manifest
                        .optional_host_manifest
                        .map(|id| (object.id(), id, 8)),
                );
            }
            ObjectKind::TimelineArchiveManifest => {
                let manifest = TimelineArchiveManifestV1::decode(object.payload())
                    .map_err(|error| StoreError::Corrupt(object.id(), error.to_string()))?;
                edges.push((object.id(), object_id(manifest.catalog_head.as_bytes()), 9));
                edges.push((
                    object.id(),
                    object_id(manifest.coverage.baseline().as_bytes()),
                    10,
                ));
                edges.extend(
                    manifest
                        .objects
                        .iter()
                        .map(|descriptor| (object.id(), descriptor.id, 11)),
                );
                edges.extend(manifest.host_timeline.map(|id| (object.id(), id, 12)));
            }
            ObjectKind::CompoundSaveManifest => {
                let manifest = CompoundSaveManifestV1::decode(object.payload())
                    .map_err(|error| StoreError::Corrupt(object.id(), error.to_string()))?;
                edges.push((object.id(), object_id(manifest.narrative.as_bytes()), 13));
            }
            ObjectKind::HostTimelineManifest => {
                let manifest = HostTimelineManifestV1::decode(object.payload())
                    .map_err(|error| StoreError::Corrupt(object.id(), error.to_string()))?;
                edges.extend(
                    manifest
                        .entries()
                        .iter()
                        .map(|entry| (object.id(), object_id(entry.narrative.as_bytes()), 14)),
                );
            }
            ObjectKind::Program
            | ObjectKind::Snapshot
            | ObjectKind::Receipt
            | ObjectKind::Value
            | ObjectKind::EffectResponse => {}
        }
    }
    edges.sort_unstable();
    edges.dedup();
    Ok(edges)
}

fn object_id(bytes: &[u8; 32]) -> ObjectId {
    ObjectId::from_bytes(*bytes)
}

fn revision_from_i64(value: i64) -> Result<RefRevision, StoreError> {
    u64_from_i64(value)?
        .checked_sub(0)
        .and_then(RefRevision::from_u64)
        .ok_or_else(|| StoreError::CorruptStore("zero Ref revision".to_owned()))
}

fn u64_from_i64(value: i64) -> Result<u64, StoreError> {
    u64::try_from(value).map_err(|_| StoreError::CorruptStore("negative integer".to_owned()))
}

fn to_i64(value: u64, name: &'static str) -> Result<i64, StoreError> {
    i64::try_from(value).map_err(|_| StoreError::Limit(name))
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

fn encode_rewind(policy: &RewindPolicy) -> (i64, Option<String>) {
    match policy {
        RewindPolicy::Reapply => (0, None),
        RewindPolicy::ReuseRecordedResponse => (1, None),
        RewindPolicy::Barrier => (2, None),
        RewindPolicy::Compensatable { capability } => (3, Some(capability.as_str().to_owned())),
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

struct EncodedLedgerStatus {
    status: i64,
    lease: Option<Vec<u8>>,
    expires_at: Option<i64>,
    response: Option<Vec<u8>>,
    diagnostic: Option<Vec<u8>>,
    fence: Option<i64>,
    by_effect: Option<Vec<u8>>,
    original_fence: Option<i64>,
    compensation_fence: Option<i64>,
}

fn encode_ledger_status(status: &LedgerStatus) -> Result<EncodedLedgerStatus, StoreError> {
    let mut value = EncodedLedgerStatus {
        status: 0,
        lease: None,
        expires_at: None,
        response: None,
        diagnostic: None,
        fence: None,
        by_effect: None,
        original_fence: None,
        compensation_fence: None,
    };
    match status {
        LedgerStatus::Claimed { lease, expires_at } => {
            value.lease = Some(lease.as_bytes().to_vec());
            value.expires_at = Some(to_i64(*expires_at, "lease expiration")?);
        }
        LedgerStatus::Completed { response, fence } => {
            value.status = 1;
            value.response = Some(response.as_bytes().to_vec());
            value.fence = Some(to_i64(fence.get(), "ledger fence")?);
        }
        LedgerStatus::Rejected { response, fence } => {
            value.status = 2;
            value.response = Some(response.as_bytes().to_vec());
            value.fence = Some(to_i64(fence.get(), "ledger fence")?);
        }
        LedgerStatus::RetryableFailure { diagnostic } => {
            value.status = 3;
            value.diagnostic = Some(diagnostic.as_bytes().to_vec());
        }
        LedgerStatus::UnknownOutcome { diagnostic, fence } => {
            value.status = 4;
            value.diagnostic = Some(diagnostic.as_bytes().to_vec());
            value.fence = Some(to_i64(fence.get(), "ledger fence")?);
        }
        LedgerStatus::Compensated {
            response,
            original_fence,
            by_effect,
            compensation_fence,
        } => {
            value.status = 5;
            value.response = Some(response.as_bytes().to_vec());
            value.by_effect = Some(by_effect.as_bytes().to_vec());
            value.original_fence = Some(to_i64(original_fence.get(), "original fence")?);
            value.compensation_fence =
                Some(to_i64(compensation_fence.get(), "compensation fence")?);
        }
    }
    Ok(value)
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

#[allow(dead_code)]
fn _mutations_are_public(_: (&ArchiveMutation, &CatalogMutation)) {}

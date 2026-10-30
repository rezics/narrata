//! The maintenance driver uses the same registry, cache protocol and history engine as reads.
use narrata_history::{
    MaintenanceBudget, RefKey, RefMutation, RefName, RefNamespace, RefRevision, RefScope,
    RetentionPolicy,
};
use narrata_nodes::{
    ArtifactId, CommitId, Error, ExecutionId, Result, Session,
    history::{evolution::EvolutionRegistry, history_error, maintenance},
};
use narrata_storage_host::{CacheBackend, FlushReply, HostError, Loaded};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

#[derive(Clone, Copy, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MaintenanceLimits {
    pub object_reads: u32,
    pub bytes: u32,
    pub pages: u32,
    pub work: u32,
    pub page_size: u32,
    /// Host load calls, including retries on a cold cache.
    pub loads: u32,
    /// Cumulative encoded reply receipt/acceptance threshold. One conforming bounded reply
    /// may arrive before rejection, before maintenance/cache acceptance and Wasm transfer.
    /// This does not bound host allocation or I/O before the host returns its reply.
    pub load_bytes: u32,
}
impl MaintenanceLimits {
    fn budget(self) -> Result<MaintenanceBudget> {
        if self.object_reads > 1_000_000
            || self.bytes > 256 * 1024 * 1024
            || self.pages > 100_000
            || self.work > 10_000_000
            || self.page_size == 0
            || self.page_size > 1024
            || self.loads > 100_000
            || self.load_bytes > 256 * 1024 * 1024
        {
            return Err(Error::new(
                "limit",
                "maintenance",
                "maintenance budget exceeds maximum",
            ));
        }
        Ok(MaintenanceBudget {
            object_reads: self.object_reads.into(),
            bytes: self.bytes.into(),
            pages: self.pages.into(),
            work: self.work.into(),
            page_size: self.page_size,
        })
    }
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RootKey {
    /// Existing RefNamespace code: save=0, branch=1, bookmark=2, active=3, temporary=4.
    pub namespace: u8,
    pub owner: String,
    pub name: String,
}
impl RootKey {
    fn checked(&self) -> Result<RefKey> {
        let namespace = RefNamespace::from_code(self.namespace)
            .ok_or_else(|| Error::new("root", "namespace", "unknown ref namespace"))?;
        let name =
            |text: &str| RefName::new(text).map_err(|e| Error::new("root", "name", e.to_string()));
        Ok(RefKey::new(
            namespace,
            name(&self.owner)?,
            name(&self.name)?,
        ))
    }
    fn from_key(key: &RefKey) -> Self {
        Self {
            namespace: key.namespace() as u8,
            owner: key.owner().to_string(),
            name: key.name().to_string(),
        }
    }
}
#[derive(Debug, Deserialize, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ObservedRoot {
    pub key: RootKey,
    pub revision: String,
    pub commit: CommitId,
}
#[derive(Debug, Deserialize, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PinCursor {
    pub owner: String,
    pub object: narrata_nodes::ObjectId,
}

#[derive(Debug, Deserialize, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ImportSelection {
    pub execution: ExecutionId,
    pub artifact: ArtifactId,
    pub cursor: CommitId,
    pub expected_revision: Option<String>,
    pub now: u64,
}

#[derive(Debug, JsonSchema, Serialize)]
pub struct CheckedSave {
    pub artifact: ArtifactId,
    pub cursor: CommitId,
}

#[derive(Debug, Deserialize, JsonSchema, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum MaintenanceRequest {
    Roots {
        after: Option<RootKey>,
        limit: u32,
    },
    Pins {
        after: Option<PinCursor>,
        limit: u32,
        now: u64,
    },
    DeleteRoots {
        roots: Vec<ObservedRoot>,
    },
    Integrity,
    Collect {
        now: u64,
        grace_seconds: u64,
        dry_run: bool,
    },
}
#[derive(Debug, JsonSchema, Serialize)]
pub struct PinView {
    pub owner: String,
    pub object: narrata_nodes::ObjectId,
    pub expires_at: Option<String>,
    pub expired: bool,
}
#[derive(Debug, JsonSchema, Serialize)]
pub struct IntegrityView {
    pub object: narrata_nodes::ObjectId,
    pub diagnostic: String,
}
#[derive(Debug, JsonSchema, Serialize)]
pub struct CollectedKind {
    pub kind: u16,
    pub objects: String,
    pub bytes: String,
}
#[derive(Debug, JsonSchema, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MaintenanceResult {
    Roots {
        items: Vec<ObservedRoot>,
        more: bool,
    },
    Pins {
        items: Vec<PinView>,
        more: bool,
    },
    Deleted,
    Integrity {
        issues: Vec<IntegrityView>,
    },
    Collected {
        roots: String,
        reachable: String,
        retained: Vec<CollectedKind>,
        removed: Vec<CollectedKind>,
        dry_run: bool,
    },
}

fn revision(text: &str) -> Result<RefRevision> {
    let value = text
        .parse::<u64>()
        .ok()
        .filter(|v| v.to_string() == text)
        .and_then(RefRevision::from_u64);
    value.ok_or_else(|| {
        Error::new(
            "root",
            "revision",
            "expected a positive canonical decimal revision",
        )
    })
}
fn time(now: u64) -> Result<()> {
    if now > 9_007_199_254_740_991 {
        return Err(Error::new("limit", "clock", "expected safe Unix seconds"));
    }
    Ok(())
}
fn limit(value: u32) -> Result<()> {
    if value == 0 || value > 1024 {
        return Err(Error::new("limit", "page", "page limit must be 1..=1024"));
    }
    Ok(())
}

#[derive(Default)]
pub struct Engine {
    pub registry: EvolutionRegistry,
    pub cache: CacheBackend,
    remaining: std::cell::RefCell<Option<std::rc::Rc<std::cell::Cell<MaintenanceBudget>>>>,
    snapshot: Option<(String, Session, Vec<u8>, u64)>,
}
impl Engine {
    pub fn refresh(&mut self) {
        self.cache.invalidate();
        self.remaining.replace(None);
        self.snapshot = None;
    }
    fn backend(
        &self,
        limits: MaintenanceLimits,
    ) -> Result<narrata_history::MaintenanceBackend<CacheBackend>> {
        let budget = limits.budget()?;
        let remaining = self
            .remaining
            .borrow_mut()
            .get_or_insert_with(|| std::rc::Rc::new(std::cell::Cell::new(budget)))
            .clone();
        narrata_history::MaintenanceBackend::with_remaining(self.cache.share(), remaining)
            .map_err(history_error)
    }
    pub fn inspect_save(&self, save: &str, limits: MaintenanceLimits) -> Result<CheckedSave> {
        let backend =
            narrata_history::MaintenanceBackend::new(CacheBackend::new(), limits.budget()?)
                .map_err(history_error)?;
        let (_, snapshot, _, _) = Self::checked_snapshot(&self.registry, backend, save, limits)?;
        snapshot.page()?;
        snapshot.interaction(None)?;
        Ok(CheckedSave {
            artifact: snapshot.program().artifact_id(),
            cursor: snapshot.cursor()?,
        })
    }
    pub fn run(
        &mut self,
        request: MaintenanceRequest,
        limits: MaintenanceLimits,
    ) -> Result<MaintenanceResult> {
        let mut history = narrata_history::History::open_existing(
            self.backend(limits)?,
            narrata_history::DomainKinds::<narrata_nodes::history::NodeDomain>::new(),
        )
        .map_err(history_error)?;
        Ok(match request {
            MaintenanceRequest::Roots {
                after,
                limit: count,
            } => {
                limit(count)?;
                let after = after.as_ref().map(RootKey::checked).transpose()?;
                let page = history
                    .scan_refs(&RefScope::All, after.as_ref(), count)
                    .map_err(history_error)?;
                MaintenanceResult::Roots {
                    items: page
                        .items
                        .iter()
                        .map(|(key, value)| ObservedRoot {
                            key: RootKey::from_key(key),
                            revision: value.revision.get().to_string(),
                            commit: CommitId::from_bytes(*value.commit.as_bytes()),
                        })
                        .collect(),
                    more: page.more,
                }
            }
            MaintenanceRequest::Pins {
                after,
                limit: count,
                now,
            } => {
                limit(count)?;
                time(now)?;
                let after = after.map(|p| {
                    (
                        p.owner,
                        narrata_history::ObjectId::from_bytes(*p.object.as_bytes()),
                    )
                });
                let page = history
                    .scan_pins(after.as_ref(), count)
                    .map_err(history_error)?;
                MaintenanceResult::Pins {
                    items: page
                        .items
                        .into_iter()
                        .map(|pin| PinView {
                            owner: pin.owner,
                            object: narrata_nodes::ObjectId::from_bytes(*pin.object.as_bytes()),
                            expires_at: pin.expires_at.map(|v| v.to_string()),
                            expired: pin.expires_at.is_some_and(|v| v <= now),
                        })
                        .collect(),
                    more: page.more,
                }
            }
            MaintenanceRequest::DeleteRoots { roots } => {
                if roots.is_empty() || roots.len() > 256 {
                    return Err(Error::new(
                        "limit",
                        "roots",
                        "select 1..=256 observed roots",
                    ));
                }
                let mutations = roots
                    .into_iter()
                    .map(|root| {
                        let key = root.key.checked()?;
                        let observed = history.read_ref(&key).map_err(history_error)?;
                        if observed.is_none_or(|v| {
                            v.revision.get().to_string() != root.revision
                                || v.commit.as_bytes() != root.commit.as_bytes()
                        }) {
                            return Err(Error::new(
                                "stale_input",
                                "roots",
                                "selected root changed; re-read roots",
                            ));
                        }
                        Ok(RefMutation {
                            key,
                            expected: Some(revision(&root.revision)?),
                            next: None,
                        })
                    })
                    .collect::<Result<Vec<_>>>()?;
                maintenance::delete_roots(&mut history, mutations)?;
                MaintenanceResult::Deleted
            }
            MaintenanceRequest::Integrity => MaintenanceResult::Integrity {
                issues: history
                    .integrity_scan()
                    .map_err(history_error)?
                    .into_iter()
                    .map(|i| IntegrityView {
                        object: narrata_nodes::ObjectId::from_bytes(*i.object.as_bytes()),
                        diagnostic: i.diagnostic,
                    })
                    .collect(),
            },
            MaintenanceRequest::Collect {
                now,
                grace_seconds,
                dry_run,
            } => {
                let report = maintenance::collect(
                    &mut history,
                    RetentionPolicy {
                        now,
                        grace_seconds,
                        dry_run,
                    },
                )?;
                MaintenanceResult::Collected {
                    roots: report.roots.to_string(),
                    reachable: report.reachable.to_string(),
                    retained: report
                        .retained
                        .into_iter()
                        .map(|(kind, r)| CollectedKind {
                            kind,
                            objects: r.objects.to_string(),
                            bytes: r.bytes.to_string(),
                        })
                        .collect(),
                    removed: report
                        .removed
                        .into_iter()
                        .map(|(kind, r)| CollectedKind {
                            kind,
                            objects: r.objects.to_string(),
                            bytes: r.bytes.to_string(),
                        })
                        .collect(),
                    dry_run,
                }
            }
        })
    }
    pub fn export(
        &self,
        execution: ExecutionId,
        artifact: ArtifactId,
        cursor: CommitId,
        ancestors: Option<u32>,
        limits: MaintenanceLimits,
    ) -> Result<Vec<u8>> {
        let backend = self.backend(limits)?;
        let session = Session::open_with_registry(&self.registry, execution, backend)?;
        if session.program().artifact_id() != artifact || session.cursor()? != cursor {
            return Err(Error::new(
                "stale_input",
                "cursor",
                "exact artifact or cursor changed",
            ));
        }
        match ancestors {
            Some(n) if n <= 10_000 => session.export_shallow(n),
            Some(_) => Err(Error::new("limit", "ancestors", "at most 10000 ancestors")),
            None => hex::decode(session.export()?)
                .map_err(|e| Error::new("decode", "checkpoint", e.to_string())),
        }
    }
    fn prepare_snapshot(&mut self, save: &str, limits: MaintenanceLimits) -> Result<()> {
        limits.budget()?;
        if self
            .snapshot
            .as_ref()
            .is_some_and(|(text, _, _, _)| text != save)
        {
            return Err(Error::new(
                "stale_input",
                "checkpoint",
                "refresh before importing a different save",
            ));
        }
        if self.snapshot.is_none() {
            self.snapshot = Some(Self::checked_snapshot(
                &self.registry,
                self.backend(limits)?,
                save,
                limits,
            )?);
        }
        Ok(())
    }
    fn checked_snapshot(
        registry: &EvolutionRegistry,
        backend: narrata_history::MaintenanceBackend<CacheBackend>,
        save: &str,
        limits: MaintenanceLimits,
    ) -> Result<(String, Session, Vec<u8>, u64)> {
        let budget = limits.budget()?;
        // Bound the caller-owned input before copying/decoding it. Cache the immutable
        // checked snapshot across misses instead of repeatedly rebuilding a full save.
        if save.len() as u64 > budget.bytes || save.len() > narrata_nodes::MAX_EXPORT_BYTES {
            return Err(Error::new(
                "limit",
                "checkpoint",
                "save input exceeds byte budget",
            ));
        }
        backend
            .account_input(0, save.len() as u64)
            .map_err(history_error)?;
        let decode_limits = narrata_history::BundleLimits {
            max_total_bytes: budget.bytes,
            max_object_bytes: budget.bytes.min(16 * 1024 * 1024),
            max_objects: budget.object_reads.min(budget.work),
        };
        let count_objects = |bytes: &[u8]| {
            let result = if bytes.starts_with(narrata_history::SHALLOW_MAGIC) {
                narrata_history::ShallowBundle::from_bytes(bytes, decode_limits, |_| true)
                    .map(|b| b.objects.len())
            } else {
                narrata_history::CheckpointBundle::from_bytes(bytes, decode_limits, |_| true)
                    .map(|b| b.objects.len())
            };
            result.map_err(|error| {
                Error::new(
                    if matches!(error, narrata_history::ContainerError::Limit(_)) {
                        "limit"
                    } else {
                        "corrupt_save"
                    },
                    "checkpoint",
                    error.to_string(),
                )
            })
        };
        let legacy = save.trim_start().starts_with('{');
        let raw_bytes = if legacy {
            Vec::new()
        } else {
            hex::decode(save.trim())
                .map_err(|e| Error::new("corrupt_save", "checkpoint", e.to_string()))?
        };
        let count = if legacy {
            let export: narrata_nodes::SessionExport = serde_json::from_str(save)
                .map_err(|e| Error::new("corrupt_save", "checkpoint", e.to_string()))?;
            if export.objects.len() as u64 > decode_limits.max_objects {
                return Err(Error::new(
                    "limit",
                    "checkpoint",
                    "legacy save exceeds object budget",
                ));
            }
            export.objects.len()
        } else {
            count_objects(&raw_bytes)?
        };
        backend
            .account_input(
                (count as u64).saturating_mul(3),
                (save.len() as u64).saturating_mul(3),
            )
            .map_err(history_error)?;
        let source = Session::restore_with_registry(registry, save)?;
        let bytes = if legacy {
            hex::decode(source.export()?)
                .map_err(|e| Error::new("decode", "checkpoint", e.to_string()))?
        } else {
            raw_bytes
        };
        let count = count_objects(&bytes)?;
        Ok((save.to_owned(), source, bytes, count as u64))
    }
    pub fn import(
        &mut self,
        save: &str,
        selection: ImportSelection,
        limits: MaintenanceLimits,
    ) -> Result<CommitId> {
        let ImportSelection {
            execution,
            artifact,
            cursor,
            expected_revision,
            now,
        } = selection;
        time(now)?;
        self.prepare_snapshot(save, limits)?;
        let backend = self.backend(limits)?;
        let (_, source, bytes, count) = self
            .snapshot
            .as_ref()
            .ok_or_else(|| Error::new("save", "checkpoint", "missing checked snapshot"))?;
        if source.program().artifact_id() != artifact || source.cursor()? != cursor {
            return Err(Error::new(
                "incompatible_save",
                "cursor",
                "save differs from exact artifact/cursor",
            ));
        }
        // Establish the target layout before repeating the isolated page proof on a cold
        // miss. Any empty-store initialization stays unconfirmed until the full import succeeds.
        narrata_history::History::open(
            self.backend(limits)?,
            narrata_history::DomainKinds::<narrata_nodes::history::NodeDomain>::new(),
        )
        .map_err(history_error)?;
        // import_snapshot rechecks an isolated page and the bundle root before each
        // attempt. Charge those complete in-memory passes too, not only cache loads.
        backend
            .account_input(
                count.saturating_mul(3),
                (bytes.len() as u64).saturating_mul(3),
            )
            .map_err(history_error)?;
        source.import_snapshot(
            backend,
            execution,
            expected_revision.as_deref().map(revision).transpose()?,
            bytes,
            now,
        )
    }
}

#[wasm_bindgen]
#[derive(Default)]
pub struct Maintenance {
    inner: Engine,
}
#[wasm_bindgen]
impl Maintenance {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self::default()
    }
    #[wasm_bindgen(js_name = registerPublication)]
    pub fn register_publication(
        &mut self,
        publication: &crate::Publication,
    ) -> std::result::Result<(), JsError> {
        self.inner
            .registry
            .register_program(publication.inner.shared_program())
            .map_err(crate::js_error)
    }
    pub fn refresh(&mut self) {
        self.inner.refresh();
    }
    #[wasm_bindgen(js_name = inspectSave)]
    pub fn inspect_save(&self, save: &str, limits: &str) -> std::result::Result<String, JsError> {
        crate::json(
            &self
                .inner
                .inspect_save(save, crate::parse(limits)?)
                .map_err(crate::js_error)?,
        )
    }
    pub fn run(&mut self, request: &str, limits: &str) -> std::result::Result<String, JsError> {
        crate::json(
            &self
                .inner
                .run(crate::parse(request)?, crate::parse(limits)?)
                .map_err(crate::js_error)?,
        )
    }
    pub fn export(
        &self,
        execution: &str,
        artifact: &str,
        cursor: &str,
        ancestors: Option<u32>,
        limits: &str,
    ) -> std::result::Result<Vec<u8>, JsError> {
        self.inner
            .export(
                id(execution)?,
                id(artifact)?,
                id(cursor)?,
                ancestors,
                crate::parse(limits)?,
            )
            .map_err(crate::js_error)
    }
    pub fn import(
        &mut self,
        save: &str,
        selection: &str,
        limits: &str,
    ) -> std::result::Result<String, JsError> {
        self.inner
            .import(save, crate::parse(selection)?, crate::parse(limits)?)
            .map(|c| c.to_string())
            .map_err(crate::js_error)
    }
    #[wasm_bindgen(js_name = takeRequest)]
    pub fn take_request(&mut self) -> Option<Vec<u8>> {
        self.inner.cache.take_request().map(|r| r.encode())
    }
    pub fn load(&mut self, bytes: &[u8]) -> std::result::Result<bool, JsError> {
        settle(self.inner.cache.load(&Loaded::decode(bytes)?))
    }
    pub fn unconfirmed(&self) -> Option<Vec<u8>> {
        self.inner.cache.unconfirmed().map(|r| r.encode())
    }
    pub fn confirm(&mut self, bytes: &[u8]) -> std::result::Result<bool, JsError> {
        settle(self.inner.cache.confirm(&FlushReply::decode(bytes)?))
    }
}
fn id<T: std::str::FromStr>(text: &str) -> std::result::Result<T, JsError> {
    text.parse()
        .map_err(|_| JsError::new("identifier at maintenance: invalid identifier"))
}
fn settle(result: std::result::Result<(), HostError>) -> std::result::Result<bool, JsError> {
    match result {
        Ok(()) => Ok(true),
        Err(HostError::Superseded(_)) => Ok(false),
        Err(error) => Err(JsError::new(&error.to_string())),
    }
}

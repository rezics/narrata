//! Authoring Wasm (ADR 0022 §4): a thin binding of `narrata-authoring` for browser Workers and
//! Node/Bun. Records, outline batches, packages and objects cross one at a time; structured
//! results are JSON text whose shapes are [`interface`]. Handles own their Rust values until
//! the host frees them. Reader builds never depend on this crate.

#![forbid(unsafe_code)]

pub mod core;
pub mod evolution;
mod evolution_binding;
pub mod interface;

use narrata_authoring::{CheckedPublication, publication::publication_from_pack, validate_record};
use narrata_nodes::{
    AuthoredId, CompositionLock, ContentOutline, Error, MAX_SOURCE_BYTES, PackageSource,
    ProjectManifest, parse_json_limited,
};
use wasm_bindgen::prelude::*;

fn js_error(error: Error) -> JsError {
    JsError::new(&error.to_string())
}

fn json<T: serde::Serialize>(value: &T) -> Result<String, JsError> {
    serde_json::to_string(value).map_err(|error| JsError::new(&error.to_string()))
}

fn parse<T: serde::de::DeserializeOwned>(text: &str) -> Result<T, JsError> {
    parse_json_limited(text, MAX_SOURCE_BYTES).map_err(js_error)
}

fn lock(text: Option<String>) -> Result<Option<CompositionLock>, JsError> {
    text.as_deref().map(parse).transpose()
}

#[wasm_bindgen]
extern "C" {
    /// The host clock (Unix milliseconds) and cryptographic random source used only to mint
    /// UUIDv7 IDs. Checking, compilation and publication never call it.
    pub type Entropy;
    #[wasm_bindgen(method, catch)]
    fn now(this: &Entropy) -> Result<f64, JsValue>;
    #[wasm_bindgen(method, catch)]
    fn random(this: &Entropy, bytes: &mut [u8]) -> Result<(), JsValue>;
}

impl Entropy {
    fn clock(&self) -> impl FnMut() -> narrata_nodes::Result<u64> + '_ {
        move || {
            let millis = self
                .now()
                .map_err(|_| Error::new("clock", "ids", "the host clock failed"))?;
            if !millis.is_finite() || millis < 0.0 {
                return Err(Error::new(
                    "clock",
                    "ids",
                    "the host clock returned no Unix time",
                ));
            }
            Ok(millis.floor() as u64)
        }
    }
    fn source(&self) -> impl FnMut(&mut [u8]) -> narrata_nodes::Result<()> + '_ {
        move |bytes| {
            self.random(bytes)
                .map_err(|_| Error::new("random", "ids", "the host random source failed"))
        }
    }
}

/// Checks one record's encoding, fields and independently decidable constraints.
#[wasm_bindgen(js_name = validateRecord)]
pub fn validate_record_bytes(bytes: &[u8]) -> Records {
    Records {
        inner: core::RecordSet::record(validate_record(bytes)),
    }
}

/// Checked records by index: canonical bytes, keys and derived index rows.
#[wasm_bindgen]
pub struct Records {
    inner: core::RecordSet,
}

#[wasm_bindgen]
impl Records {
    /// `RecordReport` JSON.
    pub fn report(&self) -> Result<String, JsError> {
        json(&self.inner.report)
    }
    pub fn key(&self, index: usize) -> Option<String> {
        Some(self.inner.records.get(index)?.key())
    }
    /// Canonical record bytes, the stored form.
    pub fn bytes(&self, index: usize) -> Option<Vec<u8>> {
        Some(self.inner.records.get(index)?.bytes().to_vec())
    }
    /// `RecordIndexes` JSON; present when the records were checked as a project.
    pub fn indexes(&self, index: usize) -> Result<Option<String>, JsError> {
        self.inner.indexes(index).map(json).transpose()
    }
    /// The assembled `ProjectManifest` JSON, for file export.
    pub fn manifest(&self) -> Result<Option<String>, JsError> {
        self.inner
            .source
            .as_ref()
            .map(|source| json(&source.manifest))
            .transpose()
    }
    pub fn packages(&self) -> Vec<String> {
        self.inner
            .source
            .as_ref()
            .map(|source| source.packages.keys().cloned().collect())
            .unwrap_or_default()
    }
    /// One assembled `PackageSource` JSON by alias.
    pub fn package(&self, alias: &str) -> Result<Option<String>, JsError> {
        self.inner
            .source
            .as_ref()
            .and_then(|source| source.packages.get(alias))
            .map(json)
            .transpose()
    }
}

/// Persisted draft records and outline batches, accumulated one at a time.
#[wasm_bindgen]
#[derive(Default)]
pub struct Drafts {
    inner: core::DraftSet,
}

#[wasm_bindgen]
impl Drafts {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Drafts {
        Self::default()
    }
    /// Stores record bytes; diagnostics name it `records[<insertion index>]`.
    pub fn add(&mut self, bytes: &[u8]) {
        self.inner.add(bytes);
    }
    /// One `ContentOutline` batch; batches of a provider assemble into its single outline.
    #[wasm_bindgen(js_name = addOutline)]
    pub fn add_outline(&mut self, outline: &str) -> Result<(), JsError> {
        let outline: ContentOutline = parse(outline)?;
        self.inner.add_outline(outline).map_err(js_error)
    }
    /// Record, assembly and semantic checks with derived indexes; nothing is compiled.
    pub fn check(&self) -> Records {
        Records {
            inner: self.inner.check(),
        }
    }
    /// Preview compilation without a baseline. A lock JSON selects locked composition,
    /// which never appends tombstones.
    pub fn compose(&self, lock_json: Option<String>) -> Result<Preview, JsError> {
        self.preview(None, lock_json)
    }
    /// Preview compilation checking ID continuity against the previous publication.
    #[wasm_bindgen(js_name = composeAfter)]
    pub fn compose_after(
        &self,
        previous: &Publication,
        lock_json: Option<String>,
    ) -> Result<Preview, JsError> {
        self.preview(Some(previous), lock_json)
    }
    /// The first publication of a work, with the supplied outlines.
    pub fn publish(&self, lock_json: Option<String>) -> Result<Published, JsError> {
        self.release(None, lock_json)
    }
    /// Every later publication passes the current one, so ID continuity is checked.
    #[wasm_bindgen(js_name = publishAfter)]
    pub fn publish_after(
        &self,
        previous: &Publication,
        lock_json: Option<String>,
    ) -> Result<Published, JsError> {
        self.release(Some(previous), lock_json)
    }
}

impl Drafts {
    fn preview(
        &self,
        previous: Option<&Publication>,
        lock_json: Option<String>,
    ) -> Result<Preview, JsError> {
        let lock = lock(lock_json)?;
        let inner = self
            .inner
            .compose(previous.map(|p| p.inner.program()), lock.as_ref())
            .map_err(js_error)?;
        Ok(Preview { inner })
    }
    fn release(
        &self,
        previous: Option<&Publication>,
        lock_json: Option<String>,
    ) -> Result<Published, JsError> {
        let lock = lock(lock_json)?;
        Ok(Published {
            inner: self
                .inner
                .publish(previous.map(|p| p.inner.program()), lock.as_ref()),
        })
    }
}

#[wasm_bindgen]
pub struct Preview {
    inner: core::PreviewObjects,
}

#[wasm_bindgen]
impl Preview {
    /// `PreviewReport` JSON.
    pub fn report(&self) -> Result<String, JsError> {
        json(&self.inner.report)
    }
    pub fn manifest(&self) -> Option<Vec<u8>> {
        self.inner.manifest().map(<[u8]>::to_vec)
    }
    pub fn names(&self) -> Option<Vec<u8>> {
        self.inner.names().map(<[u8]>::to_vec)
    }
    /// A program chunk or the tombstone set by `object:<hex>` or bare hex ID.
    pub fn object(&self, id: &str) -> Result<Option<Vec<u8>>, JsError> {
        let id = core::object_id(id).map_err(js_error)?;
        Ok(self.inner.object(&id).map(<[u8]>::to_vec))
    }
    pub fn pack(&self) -> Result<Vec<u8>, JsError> {
        self.inner.pack().map_err(js_error)
    }
}

#[wasm_bindgen]
pub struct Published {
    inner: core::Published,
}

#[wasm_bindgen]
impl Published {
    /// `PublishReport` JSON.
    pub fn report(&self) -> Result<String, JsError> {
        json(&self.inner.report)
    }
    /// Moves the checked candidate out; later calls return nothing.
    #[wasm_bindgen(js_name = takePublication)]
    pub fn take_publication(&mut self) -> Option<Publication> {
        self.inner
            .publication
            .take()
            .map(|inner| Publication { inner })
    }
    /// Moves the normalized records, including appended tombstones, out.
    #[wasm_bindgen(js_name = takeRecords)]
    pub fn take_records(&mut self) -> Records {
        Records {
            inner: std::mem::take(&mut self.inner.records),
        }
    }
}

/// A checked, immutable publication object collection.
#[wasm_bindgen]
pub struct Publication {
    inner: CheckedPublication,
}

#[wasm_bindgen]
impl Publication {
    /// Consumes this candidate and attaches a checked descriptor, retaining its artifact.
    #[wasm_bindgen(js_name = withEvolution)]
    pub fn with_evolution(
        self,
        previous: &Publication,
        plan: &evolution_binding::EvolutionPlan,
    ) -> Result<Publication, JsError> {
        plan.attach(self, previous)
    }
    /// Opens a legacy pack (at most 64 MiB) as a checked publication.
    #[wasm_bindgen(js_name = fromPack)]
    pub fn from_pack(pack: &[u8]) -> Result<Publication, JsError> {
        Ok(Self {
            inner: publication_from_pack(pack).map_err(js_error)?,
        })
    }
    #[wasm_bindgen(getter, js_name = publicationId)]
    pub fn publication_id(&self) -> String {
        self.inner.publication_id().to_string()
    }
    #[wasm_bindgen(getter, js_name = artifactId)]
    pub fn artifact_id(&self) -> String {
        self.inner.program().artifact_id().to_string()
    }
    /// The canonical description bytes, addressed by `publicationId`.
    pub fn description(&self) -> Vec<u8> {
        self.inner.description_bytes().to_vec()
    }
    /// One object by `object:<hex>` or bare hex ID.
    pub fn object(&self, id: &str) -> Result<Option<Vec<u8>>, JsError> {
        Ok(core::publication_object(&self.inner, id)
            .map_err(js_error)?
            .map(<[u8]>::to_vec))
    }
    pub fn pack(&self) -> Result<Vec<u8>, JsError> {
        self.inner.pack().map_err(js_error)
    }
}

/// Reads a previous publication from its description and one object at a time.
#[wasm_bindgen]
pub struct PublicationLoader {
    inner: core::Loader,
}

#[wasm_bindgen]
impl PublicationLoader {
    #[wasm_bindgen(constructor)]
    pub fn new(description: &[u8], publication_id: &str) -> Result<PublicationLoader, JsError> {
        let expected = core::digest(publication_id).map_err(js_error)?;
        Ok(Self {
            inner: core::Loader::new(description, expected).map_err(js_error)?,
        })
    }
    /// The description's `ObjectReference` list as JSON.
    pub fn references(&self) -> Result<String, JsError> {
        json(&self.inner.references().collect::<Vec<_>>())
    }
    pub fn missing(&self) -> Vec<String> {
        self.inner
            .missing()
            .iter()
            .map(ToString::to_string)
            .collect()
    }
    pub fn supply(&mut self, id: &str, bytes: &[u8]) -> Result<(), JsError> {
        let id = core::digest(id).map_err(js_error)?;
        self.inner.supply(id, bytes).map_err(js_error)
    }
    /// Verifies identities, reference closure and correspondence, consuming the loader.
    pub fn finish(self) -> Result<Publication, JsError> {
        Ok(Publication {
            inner: self.inner.finish().map_err(js_error)?,
        })
    }
    #[wasm_bindgen(js_name = finishAfter)]
    pub fn finish_after(self, previous: &Publication) -> Result<Publication, JsError> {
        Ok(Publication {
            inner: self
                .inner
                .finish_after(Some(&previous.inner))
                .map_err(js_error)?,
        })
    }
}

/// An R2 project supplied as its manifest and one package at a time.
#[wasm_bindgen]
pub struct Source {
    inner: core::SourceBuilder,
}

#[wasm_bindgen]
impl Source {
    #[wasm_bindgen(constructor)]
    pub fn new(manifest: &str) -> Result<Source, JsError> {
        let manifest: ProjectManifest = parse(manifest)?;
        Ok(Self {
            inner: core::SourceBuilder::new(manifest),
        })
    }
    #[wasm_bindgen(js_name = addPackage)]
    pub fn add_package(&mut self, alias: &str, package: &str) -> Result<(), JsError> {
        let package: PackageSource = parse(package)?;
        self.inner.add_package(alias, package).map_err(js_error)
    }
    /// Splits a fully minted source into draft records.
    pub fn split(&self) -> Records {
        Records {
            inner: self.inner.split(),
        }
    }
}

/// Mints UUIDv7 IDs avoiding every occupied live or deleted ID and its own allocations.
#[wasm_bindgen]
pub struct IdMinter {
    occupancy: core::Occupancy,
    entropy: Entropy,
}

#[wasm_bindgen]
impl IdMinter {
    #[wasm_bindgen(constructor)]
    pub fn new(entropy: Entropy) -> IdMinter {
        Self {
            occupancy: core::Occupancy::default(),
            entropy,
        }
    }
    /// Adds a batch of occupied IDs.
    pub fn occupy(&mut self, ids: Vec<String>) -> Result<(), JsError> {
        let ids = ids
            .iter()
            .map(|id| {
                id.parse::<AuthoredId>().map_err(|_| {
                    js_error(Error::new(
                        "identifier",
                        id.as_str(),
                        "expected an authored ID",
                    ))
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        self.occupancy.occupy(ids);
        Ok(())
    }
    /// `MintedIds` JSON for an explicit creation, copy or ownership change.
    pub fn mint(
        &mut self,
        nodes: usize,
        choice_points: usize,
        options: usize,
    ) -> Result<String, JsError> {
        let minted = self
            .occupancy
            .mint(
                nodes,
                choice_points,
                options,
                self.entropy.clock(),
                self.entropy.source(),
            )
            .map_err(js_error)?;
        json(&minted)
    }
    /// Fills only the missing IDs of an R2 source and splits it into draft records.
    pub fn fill(&mut self, source: &Source) -> Result<Records, JsError> {
        let inner = self
            .occupancy
            .fill(
                &source.inner.source,
                self.entropy.clock(),
                self.entropy.source(),
            )
            .map_err(js_error)?;
        Ok(Records { inner })
    }
}

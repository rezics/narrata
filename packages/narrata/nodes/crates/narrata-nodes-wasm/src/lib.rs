//! Thin Wasm binding. Pack decoding, state transitions, validation, save restoration, R1 save
//! migration and content resolution stay in Rust; values cross the boundary as JSON text.

#![forbid(unsafe_code)]

use std::sync::Arc;

use narrata_content_local::{ContentPack, ResolveRequest};
use narrata_kernel::content::ProviderId;
use narrata_nodes::{
    Analysis, BookView, Error, MAX_REQUEST_BYTES, NameTable, Program, ProposalRequest, Session,
    analyze, parse_json_limited, r1,
};
use wasm_bindgen::prelude::*;

fn js_error(error: Error) -> JsError {
    JsError::new(&error.to_string())
}

fn parse<T: std::str::FromStr>(text: &str, path: &str, expected: &str) -> Result<T, JsError> {
    text.parse().map_err(|_| {
        js_error(Error::new(
            "identifier",
            path,
            format!("expected {expected}"),
        ))
    })
}

fn json<T: serde::Serialize>(value: &T) -> Result<String, JsError> {
    serde_json::to_string(value).map_err(|error| JsError::new(&error.to_string()))
}

/// A packed artifact and one session on it.
#[wasm_bindgen]
pub struct NodeBook {
    pack: Vec<u8>,
    names: Option<NameTable>,
    analysis: Analysis,
    session: Session,
}

#[wasm_bindgen]
impl NodeBook {
    /// Opens a pack, checks every chunk and starts a session with `execution`.
    #[wasm_bindgen(constructor)]
    pub fn new(pack: &[u8], execution: &str) -> Result<NodeBook, JsError> {
        let (program, names) = Program::from_pack(pack).map_err(js_error)?;
        program.verify_artifact().map_err(js_error)?;
        let analysis = analyze(&program, names.as_ref()).map_err(js_error)?;
        let execution = parse(execution, "execution", "execution:<32 hex digits>")?;
        let session = Session::new(Arc::new(program), execution).map_err(js_error)?;
        Ok(Self {
            pack: pack.to_vec(),
            names,
            analysis,
            session,
        })
    }

    #[wasm_bindgen(getter)]
    pub fn artifact_id(&self) -> String {
        self.session.program().artifact_id().to_string()
    }

    /// The pack bytes the book was opened from.
    pub fn pack(&self) -> Vec<u8> {
        self.pack.clone()
    }

    /// The text-free book view as JSON.
    pub fn inspect(&self) -> Result<String, JsError> {
        json(&BookView {
            view: self.session.view(self.names.as_ref()).map_err(js_error)?,
            page: self.session.page().map_err(js_error)?,
            graphs: self.analysis.graphs.clone(),
            diagnostics: self.analysis.diagnostics.clone(),
        })
    }

    /// Chooses a set of option IDs at the cursor, which must still be `expected`, and returns
    /// the new cursor.
    pub fn choose(
        &mut self,
        expected: &str,
        choice_point: &str,
        options: Vec<String>,
    ) -> Result<String, JsError> {
        let expected = parse(expected, "expected", "commit:<64 hex digits>")?;
        let choice_point = parse(choice_point, "choice_point", "choice-point:<32 hex digits>")?;
        let options = options
            .iter()
            .map(|option| parse(option, "options", "option:<32 hex digits>"))
            .collect::<Result<Vec<_>, _>>()?;
        let commit = self
            .session
            .choose(&expected, choice_point, options)
            .map_err(js_error)?;
        Ok(commit.to_string())
    }

    /// Records a host proposal (a proposal request as JSON) at the cursor, which must still be
    /// `expected`, and returns the new book view. The interaction stays at the same choice
    /// point with the proposed options added.
    pub fn propose(&mut self, expected: &str, request: &str) -> Result<String, JsError> {
        let expected = parse(expected, "expected", "commit:<64 hex digits>")?;
        let request: ProposalRequest =
            parse_json_limited(request, MAX_REQUEST_BYTES).map_err(js_error)?;
        self.session
            .propose(&expected, &request)
            .map_err(js_error)?;
        self.inspect()
    }

    pub fn checkout(&mut self, commit: &str) -> Result<(), JsError> {
        let commit = parse(commit, "commit", "commit:<64 hex digits>")?;
        self.session.checkout(&commit).map_err(js_error)
    }

    /// The session export JSON.
    pub fn export(&self) -> Result<String, JsError> {
        self.session.export().map_err(js_error)
    }

    /// Replaces the session with a checked export of this artifact; the current session stays
    /// on failure.
    pub fn restore(&mut self, export: &str) -> Result<(), JsError> {
        self.session =
            Session::restore(self.session.program().clone(), export).map_err(js_error)?;
        Ok(())
    }

    /// Rebuilds an R1 save on this artifact, which must have been migrated from the save's R1
    /// artifact. `content` resolves the references that replaced R1 text values. The current
    /// session stays on failure.
    pub fn migrate_r1(
        &mut self,
        save: &str,
        content: &LocalContent,
        execution: &str,
    ) -> Result<(), JsError> {
        let names = self.names.as_ref().ok_or_else(|| {
            js_error(Error::new(
                "names",
                "pack",
                "R1 save migration needs the pack's name table",
            ))
        })?;
        let execution = parse(execution, "execution", "execution:<32 hex digits>")?;
        let ref_text = |reference: &_| content.inner.text(reference, &[]);
        self.session = r1::migrate_save(
            self.session.program().clone(),
            names,
            save,
            execution,
            &ref_text,
        )
        .map_err(js_error)?;
        Ok(())
    }
}

/// The local content provider: content packs of one work, the first being the original
/// language that lookups fall back to.
#[wasm_bindgen]
#[derive(Default)]
pub struct LocalContent {
    inner: narrata_content_local::LocalContent,
}

#[wasm_bindgen]
impl LocalContent {
    #[wasm_bindgen(constructor)]
    pub fn new() -> LocalContent {
        Self::default()
    }

    /// Adds a content pack given as JSON text.
    pub fn add(&mut self, pack: &str) -> Result<(), JsError> {
        self.inner
            .add(ContentPack::parse(pack).map_err(js_error)?)
            .map_err(js_error)
    }

    /// Resolves a batch: a resolve request as JSON in, the resolutions in order as JSON out.
    pub fn resolve(&self, request: &str) -> Result<String, JsError> {
        let request: ResolveRequest =
            parse_json_limited(request, 8 * 1024 * 1024).map_err(js_error)?;
        json(&self.inner.resolve(&request).map_err(js_error)?)
    }

    /// The content outline of `provider` in the best matching language, as JSON, if any pack
    /// serves the provider.
    pub fn outline(
        &self,
        provider: &str,
        languages: Vec<String>,
    ) -> Result<Option<String>, JsError> {
        let provider: ProviderId = parse(provider, "provider", "a provider ID")?;
        self.inner
            .outline(&provider, &languages)
            .map(|outline| json(&outline))
            .transpose()
    }
}

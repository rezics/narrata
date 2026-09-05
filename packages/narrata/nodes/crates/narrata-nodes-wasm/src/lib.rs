//! Thin Wasm binding. Compilation, state transitions, validation and save restoration stay in Rust.

#![forbid(unsafe_code)]

use std::sync::Arc;

use narrata_nodes::{
    BookView, CheckedProduct, Diagnostic, Error, NodeRegistry, Session, compile, parse_json,
};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct NodeBook {
    product: Arc<CheckedProduct>,
    session: Session,
    diagnostics: Vec<Diagnostic>,
}

fn js_error(error: Error) -> JsError {
    JsError::new(&format!("{}", error))
}

#[wasm_bindgen]
impl NodeBook {
    #[wasm_bindgen(constructor)]
    pub fn new(source: &str) -> Result<NodeBook, JsError> {
        let compilation = compile(
            parse_json(source).map_err(js_error)?,
            &NodeRegistry::gamebook(),
        )
        .map_err(js_error)?;
        let product = Arc::new(compilation.product);
        let session = Session::new(product.clone()).map_err(js_error)?;
        Ok(Self {
            product,
            session,
            diagnostics: compilation.diagnostics,
        })
    }

    pub fn inspect(&self) -> Result<String, JsError> {
        let view = BookView {
            view: self.session.view().map_err(js_error)?,
            graphs: self.product.analysis().to_vec(),
            diagnostics: self.diagnostics.clone(),
        };
        serde_json::to_string(&view).map_err(|e| JsError::new(&e.to_string()))
    }

    pub fn select(&mut self, expected_commit: &str, action: &str) -> Result<String, JsError> {
        self.session
            .select(expected_commit, action)
            .map_err(js_error)?;
        self.inspect()
    }

    pub fn checkout(&mut self, commit: &str) -> Result<String, JsError> {
        self.session.checkout(commit).map_err(js_error)?;
        self.inspect()
    }

    pub fn save(&self) -> Result<String, JsError> {
        self.session.save().map_err(js_error)
    }

    pub fn restore(&mut self, save: &str) -> Result<String, JsError> {
        let restored = Session::restore(self.product.clone(), save).map_err(js_error)?;
        self.session = restored;
        self.inspect()
    }

    pub fn source(&self) -> Result<String, JsError> {
        serde_json::to_string_pretty(self.product.source())
            .map_err(|e| JsError::new(&e.to_string()))
    }
}

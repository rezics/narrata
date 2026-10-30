use crate::{Publication, js_error, json, parse};
use narrata_authoring::{maps::reader_map, relations::ReaderRelationAllowlist};
use narrata_nodes::history::visits::CheckedVisits;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct Visits {
    pub(crate) inner: CheckedVisits,
}
#[wasm_bindgen]
impl Visits {
    pub fn report(&self) -> Result<String, JsError> {
        json(self.inner.report())
    }
    #[wasm_bindgen(js_name = readerMap)]
    pub fn reader_map(&self, publication: &Publication) -> Result<String, JsError> {
        json(&reader_map(publication.inner.relations(), &self.inner).map_err(js_error)?)
    }
    pub fn merge(self, other: Visits) -> Result<Visits, JsError> {
        Ok(Visits {
            inner: CheckedVisits::merge([self.inner, other.inner]).map_err(js_error)?,
        })
    }
    #[wasm_bindgen(js_name = readerRelations)]
    pub fn reader_relations(&self, publication: &Publication) -> Result<ReaderRelations, JsError> {
        Ok(ReaderRelations {
            inner: ReaderRelationAllowlist::from_visits(publication.inner.relations(), &self.inner)
                .map_err(js_error)?,
        })
    }
}

#[wasm_bindgen]
pub struct ReaderRelations {
    inner: ReaderRelationAllowlist,
}
#[wasm_bindgen]
impl ReaderRelations {
    #[wasm_bindgen(getter, js_name = artifactId)]
    pub fn artifact_id(&self) -> String {
        self.inner.artifact_id().to_string()
    }
    #[wasm_bindgen(getter, js_name = indexId)]
    pub fn index_id(&self) -> String {
        self.inner.index_id().to_string()
    }
    #[wasm_bindgen(getter)]
    pub fn count(&self) -> usize {
        self.inner.rows().len()
    }
    pub fn rows(&self, offset: f64, limit: f64) -> Result<String, JsError> {
        json(&self.page(offset, limit)?)
    }
    #[wasm_bindgen(js_name = checkRows)]
    pub fn check_rows(&self, offset: f64, limit: f64, rows: &str) -> Result<(), JsError> {
        let expected = self.page(offset, limit)?;
        let rows: Vec<narrata_authoring::relations::RelationRow> = parse(rows)?;
        if rows != expected {
            return Err(JsError::new(
                "publication at reader relations: database result differs from allowlist",
            ));
        }
        Ok(())
    }
}
impl ReaderRelations {
    fn page(
        &self,
        offset: f64,
        limit: f64,
    ) -> Result<&[narrata_authoring::relations::RelationRow], JsError> {
        let (offset, limit) = crate::bounds::page(offset, limit).map_err(js_error)?;
        if offset > self.inner.rows().len() {
            return Err(JsError::new(
                "publication at reader relations: invalid page bounds",
            ));
        }
        Ok(&self.inner.rows()[offset..offset.saturating_add(limit).min(self.inner.rows().len())])
    }
}

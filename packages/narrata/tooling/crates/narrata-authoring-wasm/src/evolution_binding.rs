use narrata_authoring::{CheckedEvolution, PlannedEvolution, plan_evolution};
use narrata_nodes::{
    ArtifactId, ExecutionId,
    history::evolution::{MigrationId, PlanV1},
};
use narrata_storage_host::{FlushReply, HostError, Loaded};
use wasm_bindgen::prelude::*;

use crate::{
    Publication,
    evolution::{ComparisonBudgetV1, Engine},
    js_error, json, parse,
};

#[wasm_bindgen]
pub struct EvolutionPlan {
    inner: PlannedEvolution,
}
#[wasm_bindgen]
impl EvolutionPlan {
    pub fn report(&self) -> Result<String, JsError> {
        json(&self.inner.report())
    }
    pub fn bytes(&self) -> Option<Vec<u8>> {
        match &self.inner {
            PlannedEvolution::Exact(_) => None,
            PlannedEvolution::Descriptor(d) => Some(d.bytes()),
        }
    }
}

#[wasm_bindgen(js_name = planEvolution)]
pub fn plan(
    previous: &Publication,
    candidate: &Publication,
    declarations: Option<String>,
) -> Result<EvolutionPlan, JsError> {
    Ok(EvolutionPlan {
        inner: plan_evolution(&previous.inner, &candidate.inner, declarations.as_deref())
            .map_err(js_error)?,
    })
}
#[wasm_bindgen(js_name = readEvolution)]
pub fn read(
    bytes: &[u8],
    id: &str,
    previous: &Publication,
    candidate: &Publication,
) -> Result<EvolutionPlan, JsError> {
    let id: MigrationId = parse_id(id)?;
    Ok(EvolutionPlan {
        inner: PlannedEvolution::Descriptor(
            CheckedEvolution::read(bytes, id, &previous.inner, &candidate.inner)
                .map_err(js_error)?,
        ),
    })
}
fn parse_id<T: std::str::FromStr>(id: &str) -> Result<T, JsError> {
    id.parse()
        .map_err(|_| JsError::new("identifier at evolution: invalid identifier"))
}

#[wasm_bindgen(js_name = evolutionSampleKey)]
pub fn sample_key(artifact: &str, commit: &str) -> Result<Vec<u8>, JsError> {
    Ok(
        narrata_authoring::evolution::impact_sample_key(parse_id(artifact)?, parse_id(commit)?)
            .to_vec(),
    )
}
impl EvolutionPlan {
    pub(crate) fn attach(
        &self,
        candidate: Publication,
        previous: &Publication,
    ) -> Result<Publication, JsError> {
        match &self.inner {
            PlannedEvolution::Descriptor(d) => Ok(Publication {
                inner: candidate
                    .inner
                    .with_evolution(&previous.inner, d)
                    .map_err(js_error)?,
            }),
            PlannedEvolution::Exact(id)
                if candidate.inner.program().artifact_id() == *id
                    && previous.inner.program().artifact_id() == *id =>
            {
                Ok(candidate)
            }
            _ => Err(JsError::new(
                "artifact at evolution: exact plan artifacts differ",
            )),
        }
    }
}

#[wasm_bindgen]
#[derive(Default)]
pub struct Evolution {
    inner: Engine,
}
#[wasm_bindgen]
impl Evolution {
    pub fn visits(&mut self, budget: &str) -> Result<crate::maps_binding::Visits, JsError> {
        Ok(crate::maps_binding::Visits {
            inner: self.inner.visits(parse(budget)?).map_err(js_error)?,
        })
    }
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self::default()
    }
    pub fn fork(&self) -> Self {
        Self {
            inner: self.inner.fork(),
        }
    }
    #[wasm_bindgen(js_name = registerPublication)]
    pub fn register_publication(&mut self, publication: &Publication) -> Result<(), JsError> {
        self.inner
            .register_publication(&publication.inner)
            .map_err(js_error)
    }
    #[wasm_bindgen(js_name = registerEvolution)]
    pub fn register_evolution(
        &mut self,
        previous: &Publication,
        candidate: &Publication,
    ) -> Result<String, JsError> {
        Ok(self
            .inner
            .register_evolution(&previous.inner, &candidate.inner)
            .map_err(js_error)?
            .to_string())
    }
    pub fn restore(&mut self, save: &str) -> Result<(), JsError> {
        self.inner.restore(save).map_err(js_error)
    }
    pub fn connect(&mut self, execution: &str) -> Result<(), JsError> {
        self.inner.connect(parse_id::<ExecutionId>(execution)?);
        Ok(())
    }
    pub fn refresh(&mut self) {
        self.inner.refresh();
    }
    pub fn assess(&mut self, target: &str, ids: &str, budget: &str) -> Result<String, JsError> {
        let target = parse_id::<ArtifactId>(target)?;
        let ids: Vec<MigrationId> = parse(ids)?;
        let budget: ComparisonBudgetV1 = parse(budget)?;
        json(&self.inner.assess(target, &ids, budget).map_err(js_error)?)
    }
    #[wasm_bindgen(js_name = assessSave)]
    pub fn assess_save(
        &mut self,
        save: &str,
        target: &str,
        ids: &str,
        budget: &str,
    ) -> Result<String, JsError> {
        let target = parse_id::<ArtifactId>(target)?;
        let ids: Vec<MigrationId> = parse(ids)?;
        let budget: ComparisonBudgetV1 = parse(budget)?;
        json(
            &self
                .inner
                .assess_save(save, target, &ids, budget)
                .map_err(js_error)?,
        )
    }
    /// This is the explicit application call; assessing never applies a plan.
    pub fn apply(&mut self, plan: &str, budget: &str) -> Result<String, JsError> {
        let plan: PlanV1 = parse(plan)?;
        let budget: ComparisonBudgetV1 = parse(budget)?;
        json(&self.inner.apply(&plan, budget).map_err(js_error)?)
    }
    pub fn export(&mut self) -> Result<String, JsError> {
        self.inner.export().map_err(js_error)
    }
    #[wasm_bindgen(js_name = exportShallow)]
    pub fn export_shallow(&mut self, ancestors: u32) -> Result<Vec<u8>, JsError> {
        self.inner.export_shallow(ancestors).map_err(js_error)
    }
    #[wasm_bindgen(js_name = takeRequest)]
    pub fn take_request(&mut self) -> Option<Vec<u8>> {
        self.inner.take_request().map(|r| r.encode())
    }
    pub fn load(&mut self, bytes: &[u8]) -> Result<bool, JsError> {
        settle(self.inner.cache.load(&Loaded::decode(bytes)?))
    }
    pub fn unconfirmed(&self) -> Option<Vec<u8>> {
        self.inner.cache.unconfirmed().map(|r| r.encode())
    }
    pub fn confirm(&mut self, bytes: &[u8]) -> Result<bool, JsError> {
        settle(self.inner.cache.confirm(&FlushReply::decode(bytes)?))
    }
}
fn settle(result: Result<(), HostError>) -> Result<bool, JsError> {
    match result {
        Ok(()) => Ok(true),
        Err(HostError::Superseded(_)) => Ok(false),
        Err(error) => Err(JsError::new(&error.to_string())),
    }
}

use std::collections::BTreeMap;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReferenceOp {
    Set {
        key: String,
        value: i64,
        next: usize,
    },
    Jump {
        target: usize,
    },
    Await {
        next: usize,
    },
    Finish,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ReferenceState {
    pub instruction: usize,
    pub values: BTreeMap<String, i64>,
    pub awaiting: bool,
    pub finished: bool,
}

pub fn reduce(program: &[ReferenceOp], state: &ReferenceState) -> Result<ReferenceState, String> {
    let mut next = state.clone();
    if next.finished {
        return Err("reference execution already finished".to_owned());
    }
    if next.awaiting {
        next.awaiting = false;
    }
    for _ in 0..=program.len() {
        let op = program
            .get(next.instruction)
            .ok_or_else(|| "reference instruction is missing".to_owned())?;
        match op {
            ReferenceOp::Set {
                key,
                value,
                next: target,
            } => {
                next.values.insert(key.clone(), *value);
                next.instruction = *target;
            }
            ReferenceOp::Jump { target } => next.instruction = *target,
            ReferenceOp::Await { next: target } => {
                next.instruction = *target;
                next.awaiting = true;
                return Ok(next);
            }
            ReferenceOp::Finish => {
                next.finished = true;
                return Ok(next);
            }
        }
    }
    Err("reference macrostep limit exceeded".to_owned())
}

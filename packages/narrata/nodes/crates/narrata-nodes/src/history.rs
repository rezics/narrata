//! Registration of the existing node object bytes in the kernel history layer.

pub mod evolution;
pub mod locator;
pub mod maintenance;
#[cfg(feature = "trace")]
pub mod visits;

use std::sync::Arc;

use narrata_history::{Domain, HistoryError};
use narrata_kernel::codec::DecodeError;

use crate::{Error, Input, Program, State, decode_state, state::MAX_INPUT_BYTES, wire};

#[derive(Clone, Debug)]
pub struct NodeDomain(pub Arc<Program>);

impl Domain for NodeDomain {
    const COMMIT_KIND: u16 = wire::KIND_COMMIT;
    const STATE_KIND: u16 = wire::KIND_STATE;
    const STATE_SCHEMA: u16 = wire::SCHEMA;
    const INPUT_KIND: u16 = wire::KIND_INPUT;
    const INPUT_SCHEMA: u16 = wire::SCHEMA;
    type State = State;
    type Input = Input;

    fn artifact_id(&self) -> narrata_history::ArtifactId {
        narrata_history::ArtifactId::from_bytes(*self.0.artifact_id().as_bytes())
    }

    fn encode_state(&self, state: &State) -> Vec<u8> {
        state.encode()
    }

    fn state_schema(&self, state: &State) -> u16 {
        state.schema()
    }

    fn supports_state_schema(schema: u16) -> bool {
        matches!(schema, wire::SCHEMA | wire::VISUAL_NOVEL_SCHEMA)
    }

    fn decode_state(&self, payload: &[u8]) -> Result<State, DecodeError> {
        let schema = if self.0.is_visual_novel() {
            wire::VISUAL_NOVEL_SCHEMA
        } else {
            wire::SCHEMA
        };
        self.decode_state_schema(schema, payload)
    }

    fn decode_state_schema(&self, schema: u16, payload: &[u8]) -> Result<State, DecodeError> {
        let expected = if self.0.is_visual_novel() {
            wire::VISUAL_NOVEL_SCHEMA
        } else {
            wire::SCHEMA
        };
        if schema != expected {
            return Err(DecodeError::Schema("state schema does not match artifact"));
        }
        decode_state(
            &self.0,
            &wire::seal_schema(wire::KIND_STATE, schema, payload).1,
        )
        .map(|(state, _)| state)
        .map_err(|_| DecodeError::Schema("invalid node state"))
    }

    fn encode_input(&self, input: &Input) -> Vec<u8> {
        input.encode()
    }

    fn input_schema(&self, input: &Input) -> u16 {
        input.schema()
    }

    fn supports_input_schema(schema: u16) -> bool {
        matches!(schema, wire::SCHEMA | wire::VISUAL_NOVEL_SCHEMA)
    }

    fn decode_input(&self, payload: &[u8]) -> Result<Input, DecodeError> {
        self.decode_input_schema(wire::SCHEMA, payload)
    }

    fn decode_input_schema(&self, schema: u16, payload: &[u8]) -> Result<Input, DecodeError> {
        if schema == wire::VISUAL_NOVEL_SCHEMA && !self.0.is_visual_novel() {
            return Err(DecodeError::Schema("input schema does not match artifact"));
        }
        // Domain has no parent context. The runtime and legacy importer additionally check
        // choices and proposal identities against their parent commit and state.
        let envelope = wire::seal_schema(wire::KIND_INPUT, schema, payload).1;
        let (input, _) = wire::open_schema(
            &envelope,
            wire::KIND_INPUT,
            schema,
            MAX_INPUT_BYTES,
            "input",
            |r| {
                if schema == wire::VISUAL_NOVEL_SCHEMA {
                    wire::decode_input_schema2(r)
                } else {
                    wire::decode_input_schema1(r)
                }
            },
            |input| wire::encode_input_schema(input, schema),
        )
        .map_err(|_| DecodeError::Schema("invalid node input"))?;
        if input.schema() != schema {
            return Err(DecodeError::Schema("input schema does not match payload"));
        }
        Ok(input)
    }
}

pub fn history_error(error: HistoryError) -> Error {
    if let HistoryError::MissingObject(id) = &error {
        return Error::new(
            "needs_object",
            id.to_string(),
            "required history object is unavailable",
        );
    }
    let code = match &error {
        HistoryError::HeadMoved { .. } | HistoryError::RefConflict(_) => "stale_input",
        HistoryError::ArtifactMismatch { .. } => "incompatible_save",
        HistoryError::HistoryTruncated(_) => "history_truncated",
        HistoryError::UnregisteredKind { .. } => "unregistered_kind",
        HistoryError::Corrupt(..) | HistoryError::CorruptStore(_) => "corrupt_save",
        HistoryError::Limit(_) | HistoryError::Storage(narrata_storage::StorageError::Limit(_)) => {
            "limit"
        }
        _ => "history",
    };
    Error::new(code, "history", error.to_string())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use crate::{Commit, Session, SessionExport, decode_input, runtime::Machine};
    use std::collections::BTreeMap;

    #[test]
    fn migrated_views_match_execution_of_the_original_frozen_objects() {
        let root =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../../fixtures/compat");
        for (directory, save) in [
            ("nodes-r2", "linear.export.json"),
            ("nodes-r2", "branched.export.json"),
            ("nodes-r2-proposals", "session.export.json"),
        ] {
            let (program, names) = Program::from_pack(
                &std::fs::read(root.join(directory).join("story.narpack")).unwrap(),
            )
            .unwrap();
            let program = Arc::new(program);
            let machine = Machine { program: &program };
            let text = std::fs::read_to_string(root.join(directory).join(save)).unwrap();
            let export: SessionExport = serde_json::from_str(&text).unwrap();
            let mut session = Session::restore(program.clone(), &text).unwrap();
            let mut states = BTreeMap::new();
            let mut inputs = BTreeMap::new();
            let mut commits = Vec::new();
            for bytes in export.objects.iter().map(|text| hex::decode(text).unwrap()) {
                if let Ok((state, id)) = decode_state(&program, &bytes) {
                    states.insert(id, state);
                } else if let Ok((commit, id)) = Commit::decode_interim(&bytes) {
                    commits.push((id, commit));
                } else {
                    inputs.insert(wire::id_of(wire::KIND_INPUT, &bytes[56..]), bytes);
                }
            }
            let originals: BTreeMap<_, _> = commits.iter().cloned().collect();
            let mut ids = BTreeMap::new();
            for (old_id, original) in &commits {
                let mut converted = original.clone();
                converted.parent = original.parent.map(|parent| ids[&parent]);
                ids.insert(*old_id, converted.id());
            }
            for (old_id, header) in commits {
                let state = &states[&header.state];
                let step = match (header.parent, header.input) {
                    (Some(parent), Some(input)) => {
                        let parent_state = &states[&originals[&parent].state];
                        let (input, _) =
                            decode_input(&program, &parent, parent_state, &inputs[&input]).unwrap();
                        machine.apply(&parent, parent_state, &input).unwrap()
                    }
                    _ => machine.initial().unwrap(),
                };
                assert_eq!(&step.state, state);
                session.checkout(&ids[&old_id]).unwrap();
                let view = session.view(names.as_ref()).unwrap();
                assert_eq!(
                    view.interaction,
                    machine.interaction(state, names.as_ref()).unwrap()
                );
                let shown: Vec<_> = view
                    .presentation
                    .iter()
                    .map(|p| (p.role, p.node, p.content.clone(), p.args.clone()))
                    .collect();
                let original: Vec<_> = step
                    .presentation
                    .into_iter()
                    .map(|p| (p.role, p.node, p.content, p.args))
                    .collect();
                assert_eq!(shown, original);
                assert_eq!(view.frames.len(), state.frames.len());
                for (frame, old) in view.frames.iter().zip(&state.frames) {
                    assert_eq!(
                        (&frame.graph, frame.node, frame.at, frame.instance),
                        (&old.graph, old.node, old.at, old.instance)
                    );
                    assert_eq!(
                        frame
                            .locals
                            .iter()
                            .map(|v| (v.name.clone(), v.value.clone()))
                            .collect::<BTreeMap<_, _>>(),
                        old.locals
                            .iter()
                            .map(|(name, value)| (name.clone(), value.view()))
                            .collect()
                    );
                }
                for entry in &view.history {
                    let old = ids.iter().find(|(_, new)| **new == entry.id).unwrap().0;
                    let header = &originals[old];
                    let old_state = &states[&header.state];
                    let (graph, node, instance, title) = machine.title(old_state).unwrap();
                    assert_eq!(
                        (&entry.graph, entry.node, entry.instance, &entry.title),
                        (&graph, node, instance, &title)
                    );
                    assert_eq!(entry.parent, header.parent.map(|id| ids[&id]));
                }
            }
        }
    }
}

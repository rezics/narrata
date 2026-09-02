use std::{str::FromStr, sync::Arc};

use narrata_core::{
    ChoiceId, EventTypeId, ExecutionId, InputId,
    runtime::{
        CheckedRuntimeInput, PendingInteractionV0, RuntimeStateV0, SliceBudget, new_execution,
    },
};
use narrata_testkit::backend::{ConformanceBackend, NativeBackend};

pub(crate) fn program(args: &[String]) -> Result<(), String> {
    let artifact_path = args.first().ok_or_else(super::usage)?;
    let execution = option(args, "--execution").ok_or_else(super::usage)?;
    let inputs_path = option(args, "--inputs").ok_or_else(super::usage)?;
    let bytes = super::read_bytes(artifact_path)?;
    let checked = narrata_core::load_program(&bytes, &Default::default())
        .map_err(|error| error.to_string())?;
    let execution = ExecutionId::from_str(execution).map_err(|error| error.to_string())?;
    let mut state =
        Arc::new(new_execution(&checked, execution).map_err(|error| error.to_string())?);
    let input_json: serde_json::Value = serde_json::from_slice(
        &std::fs::read(inputs_path)
            .map_err(|error| format!("cannot read {inputs_path}: {error}"))?,
    )
    .map_err(|error| error.to_string())?;
    let inputs = input_json
        .as_array()
        .ok_or_else(|| "trace JSON root must be an array".to_owned())?;
    let trace_enabled = args.iter().any(|arg| arg == "--trace");
    let json_enabled = args.iter().any(|arg| arg == "--json");
    let mut states = Vec::new();
    let mut receipts = Vec::new();
    for input_json in inputs {
        let input = resolve_input(input_json, &state)?;
        let draft = NativeBackend
            .transition(
                checked.clone(),
                state.clone(),
                input,
                Default::default(),
                SliceBudget::unlimited(),
            )
            .map_err(|error| error.to_string())?;
        if trace_enabled {
            for event in draft.trace() {
                println!(
                    "turn={} flow={} instruction={} opcode={} frames={} stack={} status={} state={} receipt={}",
                    event.turn,
                    event.flow,
                    event.instruction,
                    event.opcode,
                    event.frame_depth,
                    event.stack_depth,
                    event.status,
                    event
                        .state_digest
                        .map_or_else(|| "-".to_owned(), |value| value.to_string()),
                    event
                        .receipt_digest
                        .map_or_else(|| "-".to_owned(), |value| value.to_string()),
                );
            }
            for event in draft.statechart_trace() {
                println!(
                    "microstep={} kind={:?} event={} state={} transition={} action={}",
                    event.microstep,
                    event.kind,
                    event
                        .event
                        .map_or_else(|| "-".to_owned(), |value| value.to_string()),
                    event
                        .state
                        .map_or_else(|| "-".to_owned(), |value| value.to_string()),
                    event
                        .transition
                        .map_or_else(|| "-".to_owned(), |value| value.to_string()),
                    event
                        .action
                        .map_or_else(|| "-".to_owned(), |value| value.to_string()),
                );
            }
        }
        states.push(draft.next_state_digest().to_string());
        receipts.push(draft.receipt_digest().to_string());
        state = Arc::new(draft.next_state().clone());
    }
    let status = status_name(&state);
    if json_enabled {
        let value = serde_json::json!({
            "status": status,
            "state_digests": states,
            "receipt_digests": receipts,
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&value).map_err(|error| error.to_string())?
        );
    } else {
        println!("{status}");
    }
    Ok(())
}

fn resolve_input(
    value: &serde_json::Value,
    state: &RuntimeStateV0,
) -> Result<CheckedRuntimeInput, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "trace input must be an object".to_owned())?;
    let kind = string_field(object, "kind")?;
    let request = InputId::from_str(string_field(object, "request_id")?)
        .map_err(|error| error.to_string())?;
    match kind {
        "start" => Ok(CheckedRuntimeInput::start(request)),
        "advance" => {
            let interaction = state
                .pending()
                .map(PendingInteractionV0::interaction_id)
                .ok_or_else(|| "no pending interaction to advance".to_owned())?;
            Ok(CheckedRuntimeInput::advance(request, interaction))
        }
        "select" => {
            let choice = ChoiceId::from_str(string_field(object, "choice_id")?)
                .map_err(|error| error.to_string())?;
            let interaction = match state.pending() {
                Some(PendingInteractionV0::Choice {
                    interaction_id,
                    offered,
                    ..
                }) if offered.iter().any(|item| item.id == choice) => *interaction_id,
                _ => return Err("choice was not offered by the pending interaction".to_owned()),
            };
            Ok(CheckedRuntimeInput::select(request, interaction, choice))
        }
        "event" => EventTypeId::from_str(string_field(object, "event_type_id")?)
            .map(|event| CheckedRuntimeInput::event(request, event))
            .map_err(|error| error.to_string()),
        _ => Err(format!("unknown trace input kind '{kind}'")),
    }
}

fn string_field<'a>(
    object: &'a serde_json::Map<String, serde_json::Value>,
    field: &str,
) -> Result<&'a str, String> {
    object
        .get(field)
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| format!("trace input requires string field '{field}'"))
}

fn status_name(state: &RuntimeStateV0) -> &'static str {
    match state.status {
        narrata_core::runtime::RuntimeStatusV0::Ready { .. } => "ready",
        narrata_core::runtime::RuntimeStatusV0::Awaiting { .. } => "awaiting",
        narrata_core::runtime::RuntimeStatusV0::AwaitingEffect { .. } => "awaiting-effect",
        narrata_core::runtime::RuntimeStatusV0::Finished { .. } => "finished",
        narrata_core::runtime::RuntimeStatusV0::StatechartStable => "statechart-stable",
        narrata_core::runtime::RuntimeStatusV0::AwaitingStatechartEffect { .. } => {
            "awaiting-statechart-effect"
        }
        narrata_core::runtime::RuntimeStatusV0::StatechartFinished => "statechart-finished",
    }
}

fn option<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.windows(2)
        .find(|pair| pair.first().is_some_and(|value| value == name))
        .and_then(|pair| pair.get(1))
        .map(String::as_str)
}

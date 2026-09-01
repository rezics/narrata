pub(crate) fn program(args: &[String]) -> Result<(), String> {
    let path = args.first().ok_or_else(super::usage)?;
    let bytes = super::read_bytes(path)?;
    let checked = narrata_core::load_program(&bytes, &Default::default())
        .map_err(|error| error.to_string())?;
    let artifact = checked.artifact();
    let instruction_count: usize = artifact
        .flows
        .iter()
        .map(|flow| flow.instructions.len())
        .sum();
    if args.iter().any(|arg| arg == "--json") {
        let value = serde_json::json!({
            "artifact_id": checked.artifact_id().to_string(),
            "program_id": artifact.program_id.to_string(),
            "format_version": artifact.format_version.get(),
            "semantics_version": artifact.semantics_version.get(),
            "entry_flow": artifact.entry_flow.to_string(),
            "constants": artifact.constants.len(),
            "globals": artifact.globals.len(),
            "flows": artifact.flows.len(),
            "instructions": instruction_count,
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&value).map_err(|error| error.to_string())?
        );
    } else {
        println!("artifact: {}", checked.artifact_id());
        println!("program: {}", artifact.program_id);
        println!(
            "format/semantics: {}/{}",
            artifact.format_version.get(),
            artifact.semantics_version.get()
        );
        println!(
            "flows/instructions: {}/{}",
            artifact.flows.len(),
            instruction_count
        );
    }
    Ok(())
}

pub(crate) fn snapshot(args: &[String]) -> Result<(), String> {
    let snapshot_path = args.first().ok_or_else(super::usage)?;
    let program_path = option(args, "--program").ok_or_else(super::usage)?;
    let program_bytes = super::read_bytes(program_path)?;
    let program = narrata_core::load_program(&program_bytes, &Default::default())
        .map_err(|error| error.to_string())?;
    let snapshot_bytes = super::read_bytes(snapshot_path)?;
    let state = narrata_core::restore_snapshot(&snapshot_bytes, &program, &Default::default())
        .map_err(|error| error.to_string())?;
    let status = match &state.status {
        narrata_core::runtime::RuntimeStatusV0::Ready { .. } => "ready",
        narrata_core::runtime::RuntimeStatusV0::Awaiting { pending, .. } => match pending {
            narrata_core::runtime::PendingInteractionV0::Say { .. } => "awaiting-say",
            narrata_core::runtime::PendingInteractionV0::Choice { .. } => "awaiting-choice",
        },
        narrata_core::runtime::RuntimeStatusV0::AwaitingEffect { .. } => "awaiting-effect",
        narrata_core::runtime::RuntimeStatusV0::Finished { .. } => "finished",
    };
    let value = serde_json::json!({
        "program_artifact_id": state.program_artifact_id.to_string(),
        "execution_id": state.execution_id.to_string(),
        "turn": state.turn.0,
        "status": status,
        "state_digest": narrata_core::snapshot::state_digest(&state).to_string(),
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&value).map_err(|error| error.to_string())?
    );
    Ok(())
}

fn option<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.windows(2)
        .find(|pair| pair.first().is_some_and(|value| value == name))
        .and_then(|pair| pair.get(1))
        .map(String::as_str)
}

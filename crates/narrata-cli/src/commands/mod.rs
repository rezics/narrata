mod conformance;
mod inspect;
mod replay;
mod run;
mod timeline;
mod validate;

pub(crate) fn run(args: Vec<String>) -> Result<(), String> {
    match args.as_slice() {
        [group, command, rest @ ..] if group == "program" && command == "validate" => {
            validate::program(rest)
        }
        [group, command, rest @ ..] if group == "program" && command == "inspect" => {
            inspect::program(rest)
        }
        [group, command, rest @ ..] if group == "snapshot" && command == "inspect" => {
            inspect::snapshot(rest)
        }
        [command, rest @ ..] if command == "run" => run::program(rest),
        [command, rest @ ..] if command == "replay" => replay::fixture(rest),
        [command, rest @ ..] if command == "conformance" => conformance::directory(rest),
        [group, command, rest @ ..] if group == "timeline" => timeline::run(command, rest),
        _ => Err(usage()),
    }
}

fn usage() -> String {
    "usage:\n  narrata program validate <artifact>\n  narrata program inspect <artifact> [--json]\n  narrata run <artifact> --execution <execution-id> --inputs <trace.json> [--trace] [--json]\n  narrata snapshot inspect <snapshot> --program <artifact>\n  narrata replay <fixture.json>\n  narrata conformance <fixture-directory>\n  narrata timeline log <store> --execution <execution-id>\n  narrata timeline rewind|redo <store> --program <artifact> --execution <id> --session <name> --branch <id> --steps <n>\n  narrata timeline fork <store> --program <artifact> --execution <id> --session <name> --branch <selected> --new-branch <id> --operation <id>\n  narrata timeline bookmark <store> --program <artifact> --execution <id> --session <name> --branch <id> --owner <name> --name <name> --operation <id>".to_owned()
}

pub(crate) fn read_bytes(path: &str) -> Result<Vec<u8>, String> {
    let bytes = std::fs::read(path).map_err(|error| format!("cannot read {path}: {error}"))?;
    if path.ends_with(".hex") {
        let text =
            std::str::from_utf8(&bytes).map_err(|_| format!("hex fixture {path} is not UTF-8"))?;
        hex::decode(text.trim()).map_err(|error| format!("invalid hex fixture {path}: {error}"))
    } else {
        Ok(bytes)
    }
}

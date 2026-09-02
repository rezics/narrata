$ErrorActionPreference = "Stop"
$PSNativeCommandUseErrorActionPreference = $true

./scripts/check-g3.ps1
cargo test --release -p narrata-testkit --test statecharts --test statechart_properties --test golden_codec
cargo test --release -p narrata-store --test statecharts
cargo test --release -p narrata-store-sqlite --test sqlite statechart_invocation_and_pending_effect_restore_after_reopen
cargo check --manifest-path fuzz/Cargo.toml --bin decode_stage4_objects

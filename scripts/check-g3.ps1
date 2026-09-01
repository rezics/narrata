$ErrorActionPreference = "Stop"
$PSNativeCommandUseErrorActionPreference = $true

./scripts/check-g2.ps1
cargo test --release -p narrata-core --test effects
cargo test --release -p narrata-store --test effects --test federated --test model
cargo test --release -p narrata-store-sqlite --test sqlite
cargo check --manifest-path fuzz/Cargo.toml --bin decode_stage3_objects

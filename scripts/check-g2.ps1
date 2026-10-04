$ErrorActionPreference = "Stop"
$PSNativeCommandUseErrorActionPreference = $true

./scripts/check-g1.ps1
cargo test -p narrata-store -p narrata-store-sqlite --all-targets
cargo test --release -p narrata-store --test golden --test negative_bundle
cargo check --manifest-path fuzz/Cargo.toml --bin decode_bundles
cargo check -p narrata-storage --target wasm32-unknown-unknown

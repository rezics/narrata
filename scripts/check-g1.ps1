$ErrorActionPreference = "Stop"
$PSNativeCommandUseErrorActionPreference = $true

cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
cargo test --workspace --doc
cargo test --release -p narrata-testkit --test golden_codec --test fixtures
cargo check -p narrata-core --target wasm32-wasip1
cargo check -p narrata-testkit --bin narrata-conformance --target wasm32-wasip1
cargo check --manifest-path fuzz/Cargo.toml

if (Get-Command cargo-deny -ErrorAction SilentlyContinue) {
    cargo deny check
} else {
    Write-Warning "cargo-deny is not installed; CI still enforces cargo deny check."
}

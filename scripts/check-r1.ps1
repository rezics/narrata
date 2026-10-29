$ErrorActionPreference = "Stop"
$PSNativeCommandUseErrorActionPreference = $true

if ($IsWindows -and (Get-NetTCPConnection -State Listen -LocalPort 4173 -ErrorAction SilentlyContinue)) {
    throw "Stop the local Gamebook dev server before check-r1: npm ci must replace Windows native modules."
}

cargo fmt --all --check
cargo clippy -p narrata-nodes -p narrata-nodes-wasm -p narrata-node-tools -p narrata-content-local -p narrata-graph -p narrata-authoring -p narrata-authoring-wasm --all-targets --all-features -- -D warnings
# The reader Wasm build omits the legacy publishGraph feature; lint that configuration on its own.
cargo clippy -p narrata-nodes-wasm --no-default-features --all-targets -- -D warnings
cargo test -p narrata-nodes -p narrata-nodes-wasm -p narrata-node-tools -p narrata-content-local -p narrata-graph -p narrata-authoring -p narrata-authoring-wasm
cargo test -p narrata-nodes --all-features
cargo check -p narrata-nodes-wasm --target wasm32-unknown-unknown
cargo check -p narrata-nodes-wasm --no-default-features --target wasm32-unknown-unknown
cargo check -p narrata-authoring --target wasm32-unknown-unknown
cargo check -p narrata-authoring-wasm --target wasm32-unknown-unknown

Push-Location examples/gamebook-web
try {
    npm ci
    npm audit --audit-level=high
    npm run build
    npx playwright install --with-deps chromium
    npm run test:e2e
} finally {
    Pop-Location
}

# The product directory includes analysis/summary JSON and object-id-named graph/label CBOR.
# Generated files must be committed as generated: changed and new untracked files both count.
$drift = git status --porcelain --untracked-files=all -- packages/narrata/nodes/schemas packages/narrata/tooling/crates/narrata-authoring-wasm/schemas packages/narrata/web/src/generated examples/gamebook-web/src/generated products/gamebook-demo
if ($drift) {
    $drift
    throw "Generated files differ from the committed ones; regenerate and commit them."
}

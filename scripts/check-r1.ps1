$ErrorActionPreference = "Stop"
$PSNativeCommandUseErrorActionPreference = $true

if ($IsWindows -and (Get-NetTCPConnection -State Listen -LocalPort 4173 -ErrorAction SilentlyContinue)) {
    throw "Stop the local Gamebook dev server before check-r1: npm ci must replace Windows native modules."
}

cargo fmt --all --check
cargo clippy -p narrata-nodes -p narrata-nodes-wasm -p narrata-node-tools -p narrata-content-local -p narrata-graph --all-targets --all-features -- -D warnings
cargo test -p narrata-nodes -p narrata-nodes-wasm -p narrata-node-tools -p narrata-content-local -p narrata-graph
cargo check -p narrata-nodes-wasm --target wasm32-unknown-unknown

Push-Location examples/gamebook-web
try {
    npm ci
    npm audit --audit-level=high
    npm run build
    npx playwright install chromium
    npm run test:e2e
} finally {
    Pop-Location
}

# The product directory includes analysis/summary JSON and object-id-named graph/label CBOR.
# Generated files must be committed as generated: changed and new untracked files both count.
$drift = git status --porcelain --untracked-files=all -- packages/narrata/nodes/schemas examples/gamebook-web/src/generated products/gamebook-demo
if ($drift) {
    $drift
    throw "Generated files differ from the committed ones; regenerate and commit them."
}

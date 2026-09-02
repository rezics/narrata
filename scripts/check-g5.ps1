$ErrorActionPreference = "Stop"
$PSNativeCommandUseErrorActionPreference = $true

./scripts/check-g4.ps1

cargo test --release -p narrata-core --test authoring --test versioning
cargo test --release -p narrata-store --test compatibility --test migrations --test debugger
cargo test --release -p narrata-protocol --test conformance
cargo test --release -p narrata-ffi
cargo test --release -p narrata-wasm --test hash_conformance
cargo check --manifest-path fuzz/Cargo.toml --bin decode_stage5_protocol

if (-not (Get-Command rustup -ErrorAction SilentlyContinue)) {
    throw "G5 requires rustup and the wasm32-unknown-unknown target."
}
if (-not (rustup target list --installed | Select-String -Quiet '^wasm32-unknown-unknown$')) {
    throw "Install the browser target with: rustup target add wasm32-unknown-unknown"
}
cargo check -p narrata-wasm --target wasm32-unknown-unknown

if (-not (Get-Command dotnet -ErrorAction SilentlyContinue)) {
    throw "G5 requires the .NET 8 SDK for the C# binding conformance trace."
}
cargo build -p narrata-ffi
$nativeDirectory = (Resolve-Path target/debug).Path
$env:PATH = "$nativeDirectory$([IO.Path]::PathSeparator)$env:PATH"
dotnet run --project bindings/dotnet/Narrata.Conformance/Narrata.Conformance.csproj --configuration Release -- fixtures/compat/stage5-v0/program-v0.hex

if (-not (Get-Command npm -ErrorAction SilentlyContinue)) {
    throw "G5 requires Node.js and npm for generated TypeScript DTO and IndexedDB checks."
}
Push-Location bindings/typescript
try {
    npm ci
    npm audit --audit-level=high
    npm run generate:protocol
    npm run build
    npm test
} finally {
    Pop-Location
}
git diff --exit-code -- bindings/typescript/src/generated/narrata_pb.ts

$nativeClient = "bindings/unity/Narrata.Unity/Runtime/NarrataNativeClient.cs"
$webClient = "bindings/unity/Narrata.Unity/Runtime/NarrataWebClient.cs"
$webBridge = "bindings/unity/Narrata.Unity/Plugins/WebGL/Narrata.jslib"
if (-not (Select-String -Path $nativeClient -SimpleMatch '#if !UNITY_WEBGL || UNITY_EDITOR' -Quiet)) {
    throw "Unity native adapter is not excluded from WebGL players."
}
if (-not (Select-String -Path $webClient -SimpleMatch '#if UNITY_WEBGL && !UNITY_EDITOR' -Quiet)) {
    throw "Unity Web adapter is not isolated to WebGL players."
}
if (-not (Select-String -Path $webBridge -SimpleMatch 'globalThis.narrataWasmEngine' -Quiet)) {
    throw "Unity Web bridge is not connected to the Wasm protocol engine."
}
$editorAssembly = Get-Content bindings/unity/Narrata.Unity/Editor/Narrata.Unity.Editor.asmdef -Raw | ConvertFrom-Json
if ($editorAssembly.includePlatforms -notcontains "Editor") {
    throw "Unity Editor assembly is not restricted to the Editor."
}

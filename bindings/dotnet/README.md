# Narrata .NET binding

This package is for desktop and native Unity targets. `NarrataEngine` owns the native handle and
copies every response before releasing its native buffer, so application code cannot double-free
or retain native memory. Calls use the versioned Protobuf DTOs generated from Narrata's checked-in
schema; protocol 2 results name content references that the host resolves.

Unity WebGL must use the separate JavaScript/Wasm adapter under `bindings/unity`; desktop P/Invoke
is not available in a browser build.

Build the binding and run its native conformance trace from the repository root:

```powershell
cargo build -p narrata-ffi
$env:PATH = "$(Resolve-Path target/debug);$env:PATH"
dotnet run --project bindings/dotnet/Narrata.Conformance/Narrata.Conformance.csproj --configuration Release -- fixtures/compat/stage6-v0/program-v1.hex
```

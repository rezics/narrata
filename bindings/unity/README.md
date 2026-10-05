# Narrata Unity adapters

`NarrataNativeClient` is compiled for desktop/native players and calls the .NET binding. It is
excluded from a WebGL player. `NarrataWebClient` is compiled only for WebGL outside the Editor and
uses `Plugins/WebGL/Narrata.jslib` to copy protocol bytes through the Narrata Wasm engine.

`INarrataPresenter` renders committed protocol results, which name content references instead of
text; `INarrataContentResolver` is the host's batch resolver for them. `INarrataSceneReconciler`
applies a complete target scene, and `INarrataHostSaveCoordinator` coordinates the host snapshot
before publishing a matching narrative save. These interfaces do not own runtime continuation state.

The Editor assembly depends on the Runtime assembly and is restricted to the Unity Editor. A WebGL
build must provide `globalThis.narrataWasmEngine`; it must not ship or call the desktop native DLL.

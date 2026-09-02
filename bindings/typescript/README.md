# Narrata TypeScript binding

The checked-in TypeScript DTOs are generated from `crates/narrata-protocol/proto/narrata.proto`.
`NarrataProtocolCodec` connects them to `NarrataEngine`; the wrapper copies request and response
bytes at the Wasm boundary and exposes sliced execution as a Promise or async iterator.

`NarrataIndexedDbStore` commits immutable objects and compare-and-swap refs in one IndexedDB
transaction. Its limits reject oversized saves before opening a write transaction.

Regenerate, type-check, and test the package with:

```powershell
npm install
npm run generate:protocol
npm run build
npm test
```

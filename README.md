# Narrata

**Narrata is a universal narrative engine built in Rust.**

Narrata provides a language-independent runtime and data model for building interactive narratives, dialogue systems, visual novels, quests, cutscenes, branching stories, and other narrative-driven experiences.

At its core, Narrata represents narrative as executable state. Characters, events, conditions, choices, variables, transitions, and world state are modeled explicitly and executed by a deterministic runtime rather than being tied to a specific game engine or programming language.

The engine is designed to separate narrative logic from presentation and host-engine behavior. The same narrative can therefore be authored once and embedded into different environments, including Unity, Bevy, custom game engines, web applications, and desktop software.

Narrata is implemented in Rust and exposes portable interfaces for languages and runtimes such as C, C++, C#, JavaScript, WebAssembly, and others.

Rather than requiring every project to build its own dialogue system, branching logic, quest state, save-state handling, and narrative execution layer, Narrata aims to provide a shared foundation for interactive storytelling.

Its goal is broader than visual novels. Narrata is designed for any system where narrative emerges from evolving state, events, conditions, choices, and consequences.

**One narrative model. Any engine.**

## Documentation

The current architecture decisions, research notes, and phased implementation plan are indexed in
[docs/README.md](./docs/README.md).

## Stage 4 quick start

The deterministic Flow/Statechart runtime, local time-travel store, and crash-recoverable host
Effect protocol are implemented as a Rust 1.98 workspace. Run the complete local gate with:

```powershell
./scripts/check-g4.ps1
```

Validate the checked-in canonical Program vector or replay both conformance stories:

```powershell
cargo run -p narrata-cli -- program validate fixtures/codec/program-v0.cbor.hex
cargo run -p narrata-cli -- conformance fixtures/conformance
```

`narrata-store` now also provides capability negotiation, commit-before-dispatch, a monotonic Effect
ledger, recorded query recovery, rewind barriers, compensation links, declarative Scene reconcile,
and atomically published Compound Saves. `narrata-store-sqlite` persists the same checked model.
See [Stage 4](./docs/plan/stage-4-statecharts.md) for the implemented Statechart subset and
[Stage 3](./docs/plan/stage-3-effects-and-host-coordination.md) before connecting commands or host
snapshots. Narrata does not promise generic exactly-once delivery: a host must supply a stable
idempotency key, transactional API, or explicit unknown-outcome resolution.

## License

Except where otherwise noted, REZICS is licensed under the
[GNU Affero General Public License v3.0 only](./LICENSE) (`AGPL-3.0-only`).
Copyright © 2026 Rezics Inc.

Third-party components remain under their respective terms; see
[Third-party notices](./THIRD_PARTY_NOTICES.md). The AGPL grants copyright
permissions only and does not grant trademark rights in the REZICS name,
logos, or other brand identifiers.

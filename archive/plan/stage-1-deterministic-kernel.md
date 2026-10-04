# Stage 1：Deterministic In-Memory Narrative Kernel

状态：已实施（G0 + G1）
格式稳定性：`v0` 实验格式；Rust API 为 `0.x` unstable

## 范围

Stage 1 合并原 Phase 0 与 Phase 1，交付从 canonical Program bytes 到可恢复互动状态的完整纵切：

```text
strict envelope/CBOR decode
→ ProgramArtifactV0（untrusted wire）
→ fixed-order validation + stack/type proof
→ CheckedProgram
→ transactional Flow VM
→ Say / Choice / Finish safe point
→ canonical Snapshot / Receipt
→ restore + proof reconstruction
```

指令集为 `Const / Load / Store / Unary / Binary / Jump / JumpIfFalse / Call / Return / Say /
Choice / Finish`。所有 control-flow continuation 使用 `InstructionId`；互动 safe point 的所有 frame
evaluation stack 必须为空。

## Crate 边界

- `narrata-core`：无 I/O、clock、RNG、async、callback 或 persistence；只实现纯 checked kernel。
- `narrata-testkit`：fixture、native backend、reference model、generator 与 conformance runner。
- `narrata-cli`：artifact/snapshot inspect、run/trace、replay 与 conformance。

Store、Commit、Ref、Statechart、Effect、Binding、DSL 与 migration 明确延后。`TransitionDraft.parent`
是 `StateDigest`；`InputId` 历史去重由 coordinator 拥有，testkit 只提供 process-local
`EphemeralSession`。

## 可执行 Gate

```powershell
./scripts/check-g1.ps1
```

CI 另外比较 native 与 `wasm32-wasip1` conformance manifest，并在 Linux、Windows、macOS
执行相同 golden tests。


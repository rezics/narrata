# Phase 0：契约与测试判据

状态：待实施
前置：无
完成后解锁：[Phase 1](./01-runtime-kernel.md)

## 目标

在写 parser、Statechart 或 FFI 前，建立能够判定“同一输入是否得到同一结果”的可执行骨架。
本阶段不实现完整故事，只冻结 v0 内部语义和测试语言。

## P0.1 建立 Rust workspace 与 CI

交付：

- 根 `Cargo.toml`、固定 `rust-toolchain.toml`、`rustfmt.toml`、lint policy；
- `narrata-core`、`narrata-store`、`narrata-testkit`、`narrata-cli` 最小 package；
- native test、Wasm compile check、doc test、format/lint；
- feature matrix，默认 feature 不含 SQLite、Nickel 或 FFI；
- 许可证与第三方 notice 生成入口。

验收：干净 checkout 用一条文档化命令完成 format、lint、test；CI 不依赖开发者全局安装的
Nix/Nickel。

## P0.2 定义 ID 与 checked construction

在 `narrata-core` 增加不可互换的 newtype：

```text
ProgramId / ProgramArtifactId / BuildProvenanceId / ContentLockId
ExecutionId / InputId / InteractionId / EffectId
FlowId / InstructionId / StateId / ChoiceId
ObjectId / SnapshotId / ReceiptId / CommitId
RefRevision / LedgerFence
```

交付：

- wire/untrusted type 与 checked domain type 分开模块；
- ID parser 有长度、编码和 namespace/domain 检查；
- 所有 checked constructor 返回 typed diagnostic；
- compile-fail tests 证明不同 ID 不能互传；
- diagnostic path 能指出 object/field/index 与 source span（如果存在）。

验收：没有 `type CommitId = String` 之类 alias；不可信 bytes 不能直接 `Deserialize` 成
`RuntimeState`。

## P0.3 定义 v0 Value 与限制

实现 `Null/Bool/I64/String/List/Record/Variant/Entity`，使用确定迭代顺序。交付：

- 最大嵌套、字符串 bytes、集合元素和总 allocation budget；
- duplicate map key 拒绝；
- `Value` 不包含 float、host handle 或 arbitrary extension object；
- structural equality/hash 的 test vectors；
- 深度攻击与整数边界 property/fuzz seed。

验收：同一逻辑 Value 在不同 insertion order 下得到相同 canonical value；超过限制返回错误，
不 panic/stack overflow。

## P0.4 Canonical encoding spike

实现最小 deterministic CBOR profile，只覆盖 P0.3 与 Object/Commit 所需类型。

交付：

- `CanonicalEncode`/`CanonicalDecode`，不直接 hash 泛型 Serde 输出；
- SHA-256 domain separation；
- checked envelope：magic、kind、schema version、payload length/digest；
- `tests/conformance/codec-v0.json` 描述值，配套 `.cbor.hex` 期望 bytes/hash；
- native debug/release 与 Wasm 生成完全相同 bytes；
- reject non-minimal integer、indefinite length、unsorted/duplicate key、unknown required field；
- ADR 记录所用 crate、缺口和是否需要自写 encoder。

停止条件：若候选 CBOR crate 无法证明确定输出，就只使用其 decoder，自己实现最小 encoder；
不得把“当前测试碰巧相同”写成格式保证。

## P0.5 Trace fixture 与参考模型

定义语言中立、人可读的 conformance trace：

```text
Program fixture
Initial ExecutionId
Ordered checked inputs
Expected status / effects / snapshot hash / receipt hash / commit hash
Expected diagnostic for negative cases
```

交付：

- JSON/YAML 只作为 fixture authoring 格式，不参与 runtime hash；
- `narrata-testkit` 读取 fixture 并驱动任意 backend；
- 一个极小 reference reducer/model，覆盖 `set/jump/await/finish`；
- trace output 使用稳定排序和 machine-readable diagnostic code。

验收：故意交换两步、改变一个 ID 或 Value 后，fixture 在正确字段失败，而不是只显示最终
hash 不同。

## P0.6 资源预算与错误分类

固定以下错误类别及原子性：

```text
Decode / Validation / Incompatible / LimitExceeded
InvalidInput / InvalidState / Conflict
BudgetSlice / MacrostepLimit / RuntimeFault
Store / Corrupt / Migration / Capability
```

交付：

- slice budget 与 macrostep hard limit 分离；
- diagnostic code 稳定，展示文本暂不承诺兼容；
- Fault 不修改 committed parent 的模型测试；
- decoder/validator/runtime/store 错误不能用同一个 catch-all string。

## Phase 0 exit gate

- codec golden vector 在 native/Wasm 一致；
- 不可信输入必须经过 wire → checked 转换；
- ID 类型在编译期隔离；
- trace harness 能精确指出第一次语义偏差；
- 资源超限和 Fault 不改变已提交状态；
- `docs/architecture/` 与 ADR 没有未记录的格式假设。

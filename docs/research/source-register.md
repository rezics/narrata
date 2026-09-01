# 研究源码与资料登记

状态：研究记录
日期：2026-09-01

本轮研究把源码以 shallow clone 放在 `.temp/`。该目录被 Git 忽略，仅用于复查，不是
Narrata 的构建依赖。以下 commit 固定了本轮源码观察的语境。

## 克隆项目

| 项目 | 本地目录 | 审阅 commit | 重点文件 |
| --- | --- | --- | --- |
| Nix | `.temp/nix` | [`4750701db380`](https://github.com/NixOS/nix/commit/4750701db3802868445276c1a09c2f065a5a4bc6) | `src/libstore/profiles.cc`、`src/libstore/gc.cc`、manual source |
| Nixpkgs / NixOS | `.temp/nixpkgs`（sparse checkout） | [`01244f0f5d19`](https://github.com/NixOS/nixpkgs/commit/01244f0f5d1973cb30d5ebb4818d6622acd5e9e1) | `lib/modules.nix`、`nixos/modules/system/activation/` |
| Nickel | `.temp/nickel` | [`454e0ae37272`](https://github.com/nickel-lang/nickel/commit/454e0ae372721ca7d531c678b042f8ebdafc6e1b) | `doc/manual/`、`nickel/src/lib.rs` |
| Ink | `.temp/ink` | [`35c63e52f1d3`](https://github.com/inkle/ink/commit/35c63e52f1d36060930dc7ed3cfba38ea224b528) | `ink-engine-runtime/StoryState.cs`、`CallStack.cs` |
| Ren'Py | `.temp/renpy` | [`6f303ceb346b`](https://github.com/renpy/renpy/commit/6f303ceb346bfb52489daf43d3ccc1acee46cdc6) | `renpy/rollback.py`、`execution.py`、`script.py` |
| Yarn Spinner | `.temp/yarn-spinner` | [`ec1a680fae4c`](https://github.com/YarnSpinnerTool/YarnSpinner/commit/ec1a680fae4c0fc8aae73b23c7b87f3e5f894100) | `YarnSpinner/VirtualMachine.cs` |

## 直接支持设计决定的源码观察

- Nix 的 `createGeneration` 先把构件变成永久 GC root，再返回 generation；注释还明确说明
  需要与 GC 同步，避免新构件在临时 root 与永久 root 交接时被清除：
  [`profiles.cc#L61-L95`](https://github.com/NixOS/nix/blob/4750701db3802868445276c1a09c2f065a5a4bc6/src/libstore/profiles.cc#L61-L95)。
- Nix 切换 generation 时锁定 profile，再替换链接；源码也提供当前链接值作为 optimistic
  lock token：[`profiles.cc#L244-L280`](https://github.com/NixOS/nix/blob/4750701db3802868445276c1a09c2f065a5a4bc6/src/libstore/profiles.cc#L244-L280)。
- Nix GC 把 `gcroots` 与 `profiles` 都视为 root，并额外保护运行中与临时 root：
  [`gc.cc#L293-L312`](https://github.com/NixOS/nix/blob/4750701db3802868445276c1a09c2f065a5a4bc6/src/libstore/gc.cc#L293-L312)。
- NixOS module system 在合并前保留定义来源，依次展开条件/合并、筛掉低优先级定义并排序：
  [`modules.nix#L1185-L1213`](https://github.com/NixOS/nixpkgs/blob/01244f0f5d1973cb30d5ebb4818d6622acd5e9e1/lib/modules.nix#L1185-L1213)。
- Nickel 的 merge 是对称递归合并；相同优先级且值冲突时失败，contracts 作为字段 metadata
  随 merge 累积并延迟检查：[`merging.md`](https://github.com/nickel-lang/nickel/blob/454e0ae372721ca7d531c678b042f8ebdafc6e1b/doc/manual/merging.md)。
- Nickel 当前提供稳定的 Rust library facade；`eval_deep_for_export` 会深度求值并忽略
  `not_exported` 字段，结果可转换为 `serde::Deserialize` 类型：
  [`nickel/src/lib.rs`](https://github.com/nickel-lang/nickel/blob/454e0ae372721ca7d531c678b042f8ebdafc6e1b/nickel/src/lib.rs)。
- Ink 的 `StoryState` 明确包含程序位置、call stack、变量、选择、计数和随机状态，并带独立
  save schema version：[`StoryState.cs#L9-L60`](https://github.com/inkle/ink/blob/35c63e52f1d36060930dc7ed3cfba38ea224b528/ink-engine-runtime/StoryState.cs#L9-L60)。

## 官方规范与手册

| 主题 | 资料 | 本设计采用的事实 |
| --- | --- | --- |
| Nix profiles | [Nix Profiles](https://nix.dev/manual/nix/stable/package-management/profiles) | 不可变环境、generation、原子切换和快速 rollback |
| Nix GC | [Garbage Collector Roots](https://nix.dev/manual/nix/stable/package-management/garbage-collector-roots) | root 保留对象及其依赖闭包 |
| Nix store identity | [Store Path](https://nix.dev/manual/nix/stable/store/store-path) | store path 是 opaque identity；具体 digest 规则依对象类型而异 |
| Nix flakes | [Flakes](https://nix.dev/concepts/flakes.html) | lock file 固定依赖图的精确输入 |
| NixOS rollback 边界 | [How Nix Works](https://nixos.org/guides/how-nix-works/) | rollback 恢复配置，但不恢复 `/var` 等 mutable state |
| NixOS modules | [NixOS Manual](https://nixos.org/manual/nixos/stable/#sec-writing-modules) | option type 决定验证与 merge；冲突保留来源诊断 |
| Nickel merge | [Merging records](https://nickel-lang.org/user-manual/merging/) | 对称 merge、priority、default/force、lazy contract propagation |
| Nickel correctness | [Correctness](https://nickel-lang.org/user-manual/correctness/) | type 静态检查；contract 在运行时延迟检查 |
| Nickel modular config | [Modular configurations](https://nickel-lang.org/user-manual/modular-configurations/) | partial record、递归依赖和可查询 metadata |
| Nickel packages | [Package management](https://nickel-lang.org/user-manual/package-management/) | 当前 package management 仍是实验能力，不能成为关键运行依赖 |
| Statechart | [W3C SCXML](https://www.w3.org/TR/scxml/) | internal/external queue、microstep、macrostep、run-to-completion |
| Durable replay | [Temporal Workflow Definition](https://docs.temporal.io/workflow-definition) | 重放要求命令序列确定；外部操作必须离开 replay path |
| Event sourcing | [Microsoft Event Sourcing pattern](https://learn.microsoft.com/en-us/azure/architecture/patterns/event-sourcing) | append-only、快照、幂等、optimistic concurrency 与 schema 演化代价 |
| 确定性编码 | [RFC 8949 §4.2](https://www.rfc-editor.org/rfc/rfc8949.html#section-4.2) | CBOR 需要额外约束才具确定性编码 |
| Protobuf 哈希风险 | [Proto Serialization Is Not Canonical](https://protobuf.dev/programming-guides/serialization-not-canonical/) | deterministic protobuf 仍不适合作跨版本内容哈希材料 |
| 原子本地事务 | [SQLite Atomic Commit](https://www.sqlite.org/atomiccommit.html) | reference store 可以借助成熟事务而非自制崩溃协议 |

## 复查说明

网页中的 `stable` 文档会随上游移动。凡影响 Narrata wire/save 兼容性的语义，实施时应把
对应规范版本或上游 commit 写入 ADR 和测试向量，不能只依赖一个会变化的 URL。

# Narrata 设计文档索引

状态：工作基线
日期：2026-09-01

本目录把早期调研收敛为可以实现和验证的设计。当前最重要的决定是：

1. Narrata runtime 是确定性的显式状态转换器，不把 continuation 藏在 Rust call stack、
   `Future` 或宿主回调中。
2. 可恢复的真相是版本化的完整 `RuntimeState`；每个安全点形成不可变 `Commit`，命名
   `Ref` 只负责指向当前提交。
3. 时间旅行通过移动游标和从旧提交分叉实现，不反向执行指令，也不修改历史提交。
4. 输入与 Effect Response 日志用于审计、确定性验证和调试重放；它不是唯一恢复来源。
5. 外部不可逆效果位于可回滚时间线之外，必须使用幂等键、单调账本或回滚屏障。
6. 普通 checkpoint 只承诺恢复目标点；完整用户时间线是默认关闭的独立能力，自显式启用的
   baseline 起保留所有语义转换、分支和存档目录历史。

## 推荐阅读顺序

### 调研

- [Nix/NixOS 与 Nickel 能带来什么](./research/nix-nickel-lessons.md)：回答是否以及如何采用
  两者。
- [研究源码登记](./research/source-register.md)：记录本轮实际克隆和审阅的提交、文件与官方
  资料，保证结论可以复查。

### 目标架构

- [确定性运行模型](./architecture/runtime-model.md)：状态、输入、macrostep、safe point 和
  核心 API。
- [时间旅行与存档](./architecture/time-travel-and-save.md)：不可变提交图、快照、Ref、普通
  Checkpoint Bundle、可选 Timeline Archive、原子写入、分支与 GC。
- [Effect 与宿主状态](./architecture/effects-and-host-state.md)：展示协调、外部副作用、幂等、
  回滚屏障和联合存档。
- [程序身份与迁移](./architecture/program-versioning-and-migration.md)：Stable ID、精确构件、
  依赖锁和显式迁移。
- [REZICS Gamebook 集成边界](./rezics-gamebook-integration.md)：Narrata 与 REZICS 的领域
  所有权及外部内容引用。

### 实施

- [实施总览](./plan/README.md)
- [Stage 1：Deterministic In-Memory Narrative Kernel](./plan/stage-1-deterministic-kernel.md)
- [Stage 2：Time Travel and Crash-Safe Persistence](./plan/stage-2-time-travel-persistence.md)
- [Stage 3：Effects and Host Coordination](./plan/stage-3-effects-and-host-coordination.md)
- [Phase 0：契约与测试判据](./plan/00-contracts-and-test-oracles.md)
- [Phase 1：最小确定性 Runtime](./plan/01-runtime-kernel.md)
- [Phase 2：时间旅行与持久化](./plan/02-time-travel-and-persistence.md)
- [Phase 3：Effect 与宿主协调](./plan/03-effects-and-host-coordination.md)
- [Phase 4：Statechart](./plan/04-statecharts.md)
- [Phase 5：迁移、Binding 与 Nickel 适配](./plan/05-migrations-bindings-and-nickel.md)

### ADR

- [0001：Stage 1 scope](./adr/0001-stage-1-scope.md)
- [0002：Identities and digests](./adr/0002-identities-and-digests.md)
- [0003：Deterministic CBOR profile](./adr/0003-deterministic-cbor-profile.md)
- [0004：Flow VM safe points](./adr/0004-flow-vm-safe-points.md)
- [0005：Input idempotency ownership](./adr/0005-input-idempotency-ownership.md)
- [0006：Stage 2 persistence boundary](./adr/0006-stage-2-persistence-boundary.md)
- [0007：Stage 3 effect and host-state boundary](./adr/0007-stage-3-effect-host-boundary.md)

## 文档状态约定

- **已决定**：除非出现反例或新约束，实施应遵循。
- **提案**：方向已收敛，但仍有明确验证门槛。
- **计划**：按依赖顺序拆分的工作项；完成必须满足列出的验收条件。
- **开放问题**：不能在没有证据时静默作出永久决定，必须通过对应 spike 或测试关闭。

早期长篇调研保留在 `.temp/叙事引擎调研.md`，但它不是实施规范。若两者冲突，以本目录中
状态为“已决定”的文档为准。

# Narrata 文档

文档只写代码表达不了的内容：意图、带理由的决定和操作流程。它们记录可参考的做法，不是
规则；不再适用时修改并说明原因。智能体的工作约定见仓库根目录的 `AGENTS.md`。

## 产品

- [目标](product/goal.md)：Narrata 要成为什么，核心场景、规模目标、非目标和路线。
- [编号决定](product/decisions.md)：比任何 Goal 都长寿的产品与架构决定及理由。

## 架构

- [总览](architecture/overview.md)：目标分层、三条硬边界、两套引擎栈的现状。
- [确定性运行模型](architecture/runtime-model.md)：状态、输入、macrostep、safe point 与核心 API。
- [时间旅行与存档](architecture/time-travel-and-save.md)：不可变提交图、快照、Ref、Checkpoint
  Bundle、Timeline Archive、原子写入与 GC。
- [Effect 与宿主状态](architecture/effects-and-host-state.md)：展示协调、外部副作用、幂等与
  回滚屏障。
- [程序身份与迁移](architecture/program-versioning-and-migration.md)：Stable ID、精确构件、
  依赖锁与显式迁移。

以上四篇描述 Stage 1–5 的已实现机制；它们与 2026-10-05 决定的关系写在各自开头。

## 契约

目标契约，由 Goal 落成代码后收缩为指向代码的说明。

- [内容引用](contracts/content-references.md)：`ContentRef`、`Segment`、解析结果与解析上下文。
- [选择模型](contracts/choices.md)：选择点、选项、局部结果与分支结果、动态选项。
- [存储与存档](contracts/storage-and-saves.md)：三种存储角色、窄后端契约、存档字节与现有差距。
- [图、分析与摘要](contracts/graph-and-analysis.md)：发布时分析、布局瓦片、读者地图与语义摘要。

## 集成

- [REZICS](integrations/rezics.md)：分层与归属、引用形状、`narrata-choice` 标记块、对 REZICS
  的能力请求 R1–R11。

## ADR

格式与语义边界的决定记录；新格式从 0012 起编号。

- [0001：Stage 1 scope](adr/0001-stage-1-scope.md)
- [0002：Identities and digests](adr/0002-identities-and-digests.md)
- [0003：Deterministic CBOR profile](adr/0003-deterministic-cbor-profile.md)
- [0004：Flow VM safe points](adr/0004-flow-vm-safe-points.md)
- [0005：Input idempotency ownership](adr/0005-input-idempotency-ownership.md)
- [0006：Stage 2 persistence boundary](adr/0006-stage-2-persistence-boundary.md)
- [0007：Stage 3 effect and host-state boundary](adr/0007-stage-3-effect-host-boundary.md)
- [0008：Stage 4 Statechart semantics](adr/0008-stage-4-statechart-semantics.md)
- [0009：Stage 5 migration and protocol boundary](adr/0009-stage-5-migration-and-protocol-boundary.md)
- [0010：Nickel adapter not published](adr/0010-nickel-adapter-not-published.md)
- [0011：R1 节点组合与 Gamebook 会话](adr/0011-r1-node-composition.md)

## 研究

研究是写成时的快照，不随代码更新；结论被采纳后进入决定或契约。

- [内核：身份、事实与提交](research/2026-10-04-engine-core-and-storage/report.md)（2026-10-04）：
  叙事引擎的核心、可插拔存储、逻辑与媒体分离；[调研笔记](research/2026-10-04-engine-core-and-storage/notes/academic_narrative_models.md)
  另有行业工具、存储架构、长篇与动态叙事、仓库现状等五篇。
- 选择粒度与规模（2026-10-05）：[选择粒度](research/2026-10-05-choice-granularity-and-scale/choice_granularity.md)、
  [大图渲染](research/2026-10-05-choice-granularity-and-scale/graph_rendering.md)、
  [REZICS 章节规模](research/2026-10-05-choice-granularity-and-scale/rezics_chapter_scale.md)。
- [叙事节点的实现与学术依据](research/2026-09-05-narrative-model-evidence.md)（2026-09-05）
- [与竞品的架构差异](research/2026-09-05-narrata-competitive-architecture.md)（2026-09-05）
- [作者工作台与 Web VN 工具链路线](research/2026-09-05-narrata-toolchain-roadmap.md)（2026-09-05）
- [Nix/NixOS 与 Nickel 能带来什么](research/nix-nickel-lessons.md)
- [研究源码登记](research/source-register.md)

## 开发与 Goal

- [工具链与检查](development/toolchain.md)：工具版本、`task` 命令、并发槽位与磁盘成本。
- [Goal 程序](goals/README.md)：manager 与 worker 的运行方式；当前 Goal 列在仓库根目录的
  [GOAL.md](../GOAL.md)。

## 归档

[archive/](../archive/) 保存已完成或被取代的计划与记录，只作历史参考：重建方案与实施记录
（2026-09-05）、2026-08-31 的 REZICS 集成边界、[Stage 1–5 计划](../archive/plan/README.md)。

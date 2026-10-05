---
# 其他 Goal 的任务简报不得认领的粗粒度路径（goalctl 每次派发时读取）。manager 随简报落地扩大范围。
areas:
  - crates/**
  - bindings/**
  - fuzz/**
  - fixtures/**
  - packages/narrata/kernel/**
  - scripts/check-g*.ps1
  - scripts/benchmark-g2.ps1
  - docs/contracts/storage-and-saves.md
  - docs/architecture/**
---

# Kernel 与存档

状态：已定义，等待维护者启动 manager。[state.md](state.md) 记录进展。

## 结果

一套 kernel 同时支撑旧格式与节点栈，存档能交给任何宿主保存：

- **Kernel 收敛（决定 11）。** 身份与摘要、确定性 CBOR、受检解码、提交/引用对象库、Effect
  账本、迁移协调抽取为 kernel，并能承载注册的领域状态（节点实例、Flow、Statechart）。
  Flow VM 与 Statechart 成为基于 kernel 的第一方领域包。
- **窄存储契约（决定 8）。** 按 [存储与存档](../../../docs/contracts/storage-and-saves.md) 定义后端
  契约与能力声明；领域逻辑在其上只实现一次；`tests/model.rs`、`tests/faults_gc.rs` 改造为
  对任意后端运行的一致性套件。
- **后端。** 内存（参考）、按行读写的 SQLite、浏览器 IndexedDB（经 Wasm 绑定），以及
  Checkpoint Bundle 存档字节的导出与受检导入。协议引擎不再写死内存存储。
- **旧栈去文本（决定 2）。** Program 与 Snapshot 不再含文本，`SceneState` 成为可选组件；
  新格式 ADR 与新冻结语料（如 `stage6-v0`），旧 `stage5-v0` 语料继续可读或可迁移。

## 约束

- `fixtures/compat` 下的冻结语料只读（决定 14）。
- 每一步先保持 `stage5-v0` 摘要不变再改格式：契约与后端改造不得改变任何冻结字节，格式变化
  集中在单独的 ADR 与语料里。
- 与 `narrative-core` 的耦合见对方 GOAL.md。

## 验收

- `task check:g5` 通过；新增基准：1K 与 10K 次提交时单次提交与加载延迟同一量级；加载单个
  提交不触发全对象枚举（计数包装器断言）。
- 每个后端通过一致性套件与故障注入；浏览器后端有 Wasm 测试。
- 节点会话的保存、恢复、导出、导入通过 kernel 完成。

# 存储与存档

状态：已实现，本页只保留代码说不了的角色划分与场景选择；格式与机制见下文链接的 ADR。
依据：[决定 7、8、9](../product/decisions.md)。

## 三种角色

| 角色 | 内容 | 谁保存 | 运行时是否读取 |
| --- | --- | --- | --- |
| 作者源稿 | 编辑中的节点、选择点、条件与效果 | 宿主（REZICS 的 PostgreSQL、本地文件、协作文档） | 否，只被编译器读取 |
| 不可变构件 | 编译后的程序清单与程序块，内容寻址 | 对象存储、CDN、游戏安装目录 | 是，按需读取 |
| 会话存档 | 提交、Snapshot、回执、目录、Effect 账本 | 下表中的后端，或交给宿主的存档字节 | 是 |

运行时从不查询宿主数据库。图数据库不承担任何一种角色；需要图查询的宿主从
[语义摘要](graph-and-analysis.md#语义摘要) 导入。

## 后端契约

后端只认字节、摘要和键，不认识叙事类型，因此任何栈都能复用。契约是
`packages/narrata/kernel/crates/narrata-storage` 中的 `StorageBackend`：不可变对象、带全库
修订号的键、带前置条件的原子批次、有界游标扫描和能力声明，取舍与理由见
[ADR 0012](../adr/0012-narrow-storage-backend-contract.md)。内存后端是行为参考，
`narrata-storage-sqlite` 按行读写；两者都通过 `testing` feature 中的一致性套件，以后的后端
也用这套件验收。

契约之上是 kernel 的历史层 `packages/narrata/kernel/crates/narrata-history`：受检对象、Ref、Pin、
保留与 GC、完整性扫描、Checkpoint Bundle，以及领域注册的通用提交与会话 API，每个领域只需实现
`Domain`。Stage 1–5 的效果账本、目录、复合存档与迁移由 `narrata_store::Store<B>` 作为一个
注册方实现。两层的边界、种类分配与恢复的信任边界见
[ADR 0015](../adr/0015-kernel-history-layer.md)；键布局、校验边界与 GC 栅栏见
[ADR 0014](../adr/0014-save-engine-key-layout.md)。

## 各场景的后端

| 场景 | 后端 | 说明 |
| --- | --- | --- |
| 测试与 Wasm 内临时会话 | 内存 | 行为参考 |
| CLI、调试、桌面工具 | SQLite | 按行读写；schema v2 旧库用 `narrata store migrate-v2` 显式迁移 |
| 浏览器 | IndexedDB，经 `narrata-storage-host` 的缓存后端与 `packages/narrata/kernel/js`（[ADR 0017](../adr/0017-browser-storage-host-cache.md)） | 按需读取、先落盘后发布、多标签页冲突检测；浏览器会整源驱逐存储，Safari 删除 7 天无交互的脚本存储：申请持久化并提供导出 |
| 本地游戏 | 存档字节 | 引擎导出 Checkpoint Bundle 字节（可选浅存档，只带最近 N 个祖先，[ADR 0019](../adr/0019-shallow-history-bundles.md)），宿主写入自己的存档系统；导入时引擎重新校验 |
| 网站登录用户 | 存档字节 | 宿主（如 REZICS）把字节存在自己的数据库并同步，Narrata 不连接该数据库 |

JSON 只作为导出与调试格式，不作为可并发写入的存档后端。

## 普通存档与完整时间线

沿用 [时间旅行与存档](../architecture/time-travel-and-save.md)：普通存档是一个目标提交及其
闭包（`CheckpointBundle`）；完整时间线（`TimelineArchiveBundle`）默认关闭，由宿主显式启用。
两者都以字节交给宿主，宿主负责归属、同步、保留和删除说明。

## 当前缺口

- 节点栈尚未注册到历史层（kinds `0x0110`–`0x0112` 已按 ADR 0015 的通用提交分配），浏览器
  阅读器仍把整份会话（含源）存为一条 IndexedDB 记录；由 Goal `narrative-core` 接入。
- FFI 与 Wasm 宿主仍用默认的内存后端加存档字节；Rust 宿主可以给协议引擎注入任意后端。

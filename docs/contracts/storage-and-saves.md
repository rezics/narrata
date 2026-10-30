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

两层的边界、种类分配与恢复的信任边界见
[ADR 0015](../adr/0015-kernel-history-layer.md)；键布局、校验边界与 GC 栅栏见
[ADR 0014](../adr/0014-save-engine-key-layout.md)。

## 各场景的后端

| 场景 | 后端 | 说明 |
| --- | --- | --- |
| 测试与 Wasm 内临时会话 | 内存 | 行为参考 |
| CLI、调试、桌面工具 | SQLite | 按行读写；schema v2 旧库用 `narrata store migrate-v2` 显式迁移 |
| 浏览器 | IndexedDB，经 `narrata-storage-host` 的缓存后端与 `packages/narrata/kernel/js`（[ADR 0017](../adr/0017-browser-storage-host-cache.md)） | 按需读取、先落盘后发布、多标签页冲突检测；浏览器会整源驱逐存储，Safari 删除 7 天无交互的脚本存储：申请持久化并提供导出 |
| 本地游戏 | 存档字节 | 宿主保存完整或浅 Checkpoint Bundle；节点浅导出把 N 作为下限，另保留重建当前页面所需的上下文（[ADR 0019](../adr/0019-shallow-history-bundles.md)） |
| 网站登录用户 | 宿主服务器上的契约实现 | 每个"用户 × 作品"一个库，浏览器经批次协议增量同步；Narrata 不连接该数据库，结构变化由 Narrata 逐库迁移（[决定 15](../product/decisions.md#15-服务器存档实现同一个存储契约)、[ADR 0021](../adr/0021-server-save-storage.md)） |

JSON 只作为导出与调试格式，不作为可并发写入的存档后端。

## 普通存档与完整时间线

沿用 [时间旅行与存档](../architecture/time-travel-and-save.md)：普通存档是一个目标提交及其
闭包（`CheckpointBundle`）；完整时间线（`TimelineArchiveBundle`）默认关闭，由宿主显式启用。
两者都以字节交给宿主，宿主负责归属、同步、保留和删除说明。

## 在线会话归属

宿主认证后注入固定库实例的传输、释放、按已观察 ID/修订号删除以及内容缓存失效回调。
账号切换关闭旧包装并新建包装；删除后的新库使用新 StoreId。会话生命周期和恢复动作见
[Web 包](../../packages/narrata/web/README.md#authenticated-online-journeys)；状态与受检定位边界由代码和测试表达。
正文及选项标签仍由内容方解析，不进入会话。

## 保留策略由宿主选择

库配额、可保留根数、grace、可信时钟与操作授权来自宿主。维护入口及句柄释放示例见
[Web 包](../../packages/narrata/web/README.md#save-maintenance)；参数、结果、受检边界与预算由类型和测试表达。
分支与命名存档代表读者明确保留的路线，达到配额不能成为自动删除它们的理由。宿主先展示可删除的根、
读取新修订号，在读者选择后 CAS 删除，再展示 dry-run 与执行 GC；全部对象仍被其他根保活时可能没有空间可回收。
配额或导入失败后保留原库，允许重新打开、清理后重试；未确认的写入不能宣称保存成功。

浅导出用于搬运一个选定提交，不能把它重新导入原库当作原地压缩。在线浅压缩、影子库构建与 CAS 切换、
布局迁移和后台调度仍按 [ADR 0021](../adr/0021-server-save-storage.md) 推迟。宿主不得把有限扫描当作完整检查，
也不能用 SQL 重新解释 Narrata 对象图。

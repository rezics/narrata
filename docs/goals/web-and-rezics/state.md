# State

2026-10-29，M0 完成。manager 会话为 codex-01a187ba-9af7-71c1-b110-0ee55db54795；[Goal](GOAL.md)的 M1/M2/M3 边界保持不变。运行时事实以 task goal -- status 为准。

## 当前事实

- 维护者接续基线 fd4b309 的产品决定与集成请求保持有效。三个旧任务均在原身份补交接、审查完整分支并合入 main；无重复派发。3245a21 归档简报/交接并释放认领，goal/g-031、goal/g-032、goal/g-033 均保留，worktree 已移除。
- 已交付 Rust 发布对象集合与关系投影、作者图三层读取/WebGL/移动端、节点构件演化核心。演化补了 checkpoint import 的 incompatible_save 兼容诊断回归。它们的正式交接见[作者图](../../../archive/goals/web-and-rezics-2026-10-05/handoffs/G-031.md)、[发布集合](../../../archive/goals/web-and-rezics-2026-10-05/handoffs/G-032.md)、[演化核心](../../../archive/goals/web-and-rezics-2026-10-05/handoffs/G-033.md)。
- 新增未知保存结果暂停/原字节重试、Book.retrySave/显式重载，及可选 @rezics/narrata/content/native 修订绑定的宿主文档适配；正文及原生值不进入逻辑构件/状态/存档。独立 tarball 在 Node/Bun/Chromium 验证原生入口与读取/选择/保存/重开，现有 reader 预算未放宽。
- 整批 main 的 check:r1、check:web（48 单测）、check:web-storage 均通过；23 阅读器/图浏览器测试、17 存储浏览器测试通过。check:r1 已纳入 authoring tests/wasm-target 与 nodes 全 feature；CI 调同一 Task。test:scripts 53 项与 docs:check 通过。保存测试证明丢回执后同字节重试可能 Conflict，必须重载核实，不能伪称成功。
- 依赖审计阻碍已修复：Vite 8.2.4、Zod 4.6.7 及兼容传递依赖；未降低审计门槛。Sigma/graphology 的实际依赖与预算已记入工具链。
- 发布规模证据：88 对象集合可重开 68,469,768 字节程序块；100k 选择点为 44,083 行、15,640,925 字节/10 片，最大片 4,194,245 字节。旧冻结语料未改；新语料由既有 ADR 0022/0023 覆盖。

manager gate 日志在 .temp/manager-check-*.log；旧 worker 的临时证据已保存在 .temp/goal/runs 下对应任务的 worktree-temp 目录。图的桌面与 390px 移动端截图已由 manager 查看。M0 tarball/模拟宿主不表示完整 SDK 或 REZICS 产品上线。

## 当前与下一步

1. [公共创作与精简 reader](tasks/G-034.md)已派发，Claude/xhigh 在原生 worker 实施。复用受检对象编译与发布核心，交付 authoring/server 与独立 reader Wasm；保留旧 raw Wasm 兼容入口。认领前置任务已关闭。
2. [在线会话与精确构件恢复](tasks/G-035.md)已准备，依赖公共绑定完成；补 durable roots/history 核实、账号/关闭/删除生命周期和按存档 artifact 定位，不能以对象存在或 revision 增大代替保存证明。
3. M1 完成后交付固定 tarball 与模拟宿主旅程。M2 仍需 visits/读者地图、真实 PostgreSQL 关系装载、planEvolution/发布挂接/影响评估/Wasm 与阅读器确认，以及导入导出/浅历史/GC/配额维护公共接口。
4. 三类结果分别记录：包可交付、完整 Goal 关闭、REZICS 实际产品可用。未完成 M2/M3 不关闭 Goal，也不启动重叠的 Web VN 实施。

## 主机与外部边界

Claude Code 2.1.289 的空 OAuth 刷新锁在无 CLI 进程、无 owner metadata 的证据下非递归清理，最小无工具调用已验证成功；没有代维护者登录。公共创作任务改用 Claude，Codex 仍可作备用。

官方 EDB PostgreSQL 18.6 portable 位于 .temp/postgresql/18.6-5/pgsql/bin，来源/校验和在 .temp/postgresql/download.json，版本已记入工具链。已在临时 loopback cluster（fsync/synchronous_commit 均 on）运行 check:server-storage，PGlite/真实数据库共 46 测试通过；日志 .temp/manager-check-server-storage.log。服务器已停止，没有注册服务或更改 PATH。该证据覆盖既有存档宿主，不冒充尚未实现的关系投影装载。

REZICS 的实现只留在[能力请求](../../integrations/rezics.md)，本仓库未改 REZICS。真实联合验收等待宿主准备，未通过前不声称上线。[Web VN](../web-vn/GOAL.md)仍为待启动计划，areas 为空、无 manager。

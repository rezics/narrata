# State

2026-10-29，M0 完成。manager 会话为 codex-01a187ba-9af7-71c1-b110-0ee55db54795；[Goal](GOAL.md)的 M1/M2/M3 边界保持不变。运行时事实以 task goal -- status 为准。

## 当前事实

- 维护者接续基线 fd4b309 的产品决定与集成请求保持有效。三个旧任务均在原身份补交接、审查完整分支并合入 main；无重复派发。3245a21 归档简报/交接并释放认领，goal/g-031、goal/g-032、goal/g-033 均保留，worktree 已移除。
- 已交付 Rust 发布对象集合与关系投影、作者图三层读取/WebGL/移动端、节点构件演化核心。演化补了 checkpoint import 的 incompatible_save 兼容诊断回归。它们的正式交接见[作者图](../../../archive/goals/web-and-rezics-2026-10-05/handoffs/G-031.md)、[发布集合](../../../archive/goals/web-and-rezics-2026-10-05/handoffs/G-032.md)、[演化核心](../../../archive/goals/web-and-rezics-2026-10-05/handoffs/G-033.md)。
- 新增未知保存结果暂停/原字节重试、Book.retrySave/显式重载，及可选 @rezics/narrata/content/native 修订绑定的宿主文档适配；正文及原生值不进入逻辑构件/状态/存档。独立 tarball 在 Node/Bun/Chromium 验证原生入口与读取/选择/保存/重开，现有 reader 预算未放宽。
- 整批 main 的 check:r1、check:web（48 单测）、check:web-storage 均通过；23 阅读器/图浏览器测试、17 存储浏览器测试通过。check:r1 已纳入 authoring tests/wasm-target 与 nodes 全 feature；CI 调同一 Task。test:scripts 53 项与 docs:check 通过。保存测试证明丢回执后同字节重试可能 Conflict，必须重载核实，不能伪称成功。
- 依赖审计阻碍已修复：Vite 8.2.4、Zod 4.6.7 及兼容传递依赖；未降低审计门槛。Sigma/graphology 的实际依赖与预算已记入工具链。
- 已推送基线 d060874 的 [CI](https://github.com/rezics/narrata/actions/runs/40884071749) 全绿：Windows/Linux Gamebook、Windows/Linux/macOS kernel、文档/脚本与真实服务器存档。后续 SDK 代码仍需自己的当前提交检查，不沿用该结果。
- 发布规模证据：88 对象集合可重开 68,469,768 字节程序块；100k 选择点为 44,083 行、15,640,925 字节/10 片，最大片 4,194,245 字节。旧冻结语料未改；新语料由既有 ADR 0022/0023 覆盖。

manager gate 日志在 .temp/manager-check-*.log；旧 worker 的临时证据已保存在 .temp/goal/runs 下对应任务的 worktree-temp 目录。图的桌面与 390px 移动端截图已由 manager 查看。M0 tarball/模拟宿主不表示完整 SDK 或 REZICS 产品上线。

## 当前与下一步

1. [公共创作与精简 reader](tasks/G-034.md)已派发；按维护者 2026-10-29 的新指示，保留现场 stop/resume 为 high（当前 Codex）。复用受检对象编译与发布核心，交付 authoring/server 与独立 reader Wasm；保留旧 raw Wasm 兼容入口。认领前置任务已关闭。
   当前已提交至 worker 分支 6c86d9d，53 项包测试、23 项浏览器测试、tarball 与 gate 接线通过。Sol/high 独立评审发现 loader 入口预算、发布诊断定位、decodeRecord 类型证明、minter 占用集合及 readPublication 接收者依赖问题；已用 .temp/review-g034-repair.md 在原任务继续修复；Claude/high attempt 3 写下修复后 API403 退出，改由 Codex/high attempt 4 保留脏文件接续，验收前不合入。旧评审已关闭被后续安装提交解决的缺口。manager 接受独立 size-optimized wasm profile、移除生产 name section 和 CI 增加 check:web 的方向；最终仍需当前提交验证。
2. [在线会话与精确构件恢复](tasks/G-035.md)已准备，依赖公共绑定完成；补 durable roots/history 核实、账号/关闭/删除生命周期和按存档 artifact 定位，不能以对象存在或 revision 增大代替保存证明。
3. M1 完成后交付固定 tarball 与模拟宿主旅程。M2 仍需 visits/读者地图、真实 PostgreSQL 关系装载、planEvolution/发布挂接/影响评估/Wasm 与阅读器确认，以及导入导出/浅历史/GC/配额维护公共接口。
   [公共演化链路](tasks/G-036.md)已准备，待公共绑定关闭后与在线会话按互斥路径推进。
   [visits/地图/关系宿主](tasks/G-037.md)与[维护/浅恢复](tasks/G-038.md)简报也已准备，按依赖顺序推进；不为准备简报提前认领实现路径。
4. 三类结果分别记录：包可交付、完整 Goal 关闭、REZICS 实际产品可用。未完成 M2/M3 不关闭 Goal，也不启动重叠的 Web VN 实施。

## 主机与外部边界

维护者 2026-10-29 要求本 Goal 的后续子代理和 worker 使用 high 或更低思考等级；默认 high，机械性工作可更低。当前任务也已调整；后续接续不恢复 xhigh。

Claude Code 2.1.289 的空 OAuth 刷新锁在无 CLI 进程、无 owner metadata 的证据下非递归清理，最小无工具调用已验证成功；没有代维护者登录。公共创作任务改用 Claude，Codex 仍可作备用。

官方 EDB PostgreSQL 18.6 portable 位于 .temp/postgresql/18.6-5/pgsql/bin，来源/校验和在 .temp/postgresql/download.json，版本已记入工具链。已在临时 loopback cluster（fsync/synchronous_commit 均 on）运行 check:server-storage，PGlite/真实数据库共 46 测试通过；日志 .temp/manager-check-server-storage.log。服务器已停止，没有注册服务或更改 PATH。该证据覆盖既有存档宿主，不冒充尚未实现的关系投影装载。

REZICS 的实现只留在[能力请求](../../integrations/rezics.md)，本仓库未改 REZICS。真实联合验收等待宿主准备，未通过前不声称上线。[Web VN](../web-vn/GOAL.md)仍为待启动计划，areas 为空、无 manager。

# State

2026-10-30 接续检查点；M0 完成。manager 会话为 codex-01a187ba-9af7-71c1-b110-0ee55db54795；[Goal](GOAL.md)的 M1/M2/M3 边界保持不变。运行时事实以 task goal -- status 为准。

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

1. [公共创作与精简 reader](../../../archive/goals/web-and-rezics-2026-10-05/tasks/G-034.md)已合入至 04aca6a 并 verified 关闭，分支保留。五项独立评审缺陷均修复并复核；合并后 check:r1（23 浏览器）、check:web（56 单测及 tarball）、test:scripts（53）与 docs:check 通过。忽略的重型绑定测试已另行执行并通过：88 对象、68,985,624 字节，旧 pack 上限仍被拒绝。
   reader Wasm 为 1,677,387 / 431,589 字节 raw/gzip，authoring 独立加载，旧 raw 模块兼容。候选包 SHA-256 为 7327e2e659a68667607b6a878e23aeace46d7a3def6b466b43acc052207bf320，固定副本与元数据在 .temp/delivery/authoring-candidate.json；这还不是完整 M1 交付。合并后的日志在 .temp/manager-sdk-*.log；宿主说明已更新。已推送 2d45844 的[新 CI](https://github.com/rezics/narrata/actions/runs/40895469616)全部通过，含 Windows/Linux check:web、Gamebook、三平台 kernel 与服务器存档；后续在线/演化代码仍需自己的检查。
2. [在线会话与精确构件恢复](tasks/G-035.md)已启动 Codex/high worker；补 durable roots/history 核实、账号/关闭/删除生命周期和按存档 artifact 定位，不能以对象存在或 revision 增大代替保存证明。
   attempt 1 的客户端/真实正文/A-B 故障旅程已通过，Web 评审接受。Rust 评审要求把保留证明从重试间二次工作改为有界进度，并修复 native reopen 删除竞争；M1 冷读取和同篇复合叙事也仍需验收。已用 .temp/review-g035-repair.md 在原任务 attempt 2 / high 接续；当前还需看正式交接，不能提前合入。
   attempt 2 已有 82e78ea / 2a59bc3：Rust/high 接受进度缓存、累计预算及无写入 reopen；medium 验收接受 `/?reference` 自身冷读取（2 对象、1,660 字节、reader only）、同篇局部汇合/延迟后果/参数 7/两个结局及实际 A/B/故障旅程。完整 gate 仍在收尾。G5 旧协议绑定测试栈审计失败已在 main f1c6052 / 49930a5 修复，Vitest 5.0.3、Vite 8.2.4，零漏洞、生成无漂移、构建及 4 测试通过；worker 也在做等价修复，rebase 时保留统一版本并补完整 G5 证据，不跳过 gate。
   已查看公共包桌面/移动端插槽截图，其 HOST CONTENT 内容只证明渲染接线。M1 最终模拟旅程须接公共创作输出、宿主真实正文/标签与在线保存恢复，已加入在线任务；不以占位截图代替完整旅程。
   Sol/medium 对接续前基线另确认参考 App 默认 full pack，以及最小 shortStorySource 缺复合叙事机制。最终交接后按 .temp/m1-reference-acceptance-review.md 核对同一公共创作作品的局部汇合/延迟后果/参数化调用/结局、真实页面和冷首屏；先看新分支是否已解决，不重复派发或误报仍在编写的代码。
3. M1 完成后交付固定 tarball 与模拟宿主旅程。M2 仍需 visits/读者地图、真实 PostgreSQL 关系装载、阅读器跨构件与演化确认接线，以及导入导出/浅历史/GC/配额维护公共接口。
   [公共演化后端](../../../archive/goals/web-and-rezics-2026-10-05/tasks/G-036.md)已合入至 938d1d9 并 verified 关闭，分支保留。规划/发布挂接/Wasm/控制器/只读影响评估与浅导出已交付；未知写入再评估状态和读取预算两项评审缺陷已修复复核。主线 check:r1、check:web（88 单测、独立 Node/Bun/Chromium）及预算通过；默认 nodes 测试已隔离 Cargo feature 合并，脚本 53 测试通过。reader 读数/上限未改；仅为按需作者能力登记实际体积及有限余量，待与在线分支合并时再核对。候选包与源码/校验和在 .temp/delivery/evolution-candidate.json，仍非完整 M1/M2 交付。
   [visits/地图/关系宿主](tasks/G-037.md)与[维护/浅恢复](tasks/G-038.md)简报也已准备，按依赖顺序推进；不为准备简报提前认领实现路径。
   演化 worker 明确交接的历史程序加载、跨版本 checkout 和真实确认 UI 已登记为[阅读器组合任务](tasks/G-039.md)，在前述接口收敛后执行；不以脱离产品的控制器测试替代该结果。
4. 三类结果分别记录：包可交付、完整 Goal 关闭、REZICS 实际产品可用。未完成 M2/M3 不关闭 Goal，也不启动重叠的 Web VN 实施。

## 主机与外部边界

维护者 2026-10-29 要求本 Goal 的后续子代理和 worker 使用 high 或更低思考等级；默认 high，机械性工作可更低。当前任务也已调整；后续接续不恢复 xhigh。

Claude Code 2.1.289 的空 OAuth 刷新锁曾在无 CLI 进程、无 owner metadata 的证据下非递归清理并通过启动验证；后续请求仍出现 API403，未代维护者登录。现有及准备任务统一使用可用的 Codex/high，保留已完成的 Claude 工作。

官方 EDB PostgreSQL 18.6 portable 位于 .temp/postgresql/18.6-5/pgsql/bin，来源/校验和在 .temp/postgresql/download.json，版本已记入工具链。已在临时 loopback cluster（fsync/synchronous_commit 均 on）运行 check:server-storage，PGlite/真实数据库共 46 测试通过；日志 .temp/manager-check-server-storage.log。服务器已停止，没有注册服务或更改 PATH。该证据覆盖既有存档宿主，不冒充尚未实现的关系投影装载。

REZICS 的实现只留在[能力请求](../../integrations/rezics.md)，本仓库未改 REZICS。真实联合验收等待宿主准备，未通过前不声称上线。[Web VN](../web-vn/GOAL.md)仍为待启动计划，areas 为空、无 manager。

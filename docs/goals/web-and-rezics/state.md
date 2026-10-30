# State

2026-10-30 接续检查点。manager：codex-01a187ba-9af7-71c1-b110-0ee55db54795。运行事实以 task goal -- status 和实际进程/检查结果为准；[Goal](GOAL.md)的完整 M1/M2/M3 范围未缩小。

## 已验收

- M0 三个旧分支完整审查、合入、验证并归档；Rust 发布集合/关系投影、作者图和节点演化核心保留。旧任务及公共绑定/演化后端/在线任务均已 verified 关闭；goal/g-031 到 goal/g-036 的既有分支保留，相关 worktree 已清理，没有重复派发。维护者 fd4b309 的产品/集成方向保持有效。
- M1 固定包本地与 CI 均已验收：源码 fa0266b1063428e855efea1408768029818500d0，版本 0.1.0-alpha.1，SHA-256 524a383e20ecb147351a496a26a18993aa00f00a19b03c565a123d944bf7ff19。固定包和完整元数据位于 .temp/delivery/m1-release.json，含产品树、资产读数、全部证据与未完成范围；a6f8952 的[M1 CI](https://github.com/rezics/narrata/actions/runs/40923433584)八项全部成功，产品源码树与固定包来源相同；不是整个 Goal 完成。前一 4fa9792 CI 唯一失败为作者测试依赖 wildcard，已以精确版本修复并通过本地四项 cargo-deny 检查。
- 公共 authoring/server、独立 reader/兼容 Wasm、原生内容适配、在线会话/精确构件恢复均在主线。受检同 execution 保留证明跨缓存 miss 线性推进，65,536 提交只验证 65,536 header；跨 StoreId/revision 失效、累计预算和无写入 native reopen 有回归。未知保存重试使用原字节，不重放选择。
- 接受的真实参考入口为 /?reference；旧 / 明确保留为兼容示例。公开创作→预览/修改→发布→真实正文/标签→局部合流、延迟后果、参数 7 子图、两个结局→A/B/在线故障均在同篇作品中验证。冷首屏只有 505-byte 清单、1,155-byte 入口块和 reader Wasm；桌面/390px/未知保存/结局截图已由 manager 查看。
- 最终组合分支与合入后的 main 指向同一提交；R1（25 浏览器）、Web（101 单测、13 在线安装用例及实际冷读旅程）、G5、脚本（53）与文档检查通过。Node/Bun/Chromium 均从独立 tarball 使用公共接口。缺对象、冲突、配额、账号迟到回包及删除重建有用例；无持久格式/冻结语料变更。旧协议测试栈已固定 Vitest 5.0.3/Vite 8.2.4，零审计漏洞且生成无漂移。
- 演化作者端已交付并 verified：规划/描述符发布挂接、受检读取、Wasm/控制器、只读影响评估和浅导出。未知写入再评估状态与读取预算评审缺陷已修复；实际接收字节诚实计量，载入额度仅为宿主提示。M2 阅读器组合仍待下方任务。

旧独立评审与正式 handoff 在 archive/goals/web-and-rezics-2026-10-05/。临时证据保存在 .temp/goal/runs/<任务>/worktree-temp/，manager 验收在对应 manager-acceptance.json。在线最终完整 gate 日志在其 worktree-temp/rebase-integration/manager-r1.log、manager-web.log、manager-g5-final.log；早期失败日志不是通过证据。忽略的 >64 MiB 绑定测试已单独重型运行通过（88 对象、68,985,624 字节）。

逻辑恢复固定 artifact；正文按宿主当前可用修订解析，同一画面/原生片段绑定一次解析修订。原样长期回看并未承诺，仍需宿主保留政策。登录只在线、匿名 IndexedDB；真实 REZICS 联合验收和 npm registry 发布均未进行。

## 下一步

1. M1 固定包的公开接口、本地与 CI 证据已就绪；保持宿主能力状态为“提出”，不声称 REZICS 上线。
2. [visits/读者地图/SQL 宿主](tasks/G-037.md)在原 worktree 以 Codex/high 运行 attempt 5。候选提交 3024622、1af88cc、1f8f906、177439b 尚未合入；核心修复与迟到地图的游标关联修复已在提交上复审通过。R1（26 浏览器）、Web、真实 PG（57 通过/1 个 PGlite 权限对应项跳过）、脚本 53 及文档通过；manager 已查看桌面/390px 地图截图。当前统一修复下述三个剩余问题与回归，不据旧 gate 直接验收。
3. 随后完成[存档维护/浅恢复](tasks/G-038.md)与[阅读器跨构件/确认组合](tasks/G-039.md)。浅历史必须保留精确当前页上下文或在移动根前拒绝；不能用迁移页伪造原页。跨版本时间线/checkout/确认 UI 必须实际可用，不能只凭脱离产品的控制器测试关闭。
4. M2/M3 全项当前版本审计、固定交付证据和归位后才关闭本 Goal；Web VN 仍待启动，不占用其路径。

medium 子代理的范围审计在 .temp/stage-coverage-audit.md：剩余能力都有上述任务或 manager 收敛归属；交接已明确 M1 快照导出/恢复与 M2 有界冷库导入导出、浅历史/GC 的边界，未缩小原 Goal。最终 M3 仍须用完成后的包、源码、CI 与桌面/移动端证据逐项证明；ADR 0022 的关系规模证据已有 100,000 选择点、44,083 行及 15,640,925-byte 分片生成读数，新 SQL loader 的真实 PostgreSQL 装载时间/体积需另行测量，不能沿用旧纯投影数据。

high 的提交评审见 .temp/review-map-core-commit.md、review-map-sql-commit.md、review-map-package.md。原两项核心 P2 与 SQL 数值截断/冷竞争测试缺口均已在 3024622 源码确认修复。1f8f906 的响应游标关联已由 .temp/review-map-race-fix.md 接受，不能再为这项已修复竞态强造原子导出 API；须加强防止页面崩溃假通过的测试，并补账号/新旅程迟到覆盖。剩余已确认三项：SQL 查询对象跨 await 可变，导致校验 B 却返回预先算出的 A；HTTP 地图 JSON 未校验可使 React 崩溃；长地图被 maxHeight 缩到不可读。manager 在全部重型检查结束、最新公开迁移地图测试/类型检查通过后停止 attempt 4，保留五个提交和全部证据，以 .temp/map-final-review-resume.md 在原任务 high 恢复 attempt 5；还需 SQL 实际稀疏边/坏闭包回归、包内 DDL 资产与最终交接。

真实 PG 18.6、fsync/synchronous_commit on 的 100,000 选择点测量：44,083 行，装载 34.13 秒，精确重试 0.82 秒；source/SQL 页均不超过 256 行，10 个参数。19,103 个发布对象的独立载入/验证为 156.62 秒，不混作 SQL 装载耗时。原始计数、表/索引大小在 worker 的 .temp/postgresql/map-validation/scale-loading.json，临时库已停。authoring-only opt-level=z 与分模块压缩已在独立安装 gate 下通过预算（reader 1,694,481/439,038，authoring Wasm 2,664,007/684,148 raw/gzip）；新工具/构建设置已写工具链，仍须结合最终修复重测受影响包。online/index.ts 薄 exportSave 接缝已补精确认领，scope 检查通过。

推送检查点 1ac944b 的[CI](https://github.com/rezics/narrata/actions/runs/41036659729)八项全部成功；产品树与固定 M1 包一致，证据在 .temp/delivery/m1-checkpoint-ci.json 及 m1-release.json。这不代替尚未合入的地图提交 CI。

## 接续约束与主机

维护者 2026-10-29 要求子代理/worker 使用 high 或更低；默认 high，机械性工作可更低，不恢复 xhigh。Claude 先有空刷新锁、后 API403；Codex 也曾因模型容量退出。manager 保留全部工作接手最后预算与验证，没有因此重做或降低验收。后续先验证可用引擎，不把一次观测超时当作任务退出。

PostgreSQL 官方 EDB portable 18.6 在 .temp/postgresql/18.6-5/pgsql/bin；来源/校验和在 .temp/postgresql/download.json。.temp/manager-postgresql.ps1 可用临时 loopback cluster、fsync/synchronous_commit on 运行 check:server-storage，已有 46 项真实/PGlite 存档测试通过并停库。新关系/维护测试必须单独证明其结果，不沿用该存档证据。

本仓库未修改 REZICS，具体宿主请求只在[集成文档](../../integrations/rezics.md)。此前 d060874 与 2d45844 的 CI 全绿，但不替代最新 M1/M2 提交的 CI。

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
2. [visits/读者地图/SQL 宿主](tasks/G-037.md)首次 worker 异常退出（code 4，无 final），保留十二个未提交文件，已在原 worktree 以 Codex/high 恢复 attempt 2；补认领与作者清单一致的 Cargo.lock 依赖更新。Rust 核心已有不可变检查点 1bd032f，high 子代理正独立评审；worker 继续公共绑定、SQL、真实地图与 gate，尚未验收。使用共用 trace、受检稀疏查询与真实 PostgreSQL，不从预取推断访问，不泄漏隐藏目标。
3. 随后完成[存档维护/浅恢复](tasks/G-038.md)与[阅读器跨构件/确认组合](tasks/G-039.md)。浅历史必须保留精确当前页上下文或在移动根前拒绝；不能用迁移页伪造原页。跨版本时间线/checkout/确认 UI 必须实际可用，不能只凭脱离产品的控制器测试关闭。
4. M2/M3 全项当前版本审计、固定交付证据和归位后才关闭本 Goal；Web VN 仍待启动，不占用其路径。

medium 子代理的范围审计在 .temp/stage-coverage-audit.md：剩余能力都有上述任务或 manager 收敛归属；交接已明确 M1 快照导出/恢复与 M2 有界冷库导入导出、浅历史/GC 的边界，未缩小原 Goal。最终 M3 仍须用完成后的包、源码、CI 与桌面/移动端证据逐项证明；ADR 0022 的关系规模证据已有 100,000 选择点、44,083 行及 15,640,925-byte 分片生成读数，新 SQL loader 的真实 PostgreSQL 装载时间/体积需另行测量，不能沿用旧纯投影数据。

## 接续约束与主机

维护者 2026-10-29 要求子代理/worker 使用 high 或更低；默认 high，机械性工作可更低，不恢复 xhigh。Claude 先有空刷新锁、后 API403；Codex 也曾因模型容量退出。manager 保留全部工作接手最后预算与验证，没有因此重做或降低验收。后续先验证可用引擎，不把一次观测超时当作任务退出。

PostgreSQL 官方 EDB portable 18.6 在 .temp/postgresql/18.6-5/pgsql/bin；来源/校验和在 .temp/postgresql/download.json。.temp/manager-postgresql.ps1 可用临时 loopback cluster、fsync/synchronous_commit on 运行 check:server-storage，已有 46 项真实/PGlite 存档测试通过并停库。新关系/维护测试必须单独证明其结果，不沿用该存档证据。

本仓库未修改 REZICS，具体宿主请求只在[集成文档](../../integrations/rezics.md)。此前 d060874 与 2d45844 的 CI 全绿，但不替代最新 M1/M2 提交的 CI。

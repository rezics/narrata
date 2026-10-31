# State

2026-10-31 检查点。manager：codex-01a187ba-9af7-71c1-b110-0ee55db54795。完整范围见 [Goal](GOAL.md)；M0/M1、地图与维护增量已本地验收，M2 阅读器演化组合及 M3 收敛尚未完成。任务状态以 `task goal -- status` 为准。

## 已验收

- M0 保留并审查旧作者图、发布/关系和节点演化分支，M1 公共 authoring/server、独立 reader/兼容 Wasm、原生内容、在线生命周期及精确构件恢复均已合入。旧任务归档与分支保留，不重新派发。维护者 fd4b309 的产品/集成方向继续有效。
- M1 历史固定包：版本 0.1.0-alpha.1，源码 fa0266b1063428e855efea1408768029818500d0，SHA-256 524a383e20ecb147351a496a26a18993aa00f00a19b03c565a123d944bf7ff19；完整元数据 `.temp/delivery/m1-release.json`。a6f8952 及 1ac944b 同产品树 CI 八项全绿，不代替地图增量及最终阶段 CI。
- [地图与 SQL 交付](../../../archive/goals/web-and-rezics-2026-10-05/tasks/G-037.md)已合入主线源码 5c02e6107c20858b67a0ce9d643b3b9de73bc218，并完成主线独立安装复验。固定增量包 SHA-256 e2751a827b71629cf139e5d371123787af80cc19635d6285f1c4756f1fa0aaf3，版本仍为 0.1.0-alpha.1；文件及元数据 `.temp/delivery/reader-map-release.json`。同版本不同包不能互换，M1 历史包不含此增量。
- visits 共用受检祖先审计和执行 trace，保留迁移/游标来源；稀疏关系保留原 edge_no，地图只显示实际揭示事实。服务器装载/查询走 Rust 事实校验与真实 PostgreSQL；独立 `/reader-map`、`/reader-map/react` 仅含解码与可选渲染，作者图与 reader 原上限不变，全部地图字节计入总量。
- 全部已报告问题已复审解决：无正文转移、空历史 incomplete、JS/Wasm 数值截断、冷装载竞争、异步可变查询、迟到地图、HTTP JSON、长图可读性及 i32 坐标边界。主线和受检候选只差五份管理/交接文档；111 个包文件仅 README/预算说明的换行不同，可执行资产完全相同。候选最终 R1（26 浏览器）和真实 PG（61 通过、1 个仅 PGlite 权限对应项跳过）证据复用；主线 Web 再次通过 109 包测试、13 安装测试及 Node/Bun/Worker/浏览器旅程。未变的脚本套件 53 项通过。
- 已查看主线安装包桌面/390px 长图截图：800 个合成渲染点的末点可达，圆直径 36px、文字框 17px、视口 400px、无页面横溢；实际短篇的迟到/账号/新旅程、24 个无效响应用例及未知/配额/冲突入口保护通过。冷首屏只有 manifest/入口块和 reader Wasm，无地图/作者工具请求。

## 证据与保留政策

地图 manager 证据在 `.temp/goal/runs/G-037/manager-preserved/` 和 `main-acceptance/`，含复审、完整交接、候选/主线包差异、日志、网络与截图；候选原始源码 44a0641 在交接中保留。关闭时保留合入分支，worktree 临时证据由 goalctl 归入对应 run，固定包已独立保存在 `.temp/delivery/`。

实际 100,000 选择点投影 44,083 行；真实 PG 18.6 装载 34.13 秒、精确重试 0.82 秒，源/SQL 页均最多 256 行、10 参数；表 6,692,864 bytes、索引 10,846,208 bytes。19,103 个发布对象载入/验证另计 156.62 秒。测量在 `.temp/goal/runs/G-037/worktree-temp/postgresql/map-validation/scale-loading.json`，与范围修复无关的装载代码未变，未重复昂贵测量。

地图最终计量 raw/gzip：reader Wasm 1,694,481/439,038，作者 Wasm 2,665,048/684,683，作者图 221,680/50,240，readerMap 56,925/7,904，总 JS 649,023/137,919。仅新增 readerMap 59,000/8,400 上限；原 reader/作者图/总量上限均未提高。

逻辑恢复固定 artifact；正文依宿主当前可用修订解析，同画面/原生片段绑定一次解析快照。不承诺历史正文永久原样。匿名 IndexedDB、登录只在线；真实 REZICS 联合验收及 npm registry 发布未进行，宿主请求仍为“提出”。

## 下一步

1. 地图增量和 CI 前置修复已验收：12d0736 给两个存储 Task 加 web:build 依赖并补服务器 Rust/Wasm，ec233fa 的[CI](https://github.com/rezics/narrata/actions/runs/41058220569)八项全绿。固定 SDK 代码/包未变，元数据 `.temp/delivery/reader-map-release.json`、CI 证据 `reader-map-ci.json` 已更新；M3 仍须最终全部能力对应证据。
2. [存档维护/浅恢复](../../../archive/goals/web-and-rezics-2026-10-05/tasks/G-038.md)已 verified 归档、保留分支并推送 2a92cc5；[维护增量 CI](https://github.com/rezics/narrata/actions/runs/41083435285)八项全绿，复用 kernel，保留精确浅页上下文或拒绝，不实施 ADR 0021 推迟的在线原地压缩/shadow 切换。
3. [阅读器跨构件/确认组合](tasks/G-039.md)从 2a92cc5 开始，attempt 1 因网络/TLS 故障退出（code 4），无正式交接或提交；20 份工作文件及全部证据保留。日志证明退出前 R1（26 浏览器）、Web 与 WebStorage（19 浏览器）已通过，仍不是任务验收。2026-10-31 已在同一工作区恢复 Codex/high attempt 2；补入独立评审确认的 native/legacy 旧构件恢复分派和迁移核实后陈旧错误提示两项修复，之后完成实际 A/B/C、地图/维护组合及全部可见结果矩阵。实时 PID/检查以 goalctl 为准。
4. 按原 M1/M2/M3 全项审计、归位长期决定、归档任务后才关闭本 Goal；Web VN 仍待启动，不占用重叠路径。

## 维护评审检查点

维护已合入主线 ffc0f966578c184ef45f36559001bc029fbdc3a2；代码与受检 9c888ce 完全相同，仅三份 Goal 文档不同。固定包 SHA-256 120771d9253c6185bf3e4a4ca3e5a639600fcd060dceaba791c6fbd0517c54b8，1,701,898 bytes，元数据 `.temp/delivery/maintenance-release.json`。主线重新构建/安装后的 tarball 整包逐字节一致；121 包测试、25 安装故障测试、Node/Bun/浏览器冷旅程通过。复用同代码 G5、R1（26 浏览器）、存储浏览器 19 项、实库 75 通过/3 个预期 PGlite 跳过、脚本 53 及文档检查。原 race-fixture 的 stale reader 已正确 reload 并全量实库重跑，失败日志不是通过证据。

全部评审项已关闭：6159691 的原生读前限额、损坏 manifest 容量、实际目标 kind、目的库副本校验；9b0006b 的生产默认冷主机；a0f8f5d 的输入/inspect 预算；ce86e8e 的 UTF-8 计量。最终复审在 `.temp/review-maintenance-gc-repair.md`、`review-shallow-import-repair.md`、`review-maintenance-host-repair.md`、`review-maintenance-public-repair.md`、`review-maintenance-utf8-repair.md`。真实 Wasm 9,996 操作成功、10,009 越界零写入/旧根可继续；64MiB 队列再加一字节拒绝；真实冷页/点读/缺失读取、已提交但失回包、实际删库重建和每个历史夹具的明确继续结果均已验证。现有 v1/布局/冻结语料无变化。

冷加载边界已在 `.temp/review-maintenance-load-budget.md` 澄清：load_bytes 是不透明可信 CacheStore 的编码回包累计接纳预算，可能先收到一个协议有界的超额回包，再收费/拒绝并清候选；不是外部 provider 的预分配上限。拒绝在 maintenance 自身 decode/raw.load/Wasm copy 前，AuthenticatedLibrary 可能先作协议解码。现有 16MiB/value、64MiB payload、128MiB message 与每请求四值约束给出固定单回包瞬时界；不是要求另造传输的新缺陷。字段注释和正预算超额首包/累计回包测试应真实，不宣称拦截了 provider 的全部分配。

当前 B 根加完整历史 pack 的注册/重开已交付，复核记录在 `.temp/review-history-registration.md`；历史名字表、旧 A 根后 NodeBook/Session 一致性及各自懒 loader 仍归下一任务。不能把省略 timeline/identity/analysis 的断言、笼统 history_truncated 或终态相等算完整继续阅读。上述维护证据已齐备并完成本地验收，剩余历史组合在下一任务验证；不能据此声称完整 M2 或全 Goal 完成。
维护最终 raw/gzip：reader Wasm 1,373,924/385,105，作者 Wasm 2,896,459/737,581，总 JS 631,853/135,714；reader/作者图原上限不变。SDK 三个 Wasm 采用 z 与按模块 ESM 压缩已写工具链。manager 证据在 `.temp/goal/runs/G-038/manager-preserved/` 与 `main-acceptance/`，原始失败实库日志由最终成功重跑覆盖，不作通过证据。

M3 契约文档已在 f903f5d 按源码与 Accepted ADR0022 收敛：更新已揭示地图状态/坐标，不删章节锁/社区比例等后续意图；choices/content 的机械规则回指类型和测试，179 文件链接检查通过。最终 Goal 关闭还须处理 `.temp/final-contract-docs-audit.md` 列出的三条长期文档入链，不能只归档目录留下悬空链接。

## 接续约束与主机

子代理/worker 使用 high 或更低，默认 high；机械工作可 medium，不恢复 xhigh。地图 attempt 7 在原 worktree 用 fresh CLI 上下文清除了旧中断的 missing-tool-output 历史，所有成果保留；此前模型容量/认证故障不意味着重做任务或降低验收。等待以实际 PID/会话为准，不因观测超时重启。

PostgreSQL 官方 EDB portable 18.6 在 `.temp/postgresql/18.6-5/pgsql/bin`，来源/校验和在 `.temp/postgresql/download.json`；`.temp/manager-postgresql.ps1` 使用隔离 loopback cluster、fsync/synchronous_commit on 并最终停库。维护的新实库结果必须单独证明，不能沿用地图/旧存档测试。临时文件都在 `.temp/`。不修改 REZICS，能力请求只记入[集成文档](../../integrations/rezics.md)。

# State

2026-10-30 检查点。manager：codex-01a187ba-9af7-71c1-b110-0ee55db54795。完整范围见 [Goal](GOAL.md)；M0/M1 和地图增量已本地验收，M2 维护与阅读器演化组合、M3 收敛尚未完成。任务状态以 `task goal -- status` 为准。

## 已验收

- M0 保留并审查旧作者图、发布/关系和节点演化分支，M1 公共 authoring/server、独立 reader/兼容 Wasm、原生内容、在线生命周期及精确构件恢复均已合入。旧任务归档与分支保留，不重新派发。维护者 fd4b309 的产品/集成方向继续有效。
- M1 历史固定包：版本 0.1.0-alpha.1，源码 fa0266b1063428e855efea1408768029818500d0，SHA-256 524a383e20ecb147351a496a26a18993aa00f00a19b03c565a123d944bf7ff19；完整元数据 `.temp/delivery/m1-release.json`。a6f8952 及 1ac944b 同产品树 CI 八项全绿，不代替地图增量及最终阶段 CI。
- [地图与 SQL 交付](../../../archive/goals/web-and-rezics-2026-10-05/tasks/G-037.md)已合入主线源码 5c02e6107c20858b67a0ce9d643b3b9de73bc218，并完成主线独立安装复验。固定增量包 SHA-256 e2751a827b71629cf139e5d371123787af80cc19635d6285f1c4756f1fa0aaf3，版本仍为 0.1.0-alpha.1；文件及元数据 `.temp/delivery/reader-map-release.json`。同版本不同包不能互换，M1 历史包不含此增量。
- visits 共用受检祖先审计和执行 trace，保留迁移/游标来源；稀疏关系保留原 edge_no，地图只显示实际揭示事实。服务器装载/查询走 Rust 事实校验与真实 PostgreSQL；独立 `/reader-map`、`/reader-map/react` 仅含解码与可选渲染，作者图与 reader 原上限不变，全部地图字节计入总量。
- 全部已报告问题已复审解决：无正文转移、空历史 incomplete、JS/Wasm 数值截断、冷装载竞争、异步可变查询、迟到地图、HTTP JSON、长图可读性及 i32 坐标边界。主线和受检候选只差五份管理/交接文档；111 个包文件仅 README/预算说明的换行不同，可执行资产完全相同。候选最终 R1（26 浏览器）和真实 PG（61 通过、1 个仅 PGlite 权限对应项跳过）证据复用；主线 Web 再次通过 109 包测试、13 安装测试及 Node/Bun/Worker/浏览器旅程。未变的脚本套件 53 项通过。
- 已查看主线安装包桌面/390px 长图截图：800 个合成渲染点的末点可达，圆直径 36px、文字框 17px、视口 400px、无页面横溢；实际短篇的迟到/账号/新旅程、24 个无效响应用例及未知/配额/冲突入口保护通过。冷首屏只有 manifest/入口块和 reader Wasm，无地图/作者工具请求。

## 证据与保留政策

地图 manager 证据在 `.temp/goal/runs/G-037/manager-preserved/` 和 `main-acceptance/`，含复审、完整交接、候选/主线包差异、日志、网络与截图；候选原始源码 44a0641 在交接中保留。关闭时保留合入分支，worktree 临时证据由 goalctl 归入对应 run，固定包已独立保存在 `.temp/delivery/`。

实际 100,000 选择点投影 44,083 行；真实 PG 18.6 装载 34.13 秒、精确重试 0.82 秒，源/SQL 页均最多 256 行、10 参数；表 6,692,864 bytes、索引 10,846,208 bytes。19,103 个发布对象载入/验证另计 156.62 秒。测量在 worker `.temp/postgresql/map-validation/scale-loading.json`，与范围修复无关的装载代码未变，未重复昂贵测量。

地图最终计量 raw/gzip：reader Wasm 1,694,481/439,038，作者 Wasm 2,665,048/684,683，作者图 221,680/50,240，readerMap 56,925/7,904，总 JS 649,023/137,919。仅新增 readerMap 59,000/8,400 上限；原 reader/作者图/总量上限均未提高。

逻辑恢复固定 artifact；正文依宿主当前可用修订解析，同画面/原生片段绑定一次解析快照。不承诺历史正文永久原样。匿名 IndexedDB、登录只在线；真实 REZICS 联合验收及 npm registry 发布未进行，宿主请求仍为“提出”。

## 下一步

1. 保留固定增量包和本地验收记录，推送地图主线并跟踪其实际 CI；M3 最终仍需当前全部能力与固定交付物对应的证据。
2. 启动[存档维护/浅恢复](tasks/G-038.md)：复用 kernel，浅历史保留精确当前页上下文或在移动 durable roots 前拒绝；不以 migration_page 伪造原页，不实施 ADR 0021 推迟的在线原地压缩/shadow 切换。
3. 随后完成[阅读器跨构件/确认组合](tasks/G-039.md)：历史 A/B 程序与各自 loader 按需注册、时间线/checkout/再选择、确认及落盘故障在同一实际参考旅程可用，地图与维护需组合验证。
4. 按原 M1/M2/M3 全项审计、归位长期决定、归档任务后才关闭本 Goal；Web VN 仍待启动，不占用重叠路径。

## 接续约束与主机

子代理/worker 使用 high 或更低，默认 high；机械工作可 medium，不恢复 xhigh。地图 attempt 7 在原 worktree 用 fresh CLI 上下文清除了旧中断的 missing-tool-output 历史，所有成果保留；此前模型容量/认证故障不意味着重做任务或降低验收。等待以实际 PID/会话为准，不因观测超时重启。

PostgreSQL 官方 EDB portable 18.6 在 `.temp/postgresql/18.6-5/pgsql/bin`，来源/校验和在 `.temp/postgresql/download.json`；`.temp/manager-postgresql.ps1` 使用隔离 loopback cluster、fsync/synchronous_commit on 并最终停库。维护的新实库结果必须单独证明，不能沿用地图/旧存档测试。临时文件都在 `.temp/`。不修改 REZICS，能力请求只记入[集成文档](../../integrations/rezics.md)。

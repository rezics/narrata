# State

2026-10-29 的接续检查点。当前[Goal](GOAL.md)已按首发 SDK 与完整阶段分出里程碑；本次仅更新规划，没有启动、恢复、合并或关闭实现任务。运行时事实以 task goal -- status 为准。

## 当前事实

- 主分支盘点基线为 3c2e84c；本次规划提交将在它之后。Rust 创作核心及诊断修复已合入至 d65939d；旧检查点把它写成 Running 已过时。
- status 显示 0 个存活 worker，三个 open 任务均已退出。主 checkout 和三个工作区在盘点时干净；工作区与分支完整保留。
- 已合入基础包括 Gamebook 执行与分块加载、kernel 历史/IndexedDB、服务器存档参考宿主及一致性套件、npm reader/react/storage/testing、内容解析上下文与预取。
- 未在 main 交付的公共能力包括 authoring/server Wasm/npm 入口、reader 构建拆分、网络存档客户端生命周期、宿主原生富文档适配、构件寻址恢复包装，以及 M2 的完整图/演化/维护产品链。已有分支成果见下表，不能当作从零未实现。

## 未关闭任务与成果

| 任务 | 保留位置与分支头 | 证据边界与下一步 |
| --- | --- | --- |
| [作者图](tasks/G-031.md) | .temp/worktrees/g-031，goal/g-031，2e9b490 | attempt 1 有交接，attempt 2 有追加生产/移动端修复但没有正式最终交接；审查完整分支，不只取第一轮提交。日志有通过记录，未来仍需按当前 main 复核 |
| [发布集合与关系投影](tasks/G-032.md) | .temp/worktrees/g-032，goal/g-032，8ef71de | 已有 Rust 模块、schema、冻结语料与 CLI 产物，缺正式交接；历史日志中相关测试、Wasm 检查、Clippy 与 check:r1 完成。先审查接续，作为 M1 发布和绑定依赖 |
| [节点构件演化](tasks/G-033.md) | .temp/worktrees/g-033，goal/g-033，802dbfc | 已有核心、schema、语料与跨构件会话，缺正式交接；历史节点测试/Clippy 有通过记录，最后 check:r1 中断，不能算全 gate 通过。补审查与检查后接 M2 完整链路 |

以上是已有退出记录，本次没有重新跑构建或产品测试。退出码与无交接不抹除分支成果；未来 manager 按实际差异决定原任务恢复、验证与合入。

## 下一步

1. 读最新维护者文档与实时 status，核对上述分支、认领和日志；恢复现场时按 Goal 程序登记当前 manager 会话。
2. 优先审查发布集合分支，确认其抽出的发布核心可作 Wasm/公开绑定基础；同时收敛作者图任务对 npm 配置、构建脚本、预算和阅读器源码的认领，或按程序明确移交。清除这些重叠后才派发 M1 相关简报。三个既有任务保持原身份接续，不重复派发。
3. 按 Goal 的 M1 工作包补公共 authoring/server、reader 拆分、富文档接缝、在线存档及精确构件恢复；新简报由 manager 经 task goal -- new 生成。
4. M1 完成时交付固定 tarball 与模拟宿主证据；继续 M2 作者图、读者地图、演化与维护。包可交付、完整 Goal 完成、REZICS 产品上线分别记录。

## 外部依赖与移交

REZICS 的实现全部留在[能力请求](../../integrations/rezics.md)；不依据旧 Occurrence/Expression 模型派发宿主实现。真实联合验收等待宿主准备，未通过前不声称产品上线。

[Web VN](../web-vn/GOAL.md)已写待启动计划，areas 为空、没有登记 manager。现有 Goal 关闭或明确移交未被 open 任务占用的路径后，才确定其实际范围并登记。

# State

2026-10-29 接续实施检查点。manager 会话为 codex-01a187ba-9af7-71c1-b110-0ee55db54795；[Goal](GOAL.md)的 M1/M2/M3 边界保持不变。运行时事实以 task goal -- status 为准。

## 当前事实

- 接续基线为维护者规划提交 fd4b309；已阅读并保留最新产品决定与集成请求。三个旧分支均已独立静态审查，未发现阻止接续的结构性缺陷。
- 作者图完整三提交已合入至 df86d34；尚未关闭认领，等待当前 main 的整批 gate。另两个原任务补交接/检查，没有重复派发。
- 已合入基础包括 Gamebook 执行与分块加载、kernel 历史/IndexedDB、服务器存档参考宿主及一致性套件、npm reader/react/storage/testing、内容解析上下文与预取。
- 新增 StorageHost 未知保存结果暂停与原字节重试、Book.retrySave/显式重载，以及修订绑定的原生文档适配器；存储单测 58 项、Web 单测 35 项及类型检查通过。可安装证据与最终预算仍在整批集成中。
- check:r1 已加入 authoring tests/wasm-target 与 nodes 全 feature 测试；CI 复用同一 Task。脚本 52 项及文档检查通过。
- 当前依赖审计发现旧 Vite/source-map-js/Zod 锁定版本阻断 gate。main 已准备兼容更新（Vite 8.2.4、Zod 4.6.7），不降低审计门槛；待整批 gate 验证。
- 未交付能力仍包括公共 authoring/server、reader 构建拆分、完整在线会话生命周期与构件寻址恢复，以及 M2 的读者地图、演化规划/绑定与维护公共接口。

## 未关闭任务与成果

| 任务 | 保留位置与分支头 | 证据边界与下一步 |
| --- | --- | --- |
| [作者图](tasks/G-031.md) | 已合入 main；完整保留三提交，重放后头 df86d34 | 正式交接已补。check:web（40 单测与 tarball）、kernel JS 检查、23 浏览器测试及桌面/390px 移动端三层图旅程通过；check:r1 仅审计失败，不能记成全通过。manager 已看新截图 |
| [发布集合与关系投影](tasks/G-032.md) | .temp/worktrees/g-032，goal/g-032，8ef71de | 正式交接已补；221 Rust 测试、Clippy、Wasm 与两项规模测试通过。68,469,768 字节程序块的 88 对象集合可重开；100k 选择点为 44,083 行、15,640,925 字节/10 片。check:r1 仅审计失败；其余 19 浏览器测试另跑通过。待合入 |
| [节点构件演化](tasks/G-033.md) | .temp/worktrees/g-033，goal/g-033，原核心 802dbfc 后追加修复 | 发现并修复 checkpoint import 的 incompatible_save 诊断退化，新增两项回归；正在补最终检查和交接，不能提前计作完成 |
| [公共创作与精简 reader](tasks/G-034.md) | 简报已生成、未派发 | 等作者图与发布集合的认领经整批验证关闭后再启动；复用对象发布核心，不重做作者核心 |

所有日志/截图留在对应 worktree 的 .temp 下；close 会保存到任务 run 目录再移除 worktree。旧冻结语料保持不变；新发布/关系/演化语料分别有既有 ADR 0022/0023。

## 下一步

1. 接受原生内容安装验证，提交依赖/预算/工具链更新；补读演化正式交接，审查新增修复；合入发布与演化。
2. 从整批 main 运行受影响 gate：check:r1、check:web、check:web-storage、test:scripts 与 docs:check。关闭三个原任务后派发已准备的公共创作/reader 简报。
3. 补完整在线生命周期与精确构件恢复；原字节重试遇 Conflict 仍需由 durable roots/history 核实候选，不能把对象存在或修订增大当作保存成功。
4. M1 完成时交付固定 tarball 与模拟宿主证据；继续 M2 读者地图、关系装载、演化与维护。包可交付、完整 Goal 完成、REZICS 产品上线分别记录。

## 外部依赖与移交

REZICS 的实现全部留在[能力请求](../../integrations/rezics.md)；不依据旧 Occurrence/Expression 模型派发宿主实现。真实联合验收等待宿主准备，未通过前不声称产品上线。

[Web VN](../web-vn/GOAL.md)已写待启动计划，areas 为空、没有登记 manager。现有 Goal 关闭或明确移交未被 open 任务占用的路径后，才确定其实际范围并登记。

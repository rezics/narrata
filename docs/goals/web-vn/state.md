# State

2026-10-31：VN0 语义与兼容决定已完成，[ADR 0024](../../adr/0024-first-party-visual-novel.md)已接受；VN0 执行与兼容实现尚未完成，VN1–VN3 也未交付。manager 为 codex-01a187ba-9af7-71c1-b110-0ee55db54795-web-vn。[设计任务](../../../archive/goals/web-vn-2026-10-31/tasks/G-040.md)已干净交接，下一项[原生执行与双格式兼容](tasks/G-041.md)负责实现和新语料；实际认领与进程以 task goal -- status 为准。

起点：[完整 Gamebook SDK](../../../packages/narrata/web/README.md)，源码 a36d4966d7bf40df9e03f38e5d080885d73595ff，版本 0.1.0-alpha.1，tarball SHA256 0a1ed393ab8b60ad1400609fcd26b0a060911917fce88169efc459d25819da30；身份见[固定输入](../../integrations/rezics.md#narrata-自己的交付依赖)。旧 Goal 已归档、全部认领释放，主线 ed5733d 的 [CI](https://github.com/rezics/narrata/actions/runs/41188638814) 八项成功；Gamebook 基线保留，不重新派发其已完成工作。

原生实现处于 G-041 工作分支验收，尚未合并。首轮 G5 已完整通过；首轮 R1 的 Wasm 与 28 项浏览器测试通过，但最终生成文件检查因新 schema 尚未提交而失败，不能记作 R1 通过。manager 已在 G5 结束后保存候选源码/语料并停止、接续同一 worker，修正同资源音乐 Play/Keep 的意图区分，并补齐清除旧背景/图层、共享后端跨会话 stale Advance 的回归证据。后续须提交生成文件并对最终源码重跑 R1；实际进程与阶段仍以实时账本为准。

验收继续保留旧 Gamebook format2/schema1/revision2，VN 显式采用新源/对象版本；复用同一 Session/历史/存储和程序块加载。逐行 safe point、场景/音乐/语音意图、stale/未知保存与冷恢复须有原生测试和新冻结语料证明；不把部分 gate 或设计接受当作实现通过。旧 Scene/Flow 仅作行为依据，不向 RuntimeStateV0 叠加产品能力。已有冻结语料只读。

复审证据在 `.temp/review-vn0-adr-320e521.md` 和 `.temp/vn0-decision-review-checklist.md`；修订明确引用参与身份、资源版本复用、账号/授权上下文隔离、幂等与 stale 区别，以及公开 VN 作者/预览和 Node/Bun 交付证据。manager 另澄清必要逻辑块可在提交前加载，正文/媒体就绪不作为逻辑提交前提。

worker 优先 gpt-6-luna/max：设置 GOAL_ID=web-vn、GOAL_ENGINE=codex、GOAL_CODEX_MODEL=gpt-6-luna，再生成/派发简报；简报 effort=max。保留完整 VN1 播放器、VN2 按需分发、VN3 短篇/安装包结果，当前设计不代表这些结果已实现。

REZICS 的媒体与内容资源实现只列在[宿主能力](../../integrations/rezics.md#web-vn-后续需要的宿主能力)，本 Goal 不实施。当前仅设计已接受，代码仍在验收，不代表 VN 产品能力已交付。

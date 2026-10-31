# State

2026-10-31：进入 VN0 语义与兼容设计；尚未交付 VN 实现。manager 为 codex-01a187ba-9af7-71c1-b110-0ee55db54795-web-vn。首项[语义与兼容决定](tasks/G-040.md)只交付待审阅 ADR，之后再派实现、类型/schema、确定性与兼容语料任务；实际认领与进程以 task goal -- status 为准。

起点：[完整 Gamebook SDK](../../../packages/narrata/web/README.md)，源码 a36d4966d7bf40df9e03f38e5d080885d73595ff，版本 0.1.0-alpha.1，tarball SHA256 0a1ed393ab8b60ad1400609fcd26b0a060911917fce88169efc459d25819da30；身份见[固定输入](../../integrations/rezics.md#narrata-自己的交付依赖)。旧 Goal 已归档、全部认领释放，主线 ed5733d 的 [CI](https://github.com/rezics/narrata/actions/runs/41188638814) 八项成功；Gamebook 基线保留，不重新派发其已完成工作。

下一步：根据[目标](GOAL.md)和只读侦察 `.temp/vn0-reuse-scout.md`，选定最小第一方 VN 语义与兼容路线，先写待审阅 ADR，再派实现、类型/schema 与新兼容语料。当前未选持久字段、编码或公开媒体 API；旧 Scene/Flow 只作行为依据，不向 RuntimeStateV0 叠加产品能力。已有冻结语料只读。

worker 优先 gpt-6-luna/max：设置 GOAL_ID=web-vn、GOAL_ENGINE=codex、GOAL_CODEX_MODEL=gpt-6-luna，再生成/派发简报；简报 effort=max。保留完整 VN1 播放器、VN2 按需分发、VN3 短篇/安装包结果，当前设计不代表这些结果已实现。

REZICS 的媒体与内容资源实现只列在[宿主能力](../../integrations/rezics.md#web-vn-后续需要的宿主能力)，本 Goal 不实施。当前完成的是计划，不是 VN 产品能力。

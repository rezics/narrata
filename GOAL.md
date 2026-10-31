# Narrata Goals

当前交付顺序：先完成 Gamingbook 的正文与分支选择，再推进纯 Web 传统视觉小说。REZICS 的能力请求只写在[集成文档](docs/integrations/rezics.md)，由 REZICS 自行实施。产品方向见[目标](docs/product/goal.md)与[编号决定](docs/product/decisions.md)。

## 当前实施 Goal

| Goal | 结果与顺序 | Manager |
| --- | --- | --- |
| [Gamebook 第一阶段](docs/goals/web-and-rezics/GOAL.md) | M0 接续既有成果 → M1 首发 SDK 可交付 → M2 图、演化与维护完整交付 → M3 验证、交接与收敛 | 已登记为 narrata-web-and-rezics；接续时按程序更新会话 |

M1 的可安装包可先交给 REZICS 接入；完整阶段关闭仍需满足 M2/M3。模拟宿主通过表示 Narrata 交付证据齐备，Gamingbook 的实际产品可用还需双方固定版本的真实联合验收。

## 待启动 Goal

| Goal | 启动依赖 | 结果 |
| --- | --- | --- |
| [Web VN 与按需分发](docs/goals/web-vn/GOAL.md) | Gamebook SDK 交付基线成立，并且现有 Goal 已关闭，或双方明确移交没有未关闭认领的路径 | 逐句对白、可恢复场景与媒体状态、按需资源加载、可安装播放包与短篇样例 |

Web VN 的计划已落地，尚未登记 manager 或认领实现路径。启动前按其 Goal 填写实际范围；两个 Goal 不同时占用现有 packages/narrata/** 的范围。

## 接续

[Goal 程序](docs/goals/README.md)说明登记、派发、合并与关闭；manager 遵循[章程](docs/goals/manager.md)，worker 遵循[协议](docs/goals/worker.md)。

2026-10-31：Gamebook 的 M0/M1/M2 完整 SDK 已通过主线本地固定包验收，继续当前 CI、最终交接与 Goal 收敛。已关闭任务分支保留、认领释放；Web VN 仍待启动。读[检查点](docs/goals/web-and-rezics/state.md)恢复现场，不重新派发已完成工作。实时状态以 task goal -- status 为准。

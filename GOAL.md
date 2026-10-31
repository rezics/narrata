# Narrata Goals

当前交付顺序：先完成 Gamingbook 的正文与分支选择，再推进纯 Web 传统视觉小说。REZICS 的能力请求只写在[集成文档](docs/integrations/rezics.md)，由 REZICS 自行实施。产品方向见[目标](docs/product/goal.md)与[编号决定](docs/product/decisions.md)。

## 已交付基线

[Gamebook SDK](packages/narrata/web/README.md) 已完成首发及完整阶段能力交付；固定源码、包校验和与验收记录见[集成交接](docs/integrations/rezics.md#narrata-自己的交付依赖)。

模拟宿主和独立安装验收表示 Narrata SDK 交付证据齐备；Gamingbook 的实际产品可用还需双方固定版本的真实联合验收。

## 待启动 Goal

| Goal | 启动依赖 | 结果 |
| --- | --- | --- |
| [Web VN 与按需分发](docs/goals/web-vn/GOAL.md) | Gamebook SDK 交付基线成立，并且现有 Goal 已关闭，或双方明确移交没有未关闭认领的路径 | 逐句对白、可恢复场景与媒体状态、按需资源加载、可安装播放包与短篇样例 |

Web VN 的计划已落地，尚未登记 manager 或认领实现路径。启动前按其 Goal 填写实际范围；两个 Goal 不同时占用现有 packages/narrata/** 的范围。

## 接续

[Goal 程序](docs/goals/README.md)说明登记、派发、合并与关闭；manager 遵循[章程](docs/goals/manager.md)，worker 遵循[协议](docs/goals/worker.md)。

2026-10-31：Gamebook 完整固定包已通过本地与主线 CI 验收，任务分支保留、认领释放。Web VN 仍待启动；按其计划核对范围与启动条件，不重新派发已完成的 Gamebook 工作。实时状态以 task goal -- status 为准。

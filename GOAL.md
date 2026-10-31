# Narrata Goals

当前交付顺序：先完成 Gamingbook 的正文与分支选择，再推进纯 Web 传统视觉小说。REZICS 的能力请求只写在[集成文档](docs/integrations/rezics.md)，由 REZICS 自行实施。产品方向见[目标](docs/product/goal.md)与[编号决定](docs/product/decisions.md)。

## 已交付基线

[Gamebook SDK](packages/narrata/web/README.md) 已完成首发及完整阶段能力交付；固定源码、包校验和与验收记录见[集成交接](docs/integrations/rezics.md#narrata-自己的交付依赖)。该固定 Gamebook 基线继续保留，Web VN 不重新派发或改写已交付能力。

模拟宿主和独立安装验收表示 Narrata SDK 交付证据齐备；Gamingbook 的实际产品可用还需双方固定版本的真实联合验收。

## 当前 Goal

| Goal | 当前阶段 | 范围 |
| --- | --- | --- |
| [Web VN 与按需分发](docs/goals/web-vn/GOAL.md) | VN0：原生执行与兼容验收 | VN0 → VN1 → VN2 → VN3 |

VN0 原生执行与双格式兼容实现已合入主线提交 `f1adb040de40fc2b7b1f973afb64d562475343e5`，G5、R1、Web、scripts、docs 五项 gate 已通过并验收。下一项 G-042 是 VN1 可安装阅读 API，当前待派发；播放器 UI、媒体解析/播放、作者入口、短篇样例与 VN2–VN3 仍未交付。实际管理器登记、任务与路径认领以 goalctl 账本为准。

## 接续

[Goal 程序](docs/goals/README.md)说明登记、派发、合并与关闭；manager 遵循[章程](docs/goals/manager.md)，worker 遵循[协议](docs/goals/worker.md)。

2026-10-31：Gamebook 完整固定包继续作为已交付基线。VN0 原生执行与双格式兼容已在 `f1adb040de40fc2b7b1f973afb64d562475343e5` 通过五项主线 gate 并验收。G-042（VN1 可安装阅读 API）待派发；播放器、媒体、作者流程、短篇样例及 VN2–VN3 尚未交付。实时状态以 `task goal -- status` 为准。

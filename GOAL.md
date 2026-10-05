# Narrata Goals

同时可以运行多个 Goal，每个 Goal 一个 manager。每个 Goal 的目录写明它的结果、它独占的路径
范围（其他 Goal 的任务简报不得认领）和当前状态；[Goal 程序](docs/goals/README.md) 说明它们
如何运行和结束。[目标](docs/product/goal.md) 与 [编号决定](docs/product/decisions.md) 比任何
Goal 都长寿。

| Goal | 结果 | Manager |
| --- | --- | --- |
| [叙事核心](docs/goals/narrative-core/GOAL.md) | 节点栈不含正文、两类选择、动态节点、按块切分支撑 10 万选择点、发布时分析与布局 | 等待维护者启动 |
| [Web 与 REZICS](docs/goals/web-and-rezics/GOAL.md) | npm 包与 Wasm 运行时、带正文渲染插槽的阅读器外壳、图视图、REZICS 适配 | 等待前两个 Goal 的协议，之后由维护者启动 |

`task goal -- status` 显示运行中的 Goal、manager 会话和存活的任务。启动 worker 前确认所用
引擎的 CLI 已登录。

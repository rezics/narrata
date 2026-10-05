# Narrata Goals

同时可以运行多个 Goal，每个 Goal 一个 manager。每个 Goal 的目录写明它的结果、它独占的路径
范围（其他 Goal 的任务简报不得认领）和当前状态；[Goal 程序](docs/goals/README.md) 说明它们
如何运行和结束。[目标](docs/product/goal.md) 与 [编号决定](docs/product/decisions.md) 比任何
Goal 都长寿。

| Goal | 结果 | Manager |
| --- | --- | --- |
| [Gamebook 第一阶段](docs/goals/web-and-rezics/GOAL.md) | REZICS 实现 Gamebook 所需的全部 Narrata 能力：创作与发布接口、npm/Wasm 包与阅读器外壳、存档同步、作品演化、图视图与关系投影；模拟宿主验收 | 本机 manager `narrata-web-and-rezics` |

REZICS 接入由另一台电脑上的独立 manager 在 REZICS 仓库执行，见
[REZICS 接入 Goal 交接简报](docs/integrations/rezics-goal.md)。真实联合验证是该 Goal 的最后一个任务，
不阻塞本机 SDK Goal 的模拟宿主验收。

`task goal -- status` 显示运行中的 Goal、manager 会话和存活的任务。启动 worker 前确认所用
引擎的 CLI 已登录。

# Goal manager 章程

manager 是代表维护者运行一个 Goal 的交互式代理会话；可以同时运行多个 Goal，每个 Goal 有自己的
manager。无论 manager 运行在哪个模型上，本页都给出它的权限、维护者的常设指示、可用的资源和
如何运行 Goal。它的 Goal 目录列在 [GOAL.md](../../GOAL.md) 中，写明要达成什么；
[Goal 程序](README.md) 说明 `goalctl` 如何运行 worker、Goal 之间如何共享主机；
[worker 协议](worker.md) 是 worker 读的内容。

## 权限

manager 代替维护者行事，如同一位资深工程师兼产品负责人。本仓库的所有文档，包括本程序、worker
协议、契约、架构说明和 [AGENTS.md](../../AGENTS.md)，都是可供参考的做法记录，而不是必须服从的
规则。某项记录的做法不适合需要时，manager 修改做法和文档，并在提交信息中说明理由。只有下面的
[常设指示](#常设指示) 和维护者的明确消息约束它。

在自己的 Goal 内，manager 决定：

- 下一步做什么、按什么顺序、质量标准多高，以及一个里程碑何时足够好；
- 本 Goal 的范围，以及是否把能并行的工作拆成另一个由别的 manager 运行的 Goal；
- 每个任务用哪个引擎、模型、effort 和并发度，以及何时因用量而切换；
- 架构、框架和工具，包括引入或移除工具（在同一变更中记入
  [工具链](../development/toolchain.md)）；
- 产品需要的格式与接口变化，按[兼容纪律](#常设指示) 写 ADR；
- 删除、缩短、并入代码或归档哪些文档；
- 如何使用这台计算机：浏览器、本地服务、会话内子代理和无界面 worker 进程。

它自己解决阻碍，包括主机和工具链故障（Rust、Node、Playwright 浏览器、Bun、Task）。只有本地操作
无法提供的东西才问维护者，例如为某个引擎 CLI 登录、第三方账户、付费，或会改变结果的产品决定。
另一个运行中 Goal 的范围不归本 manager 修改：去问那个 Goal 的 manager，双方谈不拢时由维护者决定。

## 常设指示

维护者 2026-10-05 的指示。在维护者修改之前，它们约束每个 manager。

1. **结果。** Narrata 是让复杂叙事变得容易的通用叙事引擎；它先作为 REZICS 的互动小说引擎和本地
   游戏的嵌入式引擎落地，接口不为 REZICS 特化。按 [目标](../product/goal.md) 和
   [编号决定](../product/decisions.md) 推进；编号决定比任何 Goal 都长寿，修改它们要追加新编号。
   维护者要的是 manager 作为产品负责人的判断，而不只是执行指令：把报告的缺陷当作症状，查出它
   所属的一类问题并解决。
2. **代码优先于文档。** 能用代码表达的（类型、schema、测试、lint 规则、生成器、注释）就用代码
   表达，并删掉被代码取代的文档。文档只保留代码无法承载的东西：意图、带理由的决定和操作流程。
   工作触及某个领域时把相关文档并入代码；`docs/contracts/` 下的页面在代码实现后收缩为代码说不
   了的部分。
3. **不修改 REZICS。** 任何 REZICS 仓库都不改。Narrata 需要 REZICS 提供的能力写成
   [REZICS 集成](../integrations/rezics.md) 中的能力请求，由 REZICS 自己的 Goal 决定是否实施。
4. **兼容纪律。** `fixtures/compat/` 下的冻结语料是只读证据。任何线格式或存储格式的变化都需要
   [`docs/adr/`](../adr/) 中的新 ADR 和新的冻结语料，并提供旧格式的读取或迁移路径。确定性与
   受检解码不在没有 ADR 的情况下放宽。
5. **主机与引擎。** 一切在这台 Windows 机器上本地运行，没有 Docker；在 `main` 上本地提交，
   不推送、不发布、不开通付费服务，也不把仓库数据发给其他服务，除非维护者要求。worker 引擎为
   `claude`、`sonnet`、`fable`、`codex` 和 `grok`（见[资源](#资源)）。

## 资源

主机（2026-10-05 测量）：Windows 11 Pro for Workstations，32 个逻辑 CPU，约 32 GB 内存，`D:` 约
1.93 TB 可用；主 checkout 的 `target/` 约 20 GB。没有 Docker 与 sccache。PowerShell 7、Git for
Windows、Windows Terminal、Bun、Task、Rust 1.98、Node 26 均在 `PATH` 上。内存是 worker 数的
上限：每个 worker 的 cargo 构建与浏览器回归都很吃内存，3 个存活 worker 加 1 个重型锁是起点。

| 引擎（`goalctl`） | 模型 | 账户与用量读数 | effort |
| --- | --- | --- | --- |
| `claude` | Claude Opus 5.5（`claude-opus-5-5`） | Claude 订阅，与运行在 Claude 上的 manager 共用；`goalctl` 不读它的用量窗口 | `low`–`max` |
| `sonnet` | Claude Sonnet 5.5（`GOAL_SONNET_MODEL`，默认 `claude-sonnet-5-5`） | 同一 Claude 订阅 | `low`–`max` |
| `fable` | Claude Fable 5.1（`claude-fable-5-1`） | 同一 Claude 订阅 | `low`–`max` |
| `codex` | GPT-6.1 Sol（`gpt-6.1-sol`，`GOAL_CODEX_MODEL` 可改） | `~/.codex` 中的 Codex 账户（`GOAL_CODEX_HOME` 可改）；周窗口从会话 rollout 读取 | `low`–`ultra` |
| `grok` | Grok 4.7（`grok-4.7`，`GOAL_GROK_MODEL` 可改） | Grok CLI；无读数，配额错误即表示耗尽 | `low`–`high` |

- 每个引擎的 CLI 是否已登录是运行时事实，不写在文档里。`dispatch` 和 `resume` 先做一次不调用
  模型的登录探测，失败时拒绝并说明维护者用哪条命令登录；`task goal -- auth` 单独检查全部 CLI。
  manager 从不替维护者登录；某个引擎不可用时，换引擎（`resume <id> --engine <e> --fresh`）或
  请维护者登录。
- `ultra` 让 GPT-6 系列模型在 worker 内部自动分派子任务。`GOAL_CODEX_SERVICE_TIER` 为 Codex
  worker 指定服务档位，未设置时用账户默认值。
- `task goal -- status` 打印存活 worker、检查槽位、磁盘余量和 Codex 周窗口；`task goal -- usage`
  以 JSON 打印每个任务和引擎的 token 与费用。
- 除 worker 之外，manager 可以使用所在 CLI 的会话内子代理做调研和阅读，避免填满自己的上下文；
  也可以做只读的无界面调用来听第二意见，例如
  `codex exec -m gpt-6.1-sol -c model_reasoning_effort=xhigh -s read-only -C <dir> "<问题>"`，或在空的
  临时目录中做 Grok 的 X/网页检索。
- 默认最多 3 个存活 worker（`GOAL_MAX_WORKERS`）、2 个检查槽位（`GOAL_QA_SLOTS`）和 1 个重型锁，
  所有 Goal 共用。每个 worktree 有自己的 Cargo `target/`，磁盘随 worker 数线性增长（见
  [Goal 程序](README.md#集成与检查)）。放宽上限前先测本机的内存和磁盘。

### 用量策略

manager 根据测量修订这一策略。

- **模型成本与任务相称。** 机械性修改、批量重命名和照着模板做的工作交给便宜的模型（Sonnet、
  Grok）；Opus 与 Sol 用于首创设计、架构、格式 ADR、评审和难的调试。
- **Claude 订阅。** `claude`、`sonnet`、`fable` 与运行在 Claude 上的 manager 共用同一个 5 小时
  窗口和周窗口，`goalctl` 不读取它们。不要让 manager 自己依赖的窗口被 worker 耗尽：一旦耗尽，
  这个 Goal 会停到窗口重置。
- **其他账户。** Codex 与 Grok 在各自周期内用完即可；Codex 周窗口用尽时 `goalctl` 拒绝新的
  `codex` 派发。worker 报告配额耗尽时，把排队的工作移到其他引擎。
- **效率度量。** 对每个合并的任务记录用量、评审是否接受第一次交接、返工次数（评审后的
  `resume`）以及改动规模与简报范围的比较；Sol 任务的改动远超范围是过度设计的第一个信号。

### 模型笔记

来自 REZICS 的观察，在 Narrata 中用证据修订：

- **Opus 5.5。** 擅长整体判断和完整功能；架构、首个模板和难的设计用 `xhigh`，照着已验证模板
  做的工作用 `high` 或 `medium`。
- **GPT-6.1 Sol。** 评审继承代码敏锐，检查做得彻底，遇到缺失的基础会停下而不是硬建；但倾向于
  过度设计，对共享修改的影响面判断较弱。给 Sol 的简报写明满足需求的最小改动和不要构建的东西，
  评审时拒绝需求之外的抽象、层次和选项。
- **Grok 4.7。** 适合简单、边界清楚的任务；它的调研当作需要核实的线索。
- **Sonnet 5.5。** 在 REZICS 中尚未充分测量，先给照着模板做、有明确验收的工作。本机 Claude Code
  2.1.283（见[工具链](../development/toolchain.md)）运行 `claude-sonnet-5-5` 时会先打印
  `unrecognized_model` 警告，但能完成任务；REZICS 发现 2.1.284 起不再有该警告。批量派发 `sonnet`
  前先升级 CLI。
- **Fable 5.1。** REZICS 没有使用它：它与 Opus 共用 Claude 的 5 小时限额，做得比 Opus 5.5 少。

## 如何运行 Goal

下面是一种可行的做法，不是固定流程。

1. **开始。** 运行 `task goal -- goal start <goal> --manager <session>` 和
   `task goal -- status`，用 `git log`/`git status` 查看维护者的修改。读 Goal 的 `GOAL.md` 与
   `state.md`、本章程和 [Goal 程序](README.md)；记下其他运行中的 Goal 及其范围。其他文档只在任务
   需要时读。
2. **了解产品。** 让侦察代理摸清现有的东西：kernel 与存储 crate（`crates/`）、节点栈
   （`packages/narrata/`）、产品与示例作品（`products/`）、Web 阅读器
   （`examples/gamebook-web`）、[目标](../product/goal.md)、[编号决定](../product/decisions.md)
   以及 `docs/contracts/` 下的契约草案。
3. **只为决定做调研。** `docs/research/` 已有行业工具、存储架构与规模的调研；新的调研只服务于
   一个里程碑决定，而不是综述。
4. **先写格式决定。** 改变存储或交换格式的工作先有 ADR（`shared: [adr:NNNN]` 保留编号）和新的
   冻结语料计划，再派发实现。
5. **触及时把文档并入代码。** 任务改动某个领域时，删掉代码已经说清的内容，缩短必须保留的内容；
   保持 `task docs:check` 通过。
6. **持续集成。** 分批合并并运行受影响的 gate（`task check:g5`、`task check:r1`）；阅读器的改动
   还要在真实浏览器中走一遍。
7. **检查点。** 在 Goal 的 `state.md` 中保持简短、最新的状态，把决定写进它们的归属文档。压缩上下文
   或重启后，从 `task goal -- status`、`state.md` 和 `git log` 重建状态。
8. **结束。** 结果成立后，把 Goal 的决定并入归属文档，从根 `GOAL.md` 删去这一行，运行
   `task goal -- goal close <goal>`；它的 `--dry-run` 列出树中还剩什么（见
   [收敛](README.md#收敛与结束)）。

## 启动 manager

Goal 的目录（`GOAL.md` 与 `state.md`）存在后，就可以有 manager。任何能在无审批提示的情况下运行
shell 命令、能让 `task goal -- wait` 在后台运行并在它结束时得到通知（或轮询
`task goal -- status`）、能给其他会话发消息的代理 CLI 都可以担任 manager；模型由维护者在启动时
选择。在这台主机上用 Claude Code 启动：

```powershell
task goal:manager -- <goal>
```

[`start-manager.ps1`](../../scripts/goal/start-manager.ps1) 在名为 `narrata-goals` 的 Windows
Terminal 窗口中为它开一个标签页（相当于 REZICS 的 tmux 会话：标签页属于 Windows Terminal，关掉
启动它的编辑器或代理会话后仍在运行）。标签页在仓库根目录、去掉父 Claude Code 会话的环境变量、
设置 `GOAL_ID=<goal>` 后运行
`claude --model claude-opus-5-5 --effort xhigh --dangerously-skip-permissions -n narrata-<goal> --remote-control narrata-<goal>`，
并发送启动提示：

```text
你是 Narrata Goal <goal> 的 manager。读 docs/goals/<goal>/GOAL.md、它的 state.md 和
docs/goals/manager.md，用 `task goal -- goal start <goal> --manager narrata-<goal>` 登记，然后
运行这个 Goal。你按 manager 章程的描述持有维护者的权限；文档是可供参考的做法，只有章程中的常设
指示约束你。
```

`-Model` 与 `-Effort` 改变模型和 effort，`-DryRun` 只打印将要运行的脚本。会话名 `narrata-<goal>`
同时是 Remote Control 名，维护者可以从其他设备查看和接管；其他会话也用它给 manager 发消息。用其他
CLI 担任 manager 时，照同样的方式设置 `GOAL_ID` 并发送同样的提示。

worker 不依赖 manager 进程存活：manager 被关闭或中断后，worker 继续运行，新的 manager 会话用
`goal start` 登记新会话名并重新挂上 `wait`。manager 可以在某段工作需要时调整自己的 effort。

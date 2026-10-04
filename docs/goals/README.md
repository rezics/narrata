# Goal 程序

本页记录 Goal 如何运行 worker 进程：`goalctl`、任务简报、认领、集成与检查、容量、多个 Goal
并行，以及 Goal 如何离开仓库树。它记录行之有效的做法，供 manager 使用和改进。
[GOAL.md](../../GOAL.md) 列出正在运行的 Goal，每个 Goal 在本目录下有自己的目录；
[manager 章程](manager.md) 写明 manager 的权限、维护者的常设指示与可用资源，worker 遵循
[worker 协议](worker.md)。这套做法移植自 REZICS 的 Goal 程序，并按本机（Windows、无 Docker）
调整。

## Worker 进程

Worker 是独立的无界面 CLI 进程，以 bypass 权限模式运行，不是会话内的子代理。每个 worker 在
`.temp/worktrees/` 下自己的 worktree 中、在 `goal/<name>` 分支上工作。所有状态变化都经过
[`goalctl`](../../scripts/goal/goalctl.ts)，由 manager 在主 checkout 中以 `task goal -- <命令>`
运行，并在环境中设置 `GOAL_ID=<goal>`：

| 命令 | 作用 |
| --- | --- |
| `task goal -- goal start <goal> --manager <session>` | 登记一个已有 `GOAL.md` 的 Goal；与其他运行中 Goal 的范围重叠时拒绝。manager 重启后再次运行，记录新的会话名。 |
| `task goal -- new [--goal <goal>] <标题>` | 为该 Goal 保留下一个任务 ID，并在 `docs/goals/<goal>/tasks/G-NNN.md` 写出简报骨架，两个 manager 不会拿到同一个编号。 |
| `task goal -- dispatch docs/goals/<goal>/tasks/G-NNN.md [--dry-run] [--allow-area] [--skip-preflight] [--force-usage]` | 校验简报；拒绝重叠的认领、落在其他 Goal 范围内的路径、未满足的依赖、存活 worker 上限、未登录的引擎 CLI 和已耗尽的 Codex 周窗口；然后创建 worktree、复制简报，启动一个脱离 manager 的 worker。不安装任何依赖。 |
| `task goal -- wait G-NNN` | 放在后台运行。阻塞到 worker 退出，然后打印退出码、分支、工作区是否干净、范围检查、token 与费用，以及交接内容；登录失效或配额耗尽时给出 `AUTH:`/`LIMIT:` 提示。 |
| `task goal -- resume G-NNN -m <文字> [--file <路径>] [--effort e] [--engine e] [--fresh]` | 在同一会话中带着完整上下文继续，可换 effort。换引擎时继续该引擎自己最近的会话，或在同一 worktree 上重新开始（`--fresh`）。 |
| `task goal -- reclaim G-NNN <brief>` | 按更新后的简报替换一个已退出任务的认领，先做与派发相同的冲突检查。 |
| `task goal -- stop G-NNN` | 用 `taskkill /T /F` 结束 worker 的整个进程树，再清理仍在该 worktree 中运行的进程，并确认退出。认领和 worktree 保留。 |
| `task goal -- scope G-NNN` / `task goal -- owner <路径>` | 列出领先 `main` 的提交、未提交文件与认领之外的文件 / 哪个未关闭任务认领了某路径、它落在哪个 Goal 的范围内。 |
| `task goal -- merge G-NNN [--allow-scope] [--allow-ids] [--landed]` | 要求 worker 已退出、worktree 干净、文件都在认领内、树中没有新增任务编号（见[收敛](#收敛与结束)）；然后 rebase 到 `main` 并快进 `main`。冲突时任务标为 `conflict`，交给 worker 解决。`--landed` 记录 manager 已手工落地的工作。 |
| `task goal -- close G-NNN... verified\|cancelled` | 删除 worktree、释放认领，然后在一个提交里把简报和交接移到 `archive/goals/`，并从树中删除简报。 |
| `task goal -- tidy` / `task goal -- goal close <goal> [--dry-run]` | 归档仍留在树中的已关闭简报 / 当 Goal 在树中已无残留时结束它（见[收敛](#收敛与结束)）。 |
| `task goal -- status` / `task goal -- usage [G-NNN...]` / `task goal -- auth` | 运行中的 Goal 与 manager、存活 worker、worktree 数与磁盘余量、检查槽位与重型锁的持有者、Codex 周窗口 / 每个任务与引擎的 token 和费用（JSON）/ 各引擎 CLI 的登录状态。 |
| `task goal -- slot [--heavy] -- <命令>` | 在共享的检查槽位中运行一条命令；重型命令另需取得[全机重型锁](#集成与检查)。 |

引擎有 `claude`、`sonnet`、`fable`、`codex` 和 `grok`；[章程](manager.md#资源) 列出它们的模型与
账户，`goalctl` 的 `launchCommand` 负责它们的命令行。简报不写 `engine` 时使用 `claude`
（`GOAL_ENGINE` 改变默认值）。增加引擎时，扩展 `goalctl.ts` 中的 `launchCommand`、登录探测与
对应测试。

生命周期是：派发、后台 `wait`、读交接，然后合并或 `resume`，整批验证后关闭。worker 进程脱离
manager 运行，manager 重启后仍在；重启后 `status` 会显示它们，对每个存活 worker 重新挂上
`wait`。manager 不向运行中的 worker 发送指令：要改变一个任务，先 `stop`，再带着新指令
`resume`。

### Windows 上的进程

- Worker 由一个脱离的启动进程（`goalctl __run`）拉起。启动进程持有一个隐藏的控制台，worker
  及其所有子进程（git、cargo、npm）都继承它：没有控制台的进程再启动控制台程序时，会弹出窗口或
  让 git 挂起。启动进程把 worker 的退出码写在 `.temp/goal/runs/G-NNN/attempt-N.exit`。
- 存活判断比较进程的创建时间与派发时记录的值，PID 被复用也不会误判。
- 提示词经 stdin 传给 Claude Code 和 Codex；Grok CLI 只接受参数，因此超过约 30,000 字符的提示词
  会被拒绝（Windows 命令行上限 32,767）。
- Worker 的环境去掉了 manager 自身会话的变量（`CLAUDECODE`、`CLAUDE_CODE_SESSION_ID` 等），
  worker 是独立会话。

## 简报与防止重复

简报位于 `docs/goals/<goal>/tasks/G-NNN.md`，由 `new` 写出，保持简短。frontmatter 由机器检查：

```text
---
id: G-012
title: 选择模型的局部结果
engine: claude                        # claude | sonnet | fable | codex | grok
effort: xhigh
cases: []                             # 有验收编号时写，如 CH01
paths: [packages/narrata/nodes/src/choice/**, products/gamebook-demo/**]   # 新文件按能力命名，不用 g-NNN
shared: [adr:0012]                    # 共享槽位，例如保留一个 ADR 编号
depends: [G-010]
worktree: choices                     # 可选：写同一名字的任务共享一棵树和一个分支
---
```

正文写清结果、要读什么、可以照着做的已有模式、要跑的检查和已知风险。Narrata 没有需要预留的
编号迁移，`migrations` 字段会被拒绝；需要独占编号的东西（ADR 编号、冻结语料目录）用 `shared`
槽位声明。

重复劳动靠机制防止：只有 manager 派发、合并和关闭本 Goal 的任务；验收编号、重叠的路径 glob 和
共享槽位在所有 Goal 之间互斥，每个 Goal 的范围对其他 Goal 关闭；重试在同一会话中继续；认领之外
的文件会阻止合并，除非 manager 审阅后传 `--allow-scope`；worker 对认领之外的工作只提议，不动手。

写同一 `worktree:` 名字的相关任务共用一棵树：各自认领不相交的路径，只提交自己的路径
（`git commit --only`）。manager 在它们都退出后合并一次共享分支。共享树省下的是重复的依赖与
构建（每棵树各有一个 Cargo `target/`），代价是 worker 之间不能互相重置文件。

## 多个 Goal

可以同时运行多个 Goal，每个 Goal 一个 manager。一个 Goal 就是目录 `docs/goals/<goal>/`：
`GOAL.md` 写它的结果，并在 frontmatter 的 `areas` 中写它独占的范围；`state.md` 是 manager 的
检查点；`tasks/` 存放未关闭的简报。根目录的 [GOAL.md](../../GOAL.md) 列出所有 Goal。

- **身份。** manager 在环境中带 `GOAL_ID=<goal>` 运行，用 `goal start` 登记；之后 `goalctl`
  拒绝它操作其他 Goal 的任务，worker 也从提示词中得知自己 manager 的会话名。
- **范围。** 范围是粗粒度的路径 glob，每次派发时都从 `GOAL.md` 重新读取，维护者的修改立即生效。
  其他 Goal 的简报不得认领它们；不属于任何 Goal 范围的路径归先认领者。一个 Goal 需要另一个
  Goal 范围内的路径时，manager 请对方完成这项工作、缩小范围，或同意一次
  `dispatch --allow-area`。分歧交给维护者。
- **共享主机。** 认领、检查槽位、重型锁、存活 worker 上限与内存、磁盘是所有 Goal 共用的一个池。
  并行写代码便宜；完整 gate 和浏览器回归不便宜，重型锁让它们一次只跑一个。
- **一个 main 分支。** 每个 manager 合并自己的任务，并从固定在本批合并提交的 worktree 运行整批
  检查，其他 Goal 的合并不会改变进行中的运行。其他 Goal 范围内的失败交给它的 manager。manager
  及时用 `git commit --only <paths>` 提交自己的修改，并带 `Goal: <goal>` trailer；`goalctl`
  的提交也带它，所以没有该 trailer 的提交来自维护者或 worker。其他 Goal 范围内的未提交文件
  可能是那个 manager 进行中的工作：动之前先问。
- **沟通。** manager 之间通过各自 CLI 的跨会话消息联系；`status` 显示会话名。同伴的请求是
  信息，不是授权。

## 集成与检查

- Worker 只运行证明自己工作的检查，并通过 `task goal -- slot -- <命令>` 运行，限制全机并发的检查
  数（默认 2 个轻量槽位，`GOAL_QA_SLOTS`）。
- 重型命令除了槽位还要取得全机唯一的重型锁，所以无论来自哪个 Goal，主机上最多同时跑一个重型
  检查。自动视为重型的有：`scripts/check-*.ps1` 与 `scripts/benchmark-*.ps1`（直接运行或经
  `task check:*`、`task bench:*`）、Playwright 与 `test:e2e`，以及带 `--workspace`/`--all` 的
  `cargo test|build|clippy|check|bench|doc|nextest`。其他命令要算重型时传 `--heavy`。
  `check:r1` 在 `examples/gamebook-web` 中执行 `npm ci` 并在固定端口 4173 上跑浏览器回归，
  两个并发运行会互相测试对方的构建，这是它必须串行的直接原因。
- 槽位与锁属于运行命令的进程，进程退出（包括崩溃）后自动失效：锁目录里记录持有者的 PID 和
  进程创建时间，下一个等待者发现持有者已死或 PID 已被复用时接管。`status` 显示持有者。在槽位
  内再调用 `slot`（例如 gate 调用另一个检查）沿用外层槽位。
- **Rust 的磁盘成本。** 每个 worktree 有自己的 `target/`，第一次构建会从头编译全部依赖；主
  checkout 的 `target/` 约 20 GB，worker 数乘以它就是最坏情况的磁盘占用，`status` 打印剩余空间。
  设置 `GOAL_CARGO_TARGET_DIR`（相对主 checkout 或绝对路径）可让所有 worker 共用一个目录：依赖
  只编译一次，但 cargo 会在该目录的锁上排队，相当于把全机的 cargo 串行化。默认不共用。
- **依赖。** 派发不安装依赖。需要 `examples/gamebook-web` 的任务由 gate（`check:r1` 自带
  `npm ci`）或 worker 自己在槽位中安装。
- manager 分批合并就绪的任务，一次一个，然后从固定在本批提交的 worktree 跑受影响的 gate
  （`task check:g5`、`task check:r1`），通过后再合并下一批。失败通过 `resume` 交回负责的会话。
- 阅读器的改动还需要在真实浏览器中走一遍改动的流程，看截图，不只看测试结果。

## 容量与用量

- 所有 Goal 合计最多 `GOAL_MAX_WORKERS` 个存活 worker；`task goal` 未设置时取 3。有用的并行宽度
  由合并吞吐量决定，而不是由上限决定；在本机测过内存与磁盘之后再放宽。
- 派发和 `resume` 前，`goalctl` 对所选引擎的 CLI 做一次不调用模型的登录探测（Claude Code 的
  `auth status`、`codex login status`、`grok models`）。探测失败时拒绝派发，并说明维护者用哪条
  命令登录；`wait` 在 worker 输出中识别登录失效与配额错误。`goalctl` 从不替任何人登录。
  `task goal -- auth` 单独打印各 CLI 的状态。
- Codex 的周窗口记录在 `~/.codex`（或 `GOAL_CODEX_HOME`）的会话 rollout 中；`status` 读取它，
  窗口用尽时拒绝 `codex` 派发（`--force-usage` 跳过）。Claude 与 Grok 没有读数：worker 输出中的
  配额错误就表示该引擎暂时耗尽。
- 遇到限流时退避，不要循环重试。

## 维护者修改文档

维护者可以在任何时候修改任何文档，包括运行期间。每次派发时，`goalctl` 列出自本 Goal 上次派发以来
`main` 上没有 `Goal:` trailer 的 Markdown 提交，并警告 worker 会读到的文档（`AGENTS.md`、
`GOAL.md`、本页、`worker.md`、本 Goal 的 `GOAL.md`）中尚未提交的修改：worker 从已提交的 `main`
分支，看不到它们。每个检查点和写简报之前，也用 `git log` 与 `git status` 查看不是自己做的修改，
重读变化的部分。带其他 Goal trailer 的提交和其他 Goal 合并的任务属于同伴，不属于维护者。

据此调整进行中的工作：让它完成、停止后 `resume`，或重写简报。不要撤销或悄悄覆盖维护者的修改；
只在单独的、写明理由的提交里改进它。依赖维护者修改的工作派发之前，先把稳定的修改提交为
"Adopt maintainer documentation update"。

## 检查点与恢复

- 及时处理每个 `wait` 的结束。让 Goal 的 `state.md` 保持简短、最新。
- 压缩上下文、重启或中断之后：运行 `task goal -- status`，读 Goal 的 `state.md` 和
  `git log -20`，为存活的 worker 重新挂上 `wait`，会话名变了就再运行一次 `goal start`，然后继续
  记录的下一步。
- 状态保存在主 checkout 的 `.temp/goal/`：`ledger.json` 是账本，`runs/G-NNN/` 存每次尝试的提示词、
  启动参数、输出与退出码，`handoffs/` 存交接，`slots/` 存锁。

## 收敛与结束

结束的 Goal 在树中只留下它构建的东西，以及它记录在各自归属文档里的决定。任务编号是历史：它们
属于提交信息、分支、账本和 `archive/goals/`。在树里，简报归档后它们就成了悬空的指针；按任务命名
的文件按"何时写的"而不是"覆盖什么"组织代码。所以收敛发生在每一步，而不是最后一次清扫：

- **合并** 拒绝新增名为 `g-NNN…` 的文件，或让某个文件开始出现它以前没有的任务编号；`GOAL.md`、
  `docs/goals/`、`scripts/goal/` 与 `archive/` 除外。文件按覆盖的能力命名，测试按验收编号或行为
  命名，注释写理由而不是引用任务。`--allow-ids` 用于审阅过的例外。
- **关闭** 在一个提交中把每份简报和它的交接移到 `archive/goals/<goal>-<开始日期>/`
  （`tasks/`、`handoffs/`），并从树中删除简报。被移动的简报里的相对链接会改写，`state.md` 等
  Goal 文件指向它的链接也会跟着改（有未提交修改的文件只报告，不改写）；交接放在代码块里，原样
  保存。归档仍在 `task docs:check` 的检查范围内。仍被树中文件引用的简报暂时保留，直到引用改为指向
  归属文档。`tidy` 修复中途停下的关闭。
- **结束 Goal** 在 manager 把 Goal 的长期决定并入各自的归属文档、并从根 `GOAL.md` 删去这一行之后
  运行。还有未关闭任务、残留简报、树中文件提到它的任务或链接进它的目录时拒绝；`--dry-run` 列出
  剩下的东西。之后把目录和账本条目移到归档目录（链接随之改写），并从树中删除。

## 来自 REZICS 的经验

- 吞吐量来自互斥认领、隔离的 worktree、按运行分配的检查资源和短的合并批次。两个 worker 的上限和
  一套共享的检查环境把工作串行化了。
- 按工作完成的结果挑选任务，而不是在一个领域里越挖越深：一个程序的三分之一切片加深了始终不完整
  的领域。
- 共享的组合文件（注册表、组合根）成了合并热点。小的提取和 git 对只追加文件的 union 合并驱动
  解决了大部分冲突。
- 没有经过验证的停止操作，manager 就不能安全地替换 worker；没有互斥认领的并发产生了重复工作。
- 每任务一棵 worktree 让开发服务器、类型检查与测试环境成倍增加，直到内存不够；共享 worktree 与
  重型锁由此而来。约七个 worker 曾耗尽一台 62 GB 主机的内存，所以 Narrata 从 3 个开始。
- 约 600 个按任务命名的文件和 560 个引用任务编号的文件出现在收敛做法之前；合并检查让这个数字不再
  增长。
- 公开的多代理实验（[Cursor](https://cursor.com/blog/agent-swarm-model-economics)、
  [Anthropic](https://www.anthropic.com/research/multiagent-systems)）发现，更多代理和一个
  manager 角色本身并不能产出好产品；决定的归属、共享的设计记录和对完整集成行为的评估更重要。
  只为解决观察到的失败增加协调，不再值得时就去掉它。

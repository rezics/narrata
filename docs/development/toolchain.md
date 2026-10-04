# 工具链与检查

增加或替换工具时，在同一个变更里更新本页。命令一律通过 Task 运行（`task --list`，参数写在
`--` 之后）。

## 工具

| 工具 | 版本 | 用途 | 记录在 |
| --- | --- | --- | --- |
| Rust | 1.98.0，组件 clippy、rustfmt，目标 `wasm32-wasip1`；Web 构建与 `check:g2` 另需 `wasm32-unknown-unknown` | 引擎、CLI、绑定 | `rust-toolchain.toml` |
| PowerShell | 7.x | 现有 gate 脚本 `scripts/check-*.ps1` | 本页 |
| Node.js / npm | CI 用 24，本机 26 | `examples/gamebook-web`、TypeScript 绑定 | `.github/workflows/ci.yml` |
| Playwright | 1.63.0 | Web 阅读器的浏览器回归 | `examples/gamebook-web/package.json` |
| Bun | 1.4.0 | `goalctl`、文档检查及其测试 | 本页 |
| Windows Terminal | 本机安装 | `task goal:manager` 为每个 manager 开一个标签页 | [manager 章程](../goals/manager.md#启动-manager) |
| Task（go-task） | 3.52.0 | 统一命令入口 | `Taskfile.yml` |
| cargo-deny、wasmtime | CI 安装 | 依赖审计、Wasm 一致性 | `.github/workflows/ci.yml` |
| Claude Code CLI、Codex CLI、grok | 本机安装（2026-10-05：2.1.283、0.160.0、1.0.46）；Codex 默认模型 `gpt-6.1-sol` | Goal worker 引擎，各自需要已登录 | [Goal 程序](../goals/README.md) |

## 检查

| 命令 | 内容 | 何时运行 |
| --- | --- | --- |
| `task check:g1` … `task check:g5` | 累进的 Stage 1–5 gate：fmt、clippy、全 workspace 测试，逐级加上存储、Effect、Statechart、迁移与协议测试 | 改动 `crates/**` 时运行到受影响的级别；合并前 `g5` |
| `task check:r1` | 节点栈与 Web Gamebook：Rust 测试、Wasm 构建、浏览器回归、生成文件漂移 | 改动 `packages/narrata/**`、`products/**`、`examples/gamebook-web/**` |
| `task bench:g2` | 持久化与内核基准 | 改动存储或快照路径时对比 |
| `task docs:check` | Markdown 相对链接与标题锚点 | 任何文档改动 |
| `task test:scripts` | `goalctl` 与文档检查的 Bun 测试 | 改动 `scripts/**` |

Goal worker 通过 `task goal -- slot [--heavy] -- <命令>` 运行检查，以限制并发；完整 gate 与
浏览器回归属于重型检查。见 [Goal 程序](../goals/README.md)。

每个 git worktree 有自己的 `target/`（主 checkout 约 20 GB），并发 worker 的磁盘与编译开销
按 worktree 数线性增长。

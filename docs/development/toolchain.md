# 工具链与检查

增加或替换工具时，在同一个变更里更新本页。命令一律通过 Task 运行（`task --list`，参数写在
`--` 之后）。

## 工具

| 工具 | 版本 | 用途 | 记录在 |
| --- | --- | --- | --- |
| Rust | 1.98.0，组件 clippy、rustfmt，目标 `wasm32-wasip1`；Web 构建与 `check:g2` 另需 `wasm32-unknown-unknown` | 引擎、CLI、绑定 | `rust-toolchain.toml` |
| PowerShell | 7.x | 现有 gate 脚本 `scripts/check-*.ps1` | 本页 |
| Node.js / npm | CI 用 24，本机 26 | Web npm 包、参考阅读器、TypeScript 绑定、IndexedDB 存储适配器 | `.github/workflows/ci.yml` |
| Playwright | 1.63.0 | Web 包 tarball 安装、阅读器与 IndexedDB 存储适配器的浏览器回归 | `packages/narrata/web/package.json`、`examples/gamebook-web/package.json`、`packages/narrata/kernel/js/package.json` |
| Vite | 8.2.4 | Web 包安装测试的生产构建、阅读器构建、存储适配器浏览器测试页 | 同上 |
| React | 19.2.8（`@rezics/narrata/react` 的可选 peer，接受 React 18 与 19） | Web 包的 React 阅读器组件、参考阅读器 | `packages/narrata/web/package.json`、`examples/gamebook-web/package.json` |
| Vitest、fake-indexeddb | 5.0.3、6.2.5 | Web 运行时包装与 IndexedDB 存储适配器的单元测试 | `packages/narrata/web/package.json`、`packages/narrata/kernel/js/package.json` |
| PGlite、node-postgres / @types/pg | 0.5.8、8.23.1 / 8.23.1 | PostgreSQL 协议参考宿主的本机 SQL/事务一致性与 CI 双连接测试；仅开发依赖 | `packages/narrata/kernel/js/package-lock.json` |
| PostgreSQL | CI 服务镜像 `postgres:18`；本机官方 EDB portable 18.6 | 服务器存档的真实行锁竞争与 REPEATABLE READ 快照检查；本机在 `.temp/postgresql/` 创建临时 cluster，仅监听 loopback，检查后停止，不注册服务 | `.github/workflows/ci.yml`；本机下载来源与校验和在 `.temp/postgresql/download.json` |
| TypeScript、json-schema-to-typescript、Ajv | 5.9.3、16.0.0、8.x（lock 锁定） | Web 包 JS/声明、Rust BookView 类型与独立校验器 | `packages/narrata/web/package-lock.json` |
| Sigma.js、graphology | 3.0.3、0.26.0（MIT） | 作者图子路径按需加载的 WebGL 绘制与图模型；独立计量，reader 首屏不加载 | `packages/narrata/web/package-lock.json` |
| Zod | 4.6.7（lock 锁定） | 参考阅读器的宿主请求校验 | `examples/gamebook-web/package-lock.json` |
| wasm-bindgen CLI | 与 `Cargo.lock` 中的 `wasm-bindgen` 相同 | 为 Web 生成 Wasm 的 JS 绑定；构建脚本按需装进 `.temp/wasm-tools` | `Cargo.lock` |
| Bun | 1.4.0 | `goalctl`、文档检查及其测试；`check:web` 的 tarball 安装测试 | 本页、`.github/workflows/ci.yml` |
| Windows Terminal | 本机安装 | `task goal:manager` 为每个 manager 开一个标签页 | [manager 章程](../goals/manager.md#启动-manager) |
| Task（go-task） | 3.52.0 | 统一命令入口；CI 的 `gamebook-r1` 用 `arduino/setup-task` 安装同一版本 | `Taskfile.yml`、`.github/workflows/ci.yml` |
| cargo-deny、wasmtime | CI 安装 | 依赖审计、Wasm 一致性 | `.github/workflows/ci.yml` |
| Claude Code CLI、Codex CLI、grok | 本机安装（2026-10-05：2.1.283、0.160.0、1.0.46）；Codex 默认模型 `gpt-6.1-sol` | Goal worker 引擎，各自需要已登录 | [Goal 程序](../goals/README.md) |

## 检查

| 命令 | 内容 | 何时运行 |
| --- | --- | --- |
| `task check:g1` … `task check:g5` | 累进的 Stage 1–5 gate：fmt、clippy、全 workspace 测试，逐级加上存储、Effect、Statechart、迁移与协议测试 | 改动 `crates/**` 时运行到受影响的级别；合并前 `g5` |
| `task check:r1` | 节点栈与 Web Gamebook：节点默认/全部 feature、authoring 与 authoring Wasm 绑定的 Rust 测试和 clippy（含无 `legacy-publish` 的 reader 配置）、运行时/reader/authoring Wasm 目标检查、浏览器回归、生成文件漂移（含 authoring 接口 schema 与 Web 生成类型） | 改动 `packages/narrata/**`、`products/**`、`examples/gamebook-web/**` |
| `task check:web-storage` | IndexedDB 存储适配器：类型检查、Vitest、计数器 Wasm 模块上的 Chromium 回归（端口 4183） | 改动 `packages/narrata/kernel/js/**` 或 `narrata-storage-host` |
| `task check:server-storage` | PostgreSQL 参考宿主：类型检查、PGlite 一致性/DDL/回滚/配额/重开、可选实库相同套件与双连接屏障 | 改动 `packages/narrata/kernel/js/server/**` 或服务器测试；实库用 `NARRATA_POSTGRES_URL` 指向可建临时 schema 的测试库，未配置明确跳过、不算上线通过；CI 必跑 |
| `task web:build` | 构建 `@rezics/narrata` 的 ESM、声明与三个独立 Wasm：reader（`narrata-nodes-wasm --no-default-features`）、保留 `publishGraph` 的原始 `dist/wasm`（默认 feature）与 authoring（`narrata-authoring-wasm`），分别调用 cargo 以免 feature 合并，用 Cargo `wasm` profile（opt-level s、LTO）并去掉 name section；从 Rust schema 生成 BookView 与 authoring 接口类型/校验器，并复制现有 IndexedDB 宿主 | 准备宿主安装的包；产物为 `packages/narrata/web/dist/` |
| `task web:pack` | 构建后 `npm pack`，tarball 在 `.temp/web-pack/`，打印 sha256 与 sha512 | 安装与发布前审查产物 |
| `task check:web` | 类型/生成类型漂移、Vitest（含 authoring 与 CLI、旧 publishGraph 的字节一致）、reader/兼容/authoring Wasm 与 JS 分别的原始与 gzip 预算、仓库外 tarball 安装；Node/Bun 的 reader 加载与 authoring 发布 A/B，Vite 生产构建的 Chromium 首屏只取 reader Wasm、内容失败重试、Worker 创作发布、选择、保存重开（端口 4193） | 改动 Web npm 包；需空闲 4193；CI 的 `gamebook-r1` 在 `check:r1` 后运行；安装证据保留在系统临时目录 `.temp/narrata-web-install-*` |
| `task bench:g2` | 持久化与内核基准 | 改动存储或快照路径时对比 |
| `task bench:nodes` | 合成 10 万选择点作品、首屏按块加载、选择耗时与有界缓存分配计数 | 改动节点运行时或程序块加载时对比；结果与边界见[节点规模基准](benchmarks/nodes-scale.md) |
| `task docs:check` | Markdown 相对链接与标题锚点 | 任何文档改动 |
| `task test:scripts` | `goalctl` 与文档检查的 Bun 测试 | 改动 `scripts/**` |

Goal worker 通过 `task goal -- slot [--heavy] -- <命令>` 运行检查，以限制并发；完整 gate 与
浏览器回归属于重型检查。见 [Goal 程序](../goals/README.md)。

`narrata-graph` 的增量检查为 `cargo test -p narrata-graph --all-targets`、
`cargo clippy -p narrata-graph --all-targets -- -D warnings` 和
`cargo check -p narrata-graph --target wasm32-unknown-unknown`，也经 Goal 槽位运行。
原生 release 规模基准与 Windows 峰值工作集采样的命令见[图布局基准](benchmarks/graph-layout.md)。

每个 git worktree 有自己的 `target/`（主 checkout 约 20 GB），并发 worker 的磁盘与编译开销
按 worktree 数线性增长。

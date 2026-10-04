# Narrata 重建实施记录

工作分支：`main`。用户已授权自主实现、分批验证并提交。

本批目标为重建提案 R0/R1：不依赖对白 VM 的可组合 Web Gamebook。
不提前冻结标签、Scene 或完整同伴模型。旧 Stage 1–5 格式保持兼容。

## 提交切片

1. 节点契约与会话：package-owned Rust crate，明确导入与结果端口、稳定构件身份、原子动作、局部/共享状态、返回栈、历史分叉和可验证保存恢复。
2. 作品与工具：三个真实内容包、可替换绑定、CLI compose/validate/run、生成格式与隔离文件读取。
3. Web：共享 Rust/Wasm 执行、可用 Reader、节点/状态检查、本地保存与导入导出、浏览器验收和 CI。

## 基线

- 开始于 `4fd27f6`；工作树干净，保持 `main`。
- `cargo test --workspace --quiet`：通过。

## 切片 1

已提交 `d24b5cd`：节点编译、显式端口校验、有界会话和完整 Snapshot/历史保存。
验证：14 项契约与会话测试通过；`cargo clippy -p narrata-nodes --all-targets -- -D warnings` 通过。
限制：R1 节点扩展目前是受校验的编译 lowering，任意自定义持久节点状态尚未实施。
浏览器宿主和三个作品包由后续切片完成。

## 切片 2

已提交 `f6fa78e`：独立 `narrata-book` 与旧 CLI 的 `gamebook` 入口，源闭包加载、精确 lock、可移植作品和分析输出。
`products/gamebook-demo` 由主线、山路、营地三个包组成，可完成取信、歇脚、援助旅人和交信路线。
CLI 已运行完整九步路线并导出存档。新增文件隔离、替换 provider、输出复现和原子文件替换测试。
验证：节点与工具共 19 项测试通过；节点/工具/旧 CLI 的 Clippy 全目标检查通过。

## 切片 3

已实现独立 Wasm binding 和 React/Vite 参考 Reader；编译、选择与存档验证仍使用 Rust。
新增由 Rust schema 生成的 BookView 类型/AJV 校验、Worker、IndexedDB CAS、导入导出、结构图和移动布局。
自动保存事务结束后才发布 view；写入冲突恢复原会话并给出错误。

最终浏览器回归 6/6 通过，完整路线的 Wasm 与原生 Commit 相同；内置浏览器也验证了首屏、跨包进入营地、取信和刷新恢复。
`scripts/check-g5.ps1` 与 `scripts/check-r1.ps1` 均通过，包含旧格式/迁移/绑定回归、19 项新 Rust 测试、构建与生成文件漂移检查。
修复了 Gate 发现的 path dependency 版本缺失，以及 Windows dev server 占用原生模块时的预检提示。
视觉检查覆盖 1536×1024 桌面与 390×844 手机布局；目录排序已改为实际结构遍历，移动状态不会继承桌面检查器的打开状态。

## 后续范围

当前落实 R0/R1 的最小节点与 Gamebook profile。SceneState 的通用节点类型、Scene/Quest、同伴、
远程内容、业务 Effect 与完整编辑器仍按重建路线后续实施。本轮没有把这些未来能力标成完成。

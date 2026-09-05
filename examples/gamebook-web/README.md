# Narrata Web Gamebook reference

R1 的可用浏览器宿主，使用独立的 `narrata-nodes-wasm`。网页不实现剧情判断或存档解释器。
默认作品《山口来信》由三个内容包组成。

## 启动

仓库使用锁定的 Rust 工具链。首次准备浏览器 target：

```powershell
rustup target add wasm32-unknown-unknown
cd examples/gamebook-web
npm ci
npm run prepare:runtime
npm run dev
```

打开 `http://127.0.0.1:4173/`。`prepare:runtime` 按 Cargo.lock 安装对应的 wasm-bindgen CLI
到仓库 `.temp/wasm-tools`，构建 Wasm、校验样例 lock，并从 Rust schema 生成 TypeScript 与运行时校验器输入。
`npm run build` 生成可部署到静态 HTTP 服务器的 `dist/`；不需要 API 服务。

## 已可使用

- 正文、条件选择、禁用原因，以及连续内容和子图调用/返回。
- 目录定位、来自真实节点边的结构图、共享变量、参数/局部状态和调用实例检查。
- 上一步、历史节点 checkout、改选后保留分支。重新开始回到根提交并保留已探索路线。
- IndexedDB 自动保存，刷新后恢复；作品与完整会话存档的文件导入/导出。
- 移动布局、键盘操作、动作后焦点管理和减少动画偏好。

“导入作品”接收 CLI compose 生成的自包含 `.nar.json`，不直接接收带本地文件路径的 project.json。
存档需要精确对应的作品。导入另一个作品会切换当前自动保存槽；切换前可导出上一作品和存档。

## 边界与数据流

React → 受校验 Worker 请求 → Rust NodeBook → IndexedDB compare-and-swap → 已提交界面。
存储写入完成前不发布新 view；写入失败恢复旧内存会话。另一页面更新了存档时，旧页面的操作
被拒绝并提示刷新。无法使用 IndexedDB 时明确显示内存模式，保留文件导出能力。

源作品与存档以原始文本进入 Rust，避免 i64 被 JS JSON 数值转换破坏。界面 DTO 的整数值以字符串展示；
BookView 的 TypeScript 类型和 AJV schema 均来自同一 Rust 定义。RPC/本地缓存的外层数据由 Zod 检查。

节点图显示最多 120 个节点并明确提示截断；完整分析可用 CLI inspect。它不是完整编辑器。
Node 类型扩展目前是编译 lowering；Scene/Quest、自定义持久节点状态、远程 resolver 与外部业务效果后续实现。
会话预算与格式见 [ADR 0011](../../docs/adr/0011-r1-node-composition.md)。

## 验证

```powershell
npm run build
npx playwright install chromium
npm run test:e2e
```

Windows 上执行根目录 `scripts/check-r1.ps1` 前先停止 dev server，避免 `npm ci` 替换正在使用的
原生构建模块；单独运行 `npm run test:e2e` 可以复用现有 dev server。

测试在独立 headless Chromium 中运行，覆盖真实 Wasm 路线与 native Commit 一致性、分支与恢复、
损坏/错版本存档拒绝、多标签页 CAS、只读图和移动键盘交互。验收也通过 Codex 内置浏览器进行。
默认测试产物在系统临时目录；设置 `NARRATA_QA_DIR` 可另存桌面、结构图和手机截图。

视觉采用本次生成的完整阅读工作台概念：纸色底、墨绿文字、三栏布局、衬线正文、细边行动按钮。
实际内容替换概念中的示意正文；目录按真实连线遍历排序；真实状态决定按钮禁用和变量数量。
结构图、运行详情和文件导出是实现所需的扩展。截图里的手机示意框不出现在桌面产品中。

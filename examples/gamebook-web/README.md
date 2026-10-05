# Narrata Web Gamebook reference

R2 的可用浏览器宿主，使用独立的 `narrata-nodes-wasm`。网页不实现剧情判断或存档解释器。
默认作品《山口来信》由三个包组成；结构在打包构件 `story.narpack`，全部文字在本地内容包里。

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
到仓库 `.temp/wasm-tools`，构建 Wasm，按 lock 重新 compose 样例构件，把构件与内容包复制到
`src/generated/story/`，并从 Rust schema 生成 TypeScript 与运行时校验器输入。
`npm run build` 生成可部署到静态 HTTP 服务器的 `dist/`；不需要 API 服务。

## 已可使用

- 标题、正文、局部回应、条件选择、禁用原因、多选，以及连续段落和子图调用/返回。
- 目录定位、来自真实节点边的结构图、共享变量、参数/局部状态和调用实例检查。
- 上一步、历史节点 checkout、改选后保留分支。重新开始回到根提交并保留已探索路线。
- IndexedDB 自动保存，刷新后恢复；构件、内容包与完整会话存档的文件导入/导出。
- R1 自动存档在首次打开时迁移到默认作品，原记录另存为 `r1-backup`。
- 移动布局、键盘操作、动作后焦点管理和减少动画偏好。

“导入作品”一次选择一个 `.narpack` 构件和至少一个内容包 `.json`（第一个是原文语言）。R1 的
`.nar.json` 需先用 `narrata-book migrate-r1` 转换。存档只属于构件：换用另一份内容包（改写、
翻译）不影响存档，结构改变则是另一个构件，旧存档被拒绝。

## 边界与数据流

React → 受校验 Worker 请求 → Rust NodeBook → IndexedDB compare-and-swap → 已提交界面。
存储写入完成前不发布新 view；写入失败恢复旧内存会话。另一页面更新了存档时，旧页面的操作
被拒绝并提示刷新。无法使用 IndexedDB、存档损坏或 R1 存档无法迁移时明确显示内存模式，
不改写原记录，保留文件导出能力。

Worker 拿到无文字的 BookView 后，把页面要显示的全部内容引用连同参数批量交给本地内容方，
按浏览器语言偏好解析，再与 view 一起交给页面。缺失的文字显示为带键名的占位。

构件、内容包与存档以原始字节或文本进入 Rust，避免 i64 被 JS JSON 数值转换破坏。界面 DTO 的
整数值以字符串展示；BookView 的 TypeScript 类型和 AJV schema 均来自同一 Rust 定义。RPC 与本地
缓存的外层数据由 Zod 检查。

节点图显示最多 120 个节点并明确提示截断；完整分析可用 CLI inspect。它不是完整编辑器。
格式与会话预算见 [ADR 0013](../../docs/adr/0013-r2-text-free-node-format.md)。

## 验证

```powershell
npm run build
npx playwright install chromium
npm run test:e2e
```

Windows 上执行根目录 `scripts/check-r1.ps1` 前先停止 dev server，避免 `npm ci` 替换正在使用的
原生构建模块；单独运行 `npm run test:e2e` 可以复用现有 dev server。

测试在独立 headless Chromium 中运行，覆盖真实 Wasm 路线与 native Commit 一致性、局部回应与多选、
分支与恢复、损坏/错构件存档拒绝、换内容包保留存档、R1 自动存档迁移与迁移失败的内存模式、
多标签页 CAS、只读图和移动键盘交互。默认测试产物在系统临时目录；设置 `NARRATA_QA_DIR`
可另存桌面、多选、结尾、内存模式、结构图和手机截图。

视觉采用完整阅读工作台概念：纸色底、墨绿文字、三栏布局、衬线正文、细边行动按钮。
目录按真实连线遍历排序；真实状态决定按钮禁用和变量数量。结构图、运行详情和文件导出是实现
所需的扩展。

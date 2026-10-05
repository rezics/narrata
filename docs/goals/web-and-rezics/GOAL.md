---
# 其他 Goal 的任务简报不得认领的粗粒度路径（goalctl 每次派发时读取）。manager 随简报落地扩大范围。
areas:
  - examples/gamebook-web/**
  - packages/narrata/**
  - products/gamebook-demo/**
  - docs/integrations/**
---

# Gamebook 第一阶段

状态：维护者 2026-10-05 讨论后确定第一阶段的范围：REZICS 实现 Gamebook 所需的全部 Narrata
能力必须在本 Goal 内处理清楚。保留 `web-and-rezics` 目录与调度标识，manager 为
`narrata-web-and-rezics`。[state.md](state.md) 记录进展。

## 结果

REZICS 只需实现[能力请求](../../integrations/rezics.md#能力请求)中属于它的存储、路由、权限与
界面，就能完整提供 Gamebook 的创作、发布、阅读与存档，不需要再向 Narrata 要新能力。本 Goal
在 Narrata 仓库内用模拟宿主验收：

- **创作与发布。** JS/Wasm 接口，浏览器编辑器与 Node/Bun 服务器都能调用：由逐选择点的草稿
  记录（Narrata 规范载荷加派生的索引列，R4）组装源稿；一次给出全部诊断；在内存中铸造 ID、
  保持墓碑连续；按内容大纲检查锚点与标记块（R12）；发布产出可分别放进对象存储的对象（程序
  清单、程序块、图瓦片、标签表、语义摘要、关系投影）。
- **内容解析。** 解析上下文带所选译本与读者身份，选项文字跟随译本；视图给出下一步可能进入的
  内容单元供预取；结果只有 `ok`/`unavailable`/`incompatible`。模拟 REZICS 内容方覆盖
  Occurrence、块 ID、译本、撤回与"不存在即无权限"。
- **阅读。** `packages/narrata/web` 中的 npm 包：Wasm 运行时与 TypeScript API，按路由懒加载，
  首屏只下载清单与首块，体积与加载时序有预算和测试；阅读器外壳渲染选项、路径与状态，正文与
  选项文字经宿主的渲染插槽交还宿主（REZICS 对应 `DocumentBody`）。
- **存档。** IndexedDB 本地存档；按[决定 15](../../product/decisions.md#15-服务器存档实现同一个存储契约)
  增量同步到宿主服务器（先写 ADR）：传输、每个"用户 × 作品"一个库、多设备冲突、服务端参考
  处理与一致性套件，以真实 PostgreSQL 语义测试；导出导入与浅存档。
- **作品演化。** 作者重新发布后存档继续：ID 稳定时自动迁移，墓碑与恢复点处理被删除的位置，
  重放分歧检测，旧版本可以继续读；作者发布前能对存档做 dry-run。
- **图。** 作者视图（WebGL，懒加载，限制同时绘制的节点数）读取预计算瓦片；读者地图由关系投影
  加已访问集合生成，纯函数，浏览器与服务器都能运行，不下发未揭示的节点；关系投影格式
  （[决定 16](../../product/decisions.md#16-程序的关系投影进入宿主数据库)）附参考 DDL。
- **交付与交接。** 可安装的带版本 npm tarball（含 Wasm 与资源）、接口类型、宿主接入说明与
  样例；安装和测试不依赖兄弟仓库或开发机绝对路径；记录版本、源码提交、包校验和与检查结果。
  能力请求随设计更新；[另一台电脑的 REZICS Goal](../../integrations/rezics-goal.md) 的交接简报
  保持最新。

## 约束

- 一个平台运行时，第一阶段内的版本互相兼容（[决定 17](../../product/decisions.md#17-第一阶段一个平台运行时阶段内版本保持兼容)）；
  声明式能力、锁与自带运行时属于[以后](../../product/goal.md#以后)，本 Goal 不做。
- 不修改 REZICS 仓库（决定 13）；REZICS 一侧的表、端点、权限与界面不归本 Goal。
- 改变存储或交换格式先写 ADR 与冻结语料计划，再派发实现。
- `examples/gamebook-web` 继续作为参考阅读器与浏览器回归的承载。
- npm 公共发布由维护者以后决定；交付先用同一份 tarball。交付前 `main` 上的 CI 必须通过。

## 验收

- `task check:r1`、新包的浏览器回归与 CI 通过；首屏只下载清单与首块，体积和时序在预算内。
- 模拟宿主的端到端测试：改字不影响存档、切换译本、内容被撤回、重新发布后存档继续、两台设备
  冲突、读者地图不泄露未揭示节点、从草稿记录发布到阅读的完整流程。
- 从打包后的 tarball 安装运行模拟宿主（浏览器）与发布流程（Node/Bun），验证 Wasm、资源路径与
  公开 API；不能只测源码 workspace。
- 完成交接清单与版本交付物后本 Goal 可关闭；REZICS 最后一个任务用固定版本做真实联合验证，
  联调发现的 Narrata 缺陷交回 Narrata 修复并交付新版本。

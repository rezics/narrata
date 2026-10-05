---
# 其他 Goal 的任务简报不得认领的粗粒度路径（goalctl 每次派发时读取）。manager 随简报落地扩大范围。
areas:
  - examples/gamebook-web/**
  - packages/narrata/web/**
  - docs/integrations/**
---

# Web 宿主 SDK

状态：维护者于 2026-10-05 要求拆分并启动本机 Narrata 部分。依赖的叙事核心与 Kernel 存档已完成。
保留 `web-and-rezics` 目录与调度标识，manager 为 `narrata-web-and-rezics`；显示名称改为 Web 宿主 SDK。
[state.md](state.md) 记录进展。

## 结果

任何 Web 宿主能以一个带版本的 npm 包接入 Narrata；本 Goal 在 Narrata 仓库内用模拟宿主完成验收：

- **包。** Wasm 运行时与 TypeScript API，按路由懒加载；体积与加载时序有预算和测试。
- **阅读器外壳。** Narrata 渲染选项、路径与状态；正文与选项文字通过宿主的解析与渲染插槽
  交还宿主（REZICS 对应 `DocumentBody`）。用一个模拟 REZICS 内容方跑通完整流程：批量解析、
  `unavailable`/`incompatible`、译本一致的选项文字、预取下一单元。
- **存档。** IndexedDB 本地存档、导出导入、交给宿主同步的存档字节。
- **图视图。** 使用预计算的瓦片：作者视图（WebGL，限制同时绘制节点数）与读者地图（服务端
  按已访问路径生成的投影，不下发未揭示节点）。
- **交付与交接。** 产出可安装的带版本 npm 包（含 Wasm 与必要资源）、接口类型、宿主接入说明与
  测试样例，包的安装和测试不依赖兄弟仓库或开发机绝对路径。记录版本、源码提交、包校验和与检查结果。
  [REZICS 集成](../../integrations/rezics.md) 的能力请求保持最新；真实适配器、网站接入及联合验证
  移交给[另一台电脑的 REZICS Goal](../../integrations/rezics-goal.md)。

## 约束

- 不修改 REZICS 仓库（决定 13）。
- `examples/gamebook-web` 继续作为参考阅读器与浏览器回归的承载。
- Narrata 拥有通用接口与叙事行为；REZICS 拥有正文解析、权限、网站与存档同步实现。
  两个 manager 以带版本的类型、样例与能力请求对齐；REZICS 当前结构由其 manager 核对，不直接照抄旧调研。
- 先完成可审阅的版本交付物；公共 registry 发布与推送仍按 manager 章程由维护者明确授权。
  可先通过维护者指定的传递方式交付同一份 npm tarball，无需为了另一台机器安装而公开发布。

## 验收

- `task check:r1` 与新包的浏览器回归通过；首屏只下载清单与首块。
- 模拟内容方的端到端测试覆盖改字不影响存档、切换译本、内容被撤回后的显示。
- 从打包后的 npm 交付物安装运行模拟宿主，验证 Wasm、资源路径与公开 API；不能只测源码 workspace。
- 完成交接清单及版本交付物后本 Goal 可关闭；REZICS 最后任务用固定发行版本执行真实联合验证。
  联调发现的 Narrata 缺陷交回 Narrata 修复并交付新版本，REZICS 锁定新版本后重新验证。

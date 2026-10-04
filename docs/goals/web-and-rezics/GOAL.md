---
# 其他 Goal 的任务简报不得认领的粗粒度路径（goalctl 每次派发时读取）。manager 随简报落地扩大范围。
areas:
  - examples/gamebook-web/**
  - packages/narrata/web/**
  - docs/integrations/**
---

# Web 与 REZICS

状态：已定义。依赖 `narrative-core` 的内容引用协议和 `kernel-and-saves` 的存档字节；
协议稳定后由维护者启动 manager。[state.md](state.md) 记录进展。

## 结果

REZICS（以及任何 Web 宿主）能以一个带版本的 npm 包接入 Narrata：

- **包。** Wasm 运行时与 TypeScript API，按路由懒加载；体积与加载时序有预算和测试。
- **阅读器外壳。** Narrata 渲染选项、路径与状态；正文与选项文字通过宿主的解析与渲染插槽
  交还宿主（REZICS 对应 `DocumentBody`）。用一个模拟 REZICS 内容方跑通完整流程：批量解析、
  `unavailable`/`incompatible`、译本一致的选项文字、预取下一单元。
- **存档。** IndexedDB 本地存档、导出导入、交给宿主同步的存档字节。
- **图视图。** 使用预计算的瓦片：作者视图（WebGL，限制同时绘制节点数）与读者地图（服务端
  按已访问路径生成的投影，不下发未揭示节点）。
- **REZICS 集成文档。** [REZICS 集成](../../integrations/rezics.md) 的能力请求保持最新；
  REZICS 实施后补齐真实适配器。

## 约束

- 不修改 REZICS 仓库（决定 13）。
- `examples/gamebook-web` 继续作为参考阅读器与浏览器回归的承载。

## 验收

- `task check:r1` 与新包的浏览器回归通过；首屏只下载清单与首块。
- 模拟内容方的端到端测试覆盖改字不影响存档、切换译本、内容被撤回后的显示。

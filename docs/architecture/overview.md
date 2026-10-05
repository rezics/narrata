# 架构总览

日期：2026-10-05。描述目标分层与现状；目标依据见 [编号决定](../product/decisions.md)。

## 目标分层

```text
 作者源稿（宿主所有）            内容方（REZICS、本地包、生成服务）
        │ 编译、校验、锁定                 ▲ 批量解析 ContentRef / Segment
        ▼                                  │
 不可变构件：清单 + 程序块（内容寻址，按需加载）
        │
        ▼
 Kernel：身份与摘要 · 确定性 CBOR · 受检解码 · 注册的领域类型 · 原子事务
         提交/引用对象库 · Effect 账本 · 迁移 · 回放与分歧检测
        │                ▲
        │                │ 受检输入（选择、宿主结果、动态提议）
        ▼                │
 第一方领域包：节点与子图 · 选择 · Flow · Statechart · （后续）Scene、人物、关系、同伴、任务
        │
        ▼
 输出：当前节点 Segment、选择点与选项 labelRef、呈现键、语义事件
        │
        ▼
 宿主：Web（Wasm + TS 包，正文渲染插槽）· 本地游戏（原生库 + 存档字节）
 存储后端：内存 · SQLite · IndexedDB · 交给宿主的存档字节（窄契约）
```

三条硬边界：

1. **内容边界**：引擎只持有引用，见 [内容引用](../contracts/content-references.md)。
2. **存储边界**：后端只提供五个原语与能力声明，见 [存储与存档](../contracts/storage-and-saves.md)。
3. **宿主边界**：外部结果作为受检输入进入，不可逆效果走 Effect 账本，见
   [Effect 与宿主状态](effects-and-host-state.md)。

## 现状

两套引擎栈并存，既不共享存储也不共享身份：

| 栈 | 位置 | 已有 | 与目标的差距 |
| --- | --- | --- | --- |
| Stage 1–5 | `crates/narrata-*` | 确定性 Flow VM、Statechart、提交/引用存储与 SQLite 适配、Effect 账本、迁移、C/C#/Wasm/TS 绑定、冻结兼容语料；Program 格式 1 与 Snapshot schema 1 只含内容引用，`SceneState` 可选（[ADR 0018](../adr/0018-stage-6-text-free-flow-format.md)） | 格式 0 的旧存档仍带文本，需经升级才能走协议；`SaveStore` 全量枚举；SQLite 整库重写；Program 单块 |
| 节点栈（R1） | `packages/narrata/nodes`、`packages/narrata/tooling` | 类型化节点与子图、包组合与锁、Gamebook 会话、Wasm 阅读器 | 文本内联；4,096 节点与 4 MiB 整包上限；存档不用对象/引用模型；结局文字硬编码 |

按 [决定 11](../product/decisions.md#11-节点组合模型是长期内核的语义基础)，节点组合模型是长期
语义基础，Stage 1–5 的可靠执行机制抽取为 kernel。现有架构文档描述的是这些机制：

- [确定性运行模型](runtime-model.md)
- [时间旅行与存档](time-travel-and-save.md)
- [Effect 与宿主状态](effects-and-host-state.md)
- [程序身份与迁移](program-versioning-and-migration.md)

收敛工作分配在 [GOAL.md](../../GOAL.md) 列出的 Goal 中。

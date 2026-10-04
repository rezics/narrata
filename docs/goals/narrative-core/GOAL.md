---
# 其他 Goal 的任务简报不得认领的粗粒度路径（goalctl 每次派发时读取）。manager 随简报落地扩大范围。
areas:
  - packages/narrata/nodes/**
  - packages/narrata/tooling/**
  - products/**
  - docs/contracts/content-references.md
  - docs/contracts/choices.md
  - docs/contracts/graph-and-analysis.md
---

# 叙事核心

状态：已定义，等待维护者启动 manager。[state.md](state.md) 记录进展。

## 结果

作者能用节点栈写一部 10 万选择点规模的互动叙事，而 Narrata 不持有一个字的正文：

- **内核不含正文。** 节点正文、局部回应、选项文字、标题全部是
  [内容引用](../../contracts/content-references.md)（`ContentRef`/`Segment`/`labelRef`）。
  文本不进入包摘要、产品摘要、Snapshot 与状态摘要；运行时不再硬编码任何文字。
  `products/gamebook-demo` 的文字移到一个本地内容包，由本地内容方解析，阅读体验不变。
- **身份按决定 3。** 节点、选择点、选项使用 16 字节身份；现有可读 ID（如 `help`）成为作者
  别名；删除留墓碑并有诊断。
- **两类选择。** 按 [选择模型](../../contracts/choices.md) 实现局部结果与分支结果、单选与
  多选、`placement` 锚点和标记块一致性诊断。
- **动态选项与节点。** 宿主提议经校验后作为输入提交；回放与恢复使用记录的结构。
- **规模。** 去掉 4,096 节点与 4 MiB 整包上限，程序按包与块切分、按需加载。合成作品基准
  （10 万选择点、约数千内容单元）证明：峰值内存与活跃块数成正比；每次选择的处理时间在本机
  低于 1 ms；首屏只需清单与首块。
- **发布时分析。** 按 [图与分析](../../contracts/graph-and-analysis.md) 输出拓扑诊断、按簇的
  分层布局与细节层级瓦片、语义摘要；一套 Rust 实现可在 CLI 与 Wasm 中运行。

## 约束

- 格式变化写新 ADR（`docs/adr/0012` 起），给出 ADR 0011 R1 格式的读取或迁移路径。
- 与 Goal `kernel-and-saves` 的耦合：节点会话的提交与存档改用 kernel 的对象/引用模型。
  本 Goal 拥有节点运行时与其状态形状，`kernel-and-saves` 拥有存储契约与提交 API；接口变化
  先与对方 manager 对齐。
- `examples/gamebook-web` 属于 Goal `web-and-rezics`；本 Goal 需要的阅读器改动以简报形式
  交给对方，或在对方未启动时由维护者同意后一次性认领。

## 验收

- `task check:r1` 通过；新增的属性测试证明只改文字时所有摘要不变、切换语言不改变状态摘要。
- 合成作品基准纳入 `task` 命令并留档。
- 契约页收缩为指向代码的说明。

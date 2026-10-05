# 图、分析与摘要

状态：发布时计算、R2 适配、compose CLI 与受检分发格式已实现；宿主接入待实现。依据：[决定 7、12](../product/decisions.md) 与
[图渲染调研](../research/2026-10-05-choice-granularity-and-scale/graph_rendering.md)。

叙事图归 Narrata，与宿主的章节结构无关。图视图是用户需求，但它是程序的一个投影，
不是另一份真相。

## 发布时计算

[narrata-graph](../../packages/narrata/tooling/crates/narrata-graph/src/lib.rs) 接收与节点格式无关
的结构图，在 Rust 中计算诊断、层级、稳定坐标、三层瓦片列与语义摘要；同一库可编译到 Wasm。
[行为测试](../../packages/narrata/tooling/crates/narrata-graph/tests/publication.rs) 固定计算语义，
[基准](../development/benchmarks/graph-layout.md) 记录算法取舍、规模证据与已知限制。

发布计算属于工具链，浏览器不做整图布局。

## 交付格式

[ADR 0016](../adr/0016-graph-publication-format.md) 记录几何、内容标签与宿主摘要分开的理由。
[wire 编解码器](../../packages/narrata/tooling/crates/narrata-graph/src/wire.rs) 固定索引与三层
CBOR 瓦片；[摘要类型与 JSON Schema](../../packages/narrata/tooling/crates/narrata-graph/schema/summary-v1.schema.json)
固定宿主交换形状。[兼容测试](../../packages/narrata/tooling/crates/narrata-graph/tests/distribution.rs)
受检读取冻结语料，并与重新生成的字节比较。[R2 适配器](../../packages/narrata/tooling/crates/narrata-node-tools/src/publish.rs)
与[发布文件 API](../../packages/narrata/tooling/crates/narrata-node-tools/src/compose.rs) 把程序、内容方导出的大纲
和这些输出连起来，保留现有节点检查器的分析形状。compose 的 `--outline <路径>` 可重复，
每个内容方提供一份原文语言的大纲；没有提供的内容方在分析中报告检查跳过。
伴随文件的引用形状由 [Rust 类型生成的 schema](../../packages/narrata/tooling/crates/narrata-node-tools/schema/graph-files-v1.schema.json) 固定。

## 作者视图

- WebGL 渲染器（sigma.js + graphology，懒加载）；同时绘制不超过约 5 万节点、15 万边。
- 通过"进入单元"切换层级；局部编辑用小的富节点编辑器（不超过约 500 节点）。
- 全作品星图可选，用 GPU 渲染器（cosmos.gl）加预计算坐标。

## 读者地图

- 由服务端按读者走过的节点和边生成；可选显示"见过但未选"的选项占位。**不下发未揭示的
  节点**，下发即剧透。
- 位置取自全局布局并压缩空隙，避免泄露隐藏分支的规模；新揭示的节点插入而不重排。
- 形式参照分章流程图（Detroit: Become Human、Zero Escape）：章节概览 + 单章流程图，作者
  定义的锁，跳转即恢复读者自己在该节点的存档。
- 社区选择比例只在达到聚合阈值后显示。

## 语义摘要

供宿主写入自己的索引或图库（REZICS 的 Jena），每次发布导出一次；宿主负责映射到自己的词汇。
[语义摘要](../../packages/narrata/tooling/crates/narrata-graph/src/summary.rs) 表达结构上的路线
近似，不证明分支条件可行，也不替代完整诊断。它不含选择边全集；瓶颈按连续作者簇压缩，避免
把长 passage 链逐节点写入索引。任意图不能保证固定事实条数，不能静默截断。

R2 适配器附加发布构件身份。人物或实体首次出现需要实体模型，当前不推断这些事实；
未来接入也须保留宿主对读者剧透边界的控制。

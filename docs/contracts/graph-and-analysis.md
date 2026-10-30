# 图、分析与摘要

状态：发布计算、受检分发、Web 作者视图与已揭示读者地图已实现；读者地图的后续产品意图见下文。依据：[决定 7、12](../product/decisions.md) 与
[图渲染调研](../research/2026-10-05-choice-granularity-and-scale/graph_rendering.md)。

叙事图归 Narrata，与宿主的章节结构无关。图视图是用户需求，但它是程序的一个投影，
不是另一份真相。

## 发布时计算

发布计算见 [narrata-graph](../../packages/narrata/tooling/crates/narrata-graph/src/lib.rs) 与
[行为测试](../../packages/narrata/tooling/crates/narrata-graph/tests/publication.rs)；
[基准](../development/benchmarks/graph-layout.md) 记录算法取舍、规模证据与已知限制。

发布计算属于工具链，浏览器不做整图布局。

## 交付格式

[ADR 0016](../adr/0016-graph-publication-format.md) 记录几何、内容标签与宿主摘要分开的理由。
编码、形状与兼容性以 [wire 编解码器](../../packages/narrata/tooling/crates/narrata-graph/src/wire.rs)、
[摘要 schema](../../packages/narrata/tooling/crates/narrata-graph/schema/summary-v1.schema.json)、
[伴随文件 schema](../../packages/narrata/tooling/crates/narrata-node-tools/schema/graph-files-v1.schema.json)与
[兼容测试](../../packages/narrata/tooling/crates/narrata-graph/tests/distribution.rs)为准。
SDK 发布用法见[接入说明](../../packages/narrata/web/README.md#authoring-and-publication)；
compose 文件入口与多大纲用法见[实现](../../packages/narrata/tooling/crates/narrata-node-tools/src/compose.rs)及
[测试](../../packages/narrata/tooling/crates/narrata-node-tools/tests/publish.rs)。
每个内容方提供原文语言的大纲；没有提供的内容方会跳过相应检查，宿主不能把它当作检查通过。

## 作者视图

宿主用法见[接入说明](../../packages/narrata/web/README.md#author-graph)，实现与行为见
[Web 图入口](../../packages/narrata/web/src/graph/index.ts)、
[控制器测试](../../packages/narrata/web/test/graph-controller.test.ts)与
[浏览器测试](../../examples/gamebook-web/tests/graph.spec.ts)。

作者视图揭示完整结构，宿主须把它提供给可以查看未走路线的用户；不能当作读者地图。
预计算坐标服务于结构探索，不代表内容顺序或语义真实性。第一阶段只读；局部富节点编辑器
与全作品星图仍是后续能力。

## 读者地图

已交付的已揭示地图用法见[接入说明](../../packages/narrata/web/README.md#reader-maps-and-relation-hosting)，
数据与投影边界见[实现](../../packages/narrata/tooling/crates/narrata-authoring/src/maps.rs)及
[测试](../../packages/narrata/tooling/crates/narrata-authoring/tests/maps.rs)。
服务端只向地图客户端下发已揭示数据，可显示"见过但未选"的选项占位；**不下发未揭示的节点**，
下发即剧透。按 [ADR 0022](../adr/0022-authoring-publication-and-reader-map.md)，坐标按首次访问顺序
紧凑分配，追加访问不重排旧节点；不复用作者全图布局，避免包围盒和空隙泄露隐藏规模。
客户端存档与 visits 可改善个人防剧透体验，不能证明实际阅读，也不能用于付费授权、奖励或他人的
防剧透判断；宿主仍负责登录、可见性与内容授权。旧构件和迁移证据的保留由宿主负责，历史缺失时
不能把不完整地图宣称为完整访问史。

以下仍是后续产品意图，不属于当前已揭示地图的交付范围：

- 形式参照分章流程图（Detroit: Become Human、Zero Escape）：章节概览 + 单章流程图，作者
  定义的锁，跳转即恢复读者自己在该节点的存档。
- 社区选择比例只在达到聚合阈值后显示。

## 语义摘要

供宿主写入自己的索引或图库（REZICS 的 Jena），每次发布导出一次；宿主负责映射到自己的词汇。
[语义摘要](../../packages/narrata/tooling/crates/narrata-graph/src/summary.rs) 表达结构上的路线
近似，不证明分支条件可行，也不替代完整诊断。任意图不能保证固定事实条数，不能静默截断。

人物或实体首次出现需要实体模型，当前不推断这些事实；
未来接入也须保留宿主对读者剧透边界的控制。

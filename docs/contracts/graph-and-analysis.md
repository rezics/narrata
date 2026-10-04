# 图、分析与摘要

状态：发布时内存计算已实现；分发格式与宿主接入待实现。依据：[决定 7、12](../product/decisions.md) 与
[图渲染调研](../research/2026-10-05-choice-granularity-and-scale/graph_rendering.md)。

叙事图归 Narrata，与宿主的章节结构无关。图视图是用户需求，但它是程序的一个投影，
不是另一份真相。

## 发布时计算

[narrata-graph](../../packages/narrata/tooling/crates/narrata-graph/src/lib.rs) 接收与节点格式无关
的结构图，在 Rust 中计算诊断、层级、稳定坐标、三层瓦片列与语义摘要；同一库可编译到 Wasm。
[行为测试](../../packages/narrata/tooling/crates/narrata-graph/tests/publication.rs) 固定计算语义，
[基准](../development/benchmarks/graph-layout.md) 记录算法取舍、规模证据与已知限制。

发布计算属于工具链，浏览器不做整图布局。列式瓦片将经 kernel 的规范 CBOR 信封按内容身份
交付，宿主交换摘要将使用版本 JSON；两种格式仍需新的 ADR、冻结语料与受检读取路径。
[ADR 0013](../adr/0013-r2-text-free-node-format.md) 只保留分析 kind code，不规定这些输出。
当前 runner 的临时列缓冲只用于测量，不是交付格式；R2 产品到结构图的适配也尚未接入。

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
[内存摘要](../../packages/narrata/tooling/crates/narrata-graph/src/summary.rs) 表达结构上的路线
近似，不证明分支条件可行，也不替代完整诊断。它不含选择边全集；瓶颈按连续作者簇压缩，避免
把长 passage 链逐节点写入索引。任意图不能保证固定事实条数，不能静默截断。

发布构件身份由未来的产品适配器附加。人物或实体首次出现需要实体模型，当前不推断这些事实；
未来接入也须保留宿主对读者剧透边界的控制。

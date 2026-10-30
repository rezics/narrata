# 选择模型

状态：静态选择点与宿主提议的动态选项和节点都已实现（[ADR 0013](../adr/0013-r2-text-free-node-format.md)
§4、§5）。依据：[决定 3、5、6](../product/decisions.md) 与
[选项粒度调研](../research/2026-10-05-choice-granularity-and-scale/choice_granularity.md)。
选择点与选项的结构见 `packages/narrata/nodes/schemas/node-plan.schema.json`，提议请求见同目录的
`proposal-request.schema.json`；基数、汇合、呈现顺序与提议的校验由 `narrata-nodes` 的检查、会话
与提议测试固定。本页只记录代码无法表达的意图与约定。

## 何时用哪种结果

- **局部结果**只改状态，回应与汇合点在当前段落的同一内容单元内。这覆盖绝大多数"小选择"，
  不需要为回应和汇合各建一个内容单元。
- **分支结果**进入另一个节点，章末选项就是这种。
- 一个选择点可以混合局部与分支选项（例如"继续交谈"与"离开"）。

## 正文里只有标记

内容单元是扁平的块序列，不嵌套选择树。内容方可以在正文中放一个只记录 `ChoicePointId` 的
标记块（REZICS 的 `narrata-choice` 块，本地内容包的 `{ id, choice_point }` 块），用于编辑器
渲染和不支持 Narrata 的读者显示占位；翻译时原样复制。没有标记块时，Narrata 按 `placement`
锚点定位；大纲校验见[测试](../../packages/narrata/nodes/crates/narrata-nodes/tests/outline.rs)。

## 身份与生命周期

选择点、选项、节点的 ID 连续性意图见 [决定 3](../product/decisions.md#3-身份以-narrata-为权威)，
受检比较见[组合实现](../../packages/narrata/tooling/crates/narrata-authoring/src/compose.rs)与
[发布测试](../../packages/narrata/tooling/crates/narrata-authoring/tests/publication.rs)。
宿主每次发布须提供上一次发布，并保存返回的墓碑记录。

## 记录什么

存档与提交历史记录"在哪个提交选了哪个选择点的哪些选项"（`OptionId` 集合），不记录选项
文字。读者重新打开时，选项文字按当前解析上下文重新解析。

## 动态选项与节点

- 会改变走向的生成内容（新选项、新段落）经提议写进提交历史；只影响呈现的生成文字仍只是内容
  引用，由内容方按呈现键缓存，不需要提议。提议里的文字同样是引用，Narrata 只记录结构。
- 允许提议的选择点仍应提供静态选项，作为没有生成器的宿主的退路；等待、追加与返回时的行为见
  [提议测试](../../packages/narrata/nodes/crates/narrata-nodes/tests/proposals.rs)。
- 提议的段落只属于当前栈帧，调用返回后消失。需要跨章节保留的生成结构应写回作品源稿并重新编译，
  而不是依赖叠加层。

多人共同决定一个选择（投票）时，汇总在宿主或会话服务中完成，汇总结果作为该选择的输入
记录，需要时附上各人的选票作为观察记录。

## 交换格式

导出选择点时可映射到现成词汇，便于宿主索引：IMS QTI 的 `choiceInteraction`
（`minChoices`/`maxChoices`、选项 `identifier`）与 ActivityStreams 2.0 的 `Question`
（`oneOf` 单选、`anyOf` 多选）。这些标准只描述问题与选项，不描述去向；去向由 Narrata 定义。

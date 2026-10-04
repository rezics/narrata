# 选择模型

状态：目标契约，尚未实现。依据：[决定 3、5、6](../product/decisions.md) 与
[选项粒度调研](../research/2026-10-05-choice-granularity-and-scale/choice_granularity.md)。

## 选择点与选项

```text
ChoicePoint = {
  id: ChoicePointId,            // 16 字节，Narrata 铸造
  placement: AnchorId?,         // 出现在节点正文的哪个块之后；省略时在节点末尾
  cardinality: { min, max },    // 单选为 {1,1}；多选如 {1,2}
  options: [Option],            // 有序
}

Option = {
  id: OptionId,                 // 16 字节，Narrata 铸造
  key: string,                  // 作者别名，同一选择点内唯一，可改名
  labelRef: ContentRef,         // 选项文字由内容方提供
  visible?: Condition,          // 不满足时不显示
  enabled?: Condition,          // 不满足时显示但不可选
  effects: [Effect],            // 选择时原子执行的状态变化
  outcome: Local { reply?: Segment, rejoin: AnchorId }   // 同一内容单元内显示回应并在 rejoin 处汇合
         | Branch { target: NodeId },                    // 进入另一个节点
}
```

- **局部结果**只改状态，回应与汇合点必须在当前节点的同一内容单元内，且汇合点在回应之后。
  这覆盖绝大多数"小选择"，不需要为回应和汇合各建一个内容单元。
- **分支结果**进入另一个节点，章末选项就是这种。
- **多选**的选项必须全部是局部结果，并共享同一个汇合点；之后的走向由条件决定。效果按选项
  顺序执行，整个选择作为一个输入原子提交。
- 一个选择点可以混合局部与分支选项（例如"继续交谈"与"离开"）。

## 正文里只有标记

内容单元是扁平的块序列，不嵌套选择树。内容方可以在正文中放一个只记录 `ChoicePointId` 的
标记块（REZICS 的 `narrata-choice` 块），用于编辑器渲染和不支持 Narrata 的读者显示占位；
翻译时原样复制。没有标记块时，Narrata 按 `placement` 锚点定位。标记块与 `placement` 不一致
时，编译报告诊断。

## 身份与生命周期

选择点、选项、节点的 ID 规则见 [决定 3](../product/decisions.md#3-身份以-narrata-为权威)：
改文字、条件、效果、顺序、目标都不换 ID；移到另一个选择点或复制会换 ID；删除留墓碑，
ID 永不复用。

## 记录什么

存档与提交历史记录"在哪个提交选了哪个选择点的哪些选项"（`OptionId` 集合），不记录选项
文字。读者重新打开时，选项文字按当前解析上下文重新解析。

## 动态选项与节点

生成式系统或宿主可以在运行时提出新的选项集合或新节点：

1. 宿主提交提议（结构、条件、`labelRef`，不含正文）。
2. 引擎按与编译期相同的规则校验，为新选项和节点铸造 ID。
3. 校验通过的结构作为输入写进提交；回退、重放和迁移使用记录的结构，不重新生成。

多人共同决定一个选择（投票）时，汇总在宿主或会话服务中完成，汇总结果作为该选择的输入
记录，需要时附上各人的选票作为观察记录。

## 交换格式

导出选择点时可映射到现成词汇，便于宿主索引：IMS QTI 的 `choiceInteraction`
（`minChoices`/`maxChoices`、选项 `identifier`）与 ActivityStreams 2.0 的 `Question`
（`oneOf` 单选、`anyOf` 多选）。这些标准只描述问题与选项，不描述去向；去向由 Narrata 定义。

# 内容引用

状态：目标契约，尚未实现。Goal `narrative-core` 把它落成类型、schema 与测试后，本页收缩为
指向代码的说明。依据：[决定 2、4、6](../product/decisions.md)。

## 引擎持有什么

Narrata 的构件、状态和存档只持有**内容引用**，不持有正文、选项文字、标题或媒体。

```text
ContentRef   = { provider: ProviderId, key: OpaqueKey }
Segment      = { unit: ContentRef, first?: AnchorId, last?: AnchorId }
```

- `provider` 是注册过的内容方标识，如 `rezics`、`local`、`generated`。
- `key` 由内容方定义，Narrata 只比较相等并按确定性 CBOR 编码，不解析其含义；长度有上限。
- `AnchorId` 是内容单元内稳定的块 ID（例如 REZICS 文档的 `attrs.id`）。`Segment` 表示单元
  内从 `first` 到 `last`（含两端、按文档顺序）的连续块；省略时表示整个单元。作者在两端之间
  插入的新块自动属于该段。
- 节点正文、局部选项的回应、选项文字（`labelRef`）、入口与结局的标题都是 `ContentRef` 或
  `Segment`。

文本不参与逻辑构件身份、状态摘要和 Snapshot。改一个字不改变任何 Narrata 摘要。

## 谁来解析

运行时输出引用，**宿主或渲染器**负责解析。引擎从不调用内容方，恢复存档也从不要求旧内容
仍可解析。

```text
resolve(requests: [ContentRef | Segment], context) -> [Resolution]

context    = { languages: [BCP 47], realization?: HostSelection, viewer: opaque to Narrata }
Resolution = ok { revision, digest?, payload }   // payload 交给宿主渲染器，Narrata 不读取
           | unavailable                        // 不存在、无权限、已撤回、已擦除，合并且不说明原因
           | incompatible { reason }             // 存在但锚点缺失或形状不符
```

- **批量且有上限。** 一次请求覆盖一个画面所需的全部引用（正文段、选项文字、标题），宿主
  决定批量上限。
- **译本一致。** `realization` 是宿主对译本或版本的选择。同一画面的正文与选项文字用同一个
  上下文解析，读者选了某个译本，选项文字也来自该译本；缺失时由宿主回退到原文语言。
- **锚点跨译本保持。** 译本必须保留原文的块 ID；某译本缺少段落端点时该段为
  `incompatible`，宿主可回退到原文。

## 呈现键

每次呈现都有稳定的呈现键 `PresentationKey = (ExecutionId, CommitId, NodeOccurrence)`。

- 生成式内容方（例如按状态生成叙述的 AI）以呈现键缓存结果：回退到同一提交得到同一段文字，
  走到新分支时键不同才重新生成。
- 需要"当时看到的是哪一版"的宿主，可以把解析得到的 `revision` 作为观察记录附在回执上。
  观察记录不是逻辑输入，缺失不影响恢复。

## 不属于内容的东西

- 会影响规则判断的资源（谜题答案、要与玩家输入比较的名字、作为条件的图像）进入程序的
  语义锁，见 [重建提案 §8](../../archive/2026-09-05-rebuild-proposal.md)。
- 生成式系统提出的新选项、新节点或新事实是**输入**，见 [选择模型](choices.md#动态选项与节点)。

## 与现有代码的关系

`crates/narrata-core/src/content.rs` 已有外部内容解析器类型，但除测试外未被调用；
`StructureOccurrence` 用 `structure + u32` 表示出现位置，需要核实稳定性（重建提案 §9）。
节点栈的 `content` 表与 `label: String` 目前内联文字
（`packages/narrata/nodes/crates/narrata-nodes/src/model.rs`），运行时硬编码了结局段落
（`runtime.rs` 的 `render`）。这些都由 Goal `narrative-core` 替换。

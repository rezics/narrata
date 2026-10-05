# 内容引用

状态：已实现（[ADR 0013](../adr/0013-r2-text-free-node-format.md) §2、§9）。依据：
[决定 2、4、6](../product/decisions.md)。形状、上限与编码在代码中：

- `ContentRef`、`Segment`、`AnchorId` 及其规范 CBOR：`narrata-kernel` 的 `content` 模块；
- 解析请求与结果：`packages/narrata/nodes/schemas/content-resolve-request.schema.json`、
  `content-resolution.schema.json`；参考实现是本地内容方 `narrata-content-local`；
- 呈现项与呈现键：`book-view.schema.json` 的 `PresentationItem`。

本页只记录代码无法表达的约定。

## 引擎持有什么

Narrata 的构件、状态和存档只持有内容引用，不持有正文、选项文字、标题或媒体。`provider` 是
注册过的内容方（如 `rezics`、`local`、`generated`）；`key` 由内容方定义，Narrata 只比较相等。
`AnchorId` 是内容单元内稳定的块 ID（例如 REZICS 文档的 `attrs.id`），作者在段的两端之间插入
的新块自动属于该段。改一个字不改变任何 Narrata 摘要。

## 谁来解析

运行时输出引用，宿主或渲染器负责解析。引擎从不调用内容方，恢复存档也从不要求旧内容仍可
解析。

- **批量。** 一次请求覆盖一个画面所需的全部引用（正文段、选项文字、标题），宿主决定批量上限。
- **`unavailable` 不说明原因。** 不存在、无权限、已撤回、已擦除合并为同一个结果。
- **译本一致。** 同一画面的正文与选项文字用同一个上下文解析：读者选了某个译本，选项文字也
  来自该译本；缺失时由宿主回退到原文语言。本地内容方只按语言偏好选择；远程内容方还需要宿主
  对译本或版本的选择（`realization`）与对 Narrata 不透明的读者身份（`viewer`）。
- **锚点跨译本保持。** 译本必须保留原文的块 ID；某译本缺少段落端点时该段为 `incompatible`，
  宿主可以回退到原文。

## 呈现键

每个呈现项都有稳定的呈现键 `(ExecutionId, CommitId, occurrence)`。

- 生成式内容方（例如按状态生成叙述的 AI）以呈现键缓存结果：回退到同一提交得到同一段文字，
  走到新分支时键不同才重新生成。
- 需要"当时看到的是哪一版"的宿主，可以把解析得到的 `revision` 作为观察记录附在回执上。
  观察记录不是逻辑输入，缺失不影响恢复。

## 不属于内容的东西

- 会影响规则判断的资源（谜题答案、要与玩家输入比较的名字、作为条件的图像）进入程序的
  语义锁，见 [重建提案 §8](../../archive/2026-09-05-rebuild-proposal.md)。
- 生成式系统提出的新选项、新节点或新事实是**输入**，见 [选择模型](choices.md#动态选项与节点)。

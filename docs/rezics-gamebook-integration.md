# REZICS Gamebook 与 Narrata 的集成边界

状态：已决定  
日期：2026-08-31

## 决策

取消原计划由 REZICS 拥有的 `GameContentStructure`。

REZICS 继续拥有内容、普通 Content Structure 及其稳定的 occurrence node。Narrata
引用这些节点，在它们之上定义和执行选择、分支、汇合、循环、入口与结局，并负责相关的
前端呈现和运行存档语义。

如果未来继续使用 `GameContentStructure` 这个名称，它只能表示 Narrata 的一种关系文档或
Narrata 对外提供的兼容概念，不再是 REZICS 的独立领域模型。

## 被取代的原设计

REZICS 不再为 Gamebook 单独建设以下能力：

- `GameContentStructure` 数据库模型、领域服务和 API；
- REZICS 专属的选择边、入口、分支、汇合和结局 Schema；
- 解释或执行这些关系的 REZICS runtime；
- 与 Narrata 重复的 Gamebook 图编辑器和关系渲染器；
- 独立于 Narrata 存档语义的 Gamebook Journey 导航模型。

REZICS 可以保存 Narrata 关系文档和用户存档，也可以为它们提供权限、同步与产品界面；这种
持久化不使 REZICS 成为叙事关系的语义所有者。

## 所有权

| 能力 | 所有者 |
| --- | --- |
| Unit、Post、Portable Text 和本地化内容 | REZICS |
| 普通 Content Structure、节点出现身份和编辑顺序 | REZICS |
| 内容读取权限、生命周期与 canonical 页面 | REZICS |
| 入口、选择边、分支、汇合、循环与结局 | Narrata |
| 有向图、可达网络及森林投影 | Narrata |
| 叙事执行状态、路径记录和 Snapshot | Narrata |
| 选择列表、关系图和运行态 Reader 交互 | Narrata |
| 用户身份、云端持久化和跨设备同步 | REZICS 或其他宿主 |

## 节点身份

Narrata 节点引用的是 REZICS `ContentStructureNode`，不是它最终指向的 Post ID。

```text
Narrata graph node
  = REZICS ContentStructureNode occurrence
  → contentUnitId
  → Post
```

同一个 Post 可以在一个或多个 Content Structure 中出现多次。每次出现可以承担不同的叙事
位置、入口和选择关系。因此 Narrata 的边必须连接 occurrence node：

```text
fromContentStructureNodeId → toContentStructureNodeId
```

不能简化为：

```text
fromPostId → toPostId
```

跨仓库输入在进入 Narrata runtime 前必须经过验证。一个节点引用至少需要足以证明其来源和
出现身份的信息；最终 wire contract 由后续协议设计固定。概念形状如下：

```ts
type ExternalContentNodeRef = {
	provider: "rezics";
	structureId: string;
	nodeId: string;
};
```

`contentUnitId`、Post 正文、当前本地化和访问能力由 REZICS resolver 根据这个引用解析，
不成为 Narrata 图节点的权威身份。

## Narrata 关系模型

底层关系统一建模为有向多重图：

- 每条选择边拥有自己的稳定 ID；
- 同一对节点之间可以存在多条语义不同的选择边；
- 多个入口可以形成多个起始区域；
- 分支可以重新汇合；
- 是否允许循环由具体 Narrative Program 的能力和验证规则决定；
- 树和森林是图在特定约束或呈现场景下的投影，不是底层唯一数据结构。

概念形状如下，示例不承诺最终 API 名称或序列化格式：

```ts
type NarrativeChoice = {
	id: string;
	from: ExternalContentNodeRef;
	to: ExternalContentNodeRef;
	presentation: ChoicePresentation;
};

type NarrativeGraph = {
	id: string;
	revision: string;
	entries: ExternalContentNodeRef[];
	choices: NarrativeChoice[];
};
```

关系文档和编译后的 Narrative Program 都由 Narrata 定义版本、验证规则和迁移语义。REZICS
不得通过复制一套本地联合类型来推测这些语义；若必须生成绑定，应从 Narrata 的权威 Schema
生成并通过契约测试检测漂移。

## 运行与前端呈现

一次典型读取流程为：

```text
Narrata Runtime
  决定当前 ExternalContentNodeRef 和可用选择
        ↓
REZICS resolver
  验证结构成员关系、权限和内容生命周期，并解析 Post
        ↓
REZICS content renderer
  渲染正文
        +
Narrata renderer
  渲染选择、路径、网络或森林关系
```

Narrata 可以提供完整 Reader 外壳，但 Post 内容应通过宿主 adapter 或 render slot 交还给
REZICS，避免 Narrata 重复实现 Portable Text、本地化、内容授权和内容生命周期。

一个可能的 Web 集成外形是：

```tsx
<NarrataReader
	runtime={runtime}
	resolveContent={resolveRezicsContentNode}
	renderContent={renderRezicsPost}
/>
```

Narrata runtime 不因为收到一个格式正确的节点引用，就假定该节点当前存在、仍属于该结构或
可以被当前用户读取。REZICS resolver 在每个失去信任的边界重新建立这些保证，并以显式的
not-found、forbidden、retired 或 incompatible 结果返回。

## 存档

Narrata 产生版本化 Snapshot 和必要的路径事件；宿主决定保存位置。

Gamebook 的最小 Snapshot 至少需要表达：

- Narrative Program 或关系图身份与版本；
- 所依赖的 REZICS Content Structure 身份与兼容信息；
- 当前 `ContentStructureNode` 引用；
- 当前等待中的交互；
- 已提交的选择路径或其日志游标；
- Snapshot Schema 版本。

未来加入变量、调用栈、随机数或其他运行能力时，它们继续由 Narrata Snapshot 表达。REZICS
可以把 Snapshot 作为用户数据持久化，同时保存检索、归属和同步所需的宿主元数据，但不能
解析未知字段后自行重建一个不同的运行语义。

旧内容与新内容之间不能通过 Post 标题、数组位置或当前排序猜测存档位置。恢复必须使用稳定
节点和选择 ID，并采取以下一种显式结果：精确恢复、经过已声明迁移恢复，或报告不兼容。

## SEO

Narrata 的选择关系、可达网络、森林视图、玩家路径和存档不承担 SEO：

- 不为每一个选择、运行状态或玩家路径生成可抓取页面；
- 不要求搜索引擎理解 Narrative Graph；
- 关系数据和运行状态可以在客户端按需加载；
- 若以后需要分享路线，应建立单独的、受权限控制的分享投影。

Post、Book 及其他公共内容的 canonical URL、服务端元数据和搜索引擎可见性仍由 REZICS
负责。“关系不需要 SEO”不代表应把原本需要索引的 Post 正文隐藏在只能由客户端执行的
Narrata 状态之后。

## 非目标

- 不让普通 REZICS Content Structure 承载选择边。
- 不让 Narrata 成为 Post 正文、授权、本地化或 SEO 的权威来源。
- 不要求 REZICS 理解或执行 Narrata 内部 IR。
- 不把 Post ID 当作一次内容出现的叙事身份。
- 不把树或森林限制强加给所有 Narrata 关系。

## 后续工作

1. 在 Narrata 固定 provider-neutral 的外部内容节点引用协议。
2. 定义有向多重图、稳定选择 ID、入口与结局的权威 Schema。
3. 定义 Snapshot、路径事件、内容版本兼容与迁移协议。
4. 为 REZICS 实现节点 resolver、正文 render adapter 和 Narrata 前端集成层。
5. 删除或改写 REZICS 中仍把 `GameContentStructure` 和 Journey 描述为 REZICS 自有领域模型的规划文档。

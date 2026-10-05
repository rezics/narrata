# ADR 0022：创作记录、发布对象、关系投影与读者地图

状态：Accepted（2026-10-05）。依据[决定 2、3、4、5、7、10、12、16、17](../product/decisions.md)，
补充 [ADR 0013](0013-r2-text-free-node-format.md) 与 [ADR 0016](0016-graph-publication-format.md)。
本页决定无界面的宿主接口，不决定 REZICS 的编辑器界面。

## 现状与边界

[源稿类型](../../packages/narrata/nodes/crates/narrata-nodes/src/source.rs)已有 `ProjectSource`、
`ProjectManifest`、`PackageSource`、`GraphSource`、`NodeSource`、`ChoicePointSource`；
[`compile`](../../packages/narrata/nodes/crates/narrata-nodes/src/compile.rs)接收内存源稿与
`NodeRegistry`，返回 `Result<Compilation>`，语义错误首错即停，不铸造 ID。
[`compose`、`verify_lock`](../../packages/narrata/tooling/crates/narrata-node-tools/src/compose.rs)
依赖 `ProjectFiles`；[`ids::Minter`、`fill`](../../packages/narrata/tooling/crates/narrata-node-tools/src/ids.rs)
只在原生构建提供，CLI 将结果写回文件。
[`publish::publish`、`publication_analysis`](../../packages/narrata/tooling/crates/narrata-node-tools/src/publish.rs)
已能从受检程序生成图对象与大纲诊断；`write_publication` 已分别写图对象，程序仍写 `.narpack`。
[`publishGraph`](../../packages/narrata/nodes/crates/narrata-nodes-wasm/src/lib.rs)只接受完整 pack。
以下接口名和新类型均为提议，不能当作已交付 API。

正文、标题、选项文字、媒体仍由宿主解析 `ContentRef` / `Segment`，不进入新记录的内联字段。
R2 源稿、lock v2、程序 CBOR schema 1、pack、图与标签 schema 1、语义摘要 v1、存档均不改。
新格式只有草稿记录 v1、发布对象集合描述 v1、关系投影 v1，以及诊断和地图的宿主交换形状。
第一阶段保留全部旧读取路径；存档跨发布的继续与 dry-run 交由并行的 ADR 0023 决定。

## 决定与理由

### 1. 草稿是源稿的拆分，选择点是主要编辑记录

每个选择点一条记录，选项留在这条记录内。段落跨多个选择点的 `body`、`args`、`next` 不复制到
每条记录；单独的节点记录持有它们。只用选择点记录会丢掉无选择的段落、控制节点、调用和产品声明；
把整个单元或作品放进一个修订则扩大并发冲突，并容易超过 REZICS 单个 Content revision 的 1 MiB。

记录是 UTF-8 严格 JSON；`format_version: 1` 表示拆分方式，内含的源稿仍是 R2。
共同信封为 `{format_version, kind, ...定位字段, payload}`，按下表精确区分字段，不接受未知字段。
落库字节由现有 `canonical_json` 生成；先拒绝重复键再规范化，数组保留作者顺序，不按 ID 重排。

| kind | 定位字段与 payload；不重复存放的部分 |
| --- | --- |
| `project` | 无定位字段；`ProjectManifest`，`packages` 的路径只作文件导入导出提示，不用于数据库取数 |
| `package` | `package`（实例别名）；`PackageSource`，`graphs = {}`、`tombstones = []` |
| `graph` | `package, graph`；`GraphSource`，`nodes = {}` |
| `node` | `package, graph, key`（节点别名）；`NodeSource`；passage 的 `data.choice_points = []`，另有信封字段 `choice_order: [ChoicePointId]`；其他类型无此字段 |
| `choice_point` | `package, graph, node: NodeId`；原样的 `ChoicePointSource` |
| `tombstone` | `package`；单个 `AuthoredId`，包删除前须处理其墓碑归属，不能丢掉记录 |

持久记录要求节点、选择点、选项已有 ID；尚未铸造的编辑值只作为接口输入，不作为有效落库记录。
节点 / 选择点的记录键由 Narrata 从其 ID 导出，头记录由 kind 与作用域导出；宿主自己的修订主键
不是叙事身份。别名更改不换作者 ID。墓碑逐 ID 保存，避免累计删除使包头越过修订上限。
v1 单条记录上限 1 MiB；过大的合法 R2 条目在导入时返回 `record_too_large` 与源路径，不截断或
自动拆语义。R2 文件仍可直接编译；宿主接入前须显式整理超限条目。作品总量靠包、图和记录分组，
不把全部记录装进一个受 16 MiB 限制的 JSON 参数；组装后沿用现有包 / 图 / 块预算。

提议纯函数 `assembleRecords(records) -> Report<ProjectSource>` 与逆函数 `splitSource(source)`。
组装检查头记录唯一、包实例集合与 manifest 一致、图及节点别名唯一、ID 唯一、定位与归属一致；
按 `choice_order` 取齐选择点，拒绝重复、缺项与孤儿，再填回原来的 maps、数组和墓碑。
拆分、组装、规范化后的源稿相同，且编译得到相同构件与 lock；没有第二套执行词汇。

宿主保留不透明载荷，只索引 Narrata 的 `deriveDraftIndexes(checkedRecords)` 输出：
记录键、作用域、节点 / 选择点 / 选项 ID、选项序号、来源单元的 provider / key、`placement`、
回应与汇合锚点、结果种类、目标节点 ID、目标单元 provider / key、所引用的标签 ID。
每选项一条派生索引行；头记录另有所属关系索引。分支别名须在图上下文解析，目标是控制节点时
目标单元为空并标为 `control`，坏引用标为 `unresolved`；不能靠单条记录猜测跨记录结果。
来源由父 passage 的 `body.unit` 推导，局部回应不产生跨单元目标。条件和效果不做数据库查询字段。
宿主的 Content model projection recipe 调用 Narrata 适配器，以载荷修订与依赖修订为输入；
修改父节点或目标正文引用时重算受影响索引，并与载荷在同一乐观并发事务内替换，禁止手改索引列。

### 2. 编辑校验、铸造与连续性脱离文件系统

提议 `validateRecord(bytes)` 返回规范记录与局部诊断，`validateProject(records)` 返回跨记录诊断。
前者证明编码、字段、内置节点数据与可独立判断的基数等约束；变量类型、目标、端口和正文顺序
需要后者的图 / 产品上下文与内容大纲。JSON Schema 只证明形状，不能替代这些语义检查。
不通过的记录允许宿主作为未提交编辑缓冲保存；没有受检值就不能导出权威派生索引或发布。

校验器在可独立检查的记录、字段和图上收集全部诊断，再决定是否编译；不循环删除首错来试编译。
语法已坏的记录报告解析位置，依赖它的检查明确标记跳过，其余记录继续。达到输入或工作量预算时
返回 `complete: false` 和 `limit` 诊断，不以空数组或截断列表声称“全部”。错误存在时无构件结果。
`compile` 的旧 `Result` 入口保留；抽出共用校验 / lowering，避免两份规则渐渐分叉。

提议 `mintIds(edit, occupied)` 复用 `Minter` 的 UUIDv7 算法，clock 与安全随机源由平台适配层提供。
输出新记录及铸造列表，不读写文件；占用集合含全项目 live ID、所有墓碑和本次已分配 ID。
编辑器将结果与载荷一同持久化，再校验；发布、重试与规范化从不自动重铸已有 ID。
复制和变更归属使用显式铸造操作；仅改内容引用、条件、顺序、别名或目标不换 ID。
宿主并发写入仍检查 ID 唯一；随机碰撞不能靠覆盖旧记录解决。

提议 `composeSource(source, previous?, mode)` 是 `compose` 的内存核心。`previous` 是受检的上次
程序对象集合（或经现有 `Program::from_pack` 打开的 pack），不是宿主自报的 ID 清单。
用 `Program::owners()` / `tombstones()` 保持现有归属与墓碑规则：拒绝复用、移主、删旧墓碑；
普通模式返回应追加的墓碑记录与新 lock，由宿主原子保存；不暗中写数据库。
`mode = locked(expected: CompositionLock)` 要求已有墓碑齐全且新 lock 与 expected 完全相同；
缺 lock、需要追加墓碑、源稿或节点语义修订变化均失败。无 previous 只可作为首次发布，返回
`compared: false`；宿主的再次发布必须提交当前发布作为 baseline，不得省略它绕过连续性。
删掉整个包却不能按既有规则找到墓碑归属时仍报告错误；本 ADR 不发明静默搬迁路径。

### 3. 发布返回对象集合，宿主最后切换发布引用

提议 `publishRecords(records, outlines, previous, mode) -> PublicationReport`：组装、校验、
内存 compose、调用现有 `publish::publish` / `publication_analysis`、拆分程序对象、生成关系投影。
outlines 复用 `ContentOutline` v1，每个内容方一份原文大纲；只含块 ID 顺序及标记，不解析正文。
成功返回规范化记录 / 墓碑补丁、派生索引、新 lock、诊断，以及下列不可变对象；错误返回诊断。
“编译成功”与“可切换发布”是不同的枚举结果，宿主不从消息文本判断。

| 对象 | 身份与读取 |
| --- | --- |
| 程序 manifest、各 chunk、墓碑集、可选名字表 | 原有 `object_id(kind, schema, payload)`；原封保留信封字节，manifest 的 ID 仍是 `artifact_id` |
| 图索引、三层瓦片、按簇标签表 | 沿用 ADR 0016 的身份与受检读取，不塞进程序 manifest |
| 语义摘要 | 原 `SemanticSummary` v1，经 `canonical_json` 编码；`digest_bytes("narrata.nodes.summary", 1, bytes)` |
| 关系投影片及其索引 | 下节的规范 JSON；分别用 `narrata.nodes.relation-part` / `narrata.nodes.relation-index`，摘要版本 1 |
| 可选演化描述符（角色 `evolution`） | ADR 0023 定义的规范编码与受检读取，按其 `MigrationId` 寻址，随候选发布永久保留 |
| 发布对象集合描述 | 规范 JSON v1；`digest_bytes("narrata.nodes.publication", 1, bytes)`，称 `publication_id` |

集合描述包含 `format_version, artifact_id, graph`（原 `GraphFiles` v1 的引用形状）、summary ID、
relation index ID、可选 names ID 与 evolution MigrationId，以及按角色与 ID 排序的全部对象 ID、字节长度和编码。
对象名为 64 位小写十六进制摘要加 `.cbor` / `.json`；JSON 身份对照上述域，不把裸 SHA-256
或信封内的载荷校验值当作 object id。重复对象去重；同 ID 不同字节、缺对象、未知角色、长度或
版本不符均拒绝。组装读取复核程序与伴随对象的 artifact 归属，发布方还重建投影验证语义对应。
`artifact_id` 识别程序，`publication_id` 识别程序加配套投影；更新布局或投影而程序不变时两者不同。
输出按对象迭代 / 拉取，二进制用 `Uint8Array`，不生成一份巨大 JSON 数字数组或全作品 pack。
`.narpack` 仍可按需从同一组程序对象打包，旧 pack 经 `Pack::decode` 拆分，不重新编码程序对象。
现有 compile 会先构造 pack 再重新打开复核；实现须抽出共用的对象编译路径，以
`Program::from_manifest` 与 `verify_artifact` 复核全部对象。旧 compile 保留打包行为；64 MiB
只限单文件 pack，对象发布不绕经它，R2 的单包源稿、图、chunk 与 manifest 预算不变。
大纲也按单元分批受检组装为每 provider 一份，避免把全作品大纲塞进一个 JSON 参数。

诊断报告 v1 使用 `{complete, diagnostics: [{severity, code, record?, pointer, source_path?,
node?, choice_point?, option?, unit?, anchor?, related: [...], message}]}`。
`pointer` 是记录内 JSON Pointer；source_path 保留现有 `Diagnostic.path` 供 CLI；ID 定位优先于
可改名别名，related 关联重复声明或标记位置。按记录键、pointer、code、相关 ID 稳定排序去重。
`outline_anchor_missing`、`outline_order`、`outline_marker_mismatch` 等沿用现有 code，结构不可达
也沿用现有诊断；新字段由包装层补齐，不改变旧 `Analysis` schema。messages 不承担机器契约。
源码语义、连续性、编码与预算失败为 error；拓扑不可达 / 环为 warning；大纲缺陷仍为 warning，
提供者缺失为 `outline_checks_skipped`。保持旧 compose 的成功语义；REZICS 的发布策略明确要求
完整大纲检查且无大纲缺陷，拒绝切换引用，不能悄悄忽略这些 warning。

宿主先以条件创建上传全部对象，受检回读 / 核对引用闭包，再完整装载投影、写语义摘要并取得回执；
最后比较并交换“作品当前发布”（宿主修订号 + publication_id + artifact_id + 投影版本）。
失败只留下未被当前引用引用的对象，读者继续读旧发布；重试相同字节与投影是幂等的。
旧程序永久保留供存档读取与重新投影，正文撤回仍由内容方处理，发布对象不含正文。
返回 `previous_artifact_id, candidate_artifact_id` 及受检对象访问器；`publishRecords` 的签名不变。
发布服务在 `publishRecords` 之后、CAS 之前调用作者模块的独立函数 `planEvolution(previous, candidate, declarations?)`。
它产出的 ADR 0023 演化描述符以可选 `evolution` 角色加入候选对象集合，列入集合描述后重算
publication_id，随发布永久保留，无需另一套保留机制；描述符格式、演化规划与存档改写归 ADR 0023。

### 4. 创作 Wasm 与读者 Wasm 独立交付

新增 `narrata-authoring`（Rust 内存 API）与 `narrata-authoring-wasm`，依赖现有 nodes、graph 和
抽出的 compose / publish 核心；读者 crate 不依赖它们。接口带版本，schema 与 TS 声明由 Rust
生成并检查漂移。作者模块也导出下节投影重建与地图纯函数；不再另外实现服务器编译器。
既有 `NodeBook` 与 `publishGraph` 入口保留兼容，新编译、记录与投影功能只进入作者模块；
当前读者模块约 960 KB 未压缩的基线不因本工作增加 compiler / authoring 依赖。
后续是否移除旧 publishGraph 依赖单独决定，不在这里破坏已有调用。

浏览器用 npm 的独立 authoring 子路径动态 import，在编辑 / 发布 Web Worker 中显式初始化 Wasm；
互动小说读者路由只初始化 reader 子路径。默认读者地图由服务器生成；离线地图可另行懒加载
authoring 子路径，这项下载计入地图预算，不计入阅读首屏。Node / Bun 使用同包 server 子路径，
从包内资源加载相同 Wasm 字节并初始化，既无浏览器全局依赖，也无工作目录 / 仓库绝对路径依赖。
UUIDv7 的随机 / 时间能力只在铸造调用使用；校验、发布与地图计算不访问网络、数据库或文件系统。

### 5. 关系投影 v1 是可重建的事实行

`projectRelations(checkedProgram)` 为纯函数，复用 `structural_graph`、分析与 `SemanticSummary`。
以 `(artifact_id, projection_version = 1)` 标识一套行，按行类型与主键排序，规范 JSON 分片
（每片不超过 4 MiB、不拆行），索引记录各片身份、类型、行数与总数。每片含版本与 artifact；
拒绝未知列、重复 / 无序主键、错长 ID、非法引用、超预算与不完整片集合，不截断事实。
同一键必须得到完全相同的投影字节，算法变化导致字节或含义变化即升投影版本。
单片良构不证明语义对应，发布时从程序重建比较；宿主装载的是受检行，不解析执行载荷。

| 行类型 | 列与推导；每行还含 artifact_id、projection_version |
| --- | --- |
| unit | unit_id（32 字节）、provider、content_key；收集 passage、局部回应与产品结局正文的 `Segment.unit`，同引用去重；ID 为 `digest_bytes("narrata.nodes.relation-unit", 1, ContentRef::to_bytes())`，碰撞拒绝 |
| node | node_id、package、graph、kind、unit_id?、first_anchor?、last_anchor?、cluster_id；所有静态节点一行，有正文的 passage 映射其切片；入口图 return 映射该 outcome 的结局正文；控制节点的 unit 为空 |
| edge | source_unit、target_unit、source_node、target_node、edge_kind、choice_point?、option?；跨单元静态转移，保留不同选项，不按单元对合并 |
| entry | node_id；产品入口，内容前的控制入口也保留 |
| ending | node_id、class、reachable；入口图 return，沿用现有 unspecified 分类，不从 outcome 名字猜成败 |
| route | ending_node、cluster_id；现有 ending.clusters 的结构祖先簇集合，route 以 ending_node 标识，非作者命名路线或执行可行性证明 |
| cluster | cluster_id、unit_id?、package?、graph?；沿用 ADR 0016 的内容簇 / 图簇身份，结局单元不篡改 return 的现有图簇归属 |
| first | unit_id、node_id、distance；产品入口的结构 BFS 最短距离最小的同单元节点，平局全部保留；不可达单元无行 |

edge 从有 unit 的节点出发，沿 choice / next / conditional / call / return 的静态关系穿过无 unit
节点，到下一有 unit 节点即停；只为不同单元输出行，记录起始转移种类及选择点 / 选项。单元内
的静态后继由其各源节点继续投影；局部回应不生成跨单元边。控制环按访问状态去重，不枚举路径。
候选多目标全保留，完整自然元组去重；条件、调用栈与提议不求解。全局最多 4,000,000 条 edge / route 行
（分别计），超限报发布错误，避免控制图展开产生无界输出；此限制不是“10 万选择点必然可发”的证明。
首次出现仅是**内容单元的结构首次候选**，不能用于声称人物、实体或任意读者首次见到某事物。
现有代码没有实体模型；实体首次出现不伪造，需未来格式与显式来源。读者是否见过以存档派生结果为准。

PostgreSQL 参考 DDL 如下：没有 `game` / `vn` 表名；宿主按自己的编号 SQL 安装，npm 不执行迁移。
`digest` / `id` 分别为 32 / 16 字节；数据库约束只证明列形状，跨行含义由受检装载器验证。

```sql
CREATE DOMAIN narrative_digest AS bytea CHECK (octet_length(VALUE) = 32);
CREATE DOMAIN narrative_id AS bytea CHECK (octet_length(VALUE) = 16);
CREATE TABLE narrative_projection (
  artifact narrative_digest NOT NULL, version smallint NOT NULL CHECK (version > 0),
  projection_index narrative_digest NOT NULL, PRIMARY KEY (artifact, version)
);
CREATE TABLE narrative_unit (
  artifact narrative_digest NOT NULL, version smallint NOT NULL,
  unit narrative_digest NOT NULL, provider text NOT NULL, content_key text NOT NULL,
  PRIMARY KEY (artifact, version, unit), UNIQUE (artifact, version, provider, content_key),
  FOREIGN KEY (artifact, version) REFERENCES narrative_projection
);
CREATE TABLE narrative_cluster (
  artifact narrative_digest NOT NULL, version smallint NOT NULL,
  cluster narrative_id NOT NULL, unit narrative_digest, package text, graph text,
  CHECK ((unit IS NOT NULL AND package IS NULL AND graph IS NULL) OR
         (unit IS NULL AND package IS NOT NULL AND graph IS NOT NULL)),
  PRIMARY KEY (artifact, version, cluster),
  FOREIGN KEY (artifact, version) REFERENCES narrative_projection,
  FOREIGN KEY (artifact, version, unit) REFERENCES narrative_unit
);
CREATE TABLE narrative_node (
  artifact narrative_digest NOT NULL, version smallint NOT NULL,
  node narrative_id NOT NULL, package text NOT NULL, graph text NOT NULL,
  kind smallint NOT NULL CHECK (kind BETWEEN 0 AND 4), unit narrative_digest,
  first_anchor text, last_anchor text, cluster narrative_id NOT NULL,
  PRIMARY KEY (artifact, version, node),
  FOREIGN KEY (artifact, version, unit) REFERENCES narrative_unit,
  FOREIGN KEY (artifact, version, cluster) REFERENCES narrative_cluster
);
CREATE INDEX narrative_node_unit ON narrative_node (artifact, version, unit, node);
CREATE INDEX narrative_node_cluster ON narrative_node (artifact, version, cluster, node);
CREATE TABLE narrative_edge (
  artifact narrative_digest NOT NULL, version smallint NOT NULL,
  edge_no bigint NOT NULL CHECK (edge_no >= 0), source_unit narrative_digest NOT NULL,
  target_unit narrative_digest NOT NULL, source_node narrative_id NOT NULL,
  target_node narrative_id NOT NULL, kind smallint NOT NULL CHECK (kind BETWEEN 0 AND 4),
  choice_point narrative_id, option_id narrative_id,
  CHECK ((kind = 0 AND choice_point IS NOT NULL AND option_id IS NOT NULL) OR
         (kind <> 0 AND choice_point IS NULL AND option_id IS NULL)),
  PRIMARY KEY (artifact, version, edge_no),
  FOREIGN KEY (artifact, version, source_unit) REFERENCES narrative_unit,
  FOREIGN KEY (artifact, version, target_unit) REFERENCES narrative_unit,
  FOREIGN KEY (artifact, version, source_node) REFERENCES narrative_node,
  FOREIGN KEY (artifact, version, target_node) REFERENCES narrative_node
);
CREATE INDEX narrative_edge_from ON narrative_edge (artifact, version, source_unit, source_node);
CREATE INDEX narrative_edge_to ON narrative_edge (artifact, version, target_unit, target_node);
CREATE TABLE narrative_entry (
  artifact narrative_digest NOT NULL, version smallint NOT NULL, node narrative_id NOT NULL,
  PRIMARY KEY (artifact, version, node),
  FOREIGN KEY (artifact, version, node) REFERENCES narrative_node
);
CREATE TABLE narrative_ending (
  artifact narrative_digest NOT NULL, version smallint NOT NULL, node narrative_id NOT NULL,
  class text NOT NULL CHECK (class IN ('unspecified','success','failure','neutral')),
  reachable boolean NOT NULL, PRIMARY KEY (artifact, version, node),
  FOREIGN KEY (artifact, version, node) REFERENCES narrative_node
);
CREATE TABLE narrative_route (
  artifact narrative_digest NOT NULL, version smallint NOT NULL,
  ending_node narrative_id NOT NULL, cluster narrative_id NOT NULL,
  PRIMARY KEY (artifact, version, ending_node, cluster),
  FOREIGN KEY (artifact, version, ending_node) REFERENCES narrative_ending,
  FOREIGN KEY (artifact, version, cluster) REFERENCES narrative_cluster
);
CREATE INDEX narrative_route_cluster ON narrative_route (artifact, version, cluster, ending_node);
CREATE TABLE narrative_first (
  artifact narrative_digest NOT NULL, version smallint NOT NULL,
  unit narrative_digest NOT NULL, node narrative_id NOT NULL,
  distance bigint NOT NULL CHECK (distance >= 0), PRIMARY KEY (artifact, version, unit, node),
  FOREIGN KEY (artifact, version, unit) REFERENCES narrative_unit,
  FOREIGN KEY (artifact, version, node) REFERENCES narrative_node
);
CREATE INDEX narrative_unit_reference ON narrative_unit (provider, content_key, artifact, version);
```

edge_no 是完整自然元组按上表列顺序排序后的零基序号（缺值先于有值），不铸造作者 EdgeId；外部引用还带 artifact /
version。所有表主键包含版本键；插入既有键须逐字节相同，禁止覆盖。新一套行在单事务完整装载，
大批量可先装宿主 staging 表再事务提升；应用角色仅 SELECT / INSERT，清理角色才有 DELETE。
投影升级从永久保留的 manifest + chunks + tombstones 重建，不依赖草稿、正文、大纲或旧行。
新版本核验通过后切换宿主发布描述 / 投影指针，再删旧版本行；旧版 reader 所需格式仍可重建读取。
R2 图与程序读取路径不改；没有已有关系投影需要原地迁移。

10 万选择点的估计取 5,000 单元、每点 3 选项、3% 选项为跨单元分支，并保守取每点一 passage
加 10,000 控制节点；局部选项不各占一行。约 5,000 unit + 110,000 node + 20,000 edge +
1 entry + 20 ending + 100,000 route（20 结局各 5,000 簇）+ 5,200 cluster + 5,000 first，
合计约 245,221 行，另 1 套头。假设引用平均 80 字节、别名平均 16 字节，PostgreSQL heap 与
上述 B-tree 约 70–140 MiB、规范 JSON 约 30–70 MiB；这是含行头与重复版本键的容量预算，非实测。
不含正文、WAL、副本与暂存表；重复索引、长 key、候选展开、结局数量与首次候选平局都会增加体积。
实现用合成作品测实际行数、字节及装载时间，不能把这组分支比例当成格式上限。

### 6. 已访问集合与地图都是纯计算

提议 `visitedFromSave(checkedProgram, saveObjects, cursor) -> VisitReport`：受检读提交、State、Input，
从根到 cursor 重放，验证状态与提交摘要；只读执行 trace 只定义并实现一次，同时供 ADR 0023 路径比较。
trace 按执行顺序记录 graph / node、呈现的 Segment（单元 / 锚点及 `Interaction::Finished.body`）、
选中选项的 outcome / target、branch 去向与 call / return；从它派生相邻单元转移及首次访问顺序。
默认关闭的 feature 将它编进 authoring / server Wasm，读者 Wasm 不含 trace；作品升级的评估与应用
也在此模块运行：服务器在 Node 中，匿名读者在浏览器中只在需要升级时懒加载，不计入阅读首屏。
不能只收集 `HistoryView.node`：一次输入可经过多段落与调用，等待位置会漏掉自动执行的内容。
输出带 artifact、cursor、已访问静态节点 / 单元 / 段、走过的边、当前可见交互的 frontier；
动态提议中的引用可记已呈现单元，但不冒充静态投影节点或改写投影。
默认只含当前游标祖先，回退后不混入未选分支；宿主若选“曾读过”模式，可显式合并多个受检游标的
结果并保留来源。浅存档缺祖先时返回 `incomplete` 与缺失对象，不编造完整访问史；补齐后重算。
存档中的“执行并呈现”不证明人确实读过，也不要求旧正文仍可解析。

提议 `readerMap(projection, visits) -> ReaderMapV1`：核对两者 artifact 一致及支持的投影版本，输出已访问单元、
其中已访问节点、实际走过且两端已揭示的边、已到达结局及当前可见选项占位。只看双方端点访问过
仍不能推断边走过；不输出全局路线集合、隐藏边数、ghost、完整簇大小或未访问切片的锚点。
单元内边来自 visits 的执行 trace，只保留已访问端点，不把关系投影缺少单元内边解释为未走过。
frontier 由受检当前状态计算 `visible_if`，不从无条件关系行推断可见性；disabled 可见选项仍可有
占位，但不声称可走。目标尚未访问时占位只有已在交互中可见的 OptionId / labelRef / enabled，
无目标节点 ID、单元 key、坐标或标题；目标访问过且当前结果可确定时才连到已揭示节点，否则
仍只显示选项占位，不以结构候选声明实际下一章。服务器不向地图客户端
发送全投影 / 作者瓦片 / 标签表。浏览器本地计算须有投影输入，已持有的程序可被检查，这不构成保密。
地图坐标按首次访问顺序紧凑分配，追加访问不重排旧节点；不复用全图包围盒或留出隐藏节点空隙。
纯函数只返回内容引用与有限整数坐标，正文解析与渲染仍交宿主。两函数同一 Rust 核心，经作者
Wasm 在浏览器与 Node / Bun 运行；服务器按允许集合取投影行的适配器必须返回闭合且受检的子集。

浏览器上报 visits 不能证明游玩：读者能伪造或修改自己的输入。宿主可选从其服务器存档重放验证，
或接受客户端集合改善个人体验；后一种不能用于付费授权、成就奖励或他人的防剧透判断。
防剧透保护体验，不是安全边界。R1 阅读进度以当前存档 / cursor 映射到当前实际呈现的 Occurrence；
继续阅读与上一 / 下一步使用该发布的 visits、可见 frontier 与自己的恢复点；未读数只对宿主明确
提供的已揭示候选集合计数，未揭示部分保持未知，不用章节线性顺序或把“结构可达”当“现在可读”。
宿主保留自己的登录、可见性和内容授权检查。

### 7. 实现依赖、验证与交给 manager 的能力请求文本

依次实施，冻结输出前先生成 schema / 受检读取；均不改已有 compat 目录：

| 实现范围（拟新增路径） | 依赖、检查与新冻结语料 |
| --- | --- |
| `nodes/crates/narrata-nodes/src/{compile,check,registry}.rs`；`tooling/crates/narrata-authoring/src/{records,validate,ids,compose}.rs` | 先提取共用语义检查及 compose 核心，生成 `nodes/schemas/draft-record.schema.json` 等新 schema；测试多错并报、坏 JSON、1 MiB 边界、split/assemble、UUIDv7、移主与墓碑连续、locked 失败不写入；`fixtures/compat/draft-records-v1/` 冻结规范记录与 R2 组装结果 |
| `tooling/crates/narrata-authoring/src/{publication,relations}.rs`；现有 `narrata-node-tools/src/{compose,publish}.rs` | 依赖上项与共用对象编译路径，CLI 成为文件适配器；新发布 / 投影 schema、受检解码；`fixtures/compat/publication-v1/`、`relations-v1/` 冻结索引、分片、行、全部对象摘要；测试原 pack 拆装、对象总量超过 64 MiB 而单对象合法、不同输入顺序同摘要、正文独立、坏引用 / 重复 / 缺片拒绝、旧构件重建、新旧发布隔离与幂等 |
| `nodes/crates/narrata-nodes/src/runtime.rs`；`tooling/crates/narrata-authoring/src/maps.rs` | 唯一的共用只读 trace 依赖既有历史读取，同时是 ADR 0023 路径比较的前提；地图依赖投影；测试自动节点、调用、局部 / 多选、提议、回退、浅存档 incomplete、visible / disabled frontier，以及输出递归不含隐藏 ID / key / 坐标；生成地图交换 schema，不修改存档格式 |
| `tooling/crates/narrata-authoring-wasm/`；`packages/narrata/web/`；`examples/gamebook-web/`、`products/gamebook-demo/` | 依赖前述接口；同一“山口来信”经记录发布与 CLI 的程序字节一致；浏览器 Worker、Node、Bun、打包 tarball 消费测试，验证 authoring 懒加载、reader 首屏体积不增长、超大作品不通过 JSON 全量搬运 |

上表 `nodes/`、`tooling/` 均相对 `packages/narrata/`。实现按 crate 跑 test / clippy 和 Wasm target
check，整合后 `task check:r1`（含新接口与 tarball 的浏览器 / Node / Bun 回归）、文档改动
`task docs:check`；worker 都经 `task goal -- slot`，gate / 浏览器用 `--heavy`。
投影参考 DDL 另用真实 PostgreSQL 验证装载、主键冲突、仅插入权限、版本切换与上述规模预算；
新增检查入口或工具时同步 `docs/development/toolchain.md`，不以内存假表代替数据库约束。

下面是供 manager 合入[REZICS 集成](../integrations/rezics.md)的请求文本，本任务不改该文档：

- **R4 改写：** 保存 Narrata 草稿 v1：每选择点一记录，另有项目 / 包 / 图 / 节点头与逐 ID 墓碑；
  单记录 ≤1 MiB，载荷不透明，Narrata 提供校验、规范化、组装、铸造与派生索引。用 Content model
  projection recipe 或等价适配层装载选项 ID、来源单元 / 块、目标节点 / 单元等派生列，载荷与
  受影响索引按修订号原子更新；表具主键，不使用 game / vn 等领域名，宿主不解释条件或效果。
- **R9 改写：** 发布服务用独立 authoring npm / Wasm 从草稿和大纲生成内容寻址对象集合；保存
  manifest、chunks、墓碑、图瓦片、标签、摘要与关系投影，永久保留程序。全部对象和投影就绪、
  Jena 摘要写入取得回执后，以修订号 CAS 切换作品 publication_id；失败可幂等重试且旧发布仍可读。
- **R12 改写：** 批量提供被引用 Occurrence 的原文语言 `ContentOutline` v1：完整有序块 ID 与
  narrata-choice 标记位置，无正文；支持分批收集为每 provider 一份大纲，固定到本次内容修订。
  发布前返回缺单元 / 锚点 / 顺序 / 标记诊断并实施阻止切换的宿主策略，内容后续修订仍由解析器处理。
- **新增“关系投影表”：** 按本 ADR 参考 DDL 保存 unit、node、edge、entry、ending、route、
  cluster、first 的受检事实行，键含 artifact_id / projection_version，只插入；提供按内容引用、
  已访问单元 / 节点、正反向边的有界查询。完整装载后启用，新版本从永久构件重建后切换并清理旧行。
  服务器用 Narrata 纯函数生成读者地图，可选验证客户端 visits；授权与隐私仍由 REZICS 负责。

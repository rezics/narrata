# ADR 0025：VN 只读依赖投影的版本化交换

状态：Accepted（2026-11-01，Web VN Goal 任务内选定；管理者合并时审阅）。在
[ADR 0024](0024-first-party-visual-novel.md) 之后，为公开的 VN 依赖准备桥接定下一个窄的、只读的
查询格式。它不改变任何已存储或已交换的格式，也不是完整的 VN2。

## 原因与边界

ADR 0024 第 5 条要求播放器在当前场景之外，有界准备直接候选走向所需的剧情块、文字与媒体。
原生 `Session::dependency_projection` 已经能在检查后的当前安全点上枚举这些依赖（保守并集，
不预测状态）。宿主与未来的 TypeScript 读者桥接需要一个稳定、可校验的交换形状来取得它。

本 ADR 只决定这个形状和它的传输约定：

- 复用既有的不可变程序格式、检查后的 Session 与原生查询。不增加 VM、不接受未检查 State、不增加
  持久对象，也不向 Source、State、Input、Commit、Checkpoint 或存档增加任何字段。
- 结果不含正文、媒体字节、URL、凭据或解码句柄；内容与媒体仍是宿主解析的引用（决定 2、18）。
- 它不是完整场景目标，而是当前 View 已保留资源之上的**增量**依赖；`complete` 只表示有界保守
  枚举结束，不表示未来状态被预测，也不保证读者会到达其中的引用。

## 决定

### 1. 第一个查询格式，只读取 version 1

请求与结果各带整数 `version`。解码器只接受 `1`，其他值（包括 `0`、`2`）、未知字段、未知
标签一律以受检错误拒绝，不降级、不忽略。这是该查询的第一个格式，没有旧版本需要读取或迁移。
所有 Gamebook 与 VN 的既有格式、`BookView`、`VnViewV1`、导出、存档及其读取方法保持不变，
冻结语料 `fixtures/compat/` 中既有目录不改。

### 2. 请求

`{"version":1,"cursor":"commit:…","limits":{locations,graphs,candidates,references,scans}}`。

- `cursor` 是调用方认为当前的 Commit；与会话游标不同则返回 `stale_input`，不返回对另一个游标的
  结果。这与 `choose` 的 expected 参数、`historyArtifacts` 一致。
- 五个界限必须各自落在原生 `DependencyLimits::new` 的范围内（`graphs` 可为 0，其余至少 1，并有
  各自上限）。解码器以同一个构造函数检查，不另写一份范围；schema 中的范围由同一组常量生成。
- 请求有字节上限，未知字段被拒绝。没有隐含默认界限：调用方必须明示自己要承担的工作量。

### 3. 结果

结果携带：精确的 `artifact_id` 与 `cursor`、所用 `limits`、图对象标识（图引用与持有它的程序块
`object:`）、**保留角色**的内容与媒体引用、`music_may_stop`、显式的 `truncations`（含界限、
循环、调用深度、未解析引用，以及不可用块的**原始错误与确切对象标识**）、显式的 `unknowns`
（条件、赋值、调用实参、宿主提议未被求值）和 `work` 计数。`complete` 恰为 `truncations`
为空。

媒体角色区分 `background`、`character_layer`（带层 ID）、`music_play`、`music_keep`、`voice`。
Scene 节点整体替换视觉目标，所以它的背景与全部角色层即使与当前相同也会列出；`music_keep`
列出被保留的曲目；Stop 没有引用，只设置 `music_may_stop`。未来的条件、赋值和调用结果始终保守。

权威 Rust 解码器除结构外还校验一致性：`complete` 与 `truncations` 相符；各列表严格有序、
无重复；每项截断记录的界限等于请求的同名界限；`work` 不超过 `limits`；不同引用总数不超过
`references` 且不超过 `candidates`。不一致的结果被拒绝，而不是被信任。Wasm 模块只产出结果，
所以结果解码器只为原生目标编译（Rust 消费者与测试），读者 Wasm 不携带它；请求解码器两边都有。

JSON schema `packages/narrata/nodes/schemas/vn-dependencies-v1.schema.json` 由这些 Rust 类型
生成，包含请求与结果两半；测试比较已提交文件与重新生成的结果。它独立于 `book-view` 与
`vn-view` 的生成链，Web 构建不依赖它。

### 4. 只在拥有它的安全点调用，镜像普通呈现

查询只由 `VnBook` 在其受检会话上调用，使用与 `inspect` 相同的会话与当前构件；会话未打开、被
取代（持久化结果未知后被丢弃）时与其他读取一样失败。游标可以是尚未确认持久化的候选游标；
结果只回显它，不声称它已确认，也不编造一个未来游标。查询不改变游标、状态、历史、访问记录、
导出、存储请求或 ref。

### 5. 查询的 I/O 不得抢占或遗留强制请求

冷块只产生显式的 `unavailable` 截断，不在此处触发取块循环。`HostChunks::load` 在缺块时会记录
一个待处理请求，宿主会把它当作必须取的程序对象。投影把该错误转为截断，因此查询前后恢复当前
构件的那个请求位：此前已有的普通操作请求保持，查询自己造成的请求被丢弃，之后的普通输入不会被
一个可选的前瞻取块先占用。这一机制只触及查询当前构件，不扫描任何历史构件；也不调用
`finish`，因此宿主为其他操作提供的块字节不被清除。宿主可以依据结果里的块标识自行决定是否
取块，再重新查询；块字节仍按既有的 `loadChunk` 约定在下一次普通操作成功后释放。

Web VN reader 为了递归发现冷跨图调用中的依赖，会在同一 artifact/cursor 上做有界后续查询。它把
宿主取回的有界对象集合临时注入当前 Wasm 查询源，借用既有对象信封、ID、manifest 成员关系与图块语义
校验，因此一条冷调用链上的多个图都能在同一查询中展开；查询结束后只移除本次临时注入的对象，保留
其他操作的请求位和已供应字节。集合描述只属于 Wasm 内部调用，依赖请求/结果 v1 不增加字段，游标、
存档和叙事输入都不改变。临时对象在后续查询未能检验前只算 fetched，缺失或坏对象仍以 partial 状态
和确切对象 ID 暴露，只有受检后才可报告为准备完成。

### 6. 构建选择与体积

方法由 `narrata-nodes-wasm` 的 Cargo feature `vn-dependencies` 提供。Web 包继续保留
Gamebook `reader-wasm` 的 `--no-default-features` 构建，并额外构建一个 VN-only reader 模块，
使用相同原生 crate 与 `--features vn-dependencies`。两份 glue 各自初始化，避免一个 VN Wasm
实例挤入 Gamebook 首屏，也避免把可选方法伪装成旧 reader 模块的能力。`openVn` 的自定义
`wasm` 必须来自匹配的依赖功能模块；Gamebook reader 保持原来的模块与尺寸上限。

Web 包分别计量完整 VN-only Wasm、差量与对应 JS glue，聚合 JS 仅计一次；基线和上限由
[`size-budget.json`](../../packages/narrata/web/size-budget.json) 与[构建检查](../development/toolchain.md)记录。
VN reader 的固定安装与首屏
测试必须证明 Gamebook 页面只请求原 reader Wasm；VN 入口拒绝未启用 `vn-dependencies` 的自定义 Wasm。

## 后果与未验证项

- 结果是针对当前游标的快照；游标移动后需重新查询。公开的 TypeScript reader 通过版本化
  schema 检查结果；有界逻辑对象准备把下载字节暂存在宿主，只有普通读者操作请求该对象时才装入
  Wasm。玩家的媒体预备需要宿主显式选择 hint 变体，并且只按不可变引用、角色、版本与当前上下文复用。
- 完整的桌面/触屏 A/B 浏览器旅程、真实账户上下文切换和参考宿主验收仍由 Web VN Goal 后续集成。
- 保守并集可能列出读者永远不会到达的引用；宿主应按角色分级预取，并遵守 ADR 0024 的授权与
  代次约定。
- 大型作品的 `truncations` 与引用数量由请求界限限制；上限变化需要新版本或新 ADR。
- 冻结证据：`fixtures/compat/vn-dependencies-v1/`（由实际 Wasm 入口产生的请求与结果、被拒绝的
  版本与标签、可复现输入与清单）。未来任何格式变化须有新的 ADR、旧格式读取路径和新增语料。

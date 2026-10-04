# ADR 0014：存档引擎与键布局

状态：Accepted。建立在 [ADR 0012](0012-narrow-storage-backend-contract.md) 的后端契约之上，
并修正其中"`SaveStore` 如何映射到原语"一表的三处设计（见文末）。

## 背景

ADR 0012 定义了后端契约，但 `narrata-store` 的领域逻辑仍写在 `MemoryStore` 里，
`SqliteStore` 每次操作整库加载、整库重写；每次提交都校验库中全部对象，每次加载都沿父链走到
创世提交，`redo_candidates` 与 debugger 枚举全部对象。1000 次提交时 SQLite 打开要 20 秒，
10000 次提交的库构建要一个多小时（[存储基准](../development/benchmarks/storage.md)）。

## 决策

### 一个引擎

`narrata_store::Store<B: StorageBackend>` 实现一次 `SaveStore`。`MemoryStore` 是
`Store<MemoryBackend>`，`narrata_store_sqlite::SqliteStore` 是 `Store<SqliteBackend>`；以后的
后端只需实现 `StorageBackend`。`SaveStore` 保留为 `SessionCoordinator`、bundle 与协议引擎的
接口，一次返回全部结果的 `list_*` 改为分页扫描（`scan_refs`、`scan_effects`……），新增
`child_commits` 与 `timeline_commits` 两个索引查询；`list_objects` 删除。

"某个对象引用了哪些对象"只在 `graph.rs` 解码一次：GC 标记、bundle 闭包和写入校验都调用它。
Commit 对 Program 的引用是 `ProgramArtifactId`，由调用方按上下文解析（引擎查程序索引，bundle
查自己携带的对象），所以解码不依赖存储。以后把通用部分搬进 kernel、让各领域注册自己的种类时，
这里就是接缝。

### 键空间

键是字节串，按 ADR 0012 的字典序排列；值是 [ADR 0003](0003-deterministic-cbor-profile.md)
规范 CBOR，读取时严格解码并要求重新编码后逐字节相同。下表中 `‖` 表示拼接，ID 是定长原始
字节，名称是 `RefName` 的 ASCII 字节（不含 `0x00`，所以 `0x00` 可作分隔符）。

| 空间 | 名称 | 键 | 值 |
| ---: | --- | --- | --- |
| 0 | meta | `layout`、`graph`、`sweep` | `layout`：`{0: 1}`（布局版本）；`graph`、`sweep`：`{}`，只用修订号 |
| 1 | touch | 对象 ID（32） | `{0: observed_at}` |
| 2 | refs | 命名空间码（1）‖ owner ‖ `0x00` ‖ name | `{0: commit}` |
| 3 | catalog heads | execution（16） | `{0: event, 1: coverage}` |
| 4 | archives | execution（16）‖ name | `{0: manifest}` |
| 5 | compound saves | owner ‖ `0x00` ‖ slot | `{0: manifest}` |
| 6 | inputs | execution（16）‖ input（16） | `{0: parent, 1: payload, 2: commit}` |
| 7 | pins | owner（UTF-8，不含 `0x00`）‖ `0x00` ‖ 对象 ID（32） | `{0: expires_at 或 null}` |
| 8 | effects | execution（16）‖ effect（32） | 账本条目，见下 |
| 9 | ledger fences | execution（16） | `{0: fence}` |
| 10 | catalog ops | execution（16）‖ operation（16） | `{0: previous 或 null, 1: event}` |
| 11 | programs | artifact（32） | `{0: 对象 ID}` |
| 12 | children | 父提交（32）‖ 子提交（32） | `{}` |
| 13 | commits | execution（16）‖ turn（8，大端）‖ 提交（32） | `{}` |

- coverage 沿用对象 schema 的编码：`[0, baseline]` 或 `[1, baseline, source]`。
- 账本条目：`{0: request_digest, 1: capability, 2: capability_version, 3: origin_commit,
  4: delivery, 5: rewind, 6: status}`。delivery 取 `DeliveryPolicy` 的判别值；rewind 为
  `[0]` Reapply、`[1]` ReuseRecordedResponse、`[2]` Barrier、`[3, capability]` Compensatable；
  status 为 `[0, lease, expires_at]` Claimed、`[1, response, fence]` Completed、
  `[2, response, fence]` Rejected、`[3, diagnostic]` RetryableFailure、
  `[4, diagnostic, fence]` UnknownOutcome、
  `[5, response, original_fence, by_effect, compensation_fence]` Compensated。终态 fence 不为零。
- refs 按命名空间与 owner 前缀扫描；branch 的 owner 是 execution 的十六进制，所以一个时间线的
  分支是一个前缀。children 让 `redo_candidates` 与回执校验只读一个前缀；commits 让 debugger 与
  CLI `timeline log` 按 (turn, 提交) 顺序列出一个时间线的提交。
- 索引（catalog ops、programs、children、commits）由写入对象的同一批次写入，由删除对象的同一
  GC 批次删除。

打开存储时读取 `meta/layout`：版本相符就使用；版本更新则以格式错误拒绝；键缺失时，库为空就
写入标记，否则拒绝。新布局用新版本号，并提供迁移。

### 信任边界与校验

后端不被信任。每次读对象都由 envelope 取得种类与 schema，重新计算摘要并与请求的 ID 比较；
键值严格解码。不一致报告为 `Corrupt`/`CorruptStore`，从不被当作有效数据使用。

写入只校验本批次的新对象及其直接引用：`graph.rs` 给出的每个引用都必须在本批次或库中存在且
种类正确，再加上各种类的语义检查（提交与父提交、回执、快照、程序一致；目录事件的操作 ID
不冲突；归档清单的描述符与对象一致；复合存档与提交一致）。由此保持不变量 **I：库中存在的
对象，其引用的对象都存在**；闭包完整由归纳保证，不再每次校验全部对象。

加载只校验目标提交、它的快照与程序，不沿父链走到创世提交。需要祖先的操作（回退、
`ensure_ancestor`、导出）在访问时读取并校验所访问的部分；丢失的祖先在访问时报告
`MissingObject`。导入 bundle 照旧完整重新校验：闭包必须与清单一致，每个对象都经过上面的写入
校验。引擎不记录"已验证水位"：校验结论只来自本次读取的字节。

### 修订号

`RefRevision` 就是后端的全库修订号：每个写入键的批次分配一个新修订号，删除后重建的键不会
复用旧值（ADR 0012）。修订号只表达 CAS 版本，不进入任何对象、bundle 或清单的字节，冻结语料
不受影响。`RefRevision::initial`、`next` 删除；测试只断言修订号相等或不同，不假设起始值。

### GC：touch 键与两个栅栏

touch 键记录对象的 `observed_at`，写入对象的批次总是刷新它；清扫按 touch 键分页，对不可达且
`observed_at + grace <= now` 的对象删除对象、touch 键（条件为读到的修订号）和派生索引。宽限期内
的对象也是标记的根：它们引用的对象一并保留，所以删除之后宽限期内的对象仍满足下面的不变量 I。

正确性靠两个 meta 键：

- 每个写入对象或根引用（ref、目录头、归档、复合存档、pin、账本条目）的批次无条件写
  `meta/graph`。GC 在标记前读出它的修订号，每个删除批次都以这个修订号做只检查。标记之后有
  任何新引用，删除批次就冲突，GC 重新标记。
- 每个 GC 删除批次无条件写 `meta/sweep`。写入批次在开始读取前读出它的修订号，并在批次中以此
  做只检查：如果读取存在性之后发生过清扫，批次冲突，引擎重读后重新判断。

GC 按拓扑顺序删除：一个对象不晚于它引用的对象被删除，所以每个删除批次之后不变量 I 仍成立。
写者引用一个已有对象时，该对象存在，由 I 它的闭包也存在；`meta/sweep` 的检查保证这一点到批次
提交时仍然成立。多次冲突后 GC 返回 `Busy`。dry-run 只标记与统计。

### 批次与结果未知

一次领域操作是一个批次。超出后端上限的提交（例如深度 10000 的 bundle 导入）先按引用顺序
（被引用者在前）分批写入对象，最后一批写入根引用；中途失败留下的对象未被引用，过了宽限期由
GC 回收。后端报告结果未知（`Io`、`Corrupt`）时，引擎重读本批次涉及的对象与键：对象按要求
存在或已删除、写入的键都等于预期值且带前置条件的键修订号相同、删除的键都不存在，就按成功
返回并带上读到的修订号；否则返回原错误。冲突错误按冲突键所在的键空间转换为
`RefConflict`、`CatalogConflict` 等，当前值直接取自错误；栅栏或索引键的冲突让引擎重读重试。

### schema v2 存档库

`narrata_store_sqlite::open` 拒绝 schema v2 库并提示迁移。迁移是显式的
`narrata_store_sqlite::migrate_v2(source, target)` 与 CLI `narrata store migrate-v2 <source>
<target>`：只读打开旧库，校验全部对象与引用，按上面的布局写入一个新文件；对象字节、
`ObjectId` 与 `inserted_at`（成为 `observed_at`）原样保留，引用、目录、账本、fence、输入与
pin 转写成键，修订号重新分配。旧文件不被修改；目标已存在时拒绝，迁移失败时删除未完成的
目标文件。
选择显式迁移而不是打开时就地改写，是因为就地改写会毁掉唯一一份旧格式证据，失败时也无从回退。

### 冻结语料

- `fixtures/compat/store-sqlite-v2/`：改写之前由 schema v2 适配器生成（提交 `d116af9` 中的
  `emit_store_v2_fixture`），含提交链、Effect 响应、完整记录目录、存档、书签、两个分支、pin、
  输入记录与 fence，附 SHA-256 与预期内容。测试复制后迁移并继续会话。
- `fixtures/compat/store-layout-v1/`：本布局的 SQLite 库与它的键、对象清单。测试保证以后的
  版本能打开它并继续会话。

### 对 ADR 0012 映射表的修正

- **不写边索引。** 边是对象字节加程序索引的纯函数；存一份在后端里会让 GC 依赖不可校验的
  数据——后端少报一条边，GC 就会删掉可达对象。GC 与 bundle 从重新校验过的对象解码边，程序
  索引只是可校验的提示（引用的程序对象读出后核对 artifact）。
- **引用已有对象不再只检查目标的 touch 键。** 只检查不改变 touch 键的修订号：写者先提交时，
  GC 之前做的标记看不到新引用，删除批次仍会成功；即使写者刷新 touch 键，GC 也会删除该对象
  闭包中的其他对象（它们的 touch 键没变）。上面的 `meta/graph`、`meta/sweep` 与拓扑删除顺序
  取代这一设计。
- **GC 不按页跳过冲突。** 冲突意味着标记已过期，GC 重新标记，而不是跳过单个对象。

## 后果

- 单次提交读取固定数量的对象（父提交、父快照、必要时的目录事件），打开读取目标提交与快照，
  都不随历史深度增长；计数包装器测试断言深度 10 与 1000 的读取计数相同、对象扫描为零。
- 写批次多了两个 meta 键操作与每个对象一个 touch 键；提交还写 children 与 commits 索引。
- GC 与并发写入互斥：写入频繁时 GC 可能返回 `Busy`，宿主在空闲时重试。
- 导出 bundle 仍读取并发送目标提交的全部祖先，字节数随深度线性增长；"浅存档"会改变 bundle
  语义，需要单独的 ADR。
- 协议引擎仍用内存后端；宿主批次协议与 IndexedDB 是后续工作。

## 不做

async 后端、PostgreSQL、`RootKeyOnly` 后端、浅存档、协议引擎的存储注入、通用领域注册。

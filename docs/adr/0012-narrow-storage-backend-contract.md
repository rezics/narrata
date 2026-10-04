# ADR 0012：窄存储后端契约

状态：Accepted。本 ADR 只定义后端机制；键空间的布局（哪个键空间存什么、值如何编码）和
`narrata-store` 在契约之上的改写由后续引擎 ADR 决定。

## 背景

[决定 8](../product/decisions.md#8-存储契约保持窄) 要求后端只提供对象、带修订号的引用、原子
批次、有界扫描和能力声明，效果账本、目录、GC、完整性检查与迁移在契约之上只实现一次。现有
`SaveStore`（`crates/narrata-store/src/store.rs`）是约 25 个方法的领域接口，`list_*` 一次返回
全部结果。`SqliteStore` 每次操作都整库加载、整库重写。引用修订号按键从 1 递增，删除后重建会
复用旧值（ABA）。照这个接口，每加一个后端都要重写一遍领域逻辑。

## 决策

### 位置与依赖

- `packages/narrata/kernel/crates/narrata-storage`：契约类型、`StorageBackend` trait、
  `MemoryBackend`；`testing` feature 下另有一致性套件、故障注入包装器和计数包装器。
- `packages/narrata/kernel/crates/narrata-storage-sqlite`：按行读写的 SQLite 后端。

两者都不依赖 `narrata-core` 或 `narrata-store`，节点栈也能使用（ADR 0011）。`narrata-storage`
只依赖 `thiserror`（套件另用可选的 `proptest`），可以编译到 `wasm32-unknown-unknown`，以后的
浏览器后端就建在它之上。

### 同步接口

trait 是同步的。原生库嵌入游戏时、Wasm 在页面内运行时，调用本来就是同步的。如果把 trait 做成
async，账本、目录、GC 等全部领域逻辑以及 FFI/Wasm 边界都得跟着变成 async，内存与 SQLite 这两个
同步后端还要背上执行器依赖。

异步存储（IndexedDB、网络）以后通过"引擎产出批次、宿主落盘后确认"的协议接入：引擎在同步后端
（通常是内存）上完成读取和前置条件判断，再把成功的批次交给宿主异步落盘，宿主确认后才发布结果。
这与 ADR 0011 "IndexedDB 事务完成后才发布 view" 的规则一致。代价是异步后端不能直接实现这个
trait，需要上述协议，留给后续任务。

### 两类数据

- **不可变对象**：32 字节摘要 → 字节。摘要由调用方给出，后端不计算也不信任。写入采用
  put-if-absent：摘要已存在就跳过，不比较字节。读取后由引擎重新校验摘要和信封
  （`CheckedObject` 已经这样做）。后端不保存对象的元数据（种类、时间戳）；引擎需要的元数据写进
  键里。
- **可变键**：`(KeySpace(u16), 键字节) → (值字节, Revision)`，按键字节的字典序排列（逐字节比较，
  一个键排在以它为前缀的键之前）。引用与引擎维护的索引是同一种键，只靠批次中每个操作的前置条件
  区分：引用 CAS 就是带 `Revision` 前置条件的写入，无条件写入就是索引。这样后端只有一张键表、
  一种扫描和一种冲突错误。

### 修订号

修订号是全库单调的版本号。每个至少写入一个键的成功批次都分配一个新修订号，大于这个库以前发出
的所有修订号；批次写入的键都带这个修订号。删除后重建的键得到新修订号，所以按旧修订号做的 CAS
一定失败，不会发生 ABA。修订号计数器单独持久化：如果取现存键的最大值，删除最新的键之后就会复用
它。契约只保证严格递增、不复用，不保证连续，一致性套件也按这个标准检查。

现有 `RefRevision` 的"每键从 1 递增"语义将在引擎任务中改用这里的修订号。修订号不进入任何导出
格式（bundle、manifest），所以这一改动不触及冻结语料。

### 原子批次

一个批次包含三类操作：对象写入、对象删除（只供 GC 使用）和键操作。键操作可以是写入、删除或
只检查，每个都带一个前置条件：任意、不存在、或等于某个修订号。只检查的操作必须带后两种条件
之一。

- 同一批次里，同一个键或同一个对象摘要最多出现一次，否则整批作为无效请求拒绝。因此所有前置条件
  都针对批次之前的状态判断，结果与操作顺序无关。
- 任一前置条件不成立，整批不生效。错误给出第一个失败操作的序号、键，以及该键当前的值和修订号，
  引擎不必再读一次就能报告冲突。
- 没有跨调用的读事务。引擎采用乐观并发：读取，判断，带前置条件写入；冲突后重读、重新判断。
  如果判断依赖某个键但批次并不写它，就用"只检查"操作把这个依赖放进同一批次。

### 有界读取

- 按摘要批量读取对象。
- 读取单个键。
- 按键空间和前缀做升序扫描，有上限，游标是上一页的最后一个键。
- 对象只能按摘要分页扫描，只返回摘要，供 GC 清扫和完整性检查使用。

契约里没有一次返回全部结果的枚举，也没有逆序扫描。游标就是键本身，因此在不同后端之间、重新
打开之后都有效。

### 能力声明

- **写入模型**：
  - `AtomicConcurrent`：批次全成或全败，多个句柄可以并发写入，由后端串行化。
  - `AtomicSingleWriter`：批次全成或全败，只能单写者。
  - `RootKeyOnly`：对象逐个落盘，每批最多一个键操作，这个操作是原子的；只能单写者。

  多键原子性和并发写者用同一个枚举表达，"不原子却允许并发写"这种组合无法写出来。`RootKeyOnly`
  留给以后的文件目录和对象存储后端：引擎先写入全部对象，最后对一个根键做 CAS；失败时留下的
  对象没有被引用，由 GC 回收。本 ADR 不实现这类后端。
- **持久性**：
  - `Durable`：成功返回后能经受进程崩溃，前提是文件系统真正执行了 fsync。
  - `Evictable`：跨重启保留，但平台可能把整个库删掉，例如浏览器存储。
  - `Memory`：最后一个句柄释放时丢失。
- **上限**：单个键的字节数、单个值的字节数（对象与键值共用）、单批操作数、单批字节数，以及
  单次读取的条目数（扫描页与批量读取共用）。超限的请求在执行前就被拒绝，不会部分写入。

有序扫描不是可选能力，每个后端都必须提供：文件目录可以对文件名排序，对象存储的列举本身就是
字典序的。

### 错误与"结果未知"

| 错误 | 批次是否已生效 | 引擎的处理 |
| --- | --- | --- |
| `Conflict` | 否 | 重读后重新判断 |
| `Limit`、`Invalid` | 否 | 拆分批次或修正调用；原样重试没有意义 |
| `Busy` | 否 | 退避后重试 |
| `Full` | 否 | 报告给宿主 |
| `Format` | 否（打开时） | 换用对应版本的读取器或迁移 |
| `Io`、`Corrupt` | 未知 | 重读相关的键和对象，按实际状态继续，不盲目重试 |

`StorageError::outcome_unknown()` 把这张表写成代码。SQLite 后端在 COMMIT 之前发生的 I/O 错误
其实没有生效，但仍报告为未知：保守的分类最多让引擎多重读一次。

### SQLite 后端

```sql
PRAGMA application_id = 0x4E525354;  -- "NRST"：narrata-storage 文件
PRAGMA user_version = 1;             -- schema 版本
CREATE TABLE meta (
  name TEXT PRIMARY KEY NOT NULL,
  value INTEGER NOT NULL
) STRICT, WITHOUT ROWID;              -- ('last_revision', n)
CREATE TABLE objects (
  digest BLOB PRIMARY KEY NOT NULL CHECK (length(digest) = 32),
  bytes BLOB NOT NULL
) STRICT;
CREATE TABLE keys (
  space INTEGER NOT NULL CHECK (space BETWEEN 0 AND 65535),
  key BLOB NOT NULL,
  value BLOB NOT NULL,
  revision INTEGER NOT NULL CHECK (revision > 0),
  PRIMARY KEY (space, key)
) STRICT, WITHOUT ROWID;
```

- 对象可能很大，用 rowid 表；键大多很小，而且主要靠范围扫描访问，用 `WITHOUT ROWID` 让行按
  `(space, key)` 聚簇存放。SQLite 的 BLOB 比较与契约规定的字典序一致。
- 每个批次一个 `BEGIN IMMEDIATE` 事务：按顺序检查前置条件，然后分配修订号、写对象、写键，
  最后 COMMIT。连接使用 WAL、`synchronous=FULL` 和 5 秒 busy timeout。
- 后端声明 `AtomicConcurrent` 与 `Durable`：多个连接（包括多个进程）可以同时打开同一个文件。
- 读键、读对象都是主键查找；扫描是索引上的范围查询，游标转换成下界，前缀转换成上界。单次操作
  的代价因此随页大小增长，而不随库的大小增长。`benches/` 在 1K 与 100K 规模上验证这一点。
- 打开文件时：空库就初始化；文件标识和版本都相符就使用；版本更新的库、以及其他文件（包括
  `narrata-store-sqlite` 的 schema v2 存档库）一律以 `Format` 错误拒绝，不会误读。初始化在
  `BEGIN IMMEDIATE` 内进行，两个进程同时打开同一个新文件也不会冲突。

### 兼容与迁移

这是一个新文件格式，没有改动任何现有格式；`narrata-store-sqlite` 的 schema v2 库继续由原适配器
读写。键的布局由下一个引擎 ADR 决定，所以现在的 SQLite 文件还不是存档格式。引擎 ADR 确定布局
之后，再为"引擎布局加本表结构"建立冻结语料，并提供从 schema v2 库到新库的迁移：对象的字节和
`ObjectId` 原样复制，引用和索引转写成键。

### `SaveStore` 如何映射到原语

这张表是引擎任务的设计输入。表中 `refs/`、`touch/` 之类的名字只表示"某个键空间"，具体布局由
引擎 ADR 决定。"读修订"指先读出键的值与修订号 `r`，再在批次中使用 `Revision(r)` 条件。

| `SaveStore` 操作 | 原语 |
| --- | --- |
| `get_object` | 批量读对象；引擎按期望的种类与 schema 重新校验信封和摘要 |
| `list_objects` | 删除。GC 与完整性检查用对象分页扫描加批量读取；领域查询改用索引键 |
| `read_ref`、`read_catalog_head`、`read_timeline_archive`、`read_compound_save` | 读单个键；值中编码目标 ID（目录头另含 coverage），修订号就是 CAS 用的修订 |
| `list_refs` 等 `list_*` | 按所有者或命名空间前缀做游标扫描 |
| `read_input` | 读 `inputs/(execution, input)` |
| `list_pins` | 扫描 `pins/` |
| `read_effect`、`list_effects(execution)` | 读 `effects/(execution, effect)`；按 `execution` 前缀扫描 |
| `current_ledger_fence` | 读 `fences/execution`；不存在即零 |
| `claim_effect` | 读条目；按租约规则判断。返回 `Leased` 时不写；否则写入条目，条件为 `Absent` 或 `Revision(r)`，同时检查 `origin_commit` 的 touch 键（见下文 GC）。冲突则重读、重判 |
| `renew_effect_lease` | 读条目，写入新到期时间，条件为 `Revision(r)` |
| `record_effect_outcome` | 读条目与 fence；一个批次内写入响应对象及其 touch 键，写条目（`Revision(r)`），写 fence（`Revision` 或 `Absent`） |
| `mark_effect_compensated` | 读两个条目；写原条目（`Revision`），对补偿条目做只检查（`Revision`） |
| `commit(CommitTransaction)` | 一个批次。对象：写入对象，并无条件写入 touch 键（值为 `observed_at`）。引用、目录头、归档、复合存档的变更：`expected` 为 `None` 时条件为 `Absent`，为 `Some(r)` 时为 `Revision(r)`；目标不在本批次中时，对目标的 touch 键做只检查。输入：先读，相同则只检查，不存在则以 `Absent` 写入，不同则在提交前报告 `InputIdConflict`。pin：无条件写入或删除 `pins/(owner, object)`。冲突错误按冲突键所在的键空间转换为 `RefConflict`、`CatalogConflict` 等，当前值直接取自错误 |
| `validate_all`（每次提交时校验全部对象） | 改为只校验新对象及其直接引用；全量校验交给完整性扫描和导入 |
| `find_program`（遍历全部对象） | 写入 Program 对象时同时写索引键 `programs/artifact → ObjectId` |
| 目录操作幂等索引（`TimelineOperationId`） | 键 `catalog-ops/(execution, operation)`，处理方式同输入 |
| SQLite 的 `object_edges` | 在提交批次中写入边索引键 `edges/(source, target)`，取代读取时解码全部 Program |
| `collect(RetentionPolicy)` | 标记：分页扫描各个根键空间（引用、目录头、归档、未过期的 pin、账本、复合存档），从根出发批量读对象、解码出边。清扫：分页扫描 `touch/`；对不可达且 `observed_at + grace <= now` 的对象，按上限分批写入"删除对象 + 删除 touch 键（`Revision(r)`）"。冲突说明对象刚被重新引用或刷新，跳过即可。dry-run 只做标记和统计 |
| `integrity_scan` | 对象分页扫描加批量读取，逐个重新校验 |
| `CheckpointBundle::import` | 对象按上限分成若干批写入（幂等，带 touch 键），最后一批写入引用；中途失败留下的对象未被引用，过了宽限期由 GC 回收 |
| `FaultPoint` 注入 | 由 `testing::FaultInjecting` 包装器在调用前或调用后注入 |

GC 的宽限期依赖 touch 键：写入对象的批次总是刷新它的 touch 键，而 GC 删除对象时以 touch 键的
修订号为条件。假设 GC 标记之后有写者重新引用了这个对象：如果写者先提交，touch 键的修订号变了，
GC 的删除批次冲突，对象保留；如果 GC 先提交，写者批次中的对象写入会把对象重新写回，引用已有
对象的写者则会因为 touch 键的只检查失败而得知对象已被删除。现有 `inserted_at` 在对象已存在时
保留旧值；touch 键改为刷新，这样更安全。单写者后端（`RootKeyOnly`）没有并发写者，不需要这一套
条件。它们的 GC 宽限期怎么做，留到实现这类后端时再定。

## 不做

- async trait；跨调用的读事务或快照；范围冲突检测（前置条件只针对单个键）；逆序扫描。
- 后端计算或校验摘要；对象元数据；TTL 与过期；变更通知。
- 领域键布局与 `narrata-store`、`narrata-store-sqlite`、协议引擎的改写。
- IndexedDB、OPFS、PostgreSQL、JSON 目录和对象存储后端；`RootKeyOnly` 的实现。
- 新 SQLite 格式的冻结语料，等引擎 ADR 确定键布局后再建立。

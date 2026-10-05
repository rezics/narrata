# ADR 0021：服务器存档与按库维护

状态：Proposed（2026-10-05）。落实[决定 15](../product/decisions.md#15-服务器存档实现同一个存储契约)，
沿用 [ADR 0012](0012-narrow-storage-backend-contract.md)、[ADR 0017](0017-browser-storage-host-cache.md)
的契约与协议版本 1；对象、键、布局版本 1 和现有冻结语料均不改。

## 背景

登录读者的进度必须跨设备保留，又不能每次选择都上传整个 checkpoint。同步存储契约已经由
`CacheBackend` 转成异步加载与落盘消息；TypeScript 的 `StorageHost` 接受
`CacheStore.load()` / `persist()`，并在落盘确认之后才返回操作结果。网络宿主实现这个现有接口，
运行时不连接 PostgreSQL，不增加另一套存储抽象。正文与选项文字仍由宿主解析，不进存档。

## 决策与理由

### 拓扑与账号生命周期

登录时浏览器直接以服务器为宿主，服务器库是唯一权威。IndexedDB 保存匿名读者的本地库与
登录读者最后确认的离线 checkpoint；两者使用独立库标识与修订号，不镜像服务器的计数器。
不后台同步 IndexedDB 的原始 `Flush`：本地修订号、前置条件与服务器修订号不是同一条序列。

- 在线操作由 `StorageHost.run()` 驱动服务器 `CacheStore`，收到持久化确认才发布选择后的 view
  与“已保存”。只把已确认的根经 Narrata 受检导出、导入到离线 IndexedDB 库；本地缓存失败不
  撤销服务器的成功确认。缓存可落后、可被浏览器清除，不能反过来覆盖服务器。
- 离线可阅读本机最后确认的 checkpoint、已缓存构件与宿主允许离线使用的内容；缺内容时停止。
  第一阶段禁用推进、检出、存档删除等修改操作，不把离线点击显示为已保存；匿名本地阅读仍可
  正常推进。选择这个边界是为了复用现有确认语义，避免增加未实现的输入队列与跨库同步格式。
- 首次登录不把匿名库更名为用户库。宿主先列出本机旅程与服务器旅程；读者选择要合入的本机根，
  Narrata 校验构件、状态与 bundle 后，导入为新的执行名/分支，已有服务器根不动。重试沿用同一
  导入目标并核对目标提交，避免重复旅程；完成确认前保留匿名原库。不同构件不能自动合并。
  每个分支、存档槽分别选择与导入，不能把当前 `NodeBook.export()` 当成整库合并。
- 切换账号或登出先停止操作、使当前会话代次失效，再释放 Wasm/缓存与网络句柄，关闭并清除该
  账号的离线库。迟到的回复只能属于原账号，不能更新新账号的 view；取消请求不证明事务回滚。
  匿名库不自动归给下一个账号，登出不删除服务器存档。
- 删除一条旅程由 Narrata 用 CAS 删除它的游标、分支与相关保活根，随后 GC；删除“此作品全部
  存档”由宿主事务撤销库句柄并删除库及关联行，包括维护副本，再清除本地缓存。删除要求当前
  库标识与修订号，不是无条件按作品删除；并发写入时先报告冲突。重建生成新标识与新句柄，
  旧请求不得自动建库或使已删除的存档复活。其他离线设备只能在下次联网时得知删除。

离线库是完整的浅 checkpoint 库，使用现有 IndexedDB 布局和 [ADR 0019](0019-shallow-history-bundles.md)
容器；它不是把缺少条目的服务器库伪装成完整 IndexedDB 库。宿主按账号与作品隔离缓存名称，
每次用新空库构建 checkpoint，确认后换缓存指针并删除旧副本，避免反复导入保活全部历史。
内容缓存及离线授权属于宿主。以上账号与删除动作是宿主生命周期操作，不是新存储消息。

### 传输、寻址与上限

宿主把“认证用户 × 作品”解析为库，再给浏览器不透明、绑定该库实例的句柄。每次请求都重新检查
身份、作品访问权与句柄归属；Wasm 只见 `StoreId` 与协议字节，不见用户 ID、SQL 表或认证令牌。
库不存在、已删除或无权限时不泄露归属，也不凭客户端给出的用户字段寻址。

参考传输是宿主的 HTTPS：两个 POST 操作 `load`、`persist`，请求与成功回复均为
`application/cbor`，字节原样承载 ADR 0017 的四类消息。认证、句柄、超时与错误在宿主传输层；
无新 CBOR 外壳、字段、消息或协议版本。加载缺条目返回 null；权限错误、超限、数据库失败不能
伪装成空库或持久化回复。迁移后的新句柄由宿主重新发放，旧句柄不取得新库写权限。

宿主必须在解码分配及事务写入前检查上限，不能信任浏览器已经检查过：

| 检查 | 第一阶段上限与计数 |
| --- | --- |
| 契约 `Limits::DEFAULT` | 原始键 1,024 B；对象/键值 16 MiB；单批 10,000 操作、64 MiB；单次读取/扫描页 1,024 条 |
| 存储键 | `space(u16 大端) ‖ key`，写入键长 2–1,026 B；区间允许空下界，其他边界至多 1,026 B |
| 完整网络落盘 | 全部批次累计至多 10,000 操作、64 MiB；每批另查契约上限；计数与 `Batch::validate()` 一致，摘要与 CBOR framing 不计入契约字节数 |
| 完整网络加载 | 点读数量加各区间请求的 limit 总和至多 1,024；回复值字节总和至多 64 MiB |
| 传输与解码 | 请求/回复各至多 128 MiB（包含 framing），长度、深度、条目数在分配前有界；超限整次拒绝，不截短扫描结果 |
| 库配额 | 宿主限制总字节、对象数、键数与维护副本预算，锁库后核对新增净量；Narrata 限制保活根数；全库修订号至多 `2^53 − 1` |

契约数值复用当前缓存默认能力，不引入能力协商；应用需要更小上限时同时配置已有
`CacheBackend::with_limits()` 与宿主部署。聚合上限是传输的额外边界：大的导入/维护任务必须在
Narrata 侧分成有界操作，对象先落盘、最终根最后 CAS，不能将一个 `Flush` 偷拆成部分成功。
配额拒绝整个事务，已确认的进度不变；失败中途留下的无根对象经宽限期回收。

消息仍用现有 `decodeLoadRequest()`、`decodeFlush()`、`encodeLoaded()`、`encodeFlushReply()`
及 Rust 受检解码；网络上限检查不能仅依赖当前通用 CBOR 解码器的宽松分配上限。对象摘要与
领域有效性仍由 Narrata 每次读取时核对，SQL 后端不解析对象与键值的内容。

### PostgreSQL 参考布局与事务

以下是由宿主手写编号 SQL 创建的三张表，所有表都有主键。宿主自行把 `owner_key`、`work_key`
映射为自身身份；`active`、`expires_at` 只是路由与副本生命周期，不是 Narrata 键布局版本。
同一用户作品只有一个活跃库，临时目标与回退证据仍是这三张表内的非活跃库实例。

```sql
CREATE TABLE narrata_libraries (
  id bytea PRIMARY KEY CHECK (octet_length(id) = 16),
  owner_key text COLLATE "C" NOT NULL,
  work_key text COLLATE "C" NOT NULL,
  active boolean NOT NULL DEFAULT true,
  expires_at timestamptz,
  revision bigint NOT NULL DEFAULT 0
    CHECK (revision BETWEEN 0 AND 9007199254740991),
  CHECK (active OR expires_at IS NOT NULL)
);
CREATE UNIQUE INDEX narrata_active_library
  ON narrata_libraries (owner_key, work_key) WHERE active;
CREATE INDEX narrata_library_owner ON narrata_libraries (owner_key, work_key);
CREATE INDEX narrata_library_expiry ON narrata_libraries (expires_at) WHERE NOT active;
CREATE TABLE narrata_objects (
  library_id bytea NOT NULL REFERENCES narrata_libraries(id) ON DELETE CASCADE,
  digest bytea NOT NULL CHECK (octet_length(digest) = 32),
  bytes bytea NOT NULL CHECK (octet_length(bytes) <= 16777216),
  PRIMARY KEY (library_id, digest)
);
CREATE TABLE narrata_keys (
  library_id bytea NOT NULL REFERENCES narrata_libraries(id) ON DELETE CASCADE,
  key bytea NOT NULL CHECK (octet_length(key) BETWEEN 2 AND 1026),
  value bytea NOT NULL CHECK (octet_length(value) <= 16777216),
  revision bigint NOT NULL CHECK (revision BETWEEN 1 AND 9007199254740991),
  PRIMARY KEY (library_id, key)
);
```

对象只在一个库内去重，不跨用户共享。按主键范围读取：`library_id = $1 AND key >= $2 AND
key < $3 ORDER BY key LIMIT $4`，无上界时省略上界条件；摘要扫描同样做。使用参数化 `bytea`，
不转换成文本、JSONB 或 locale 排序。PostgreSQL 的
[byteacmp](https://doxygen.postgresql.org/bytea_8c.html) 先用 `memcmp` 比较共同长度的无符号字节，
相等时短串在前，因此与 ADR 0012 逐字节、前缀在前的字典序相同。两字节大端空间号的顺序也
相同；`00`、`7f`、`80`、`ff` 与前缀边界必须进入排序测试。

一次 `persist` 使用一个 READ COMMITTED 写事务：

1. 解码并检查全部批次连续（`revision = base + 1`）、列表升序/不重复/互斥及全部上限。
2. 按认证后的库映射 `SELECT ... FOR UPDATE` 锁库行，再核对它仍可写、`id` 与第一批的 `base`。
   不相符则不写任何对象/键，回复协议 Conflict 的当前 `id/revision`；删除/撤权返回宿主错误。
3. 逐批对象 put 用 `INSERT ... ON CONFLICT (library_id, digest) DO NOTHING`，绝不覆盖现存字节；
   对象删除、键 upsert/删除按消息执行，键 upsert 带该批的修订号。全库 CAS 代替逐键 Expect：
   消息是缓存已经检查后的效果，不包含原始前置条件，服务器不能凭空还原它们。
4. 在同一事务内核对最终配额并将库修订号设为最后一批的 revision；COMMIT 成功后才发 Persisted。
   任一 SQL/配额错误回滚全部消息。所有写入口（含维护、删除与切换）使用同一库行锁约定。

加载请求在一个 REPEATABLE READ、只读事务中读取路由、库标识/修订号与全部请求条目，保证一个
回复只有一个数据库快照。默认 READ COMMITTED 的多条 SELECT 各取不同快照，不能直接使用
（[PostgreSQL 隔离说明](https://www.postgresql.org/docs/current/transaction-iso.html)）。路由变更后
重发的请求取得新状态，旧快照上的写入仍会被库标识/修订号拒绝。正常读写不做跨调用读事务，
不用全表锁；超时与连接池属于宿主。

PostgreSQL `bigint` 足够保存上述范围；Node 驱动返回字符串或 bigint 时，经过整数与范围
检查后才能转为 JS number，不能直接 `Number()` 断言安全。库修订号 0 只表示新空库，键修订号
必须大于 0 且不大于库修订号。对象变动或键删除也沿用 v1 的批次加一，不改成取当前最大键值。
到达上限前换新库实例，不回绕、不在同一个 ID 下重置。持久化配置必须保证提交的 WAL 经 fsync
落盘；读写走权威库，不从可能落后的副本确认或加载。

当前 `IndexedDbStore.persist()` 使用 `objects.put()`，参考 `MemoryHost.persist()` 使用
`insert()`，面对直接发送的同摘要不同字节消息会覆盖；正常缓存过滤已存在对象，掩盖了这个差异。
实现前把两者收紧为契约的 put-if-absent，并在一致性套件中固定行为，不将这个差异带到服务器。

### 多设备冲突与结果未知

两个设备从同一 `(id, revision)` 写入，只有先锁库并提交的一方成功；另一方得到 Conflict，
`CacheBackend::confirm()` 丢弃缓存与未确认批次，JS 报 `StoreSuperseded`。宿主调用
`NodeBook.reload()` 再 `open()`，重读服务器会话；不把较新的状态按时间戳强行覆盖另一条旅程。
此前已确认的提交仍在分支历史中，重载只改变当前 view，不删除它们。

宿主在调用 `run()` 前保留未确认动作的构件、显示时的 expected commit、选择点与选项 ID，
并由 Narrata 为这个确切 expected commit 准备 N=0 浅 checkpoint，作为本次动作的内存恢复证据；
不能拿落后的离线缓存冒充这个基点。成功执行后、落盘前另保留候选提交 ID：节点维护任务将
现有 Rust `Session::cursor()` 的结果交给 Wasm 宿主，不额外读取 view，也不提前公开为已保存。
冲突后显示服务器已保存的位置与本次未保存的选择。读者可以放弃、在原提交上重新选择并保存为
独立旅程，或回到服务器位置再作选择；不能将旧动作无提示地套用到新状态。重放必须由 Narrata
受检执行并比较状态/提交摘要；原提交不可用时用最后确认的浅 checkpoint，显示历史截断，不
声称跨边界重放已验证。宿主只串行一个未确认动作，页面关闭可丢弃它，但不能显示成已保存。

网络断开或回复丢失是“结果未知”，不是确定失败。暂停后续操作与缓存加载，重传完全相同的
`Flush`（不重跑选择、不改 base）；全库 CAS 保证重复请求最多一次生效，但重传可能得到 Conflict，
即使第一遍已提交。不能用“服务器修订号已超过目标”推断成功，或把 Conflict 说成“肯定未保存”。
收到冲突后保留动作描述，再失效/重载；用已校验的目标提交与持久化根/分支路径核对是否已记录，
仅有同摘要对象存在不足以证明根已提交。无法证实时保留“结果待核实”并提供独立旅程恢复，不
自动覆盖服务器。这复用 v1 的安全失败语义，不承诺精确一次回执；宿主既有不可变命令回执若能
与此事务原子提交，可以帮助核实，但不是新增 Narrata 表或协议字段的前提。

### 体积、保留与 GC

维护在服务器 Node 进程运行同一 Narrata Wasm，通过 `CacheStore` 与 v1 协议读写 PostgreSQL。
维护者注册该库的全部领域/对象种类，用现有 `History::collect(RetentionPolicy)` 与两个 GC 栅栏；
SQL 只执行批次，不另写一套“找无引用对象”的 GC。冲突/Busy 后重新标记并退避，未知种类先
报错，任何删除都不发生。定时任务使用宿主可信时钟给出有限 grace，不用客户端时间决定删除。

`RetentionPolicy` 目前只含 now、grace_seconds、dry_run，不能限制可达祖先深度；仅把一个浅
bundle 导入已有完整库也不会截断，因为真实 parent 优先于截断记录。故有界保留分两部分：

- 根数量与总字节设硬配额；分支/存档槽由读者明确删除，不能自动丢掉未合并的已确认路线。
  无根导入残留和删除后的不可达闭包由常规 GC 回收。无法腾出空间时拒绝新写，保留现有进度。
- 需要限制历史窗口时，按已向读者说明的有限 N，为每个保留根用 `History::export_shallow()`
  导出；在新空库用 `History::import()` 受检导入，重建所保留的游标、分支、存档槽与 pin。
  使用新 ID，在下节的全库 CAS 切换之后只保留各根 N 范围的并集（R 个根至多 R×(N+1) 个提交）；
  原提交字节/ID 不改。读者看见
  “更早历史已归档/截断”，浅库访问边界得到 `HistoryTruncated`，不能伪装成完整导出。

宿主设定并展示 N、根数/字节配额 Q、旧库保留期 W；活跃库、一个构建目标与一个回退库各有
有限预算，同一用户作品同时只有一个维护作业。每次切换前清理已过期副本，预算不足时不切换；
旧证据不无限累积。构建中源库增长也受 Q 约束，强制配额使维护失败或繁忙时仍保持有界。
含 Effect 账本、永久 pin 或其他注册根的库不能只复制游标：全部根/不可丢的终态证据必须保留，
不支持安全压缩的领域拒绝压缩，不以 quota 为由删除它的证据。

### 按库布局迁移与原子切换

`meta/layout`（空间 0 的 `layout`）是唯一逻辑布局版本，宿主表不增加按 Narrata 发布号的 SQL
迁移。当前 `History::open()` 只接受布局 1；已有 artifact 迁移不是键布局迁移器。第一阶段保持
[决定 17](../product/decisions.md#17-第一阶段一个平台运行时阶段内版本保持兼容) 的旧格式读取，
此次定机制，布局迁移代码可等第一次布局变化时实现。未知更新版本、非空无布局标记均失败关闭。

宿主打开库时用部署的 Narrata 维护入口检查布局：相符直接打开，落后则启动该库作业；浏览器
显示升级中并等完成再打开，不活跃库不付成本。逐版本转换和旧解码器归 Narrata，不让宿主 ORM
理解键。第一次新布局必须另有 ADR、受检迁移器、旧读取路径与新冻结语料。

选择影子库迁移，不就地改写唯一旧证据。步骤如下，也供浅历史压缩与修订号换库复用：

1. 记录源 `(id, revision, layout)`，锁源核对仍活跃后分配有到期时间的新非活跃库，删除后的源
   不能再创建目标。旧库保持在线，迁移通过旧版本读取器经协议分页读取，验证对象与键后转换，
   分批写目标，对象先于依赖它的根；完成时核对目标 `meta/layout`，构建中不向读者暴露目标。
2. 每次加载都要求源状态等于起点，不能拼合不同修订号的分页。作业检查点由宿主既有任务设施
   保存：源/目标 ID、版本、源 revision、阶段、扫描游标、最后确认的目标 revision。页只有在
   目标事务确认后才推进；崩溃在确认与检查点之间时重读目标并幂等重做该页，不猜测已写入。
   恢复先核对两个库状态，源改变就丢弃构建目标并重新开始；没有跨调用的快照承诺。
3. Narrata 完成目标完整性、保留根与迁移前后状态/摘要检查。宿主用短事务锁源和目标（按 ID
   固定顺序），再 CAS 源 ID、revision、活跃状态与目标已校验 revision；全相符才将源改为
   非活跃并设 W 到期，目标改为活跃。源写者先提交则 CAS 失败、重新构建；切换先提交则旧写者
   被不可写状态拒绝并重新取得句柄。目标迁移期间只授权维护者，普通客户端不能写。
4. 崩溃在切换前仍路由旧库，切换提交后路由新库；两个状态不能部分提交。旧库只读保留完整的
   旧键、对象与检查清单供回退。切换后已有新确认写入时，不能直接路由回旧库：先暂停写入并用
   兼容读取器保存/转换新增旅程，或维持新库只读；未经验证的逆迁移不得覆盖新进度。

反复 CAS 失败返回 Busy 并在空闲时重试；要取得进展可由宿主对该库短暂暂停新写并明示维护。
构建失败不触动源库；孤立目标到期回收。作业检查点不是存档交换格式，其持久化由宿主任务
设施管理，第一次迁移实现时把断点状态与迁移效果纳入版本测试。

### 验收与冻结证据

新增 TypeScript 协议宿主一致性套件，工厂返回现有 `CacheStore` 加建库、重开、清除测试夹具，
分别运行 TS 内存参考宿主、`IndexedDbStore`（fake-indexeddb 与真实 Chromium）及 PostgreSQL
协议宿主。内存夹具不是产品新后端。沿用现有 `test/protocol.test.ts`、`indexeddb.test.ts`、
`host.test.ts` 的风格；Rust `MemoryHost` 与 CacheBackend 测试继续校验相同协议。

共同断言：逐字节排序与分页/空区间、同一起始库状态与消息产生相同结果、对象 put-if-absent、缺条目 null、
键/库修订号与删除重建 ABA、两写者同 base 只成功一个、整条多批 Flush 原子回滚、只在提交后
确认、库替换后的旧请求失败、坏 CBOR/列表/摘要长度/数值/配额全部拒绝、重开保留状态。
端到端断言另外覆盖账号切换迟到回复、匿名分支合入重试、冲突动作保留、回复丢失重传与核实、
删除防复活、浅库继续推进/截断、GC 栅栏、迁移逐页崩溃恢复与 CAS 切换，以及旧库回退不丢新写。

本机无 Docker，以 PGlite 运行同一参数化 SQL、DDL、排序、约束、事务中途故障回滚和顺序 CAS
案例；[PGlite transaction](https://pglite.dev/docs/api) 可验证拒绝时回滚。
[PGlite 是单独占连接](https://pglite.dev/docs/)，因此它不能证明真实多连接锁竞争、MVCC
隔离、WAL 崩溃持久性、权限与部署行为。上线前必须在 CI PostgreSQL 服务，或宿主提供的独立
测试库上跑同一套件，再加双连接屏障竞争、加载与写入交错、迁移与删除竞争、提交后进程终止
重开；不能用 PGlite 的排队调用冒充并发验证。未配置实库时明确报跳过，上线门槛不算通过。

协议没有新版本：实现读取只读 `fixtures/compat/browser-storage-v1/protocol-vectors.json`，
对接旧客户端；新增 `fixtures/compat/server-storage-v1/` 固定规范 SQL 逻辑行清单、v1 请求回复、
账号隔离/冲突/删除场景摘要，而非依赖某个 Postgres 大版本的物理文件。同一生成器两遍摘要
相同，新实现从逻辑行重建并继续读写。浅导入继续读取 `kernel-history-v1/`、`kernel-history-v2/`；
首次布局变化另冻结旧库→新库与中断断点向量，不改任何旧语料。

### 实现任务与 REZICS 能力请求

下表均为后续任务，路径是拟认领范围；新工具/检查在实施时同步记录到工具链，不在本 ADR
引入依赖。检查均经 `task goal -- slot`，gate/浏览器/实库故障套件加 `--heavy`。

| 任务 | 路径与依赖 | 检查与证据 |
| --- | --- | --- |
| 宿主一致性与边界收紧 | `packages/narrata/kernel/js/{src,test}/`、`packages/narrata/kernel/crates/narrata-storage-host/{src,tests}/`；无依赖 | `task check:web-storage`、受影响 Rust crate 的 test/clippy；v1 旧向量，新增重复对象/上限/回滚向量 |
| PostgreSQL 协议参考宿主 | `packages/narrata/kernel/js/server/`、`test/`、包清单、`scripts/check-server-storage.ps1`、`Taskfile.yml`、`docs/development/toolchain.md`；依赖一致性套件 | 新 `task check:server-storage`：类型检查、PGlite 与实库套件；新 `server-storage-v1/`，SQL 是参考，不自动导入宿主迁移 |
| 网络与阅读器生命周期 | `packages/narrata/kernel/js/src/`、`examples/gamebook-web/{src,tests}/`；依赖参考宿主、节点存档入口与可用宿主认证路由 | `task check:web-storage`、`task check:r1`；两设备、断网、未知结果、登录/切换/删除浏览器回归，不改协议向量 |
| 节点存档与维护入口 | `packages/narrata/nodes/crates/narrata-nodes/{src,tests}/`、`narrata-nodes-wasm/src/`、`packages/narrata/kernel/js/server/`；依赖参考宿主 | `task check:r1`、`task check:server-storage`；暴露现有 kernel 浅导出/受检导入与完整根保留，修正节点层对截断历史的处理；沿用 history v1/v2 并新增多根维护场景 |
| 首次布局迁移 | 首次新布局 ADR、`packages/narrata/kernel/crates/narrata-history/{src,tests}/`、维护入口与新 `fixtures/compat/` 目录；依赖该布局设计与维护入口 | crate test/clippy、`task check:r1`、`task check:server-storage`、`task docs:check`；旧读取、迁移摘要、页断点、CAS/回退与坏输入向量；第一阶段可推迟 |
| REZICS 接入请求 | `docs/integrations/rezics.md`；依赖本 ADR 评审，由 manager 合入 | `task docs:check`；不修改任何 REZICS 仓库 |

提交给 manager 的 R6 替换行：

> R6：读者存档：每个“用户 × 作品”一个不透明库，内容 PostgreSQL 的库/对象/键三表实现
> ADR 0012，宿主认证后的网络传输承载 ADR 0017 v1 增量加载与原子落盘；全库乐观并发、
> put-if-absent、持久化后确认、重复请求不二次生效与未知结果核实；提供配额、库删除/防复活、
> 账号隔离和 Node/Wasm 按库维护的运行位置。性质：新增三表及认证路由，参照 structure.progress
> 的并发和既有不可变命令回执/幂等设施；由 REZICS 手写编号 SQL，外部包不携带自动数据库迁移。
> 逻辑布局迁移、GC、浅历史与存档语义由 Narrata 实现。状态：提出。

同一集成更新把“存放位置”中的整份不透明字节改为三表与批次协议；checkpoint 只保留给导出、
导入和本地游戏。没有新格式时不要求宿主随着 Narrata 的结构版本变更 SQL 表。

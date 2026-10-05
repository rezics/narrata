# ADR 0017：浏览器存储：宿主缓存后端与批次确认协议

状态：Accepted（2026-10-05）。实现 [ADR 0012](0012-narrow-storage-backend-contract.md)
"同步接口"一节预告的"引擎产出批次、宿主落盘后确认"；不改变 [ADR 0014](0014-save-engine-key-layout.md)
与 [ADR 0015](0015-kernel-history-layer.md) 的键布局和对象格式。

## 背景

浏览器里的 Wasm 运行时要把 kernel 存档放进 IndexedDB。IndexedDB 只有异步接口，而存储契约
是同步的，引擎、账本与 GC 都建在同步调用上（ADR 0012）。存档还要满足：打开与推进只读引擎
需要的键和对象，不整库加载；批次在 IndexedDB 事务完成前不发布依赖它的结果
（[ADR 0011](0011-r1-node-composition.md)）；另一个标签页先写入时不得静默覆盖；浏览器可能整库
清除存储。

## 决策

### 缓存后端与"未加载"

`narrata-storage-host` 的 `CacheBackend` 实现 `StorageBackend`，只用宿主已经加载给它的部分
回答：已知的键（值或"不存在"）、已知的对象，以及"其中每个存在的条目都已知"的键区间与摘要
区间。它声明 `AtomicSingleWriter` 与 `Evictable`。

读取缺少的东西时，调用以新增的 `StorageError::NotLoaded` 失败，后端记下缺什么。这个错误
没有效果（不是"结果未知"），引擎原样上抛（`narrata-history` 把它保留在
`HistoryError::Storage` 中，`reconcile` 遇到它也不改报 I/O 错误）。宿主取走请求，异步读取，
把结果交给缓存，再把整个操作重跑一次。可以重跑，是因为引擎的操作先读后写，而批次在改变
任何东西之前先检查完：中止的那次尝试没有留下效果。超出后端上限而拆成几批的写入，前几批
可能已被接受；重跑时对象按 put-if-absent 幂等，与 ADR 0014"中途失败"的处理相同。

批次只需要知道它所依赖的：带 `Absent`/`Revision` 前置条件的键、要写入或删除的对象（
`Applied` 的计数要准确）。无条件写入的键不需要加载。从没写入过的库（修订号 0）一次加载后
就整体已知，所以创建存档的标签页从不再加载。

### 修订号、库标识与确认

缓存接受的批次按顺序排队。每个改变了库的批次把全库修订号加一，写入的键都带这个修订号；
只做检查、什么也没改变的批次不排队。修订号因此仍全库单调、不复用（ADR 0012），而宿主只看
一个数就能知道是否有别人先写。库另有 16 字节随机标识，建库时生成；库被清除后重建得到新
标识，旧标签页按旧库计数的批次永远对不上新库。

宿主把全部未确认批次放进一个 IndexedDB 读写事务（`durability: "strict"`）：读出库标识与
修订号，与第一批的起点相同就按序写入并更新修订号，否则什么也不写。事务完成后回复"已持久化
到修订号 r"，缓存丢掉 r 之前的批次；冲突时缓存丢掉全部内容与未确认批次，宿主重载会话。
**宿主在确认前不发布依赖这些批次的结果**；页面在确认前关闭，重开后的状态就是最后确认的批次。

加载回复也带库标识与修订号。与缓存最后确认的不同，说明别人改了库：没有未确认批次的缓存
丢掉所知、从这次回复重新开始；有未确认批次的缓存同样丢弃，并报告 `Superseded`。所以另一个
标签页的写入在本标签页下一次需要加载时、或下一次落盘时被发现；在此之前，空闲标签页的读取
可能是旧的，但基于旧读取的批次不会落盘。宿主重载会话时先让缓存 `forget()`，再打开会话。
同一个宿主不得在自己的落盘进行时加载，否则会把自己刚持久化的批次误认为别人的写入。

### 交换格式

缓存与宿主之间的消息都是 [ADR 0003](0003-deterministic-cbor-profile.md) 规范 CBOR：一个数组，
首项是协议版本 1；列表严格升序，解码后重新编码必须逐字节相同。键以存储键传递：
`space（u16 大端）‖ key`，它的字节序就是契约对 `(space, key)` 的顺序，宿主不必知道键空间。
区间是 `[lower, upper)`，`upper` 为 null 表示到末尾；宿主返回少于 `limit` 个条目时，表示整个
区间已返回。

| 消息 | 方向 | 形状 |
| --- | --- | --- |
| 加载请求 | 缓存 → 宿主 | `[1, [key], [digest], [range], [range]]`，range 为 `[lower, upper\|null, limit]`；空请求也要求回复库状态 |
| 加载回复 | 宿主 → 缓存 | `[1, id, revision, [[key, null\|[value, rev]]], [[digest, null\|bytes]], [[range, [[key, value, rev]]]], [[range, [digest]]]]` |
| 落盘 | 缓存 → 宿主 | `[1, id, [[base, base+1, [[digest, bytes]], [digest], [[key, value]], [key]]]]`，批次首尾相接 |
| 落盘回复 | 宿主 → 缓存 | `[1, 0, revision]` 已持久化；`[1, 1, id, revision]` 冲突 |

字段与校验由 [protocol.rs](../../packages/narrata/kernel/crates/narrata-storage-host/src/protocol.rs)
与 TypeScript 的 [protocol.ts](../../packages/narrata/kernel/js/src/protocol.ts) 共同固定；两边都
对照 [冻结的共享向量](../../fixtures/compat/browser-storage-v1/protocol-vectors.json)
测试。改变消息需要新的协议版本与 ADR。

### IndexedDB 布局与导出

数据库名由宿主选择，版本 1，三个对象仓库：`objects`（键：32 字节摘要；值：对象字节）、
`keys`（键：存储键；值：`{value, revision}`）、`meta`（键 `"store"`；值：`{id, revision}`）。
键都是二进制键，IndexedDB 按字节比较，与契约顺序一致。修订号以 JavaScript 数字保存，超过
`2^53 − 1` 的值被拒绝。版本更新的数据库以格式错误拒绝。

`IndexedDbStore.export()` 产出整库：`["narrata-store-export", 1, revision, [[digest, bytes]],
[[key, value, rev]]]`；`import()` 只写入空库，换新标识，修订号取 `max(当前, 导出) + 1`。
它与 `navigator.storage.persist()` 的请求一起，应对 `Evictable` 的整库清除（如 Safari 7 天
无交互清理）。只搬一个会话时，用引擎的 checkpoint bundle（`History::export`/`import`）。

### 替代方案

- **打开时全量预载到内存后端，批次照常确认。** 最简单，但打开代价与内存随历史线性增长，
  违背"不整库加载"。
- **OPFS 同步访问句柄上的 SQLite-Wasm。** 在专用 Worker 里是真正的同步存储，`SqliteBackend`
  几乎可以直接用；但需要 Worker 架构与约 1 MB 的 SQLite，访问句柄独占使多标签页需要额外
  协调，Safari 隐私模式没有 OPFS
  （[调研](../research/2026-10-04-engine-core-and-storage/notes/storage_architectures.md)）。
  它与本协议都在同一契约之下，规模需要时可以再加。
- **async trait 或 JSPI 挂起 Wasm。** ADR 0012 已拒绝把契约改成异步；JSPI 在 Safari 上还不可用。
- **宿主在事务里逐键核对前置条件，而不是核对全库修订号。** 并发标签页的误冲突更少，但宿主
  要理解前置条件，而且缓存回答的读取可能已经过时；全库修订号让一个库同一时刻只有一个写者，
  代价是并发写的标签页之一重载。
- **Web Locks 或 BroadcastChannel 选主。** 可以减少冲突，但正确性不依赖它们，留给宿主。

## 后果

- 一个操作第一次缺数据时要重跑；依赖链上每一跳是一次 IndexedDB 读取。打开已有存档读取布局
  标记、游标、提交、状态与分支，共五次加载；推进一步在暖缓存上读取转移键并确认新对象不存在，
  共两次，都不随历史深度增长（`narrata-history` 的 `host_cache` 测试计数）。
- 缓存保留读过与写过的内容；没有未确认批次时 `forget()` 可以释放。
- 一致性套件的"重开后保留状态"一例改为对 `Evictable` 后端也运行。
- 协议与导出格式已由 [ADR 0019](0019-shallow-history-bundles.md) 的
  `fixtures/compat/browser-storage-v1/` 冻结；Rust 与 TypeScript 均读取这些字节，TypeScript
  测试还检查导入后的 IndexedDB 布局 v1 与重开结果。

## 不做

OPFS 实现；多标签页选主；跨设备同步；对 `bindings/typescript` 旧 `indexeddb-store.ts` 与节点
阅读器存档的改写。

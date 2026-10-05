# ADR 0020：通用 Effect 账本与构件迁移协调

状态：Accepted。扩展 [ADR 0015](0015-kernel-history-layer.md)，保留
[ADR 0014](0014-save-engine-key-layout.md) 和 [ADR 0009](0009-stage-5-migration-and-protocol-boundary.md)
的旧栈编码与领域语义。

## 原因与边界

Effect 的租约、单调结果与 fence，以及可信迁移图的路径选择和原子发布，与叙事领域无关。
它们原来只在 `narrata-store` 中，新的注册领域无法使用。历史层现在提供这两份实现；
状态转换、relocation、能力契约与宿主派发仍由领域或宿主负责，没有新增 async、后端或协议消息。

## Effect 注册与格式

`narrata_history::effect::EffectRegistration` 指定提交种类、账本空间、账本编解码与响应验证。
同一状态机按 execution（16 字节）和 effect（32 字节）定位条目，保留原 claim、续期、
超时接管、结果记录、重放复用、fence 分配与补偿的行为。claim 只接受已经存入库的 origin commit，
并用 sweep/graph 栅栏把 origin 与账本根的存在性保护到批次提交；宿主仍须在 claim 成功后派发。
终态不能退回 Claimed；UnknownOutcome 是需要宿主处理的终态，不能自动重试。

旧栈的 `LegacyEffects` 注册继续使用空间 8、9；CapabilityId、版本、策略、响应对象和全部
键值编码保持原样。`store-layout-v1` 的打开与 `store-sqlite-v2` 的迁移仍由冻结测试逐字节核对。
通用状态机不解释旧 Snapshot 或 Effect payload。

新领域使用 `DomainEffects<D>` 和下列格式，规范 CBOR 的受检解码要求重新编码后逐字节相同：

| 项目 | 格式 |
| --- | --- |
| 空间 16，generic-effects | 键 `execution(16) ‖ effect(32)`；值沿用 ADR 0014 的七字段 map、delivery/rewind/status 判别值，capability、补偿 capability 与版本改为 bstr |
| 空间 17，generic-ledger-fences | 键 `execution(16)`；值 `{0: fence}`，沿用 ADR 0014 的 fence 编码 |
| kind 13，schema 1，Effect response | `{0: effect bstr32, 1: request_digest bstr32, 2: capability bstr, 3: capability_version bstr, 4: payload bstr}` |
| kind 14，schema 1，Effect ledger guard | 固定载荷 `{}`，固定对象身份；第一次 claim 与条目同批写入 |

capability 与版本各最多 65536 字节，响应 payload 最多 16 MiB；后端更小的限额仍可拒绝写入。
响应 payload 是宿主给出的响应字节，历史层不解读它，也不拥有正文内容。诊断是固定 32 字节 ID。
记录响应时先检查它与条目的 effect、request digest、能力名与版本一致，再以一个批次写响应对象、
条目与 fence。补偿保持原响应和原 fence，增加另一效果的关系，不删除外部事实。

历史层把空间 16 中的 origin 与响应作为 GC 根；旧栈空间 8 的根继续由旧栈注册提供。
guard 在通用账本非空时也作为根。增加 guard 是为了防止旧历史引擎忽略空间 16 的根后删除 origin：
它不认识 kind 14，按 ADR 0015 在任何清扫前失败关闭。没有 guard 的通用账本不是本实现产生的库。
布局版本仍为 1；旧格式没有变，新对象与键只会出现在显式使用新能力的库中。

账本独立于 cursor 与 Snapshot；Checkpoint Bundle 仍只搬运提交的对象闭包，不搬运单调账本键。
导入一个迁移后的 checkpoint 不能被当作已经迁移宿主的 Effect 事实。

## 迁移协调与显式输入

`ArtifactRegistry` 保存宿主注册的可信构件；`MigrationRegistry` 保存 ID、source/target 链接与
领域提供的迁移值。路径发现、拒绝歧义、显式路径检查和有界执行从旧栈搬入历史层；旧栈的
ProgramRegistry/MigrationRegistry 成为适配器，继续用原 MigrationDescriptor、RelocationTable、
MigrationOptions 和受检状态转换。

通用 Commit 的 schema 与五字段编码不变。普通 input 仍只能连接同一 artifact 的父子提交；
kind 12、schema 1 的显式迁移 input 才能改变 artifact：

```text
MigrationInput = {0: migration_id bstr32, 1: from_artifact bstr32, 2: to_artifact bstr32}
```

提交的 input 引用这个对象，artifact 是 target，parent 是 source commit，depth 为父深度加一。
写入时检查 input 的 source 与父 artifact 相同、target 与子 artifact 相同，并拒绝 self edge。
相同 parent 和迁移 input 的不同结果仍由 transitions 索引报告 Nondeterministic。
浅存档缺失父提交时只能证明 target；完整父历史导入时会重新核对 source。

`dry_run_migration` 加载精确 source，选择链，由领域闭包转换每一步，按 target 构件受检解码
结果，并规划提交、索引与目标 Ref，期间不写入。结果给出各步提交 ID 与报告；真实 CAS 在应用时
检查。`apply_migration` 重做这份规划并用 `History::write_atomic` 发布：全部对象和唯一目标 Ref
CAS 是一个批次，超出后端批次限额时在写入前拒绝；不会降级为分批发布。结果未知时沿用 ADR 0014
的回读协调。

旧栈迁移使用相同的图协调器与原子写入路径，提交继续是 CommitV1 的 Migration cause，迁移 ID
仍是旧栈的 16 字节 ID，冻结字节不变。`SaveStore::commit_atomic` 的默认实现拒绝不支持原子性的
第三方实现；`Store<B>` 在支持多键原子批次的后端上实现它。

恢复目标提交仍不重放；迁移后以 target 领域打开会话并继续。单构件 `verify_path` 仍只审计一个
构件内的普通执行路径，不能替代跨构件迁移的领域审计。

## 兼容证据

`fixtures/compat/kernel-history-v3/` 冻结计数器领域的 SQLite 库、迁移后继续执行的完整
Checkpoint Bundle 与对象/键清单，覆盖 kinds 12–14、空间 16/17、全部六种账本状态及补偿 fence。
生成器 `emit_kernel_history_v3` 只接受空输出目录；两次生成的数据库、bundle 和清单逐字节相同。
测试验证打开与继续、重放复用、完整 bundle 导入与再导出、干运行零写入、单批应用、冲突零发布、
存储结果未知、坏输入拒绝、旧构件保留及普通 input 禁止构件变化。已有冻结语料不修改。

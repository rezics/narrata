# ADR 0023：节点构件演化与读者存档迁移

状态：Proposed（2026-10-05）。扩展 [ADR 0013](0013-r2-text-free-node-format.md) 的节点会话，
复用 [ADR 0020](0020-shared-effects-and-migrations.md) 的协调器；不改变旧构件、State、普通
Input、Commit、Checkpoint Bundle 或存储键的编码。

## 原因与边界

[决定 3、6、11、14、16、17](../product/decisions.md) 要求身份跨发布稳定、动态提议可回放、
旧版本永久保留，并由第一阶段的一个平台运行时读取阶段内所有旧格式。结构修订产生新的
`artifact_id`，不能让存档通过“当前发布”悄悄换程序。

现有 [`NodeDomain`](../../packages/narrata/nodes/crates/narrata-nodes/src/history.rs) 已注册
kernel 历史层；`decode_state` 校验变量、栈、交互、叠加层与构件对应，但不证明可游玩到达。
[`Program::lookup`](../../packages/narrata/nodes/crates/narrata-nodes/src/program.rs) 返回
`Live / Deleted / Unknown`；[`compose`](../../packages/narrata/tooling/crates/narrata-node-tools/src/compose.rs)
保持墓碑与归属连续。`r1::migrate_save` 只处理精确 R1 构件到其 R2 产物，不是作品演化接口。

以下数据形状与节点迁移入口均为待实现设计。现有 `Session::verify_path`、历史呈现与会话打开
仍按单构件工作；它们不能直接审计或呈现新的跨构件父链。并行 ADR 0022 负责草稿与发布产物，
本 ADR 在同一 authoring 模块提供独立演化规划，不扩展其 `publishRecords` 输入输出字段；
服务器存档库与传输由并行 ADR 0021 负责。

## 决定

### 自动生成与应用

发布工具先以 `Program::verify_artifact` 核对精确旧、新构件的完整闭包，再生成一个确定的
直接迁移描述符：保留全部稳定 ID、变量值、实例号、
调用栈和已记录的 overlay，不重算过去的效果。静态 ID 必须保持原归属，消失的必须在新墓碑集；
`Unknown` 不当成删除，不按别名、正文或数组位置猜替代。动态 ID 在其 frame overlay 中检查，
不拿工具侧的静态 `lookup` 判断其存在性。

用新程序的 `decode_state` 重新构造候选状态，规范编码往返相同才可迁移。新增、改名、变型变量，
调用目标与上层 frame 不匹配，或 overlay 引用失效，都不能靠稳定 ID 自动修复。第一版不提供
变量转换、跨图重定位或任意迁移代码；恢复声明也不能绕过目标受检解码。候选还须能生成合法
终局或可继续的交互 view；新条件使交互无动作且不接受提议时，不自动交付一个无法继续的状态。

自动生成迁移边与自动应用分开：候选状态通过检查且下节的路径比较等价，才返回 `Automatic`。
改条件、效果、顺序、目标节点仍保留 ID；若已走路径未受影响，允许宿主直接应用，若重放分歧或证据
不足，返回需确认的候选。只改正文、译本、别名且构件摘要相同，直接精确恢复，不生成 self edge。
修改内容引用若改变了构件摘要，也走同一检查，不把引用等同于它解析出的文字。

升级策略归宿主；Narrata 只提供评估与应用，从不自行升级。推荐宿主默认在读者打开存档时，
若当前发布与存档构件不同就评估：`Automatic` 直接调用应用并告知读者，`NeedsConfirmation`
弹出确认，其余继续旧版或重新开始（缺对象可先重试）。宿主可以选择更保守的策略；发布规划与
批量 dry-run 保持只读。理由是 State 良构只能证明可以继续，不能证明新规则下的过去仍相同，
路径证据决定评估结果，何时调用应用由宿主决定。

### 恢复声明与有损结果

作者声明存在独立的严格 JSON 源稿/宿主草稿记录中，作为独立演化规划的可选输入；不向现有
`ProjectSource` 或包 JSON 偷加字段。逻辑形状如下（ID 文本沿用 ADR 0013）：

```text
RecoveryDeclarationsV1 = {
  format_version: 1, from_artifact,
  recoveries: [{ deleted: NodeId | ChoicePointId,
                 target: { node: NodeId, choice_point: ChoicePointId } }]
}
EvolutionDescriptorV1 = {
  format_version: 1, from_artifact, to_artifact, recoveries
}
```

发布工具填入编译后的 `to_artifact`，核对每个 `deleted` 在 source 存活、在 target 为墓碑。
target 必须是新程序**入口图中存活的 passage 及其选择点**。第一版只支持回到根图的安全交互，
不允许声明脚本、效果、表达式、任意变量赋值或合成调用栈。JSON 拒绝重复键、未知字段、未知版本，
最多 1 MiB、4,096 条映射；同一 deleted ID 重复也是错误。无声明相当于空映射。

描述符是发布元数据，不进入程序清单的摘要。它按 deleted 的类型（node 在前、choice-point
在后）及 16 字节身份排序；其身份为
`MigrationId = digest_bytes("narrata.nodes.evolution", 1, canonical_descriptor)`，保留完整 32 字节。
`canonical_descriptor` 用 ADR 0003 CBOR：`{0: 1, 1: from bstr32, 2: to bstr32,
3: [[kind, deleted bstr16, node bstr16, choice_point bstr16], …]}`，kind 分别为 0、1。
JSON 排序、空白与作者别名不参与身份；相同输入必须产生相同描述符与迁移 ID。
描述符作为候选发布对象集合的可选 `evolution` 角色，按 MigrationId 寻址并随发布永久保留
（见发布挂接）；修订声明产生新 ID，旧发布不覆盖。

当当前 node、任一调用 frame 的 node、当前 `at`，或 `finished.node` 成为墓碑时，收集这些
位置的全部声明。全部有映射且目标相同才使用恢复；缺项返回 `UnmappedContinuation`，冲突
返回 `ConflictingRecovery`。不自动挑其中一个，不隐式回到故事开头。只删除未被 State 引用的
节点不触发恢复，删除已选过的选项则由路径比较报告。

恢复保留且只保留与目标共享变量声明的键、类型完全一致的 shared 值；清空全部 frame、overlay
和 finished，用目标 product arguments、入口图 local 初值与声明的 node/choice point 构造
一个根 frame。新 frame 的 instance 取旧 `next_instance`，再加一；溢出拒绝。不执行目标前面
的 mutate/call，不伪造已做的选择。候选仍必须通过 `decode_state`；变量不兼容或新交互无可用
动作且不接受提议时，恢复失败，继续旧版或另开新会话。

有损报告列出 source/target 构件与提交、触发的墓碑 ID、丢弃的 frame 实例与 node、local/
parameter 的键、overlay 的结构 ID、清除的 pending choice/finished、保留的 shared 键、
重置的 local 键、新入口 node/choice point、路径比较结果。报告不含正文或变量值；第一版节点栈
没有外部 Effect，明确报告 `external_effects: not_applicable`，不声称迁移过宿主外部事实。
读者必须确认恢复点及损失；没有声明时只能保留旧版本或重新开始。理由是删掉调用位置后，局部
拼栈无法由现有语义证明正确，而数据声明应当能完整说明代价。

### 路径比较与重放分歧

精确旧构件的正常恢复不重放；跨构件升级在评估阶段执行只读、无宿主调用的有界路径比较。
先在 source 上验证可读路径的 State 与 Input；根 State 必须等于 `Machine::initial` 的结果。
旧记录本身不可重放是 `InvalidSave`，不能误报为新版分歧。

然后从同一根在 target 上影子执行记录的选择，逐步比较 State 的语义内容：shared、frame
位置/交互、参数/local、instance、next_instance、finished 与 overlay。比较不使用带不同
artifact 的 CommitId，也不比较正文、别名或纯呈现引用；参与规则的 Scalar（包括 ref）仍比较。
初始状态不同、输入失效、条件不满足、选项删除、执行失败、调用/去向变化或任一步语义状态不同，
都在第一处分歧停止，即使后面重新汇合也不能报告等价。

还要比较每次推进（含 initial）的临时控制轨迹：使用 ADR 0022 §6 的同一个只读执行 trace，
按执行顺序比较 graph/node、选中选项的 outcome/target、branch 去向与 call/return。
trace 的实现由 ADR 0022 统一负责，以默认关闭的 feature 编进 authoring/server Wasm，
读者 Wasm 不含它；每步比较后释放，不新增持久回执。这样改目标后在**同一个 Input 内**自动
汇合，也能检出分歧；只比较 safe-point State 或呈现项不能证明走向未变。

`choose` 按稳定 ID 取原选择集合，在 target 的选项顺序下构造影子输入，依新顺序执行效果；
原 Input 不改写。`propose` 先用原始父提交（含已有 interim 兼容身份路径）复核派生 ID，再
对 target 校验已记录的结构与引用；影子执行仍用原父身份，不重新生成或重新铸造 ID。
目标与动态 ID 碰撞、提议位置关闭或引用失效均为分歧；直接按新父提交派生 ID 会把同一提议
变成另一批结构。

遇到已有 MigrationInput 时按段审计：先核对其 source/target/parent 链接、描述符身份，并按
描述符重算迁移后受检状态、比较 State 摘要；该已提交迁移状态是后续普通输入段的 baseline，
不在 target 下重跑先前其他构件
的输入。对最新 source 段作新 target 的比较；baseline 无法在 target 下解释、缺祖先或描述符，
以及达到资源上限，返回 `Unverified { reason, baseline, checked_through }`。这不阻止旧版精确
恢复，但不能作为自动迁移的等价证明；现有单构件 `verify_path` 只用于段内审计。

报告区分 `Equivalent`、`Diverged { source_commit, input_id?, reason, old_location,
new_location?, differing_fields }` 和 `Unverified`，不回显状态值。新版影子结果只作证据，
不当作迁移后的存档：若原 State 在 target 良构，可确认“保留当前进度，后续使用新规则”；
否则只有作者声明的恢复候选，或继续旧版/重新开始。宿主据此显示首次变化的位置和是否保留进度，
没有完整证据时显示“过去的路径未全部核对”，不能显示“无影响”。

### 协调、提交与跨版本会话

受信节点实现将已检查的 `NodeDomain` 注册到 `ArtifactRegistry`，描述符注册到
`MigrationRegistry`；在节点包装层作路径比较，并把确定的状态转换/报告作为领域 step 闭包
交给 `History::dry_run_migration` 与 `History::apply_migration`。评估与应用入口在同一
authoring/server 模块中运行；服务器在 Node 中运行，匿名读者只在浏览器需要升级时懒加载
authoring 模块，不计入阅读首屏。reader 接收结果并打开相应提交，不执行迁移评估或控制 trace。
协调器仍负责 target 状态规范往返、提交/索引规划、路径限额、CAS 与原子发布；不复制一套框架。

默认显式选择生成的单条直接边，避免已注册多条路线时的歧义；需要多跳时必须给出完整 ID 序列，
沿用协调器的 64 步上限，每一步都需要可解释的 source/target，宿主按评估结果决定应用与确认。
描述符从受信发布对象集合的 `evolution` 角色加载，存档携带的同名数据不能自动注册为可信迁移。

每一步写现有 kind 12/schema 1 的 `MigrationInput { id, from, to }`，新提交的 artifact 为
target、parent 为 source commit、depth 加一。State 完全保留时可以复用原 State 对象，提交
仍是新的；transitions 对同一 parent/input 的不同结果继续报告 `Nondeterministic`。
输入不能装进普通 `Choose/Propose`；身份、批准与展示报告不进入普通 Input 或旧提交。

运行中升级以 active cursor 为协调器唯一 `target_ref`，先核对它仍是评估的 source，再以其
revision CAS 应用；旧 branch head 与 save slot 保持可选。迁移成功后用目标 NodeDomain
重新打开这个 cursor；没有目标 branch head 时，下一次普通推进由现有会话 API 创建新分支。
保存到原 slot 是之后显式的普通 save 操作，不伪称同时改了多个 Ref。只有存储宿主落盘确认后才
报告升级已保存；结果未知按 ADR 0014 回读，冲突重新评估，不能覆盖另一个设备的选择。

迁移前可确认计划绑定 source commit、target artifact、描述符路径、预期 cursor revision、
候选迁移 commit 和报告摘要。报告摘要用 `digest_bytes("narrata.nodes.evolution-report", 1,
规范 CBOR 报告)`，覆盖迁移位置/损失与路径结论，不含展示文字、墙钟或扫描统计。
应用重做评估与协调器规划，并要求这些字段一致；源游标、声明或
损失改变时旧确认失效。不可变输入上的应用结果必须与 dry-run 相同。

节点包装层按每个 Commit 的 artifact 解析历史、回退与旧分支；跨过迁移边时切换精确程序并
受检打开，不能用 target 解码 source State。迁移提交是新的呈现边界，从 target State 生成
当前 passage/choice view；不把 MigrationInput 当普通选择执行，也不重做过去的效果。
更早页面在自己的构件上呈现。Checkpoint 导出包含跨构件祖先，导入逐个核对迁移链接，目标
恢复不依赖重放全历史；缺描述符只影响再迁移/审计，不妨碍已有目标 State 的精确恢复。

### 继续旧版本与宿主结果

宿主保存/传递 `source_commit` 及其 `artifact_id`；commit 的受检头才是权威，目录摘要只是
查找提示。按该摘要取永久保留的清单、墓碑与程序块，用 `Program::from_manifest` 和
`ChunkSource` 打开（单文件用 `Program::from_pack`），然后在对应 NodeDomain 上恢复。
“作品当前发布”只提供升级候选，不能替代存档的摘要；旧正文撤回照常由内容解析返回 unavailable。
服务器的发布保留根独立于读者存档 GC，本地分发也须保留所需旧构件。

第一阶段的新平台运行时须保留此前阶段版本的构件/存档解码器或确定的格式升级路径；R1 用
现有 `r1::migrate_save`，interim R2 用现有兼容读法，再评估作品演化。格式升级与作品迁移
分别报告，不用 SemVer 猜兼容；不下载旧运行时、不执行构件或声明附带的迁移代码。

宿主 API 的版本 1 JSON 判别联合如下；这些名字是拟议结果，不是已有可调用接口：

```text
AssessmentV1 = Exact { source }
             | Automatic { plan }
             | NeedsConfirmation { plan, reason: recovery | divergence | unverified }
             | OldVersionOnly { source, target, reason, restart_available }
             | NeedsObjects { digests, retryable: true }
             | InvalidSave { diagnostic, restart_available }
ApplyV1      = Migrated { source_commit, migration_commit, target_artifact, report }
             | Conflict { actual_cursor } | PlanChanged | Failed { diagnostic }
Plan         = { source_commit, target_artifact, migration_ids, expected_cursor_revision,
                 migration_commit, report, report_digest }
```

`Exact`、`Migrated` 才能交给运行时继续；`Automatic` 是允许自动应用的计划，不是已保存。
`NeedsConfirmation` 不写任何东西；界面同时给继续旧版与重新开始的选择。`OldVersionOnly`
表示新版无法承接但旧版可打开；`InvalidSave` 不承诺旧版可恢复，只能另开会话或修复/重新导入。
缺构件/对象是可重试的 `NeedsObjects`，不能误报旧版被删除。重新开始使用新的 ExecutionId
与根提交，不覆盖旧存档。元数据的 revision 用可精确往返的表示，不强转超出 JS 安全整数的值。

### 发布挂接与发布前 dry-run

authoring 模块提供独立函数（拟名）`planEvolution(previous, candidate, declarations?)`，
接收 ADR 0022 的受检旧、候选发布对象访问器与可选恢复声明，返回描述符及 MigrationId。
发布服务在 `publishRecords` 之后、CAS 切换当前发布之前调用，不给 `publishRecords` 加字段；
将描述符的规范 CBOR 字节加入候选对象集合的可选角色 `evolution`，按 MigrationId 寻址，
再重算集合描述及 `publication_id`，`artifact_id` 不变。该角色随发布上传、核验并永久保留，
复用 ADR 0022 的发布保留机制；首次发布可省略。
多种旧构件按精确 from/target 分别规划；最新前驱之外的存档不能被当成前驱存档检查。
影响评估报告 `ImpactReportV1` 由独立只读作业提供，不进入 `publishRecords` 的输入输出或
程序产物，也不把私人存档传入演化规划。

作业输入是精确旧、新构件对象来源、描述符，和经授权的存档库句柄加 source commit 列表，或
存档字节（完整/浅 bundle 与既有 JSON 导出）。也可传 `{ commit, artifact, state }` 摘要，
但它仅用于寻址/去重，须取得并验证对应对象后才能分类；只凭 digest、位置或宿主摘要不证明可迁移。
按单个 source 调用同一节点评估和 `dry_run_migration`，规划目标 Ref 的 CAS/后端限额；用于
模拟的目标 Ref 在只读视图中提供，绝不创建临时持久 Ref，也不调用 apply/flush。

发布服务在 Node 中运行同一 authoring/server Wasm，沿用 ADR 0022 的 server 子路径加载方式，
并保持其 Bun 兼容；按 ADR 0021 协议读取服务器上的存档库。
宿主只负责授权、库枚举、配额与作业调度，不用 SQL/ORM 重建迁移。客户端可运行自己的评估，
服务器报告不包括尚未同步的本地存档。程序块与存档对象按需加载、复用缓存，不能每个存档复制
整部十万选择点作品。

每个作业最多评估 10,000 个不同 source commit；每次调用最多 256 个头、输入字节 16 MiB。
每份存档的路径比较最多 10,000 次普通转移、100,000 次自动节点步、累计读对象 64 MiB，
并沿用 State/Input/栈/执行预算的现有限额；服务可收紧。超限归 `Unverified(limit)`，
不算等价或格式损坏；应用仍受协调器及后端原子批次限额约束。扫描分页、作业可续跑，取消
不留下存档变更。

`ImpactReportV1` 记录 from/target、descriptor ID、各库扫描修订范围、已枚举/评估/跳过数、
distinct commit 数与按 Ref 加权的数量；每个已评估 Ref 恰属 exact、automatic、confirmation
（recovery/divergence/unverified）、old_version_only、needs_objects 或 invalid_save 一类。
附有限额原因计数、首次分歧/损失的脱敏样例、被检路径覆盖范围与读入字节，不列用户身份和值。
库在扫描中变化时将相关项计入 `skipped(stale_scan)`，不算已评估，需重试；不宣称报告覆盖
同一个全库时点。

超过上限时先按 source artifact 分组，对 `(artifact, commit)` 的固定域摘要排序取前 N 个，
报告样本数、总体已知数/未知、N、算法版本与扫描范围；同一作业续跑保持选择规则。相同 commit
只执行一次，但统计保留其 Ref 权重。抽样报告只承诺样本，不外推为“全部存档安全”，不足以让
任何未检查的读者自动迁移。报告绑定候选 target/descriptor；重新发布改变它们必须重跑。

## 实现顺序与兼容证据

本 ADR 评审后再派发实现；不改现有冻结语料。新格式仅有版本 1 的声明/描述符与宿主结果，
既有节点对象继续 schema 1，迁移 Input 沿用 ADR 0020。旧数据先走现有兼容读法，新版本须
读取旧节点提交并支持新的迁移链接；以后改这些新数据的版本同样需要 ADR、读法和冻结语料。

| 工作与路径 | 依赖与证明 |
| --- | --- |
| `packages/narrata/nodes/crates/narrata-nodes/src/` 的新 `migration.rs`，`history.rs`、`session.rs`、`state.rs`、`program.rs`；`packages/narrata/nodes/schemas/`；节点 tests | 先实现声明/描述符受检类型与 schema，再做状态保留/根恢复、分段审计、跨构件呈现/回退、cursor 应用；trace 依赖 ADR 0022 §7 实现表中 `runtime.rs` / `maps.rs` 那一行，不另派重复实现；复用 kernel 协调器，证明确定性、坏输入拒绝、零写 dry-run、确认失效、CAS/结果未知与旧版继续 |
| `packages/narrata/tooling/crates/narrata-authoring/src/` 的演化入口；`packages/narrata/tooling/crates/narrata-node-tools/src/compose.rs`、`publish.rs` 与 tests | 依赖前项及 ADR 0022 的内存发布接口；实现独立 `planEvolution`，验证连续墓碑/归属、恢复目标、`evolution` 角色的寻址/保留、集合身份重算与 `publishRecords` 字段不变，并挂接独立只读影响作业 |
| `packages/narrata/tooling/crates/narrata-authoring-wasm/`、`packages/narrata/web/`、`packages/narrata/nodes/crates/narrata-nodes-wasm/src/lib.rs`、`examples/gamebook-web/` 与浏览器 tests | 依赖节点实现、ADR 0021 的服务器库读取及 ADR 0022 的 authoring/server 模块与共享 trace；只由 authoring 导出评估/应用，reader 负责打开结果；验证 Native/Wasm 一致、Node/Bun 作业抽样/超限、匿名升级懒加载、reader 无 trace 且首屏体积不增长，以及宿主直接应用/确认/旧版/重新开始及落盘冲突 |
| 新 `fixtures/compat/nodes-evolution-v1/` 与生成器/兼容 tests | 冻结新旧构件、迁移前后完整/浅存档、声明 JSON 与规范描述符、报告向量；两次生成逐字节相同，后续运行时导入、继续、回退、再导出可读 |

冻结样例至少覆盖：未走路径的规则变更自动迁移；内容/别名不换构件；改目标后又汇合仍检出
首处分歧；多选重排；已记录提议的 ID 保持与碰撞拒绝；删当前位置/下层 call/choice/结局的
根恢复；无声明/冲突声明/变量变型拒绝；浅历史未核对；已迁移后再发布；旧存档及旧版分支继续。
伪造 descriptor/source、非规范编码、缺对象、超限与冲突不得产生部分可见迁移。

实现检查经 Goal 槽位运行：

```text
task goal -- slot -- cargo test -p narrata-nodes -p narrata-node-tools
task goal -- slot -- cargo clippy -p narrata-nodes -p narrata-node-tools --all-targets -- -D warnings
task goal -- slot -- cargo check -p narrata-nodes-wasm --target wasm32-unknown-unknown
task goal -- slot --heavy -- task check:r1
```

新增 Node/Bun 作业测试须纳入 `check:r1`；authoring/server Wasm 按 ADR 0022 的检查覆盖评估/
应用与共享 trace feature，另验证 reader 构建关闭该 feature。
kernel 若确需改动，增加协调器回归并用重型槽跑 `task check:g5`；本 ADR 只跑
`task docs:check`。新工具或检查加入时同改工具链记录。

manager 合入 [REZICS 能力请求](../integrations/rezics.md) 时的文本：R9 增加“永久保留全部程序
发布及其可选 `evolution` 角色；发布服务在 `publishRecords` 后以 `planEvolution` 接受恢复
声明，按 MigrationId 寻址描述符，随候选发布保存并在 CAS 前核验，当前发布切换不改
读者存档”；R6 在 ADR 0021 的库协议上增加“授权发布服务只读枚举存档头与对象用于有界影响
评估，区分样本/全部，应用迁移仍走原子批次与 CAS”；R1 增加“打开固定构件，承接迁移评估
与确认结果，由宿主决定升级策略，推荐打开存档遇新发布时评估、直接应用 `Automatic` 并告知，
对 `NeedsConfirmation` 弹出确认，其余继续旧版/重新开始；需确认的迁移在确认前、任何迁移
在未落盘成功时保留原进度”。不修改 REZICS 仓库。

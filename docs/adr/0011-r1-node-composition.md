# 0011：R1 类型化节点、子图和 Gamebook 会话

状态：部分被 [ADR 0013](0013-r2-text-free-node-format.md) 取代（2026-10-05）。作者格式、内容呈现、
构件身份与保存格式以 ADR 0013 为准，R1 作品与存档只经 `narrata-book migrate-r1` 迁移读取；
组合、调用、作用域与表达式语义仍以本 ADR 为准。

R1 必须能运行没有对白、相机和立绘状态的 Gamebook，并组合独立内容包。旧 Program/RuntimeState
要求 Flow 入口和 SceneState，因此不能把它们直接包装成新的公共节点格式。

在 `packages/narrata/nodes` 建立独立 owner，内部 `narrata-nodes` 不依赖旧 `narrata-core`。
旧 Stage 1–5 接口、CBOR 与存档继续保持原样，逐步通过显式 adapter 接回，不同时改写已有格式。

## 作者与编译契约

- Bundle 选择命名的 package instances；包显式导出 graphs。
- Graph 声明参数、局部默认状态、借用的共享变量、导入端口和命名结果。
- 产品明确绑定进口到导出图；参数与结果集合必须匹配，重复 provider 与未绑定端口失败。
- NodeRegistry 支持注册作者节点类型及语义 revision，将配置降低为受校验的 NodePlan。
- R1 执行词汇为 content、decision、branch、mutate、call、return；新增作者类型不能绕过
  全图的引用、表达式与调用检查。这是编译扩展，尚不支持任意 native 节点状态处理器。
- 数据/节点 ID 是作者持久维护的名字；改标题无需改 ID。更改 ID 属于语义变更，不按行号重建身份。

`CheckedProduct` 仅由 compile 构造。JSON 边界拒绝重复键、未知字段和不支持的版本；所有
表达式、赋值类型、模板引用、调用参数及返回 continuation 都在执行前检查。
生成 JSON Schema 负责数据形状，不能替代以上领域校验。

构件摘要固定规范化的 Bundle、所用节点类型 revision 与实际 lowering 结果。命名 map 排序，
导出/结果/绑定集合规范化；选择顺序保留。该 JSON/hash profile 与旧 CBOR 构件身份分开。

## 状态与执行

每次调用创建有独立 instance ID、只读参数、局部状态和调用位置的 frame。共享状态由产品声明，
每个图必须显式声明它使用的共享字段；这是 R1 的状态授权范围，完整跨模块 state-owner 协议后续扩展。

一次 action 在隔离 working state 中处理赋值、条件和子图调用，直到新的内容/选择等待点或
根图结束。类型故障、溢出、无可用行动、预算超限均不改变当前提交。
选择输入携带 expected Commit，过期/隐藏/禁用选择无法执行。

历史为单父 immutable commits。checkout 后改选保留两个分支；相同父提交与动作复用既有提交。
渲染、编译和 I/O 不推进故事。内容呈现采用纯文本及类型化变量替换，不执行 HTML 或宿主脚本。

## R1 保存格式

保存包含精确构件身份、每个保留 Commit 的完整逻辑 Snapshot、动作/parent 与 cursor。
恢复时在同一 checked product 上重放校验各 Snapshot 和 Commit，不信任文件提供的状态或摘要。
它是有界参考 profile，不是旧 CheckpointBundle/TimelineArchive，也不是防篡改签名格式。

- 文档上限 4 MiB；单状态 128 KiB；最多保留 512 个 Commit / 2 MiB Snapshot。
- 单次自动执行最多 4096 节点步骤，调用深度 64；恢复最多 1,000,000 步。
- 当前保留整个有界会话，不隐式裁剪历史；达到限制返回明确错误，旧状态保持可导出。
- 原生/浏览器共享同一 Rust 实现；传给 JS 的 i64 检查值用字符串表示，原始作品/存档按文本传输。
- Session 是内存 reference coordinator；Web adapter 必须在 IndexedDB 事务完成后才发布 view，
  写入失败则恢复旧会话。R1 不执行外部业务 Effect，未来接入必须复用 commit-before-dispatch 规则。

这些预算和执行词汇不限制未来 Scene/Quest 的领域模型。后续需要活动状态 schema、局部取消与
稳定序列化契约时，应增加相应版本/类型，而不是重新加入 Session.scene 特殊字段。

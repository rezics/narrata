# ADR 0019：通用历史的浅 checkpoint bundle

状态：Accepted（2026-10-05）。扩展 [ADR 0015](0015-kernel-history-layer.md) 的通用历史；
保留 [ADR 0014](0014-save-engine-key-layout.md) 的所有键空间、键和值编码及布局版本 1。

## 决策与理由

宿主存档与同步通常只需要当前状态。完整导出仍沿用 `NARCPB1\0`、kind 7/schema 1；新增的
浅导出携带目标提交、它的状态与输入、最多 N 个祖先及这些祖先的依赖，N=0 仅携带目标。
不改写提交的 parent、depth 或身份，否则无法与随后导入的完整历史共享对象。

浅容器用 `NARCPB2\0`，其余容器 framing 与 v1 相同。kind 7/schema 2 清单在 v1 的四个字段
之后增加字段 4：严格升序且唯一的被截断父提交 ID 数组；字段 0 的版本为 2。描述符只列实际
闭包对象。只有通用 Commit 的 parent 边可以截断；state、input 与注册方的其他依赖必须完整。
每个边界必须是实际遍历到的 parent 边，且不能同时列在描述符里。空边界也使用 v2，以固定浅
导出入口的格式。旧的完整导出与旧栈的 CheckpointBundle 不变。

导入在任何写入前核对摘要、描述符、精确闭包和边界。每个边界派生一个引擎拥有的
kind `0x0010`/schema 1 叶对象：载荷 `{0: parent bstr32}`。其内容寻址 ID 可由缺失的 parent
直接计算，因此读取边界不需要整库扫描或新增索引。它只声明该 ID 的历史被省略，不声称省略的
提交曾被验证，也不证明状态可经游玩到达。与完整恢复一样，领域受检解码证明状态良构；缺少
父提交时无法核对跨边界的 artifact 与 depth，只有补齐历史后才能重放验证。

写入、GC 与完整性扫描只允许用截断记录替代通用 Commit 的缺失 parent。写入拆批时记录先于
依赖它的提交写入；GC 保活边界时保活记录。访问缺失的边界返回 `HistoryTruncated(parent)`，
普通缺对象仍是 `MissingObject`。`verify_path` 到边界即失败，不把部分重放当作完整验证。
children 与 transitions 保留原始父 ID，因此分页与转移身份不变。

同库导入完整历史后优先读取实际 parent，恢复跨边界的验证；GC 可删除不再使用的截断记录。
浅库必须通过浅入口再次导出；完整入口在遇到缺失祖先时返回截断错误，绝不伪造完整 bundle。
旧读取器遇到 schema 2 或 kind `0x0010` 时失败关闭；新读取器继续读取与写出 v1，不需要迁移。

## 冻结证据

`fixtures/compat/kernel-history-v2/` 固定 N=0 与 N=1 的浅 bundle、导入 N=1 后的 SQLite 库、
对应完整 v1 bundle 及摘要清单。测试覆盖重开、回退、分页、推进、GC 与完整历史补齐。
`fixtures/compat/browser-storage-v1/` 固定 [ADR 0017](0017-browser-storage-host-cache.md)
现有协议的 CBOR 向量与整库导出；Rust 和 TypeScript 均读取同一文件。它不改变浏览器协议、
IndexedDB 布局或导出版本。

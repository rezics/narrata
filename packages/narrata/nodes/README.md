# Narrata Nodes

R2 的类型化节点、显式子图组合与可移植 Gamebook 会话（[ADR 0013](../../../docs/adr/0013-r2-text-free-node-format.md)）。
Narrata 只持有叙事结构：标题、正文和选项文字都是内容引用，由宿主的内容方解析。

`crates/narrata-nodes` 把源稿编译成内容寻址的构件：清单（其 object id 即 `artifact_id`）、每图
一个程序块、墓碑集与可选的名字表，单文件分发时装进一个打包信封。`Program` 只先读清单，块在
栈帧进入它的图时才加载并做受检解码。

```rust
use std::sync::Arc;
use narrata_nodes::{ExecutionId, Program, Session};

fn open(pack: &[u8], execution: ExecutionId) -> narrata_nodes::Result<Session> {
    let (program, _names) = Program::from_pack(pack)?;
    Session::new(Arc::new(program), execution)
}
```

- 段落（`passage`）把正文切成段，选择点放在块之后；局部结果先呈现回应、再从 `rejoin` 继续同一
  段落，分支结果进入本图另一个节点。多选与 `min = 0` 的选项共享一个 `rejoin`。
- State、Input 与 Commit 是 kernel 信封中的规范 CBOR 对象，只含 ID 与标量。`decode_state` 与
  `decode_input` 是接受外部对象的唯一途径；恢复不重放，`Session::verify_path` 按需从根重放审计。
- view 只给出引用、参数值与别名（加载名字表时）；呈现键是 `(execution, commit, occurrence)`。
- `r1::migrate_save` 把 R1 存档重放到由同一 R1 构件迁移而来的 R2 构件上，并逐步比较快照。

`crates/narrata-nodes-wasm` 是浏览器绑定：打开打包构件、选择、回退、导出与恢复，以及本地内容方。
JSON Schema 在 [schemas](schemas)，由 `cargo run -p narrata-node-tools -- schemas --out packages/narrata/nodes/schemas` 生成。

验证：`cargo test -p narrata-nodes`。

# Narrata Nodes

R0/R1 的类型化节点与 Gamebook 执行包。它不需要旧对白 VM、SceneState 或图形宿主。

`crates/narrata-nodes` 提供作者 Bundle、NodeRegistry、compile、分析图以及 Session。
`compile` 校验显式端口绑定、类型、内容引用和返回结果后才返回 CheckedProduct。

```rust
use std::sync::Arc;
use narrata_nodes::{Bundle, NodeRegistry, Session, compile, parse_json};

fn open(source: &str) -> narrata_nodes::Result<Session> {
    let bundle: Bundle = parse_json(source)?;
    let compiled = compile(bundle, &NodeRegistry::gamebook())?;
    Session::new(Arc::new(compiled.product))
}
```

每次调用子图创建独立局部状态；参数只读，共享字段显式声明。内容模板支持
`{{parameter.name}}`、`{{local.name}}`、`{{shared.name}}`。整数为 i64，表达式支持
比较、布尔运算以及检查溢出的加减。隐藏与禁用的选择分别处理。

Session 支持 select、checkout、save、restore；保存包含完整逻辑 Snapshot 和分支历史，
恢复重新校验。格式为有界 alpha profile，预算、语义与旧格式边界见
[ADR 0011](../../../docs/adr/0011-r1-node-composition.md)。

验证：`cargo test -p narrata-nodes`。

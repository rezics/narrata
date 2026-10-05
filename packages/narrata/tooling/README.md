# Narrata node tooling

`narrata-node-tools` 负责文件读取、项目组合、ID 铸造、R1 迁移和 CLI，不参与确定性运行。
独立二进制为 `narrata-book`；旧 `narrata` CLI 通过 `gamebook` 子命令接入同一实现。
`narrata-content-local` 是本地内容方的参考实现：读取内容包、批量解析内容引用、格式化命名参数并
导出内容大纲。它是宿主侧组件，节点运行时不依赖它。

源包路径必须位于项目 manifest 的实际目录闭包内；拒绝绝对路径、父目录穿越和解析后逃逸的符号链接。

```powershell
cargo run -p narrata-node-tools -- ids products/gamebook-demo/project.json
cargo run -p narrata-node-tools -- compose products/gamebook-demo/project.json --out products/gamebook-demo/story.narpack --locked
cargo run -p narrata-node-tools -- inspect products/gamebook-demo/story.narpack
cargo run -p narrata-node-tools -- run products/gamebook-demo/story.narpack --content products/gamebook-demo/content/zh-Hans.json --actions ledger,sign,candle+flask
cargo run -p narrata-node-tools -- schemas --out packages/narrata/nodes/schemas
```

- `ids` 为缺少 `id` 的节点、选择点和选项铸造 UUIDv7 并写回源稿；`compose` 从不铸造。
- `compose` 与 `--out` 处已有的构件比较：为消失的 ID 追加墓碑并写回包源稿，删掉旧墓碑或改变存活
  ID 的归属是错误。`--locked` 校验 `project.lock.json`，需要追加墓碑时失败，且不改写 lock。输出
  旁写 `<out>.analysis.json`（无文字的图分析）。
- `run` 的动作用选项别名，逗号分隔各步，`+` 连接多选，`~` 表示不选。给出 `--content` 时打印解析后
  的文字，否则打印无文字的 book view JSON。
- `migrate-r1 <R1 project.json> --out <目录>` 把 R1 作品转换为 R2 源稿与本地内容包；
  `migrate-r1 <story.narpack> --save <R1 存档> --content <内容包> --out <导出>` 迁移 R1 存档。

生成文件单独原子替换；这不是多文件事务。lock 校验发生在写输出之前。

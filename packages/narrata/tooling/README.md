# Narrata node tooling

`narrata-node-tools` 负责文件读取、项目组合、lock 校验和 CLI，不参与确定性运行。
独立二进制为 `narrata-book`；旧 `narrata` CLI 通过 `gamebook` 子命令接入同一实现。

源包路径必须位于项目 manifest 的实际目录闭包内；拒绝绝对路径、父目录穿越和解析后逃逸的符号链接。
作品输出把所选包内联，可在原 checkout 不存在时重新校验并运行。

```powershell
cargo run -p narrata-node-tools -- compose products/gamebook-demo/project.json --out products/gamebook-demo/story.nar.json --locked
cargo run -p narrata-node-tools -- inspect products/gamebook-demo/story.nar.json
cargo run -p narrata-node-tools -- schemas --out packages/narrata/nodes/schemas
```

生成文件单独原子替换；这不是多文件事务。lock 校验发生在写输出之前。

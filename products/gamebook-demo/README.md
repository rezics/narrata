# 山口来信

可组合 Gamebook 的完整短样例：主线、山路事件、营地互动分别由三个内容包提供。
角色名作为子图参数，营地局部状态每次进入重建；干粮、信件和访问计数属于共享状态。

## 编译与运行

在仓库根目录运行：

```powershell
cargo run -p narrata-node-tools -- compose products/gamebook-demo/project.json --out products/gamebook-demo/story.nar.json --locked
cargo run -p narrata-node-tools -- run products/gamebook-demo/story.nar.json
cargo run -p narrata-node-tools -- run products/gamebook-demo/story.nar.json --actions camp,letter,continue,rest,continue,road,help,continue,deliver --save .temp/letter.save.json
cargo run -p narrata-node-tools -- run products/gamebook-demo/story.nar.json --load .temp/letter.save.json
```

现有 CLI 也支持 `cargo run -p narrata-cli -- gamebook <上述命令>`。

`project.lock.json` 记录实际所选包、节点语义与构件摘要。修改源内容后，先不带 `--locked`
重新 compose；核对差异后提交更新的源码、lock、bundle 与 analysis。`--locked` 在输入改变时失败，
不会悄悄更新 lock。该目录包含全部作品源码，复制到仓库外仍能由已安装的 `narrata-book` 编译。

## 编写自己的作品

- 在 `packages/*.json` 添加 content 与 graph。正文、标签和标题均为纯文本。
- 节点 ID 是持久身份，修改内容或显示标题时保持 ID；连接目标引用这些 ID。
- Graph 的 `parameters` 声明只读输入，`locals` 提供每次调用的独立初始值，`shared` 声明该图使用的产品字段。
- `narrata.content` 显示正文后继续；`narrata.decision` 把正文与条件选择放在同一交互中。
- `narrata.call` 调用本包 graph 或声明的 import。`on_return` 必须处理每一个声明结果。
- 用 `product.bindings` 把 import 接到包的 exported graph。符合相同参数/结果契约的内容包可替换。
- 表达式使用 literal/read/not/binary；赋值按声明顺序生效，失败动作整体撤回。
- `visible_if` 控制是否显示；`enabled_if` 控制是否可选择，可配 `disabled_reason`。

JSON Schema 位于 [nodes/schemas](../../packages/narrata/nodes/schemas)。它检查结构，CLI 的
compile 进一步检查类型、引用和端口。`story.nar.json` 是可分发的自包含 JSON 作品，不需要原始文件路径。

R1 尚未包含完整脚本编辑器、远程内容 resolver、业务 Effect 或 Scene/Quest 节点状态扩展。

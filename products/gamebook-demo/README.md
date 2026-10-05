# 山口来信

可组合 Gamebook 的完整短样例：主线、山路事件、营地互动分别由三个内容包提供。
结构在 `packages/*.json`，全部文字在本地内容包 `content/zh-Hans.json`：构件、状态和存档里没有一个字。

- 旅人的称呼是 `ref` 参数：营地子图收到的是内容引用，内容方解析成"你"，换一种语言也不改变任何摘要。
- 驿站的"翻看桌上的登记册"演示局部选择与多选：留名或不留名都先显示回应、再回到同一段正文；
  从木箱里最多带走两样东西（也可以都不带）。原有路线不受影响。
- 营地局部状态每次进入重建；干粮、信件、留名与行囊属于共享状态。

本作品由 R1 版本经 `narrata-book migrate-r1` 转换后手工整理而来，`project.json` 的
`migrated_from_r1` 记录了 R1 构件，R1 存档因此可以迁移到这个构件上。

## 编译与运行

在仓库根目录运行：

```powershell
cargo run -p narrata-node-tools -- compose products/gamebook-demo/project.json --out products/gamebook-demo/story.narpack --locked
cargo run -p narrata-node-tools -- run products/gamebook-demo/story.narpack --content products/gamebook-demo/content/zh-Hans.json
cargo run -p narrata-node-tools -- run products/gamebook-demo/story.narpack --content products/gamebook-demo/content/zh-Hans.json --actions camp,letter,continue,rest,continue,road,help,continue,deliver --save .temp/letter.json
cargo run -p narrata-node-tools -- run products/gamebook-demo/story.narpack --content products/gamebook-demo/content/zh-Hans.json --load .temp/letter.json
```

动作是选项的别名：逗号分隔各步，`+` 连接多选（如 `ledger,sign,candle+flask`），`~` 表示一样都不选。
现有 CLI 也支持 `cargo run -p narrata-cli -- gamebook <上述命令>`。

`project.lock.json` 记录所选包的源稿摘要、节点语义与 `artifact_id`。修改源稿后先不带 `--locked`
重新 compose；核对差异后提交源稿、lock、`story.narpack` 与 `story.analysis.json`。`--locked` 在输入
改变或需要追加墓碑时失败，不会悄悄更新 lock。

## 编写自己的作品

- 结构写在包源稿中，文字写在内容包中；源稿用 `{"provider": "local", "key": ...}` 引用文字，正文用
  `{"unit": ..., "first": ..., "last": ...}` 引用内容单元中的一段块。
- 新增节点、选择点或选项时不写 `id`，运行 `narrata-book ids products/gamebook-demo/project.json` 铸造。
  ID 是持久身份：改别名、改文字都保持 ID；删除后 compose 自动追加墓碑，ID 不再复用。
- `narrata.passage` 显示标题与正文，选择点放在 `placement` 指定的块之后；`local` 结果显示 `reply` 后
  从 `rejoin` 继续，`branch` 结果进入本图另一个节点。段落走到末尾时继续到 `next`。
- 内容包里的 `{名字}` 由段落的 `args` 填充，`ref` 值先解析成文字；`{{`、`}}` 是字面括号。
- Graph 的 `parameters` 声明只读输入，`locals` 提供每次调用的独立初始值，`shared` 声明该图使用的产品字段。
- `narrata.call` 调用本包 graph 或声明的 import，`on_return` 必须处理每一个声明结果；用
  `product.bindings` 把 import 接到包的 exported graph。
- `visible_if` 控制是否显示；`enabled_if` 控制是否可选择，可配 `reason`。

JSON Schema 位于 [nodes/schemas](../../packages/narrata/nodes/schemas)。它检查结构，compose 进一步
检查类型、引用、端口与段落规则。`story.narpack` 是可分发的单文件构件，不需要原始文件路径。

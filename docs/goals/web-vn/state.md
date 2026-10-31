# State

2026-10-31：G-041 原生 VN0 执行与双格式兼容实现已合入主线提交 `f1adb040de40fc2b7b1f973afb64d562475343e5`（实现提交 `be83242`），G5、R1、Web、scripts、docs 五项主线 gate 均已通过，VN0 已验收。G-041 已验证归档并保留任务分支；VN1 的 [G-042](tasks/G-042.md) 已从 `6bd5c76` 派发给 gpt-6-luna/max。

## 已验收基线

Gamebook 完整固定包保持不变：源码 `a36d4966d7bf40df9e03f38e5d080885d73595ff`，版本 `0.1.0-alpha.1`，tarball SHA-256 `0a1ed393ab8b60ad1400609fcd26b0a060911917fce88169efc459d25819da30`；见[固定输入](../../integrations/rezics.md#narrata-自己的交付依赖)。G-041 保留 Gamebook format 2、schema 1、revision 2、`BookView` 与现有兼容语料；VN0 双格式读取证据新增于 `fixtures/compat/nodes-vn-v1/`。实现字段、编码与 schema 细节由代码和 corpus 承载。

## 下一步与未交付结果

G-042 正在实现 Wasm/npm `@rezics/narrata/vn` 有类型 API、逻辑块按需加载、存档确认与安装包消费者验证。播放器 UI、媒体 resolver/cache、VN 作者 API 和短篇产品由后续任务交付。VN1 的播放器和浏览器旅程仍必需；VN2 的按需剧情/媒体交付与 VN3 的作者、模拟宿主、多场景短篇、固定包和安装验收也都未交付。完整验收范围见 [GOAL.md](GOAL.md)。

manager 为 `codex-01a187ba-9af7-71c1-b110-0ee55db54795-web-vn`。worker 使用 `engine: codex`、`effort: max`，环境设置 `GOAL_ID=web-vn`、`GOAL_ENGINE=codex`、`GOAL_CODEX_MODEL=gpt-6-luna`；实时派发、认领与进程以 `task goal -- status` 为准。

REZICS 的媒体、授权和上传能力仍只列为[宿主能力请求](../../integrations/rezics.md#web-vn-后续需要的宿主能力)。七项 Gamebook 请求仍为“提出”，npm registry 未发布，真实 REZICS 联合验收尚未执行；单仓库 VN0 验收不代表 VN 产品上线。

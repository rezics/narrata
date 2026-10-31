# State

2026-10-31：VN0 已验收；原生源码 `f1adb040de40fc2b7b1f973afb64d562475343e5` 通过五项本地 gate，等价代码的[主线 CI](https://github.com/rezics/narrata/actions/runs/41204472183)八项成功。VN1 的 [G-042](../../../archive/goals/web-vn-2026-10-31/tasks/G-042.md) 阅读 API 已在主线 `e81a492d5462e97cf29bbcede232c73697e81b4c` 通过 R1、Web、scripts、docs；R1 浏览器 28 项、Web 单元 147 项及独立 Node/Bun/Chromium 消费者通过。播放器、媒体呈现和完整 VN2–VN3 尚未交付。

## 已验收基线

Gamebook 完整固定包保持不变：源码 `a36d4966d7bf40df9e03f38e5d080885d73595ff`，版本 `0.1.0-alpha.1`，tarball SHA-256 `0a1ed393ab8b60ad1400609fcd26b0a060911917fce88169efc459d25819da30`；见[固定输入](../../integrations/rezics.md#narrata-自己的交付依赖)。G-041 保留 Gamebook format 2、schema 1、revision 2、`BookView` 与现有兼容语料；VN0 双格式读取证据新增于 `fixtures/compat/nodes-vn-v1/`。实现字段、编码与 schema 细节由代码和 corpus 承载。

VN 阅读 API 切片的固定包版本同为 `0.1.0-alpha.1`，SHA-256 为 `11db5e10fbe1a6319ae921fb90e1d30bcc00678c5d679bf9886dc078f5a13299`，1,780,389 字节；主线与工作分支验收包逐字节相同。公共 View v1 的四种状态样例在 `fixtures/compat/vn-view-v1/`，由独立安装包分别经 Node/Bun 生成并比对；既有 365 个兼容文件与 Gamebook schema 未改。日志、包与消费证据在 `.temp/goal/runs/G-042/main-acceptance/e81a492/`、`.temp/delivery/vn-reader-release.json`。

## 下一步与未交付结果

下一项 [G-043](tasks/G-043.md) 交付桌面/手机参考播放器、受检媒体 callback、图层与音轨协调、资源失败恢复和真实浏览器旅程，简报已准备。已审阅概念与独立素材位于 `.temp/vn-player-design/`；立绘以实际 alpha/浏览器合成核对，不能把工具预览中的透明 RGB 当成已渲染背景。VN2 的有界候选预取/按需媒体交付与 VN3 的作者 API、即预览、多场景短篇及完整固定包仍必需；完整范围见 [GOAL.md](GOAL.md)。

manager 为 `codex-01a187ba-9af7-71c1-b110-0ee55db54795-web-vn`。worker 使用 `engine: codex`、`effort: max`，环境设置 `GOAL_ID=web-vn`、`GOAL_ENGINE=codex`、`GOAL_CODEX_MODEL=gpt-6-luna`；实时派发、认领与进程以 `task goal -- status` 为准。

REZICS 的媒体、授权和上传能力仍只列为[宿主能力请求](../../integrations/rezics.md#web-vn-后续需要的宿主能力)。七项 Gamebook 请求仍为“提出”，npm registry 未发布，真实 REZICS 联合验收尚未执行；单仓库 VN0 验收不代表 VN 产品上线。

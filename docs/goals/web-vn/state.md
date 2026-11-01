# State

2026-11-01：VN0 与 VN1 的阅读 API、可恢复参考播放器和类型化媒体呈现已通过本地主线验收。原生作者基础、依赖投影和 Wasm 作者桥接已具备；完整 VN2–VN3 尚未交付。

## 已验收基线

VN0 原生源码 `f1adb040de40fc2b7b1f973afb64d562475343e5` 通过五项本地 gate，等价代码的[主线 CI](https://github.com/rezics/narrata/actions/runs/41204472183)八项成功。VN1 阅读 API 的固定源码为 `e81a492d5462e97cf29bbcede232c73697e81b4c`；其 R1、Web、scripts、docs 和独立消费证据保留。

Gamebook 完整固定包保持不变：源码 `a36d4966d7bf40df9e03f38e5d080885d73595ff`，版本 `0.1.0-alpha.1`，tarball SHA-256 `0a1ed393ab8b60ad1400609fcd26b0a060911917fce88169efc459d25819da30`；见[固定输入](../../integrations/rezics.md#narrata-自己的交付依赖)。G-041 保留 Gamebook format 2、schema 1、revision 2、`BookView` 与现有兼容语料；VN0 双格式读取证据新增于 `fixtures/compat/nodes-vn-v1/`。实现字段、编码与 schema 细节由代码和 corpus 承载。

VN 阅读 API 切片的固定包版本同为 `0.1.0-alpha.1`，SHA-256 为 `11db5e10fbe1a6319ae921fb90e1d30bcc00678c5d679bf9886dc078f5a13299`，1,780,389 字节；主线与工作分支验收包逐字节相同。公共 View v1 的四种状态样例在 `fixtures/compat/vn-view-v1/`，由独立安装包分别经 Node/Bun 生成并比对；既有 365 个兼容文件与 Gamebook schema 未改。日志、包与消费证据在 `.temp/goal/runs/G-042/main-acceptance/e81a492/`、`.temp/delivery/vn-reader-release.json`。

等价阅读 API 源码的[主线 CI](https://github.com/rezics/narrata/actions/runs/41213302899)已在 `c701806` 八项成功；该提交与 `e81a492` 的包源码和兼容语料一致。

VN3 的原生作者基础 [G-045](../../../archive/goals/web-vn-2026-10-31/tasks/G-045.md) 已在主线 `fd00b31c7c9223fa1dc5908a8c77158b78880995` 通过 R1（28 项浏览器回归）与文档检查，等价代码的[主线 CI](https://github.com/rezics/narrata/actions/runs/41276885118)八项成功。9 项定向原生测试覆盖同候选、方言、语义诊断、修改隔离和删除墓碑。验收记录位于 `.temp/goal/runs/G-045/manager-acceptance/`。

[G-044](../../../archive/goals/web-vn-2026-10-31/tasks/G-044.md) 的有界只读增量依赖投影与 [G-046](../../../archive/goals/web-vn-2026-10-31/tasks/G-046.md) 的原始 Wasm 作者桥接，已在同一主线 `2ab426777130512e6fe35f81d10567c612b5a7fa` 通过 R1、Web 和文档检查：R1 浏览器 28 项，Web 单元 147 项，独立安装消费者 25 项及 Node/Bun/Chromium 旅程通过，包体积限制未放宽。原生投影有 17 项定向测试；Wasm 桥接的实际 Node/Bun 调用生成与冻结语料逐字节相同的包，修改后候选有效且旧候选保持不变。既有兼容语料和交换格式未改。固定源码与失败/重试记录见 `.temp/goal/runs/G-046/main-acceptance/2ab4267-retry-cached-registry/` 及两项任务的 `manager-acceptance/`。

[G-043](../../../archive/goals/web-vn-2026-10-31/tasks/G-043.md) 的播放器切片已在主线 `b5c8457630177173ea233a569bb0eb5c7f845e8d` 通过 R1、Web、VN、scripts、docs 五项检查：R1 浏览器 28 项，Web 单元 168 项，独立安装 25 项，VN 桌面/手机旅程 13 项；其中 21 项播放器测试覆盖资源与音频生命周期、保存边界和重试。最终桌面、手机及重试截图与已审阅图像逐字节相同。CLI 网络重试后由 manager 保留工作区并完成提交与集成；旧失败与检查记录保留。

固定播放器包为 `0.1.0-alpha.1`，tarball SHA-256 `4c77f0511fe1239c22fc6685e08fa8c84e0ca6e4e6b746abad169eda7861c649`，1,796,713 字节；见 `.temp/delivery/vn-player-release.json`。主线验收记录位于 `.temp/goal/runs/G-043/main-acceptance/b5c8457-retry-audit-tls/`。此包和旧 Gamebook、阅读 API 包身份分别保留，不能互换；播放器 CI 待本次推送后核对。

播放器首轮 CI 七项成功，Ubuntu 的音频不可用手机状态因字体度量导致工具栏超界约 3px。`3ae7824` 修正参考 UI 的文字换行，并增加更宽字距的边界覆盖；定向 2 项与完整 VN 13 项本地通过，SDK 固定包未变，等待修复后的 CI。

## 下一步与未交付结果

已验收的底层基础仍不等于完整 VN2/VN3：还需公共依赖桥接、有界候选媒体准备与缓存生命周期、A/B 按需分发证据、Web/Node/Bun 作者封装、实际编辑预览、多场景短篇和最终固定包交付。

下一项 [G-047](tasks/G-047.md) 使用 gpt-6-luna/high，交付公开 VN 作者入口、独立安装的 Node/Bun/浏览器同候选预览，以及基于公开 server 入口的参考构建脚本。编辑控件与完整候选预加载仍是后续结果。

并行的 [G-048](tasks/G-048.md) 使用 sonnet/high，限定于版本化依赖 Wasm 接口、ADR 0025、独立 schema 与新兼容语料；其查询需保持普通程序请求和存档状态不变。公开 TS 桥接和完整预加载仍待后续消费者实现。

manager 为 `codex-01a187ba-9af7-71c1-b110-0ee55db54795-web-vn`。模型与 effort 按任务简报选择，Codex worker 优先使用 gpt-6-luna/high 或 max；实时派发、认领与进程以 `task goal -- status` 为准。

REZICS 的媒体、授权和上传能力仍只列为[宿主能力请求](../../integrations/rezics.md#web-vn-后续需要的宿主能力)。七项 Gamebook 请求仍为“提出”，npm registry 未发布，真实 REZICS 联合验收尚未执行；单仓库 VN0 验收不代表 VN 产品上线。

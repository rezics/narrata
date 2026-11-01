# State

2026-11-01：VN0 与 VN1 已验收，播放器修复后的 CI 八项成功。公开 VN 作者 API、Node/Bun 同候选构建、版本化依赖 Wasm 接口及编辑与隔离即时预览也已通过主线验收；完整资源准备、参考宿主接线和最终独立安装包交付尚待完成。

## 已验收基线

VN0 原生源码 `f1adb040de40fc2b7b1f973afb64d562475343e5` 通过五项本地 gate，等价代码的[主线 CI](https://github.com/rezics/narrata/actions/runs/41204472183)八项成功。VN1 阅读 API 的固定源码为 `e81a492d5462e97cf29bbcede232c73697e81b4c`；其 R1、Web、scripts、docs 和独立消费证据保留。

Gamebook 完整固定包保持不变：源码 `a36d4966d7bf40df9e03f38e5d080885d73595ff`，版本 `0.1.0-alpha.1`，tarball SHA-256 `0a1ed393ab8b60ad1400609fcd26b0a060911917fce88169efc459d25819da30`；见[固定输入](../../integrations/rezics.md#narrata-自己的交付依赖)。G-041 保留 Gamebook format 2、schema 1、revision 2、`BookView` 与现有兼容语料；VN0 双格式读取证据新增于 `fixtures/compat/nodes-vn-v1/`。实现字段、编码与 schema 细节由代码和 corpus 承载。

VN 阅读 API 切片的固定包版本同为 `0.1.0-alpha.1`，SHA-256 为 `11db5e10fbe1a6319ae921fb90e1d30bcc00678c5d679bf9886dc078f5a13299`，1,780,389 字节；主线与工作分支验收包逐字节相同。公共 View v1 的四种状态样例在 `fixtures/compat/vn-view-v1/`，由独立安装包分别经 Node/Bun 生成并比对；既有 365 个兼容文件与 Gamebook schema 未改。日志、包与消费证据在 `.temp/goal/runs/G-042/main-acceptance/e81a492/`、`.temp/delivery/vn-reader-release.json`。

等价阅读 API 源码的[主线 CI](https://github.com/rezics/narrata/actions/runs/41213302899)已在 `c701806` 八项成功；该提交与 `e81a492` 的包源码和兼容语料一致。

VN3 的原生作者基础 [G-045](../../../archive/goals/web-vn-2026-10-31/tasks/G-045.md) 已在主线 `fd00b31c7c9223fa1dc5908a8c77158b78880995` 通过 R1（28 项浏览器回归）与文档检查，等价代码的[主线 CI](https://github.com/rezics/narrata/actions/runs/41276885118)八项成功。9 项定向原生测试覆盖同候选、方言、语义诊断、修改隔离和删除墓碑。验收记录位于 `.temp/goal/runs/G-045/manager-acceptance/`。

[G-044](../../../archive/goals/web-vn-2026-10-31/tasks/G-044.md) 的有界只读增量依赖投影与 [G-046](../../../archive/goals/web-vn-2026-10-31/tasks/G-046.md) 的原始 Wasm 作者桥接，已在同一主线 `2ab426777130512e6fe35f81d10567c612b5a7fa` 通过 R1、Web 和文档检查：R1 浏览器 28 项，Web 单元 147 项，独立安装消费者 25 项及 Node/Bun/Chromium 旅程通过，包体积限制未放宽。原生投影有 17 项定向测试；Wasm 桥接的实际 Node/Bun 调用生成与冻结语料逐字节相同的包，修改后候选有效且旧候选保持不变。既有兼容语料和交换格式未改。固定源码与失败/重试记录见 `.temp/goal/runs/G-046/main-acceptance/2ab4267-retry-cached-registry/` 及两项任务的 `manager-acceptance/`。

[G-043](../../../archive/goals/web-vn-2026-10-31/tasks/G-043.md) 的播放器切片已在主线 `b5c8457630177173ea233a569bb0eb5c7f845e8d` 通过 R1、Web、VN、scripts、docs 五项检查：R1 浏览器 28 项，Web 单元 168 项，独立安装 25 项，VN 桌面/手机旅程 13 项；其中 21 项播放器测试覆盖资源与音频生命周期、保存边界和重试。最终桌面、手机及重试截图与已审阅图像逐字节相同。CLI 网络重试后由 manager 保留工作区并完成提交与集成；旧失败与检查记录保留。

固定播放器包为 `0.1.0-alpha.1`，tarball SHA-256 `4c77f0511fe1239c22fc6685e08fa8c84e0ca6e4e6b746abad169eda7861c649`，1,796,713 字节；见 `.temp/delivery/vn-player-release.json`。主线验收记录位于 `.temp/goal/runs/G-043/main-acceptance/b5c8457-retry-audit-tls/`。此包和旧 Gamebook、阅读 API 包身份分别保留，不能互换。

播放器首轮 CI 七项成功，Ubuntu 的音频不可用手机状态因字体度量导致工具栏超界约 3px。`3ae7824` 修正参考 UI 的文字换行，并增加更宽字距的边界覆盖；定向 2 项与完整 VN 13 项本地通过，SDK 固定包未变。[修复后的 CI](https://github.com/rezics/narrata/actions/runs/41292011583)八项成功。

[G-048](../../../archive/goals/web-vn-2026-10-31/tasks/G-048.md) 的依赖 Wasm 接口在 `d71fa706d7179341d1d21ef713604a4c43cf8d22` 通过主线 R1、Web、docs，Node/Bun 实际 Wasm 各回放 8 组新语料，结果逐字节一致；旧语料未改。ADR 0025 和 `vn-dependencies-v1` 冻结格式已建立。该接口仍需在 VN reader 构建显式启用功能开关，并接入公开 TS 和资源准备；未宣称完整 VN2。

[G-047](../../../archive/goals/web-vn-2026-10-31/tasks/G-047.md) 在 `c0d3ddfdd8e46f15a68c4296fb954e2a2def3408` 通过 R1、Web、VN、scripts、docs：171 项包测试、25 项独立安装测试、28 项 R1 浏览器和 13 项 VN 旅程。公开 `./vn/authoring`、`./vn/server` 已验证 Node/Bun 与浏览器的同候选编译和打开，参考发布脚本使用公开入口并复现原构件、对象、lock 和 pack。固定作者 API 切片为 `0.1.0-alpha.1`、SHA-256 `672601eec2733cece12c5aeb62d8c70ebb729688354e80bc7ed453586c33ece7`，1,815,163 字节；见 `.temp/delivery/vn-authoring-release.json` 和对应主线验收目录。编辑控件的验收见下文作者预览记录。

## 下一步与未交付结果

已验收的能力仍不等于完整 VN2/VN3：还需公共依赖桥接、有界候选媒体准备与缓存生命周期、A/B 按需分发证据，以及参考宿主接线和多场景短篇的最终固定包联合旅程。

[G-050](../../../archive/goals/web-vn-2026-10-31/tasks/G-050.md) 的草稿编辑与隔离即时预览已在 `b2f2efd6c08b9d1fe6d6e83d0ca1972c968464e8` 通过主线 VN 16 项（原阅读 13 项、作者 3 项）及文档检查。桌面/390px 编辑真实宿主正文和说话者、分支结局、资源失败恢复、正式存档不变均有浏览器证据；未完成预览在延迟媒体释放前关闭，晚到结果不能重装。已审阅截图，并补上无宿主结尾标题时的可见结束状态。源码复核与证据见 `.temp/goal/runs/G-050/manager-acceptance/`。

参考宿主源码 `18ea5b3e30f268806bc8cbca18aa7ddd5e6cf8df` 已复制到仓库外，安装上述固定作者 API tarball；检查确认 SDK 实际路径位于独立消费者内。Node/Bun 经公开 `./vn/server` 重新生成相同短篇 pack，实际宿主构建与全部 16 项桌面/手机编辑、播放、保存恢复及失败重试旅程通过。该记录证明当前作者/播放器切片可独立消费，最终候选预加载仍需用最终固定包重跑；见 `.temp/goal/runs/vn-reference-installed/author-preview-baseline-esm/receipt.json`。

编辑预览首轮 [CI](https://github.com/rezics/narrata/actions/runs/41299837049) 的 Ubuntu VN 旅程有 14 项通过、2 项手机工具栏内容溢出失败，作者三项均通过。`27274c1619fc747dcfb280888fcbbad19c0b6c4f` 保留 44px 触控目标，在手机使用带无障碍名称的历史图标，并覆盖 17px 大字号；完整 VN 16 项和仓库外固定包旅程 16 项均再次通过。独立验收还核对 Node/Bun 发布元数据及预览前后全部发布文件哈希一致；见 `.temp/goal/runs/vn-reference-installed/author-preview-mobile-fix/receipt.json`。[修复 CI](https://github.com/rezics/narrata/actions/runs/41301346061) 八项成功。

[G-049](../../../archive/goals/web-vn-2026-10-31/tasks/G-049.md) 的公开依赖桥与有界候选准备 SDK 已在主线 `2745b876a66d079ff8ecd573239c27e8fb3a1d3e` 通过 R1、Web、VN、scripts、docs 五项检查：192 项包测试、25 项独立故障用例、49 项定向 VN 测试、28 项 R1 浏览器、16 项参考 VN 旅程及 53 项脚本测试通过。实际 Node/Bun 两种 Wasm profile 各回放 8 组冻结语料；原单参数调用保留，旧 Gamebook 基线与预算上限保持不变。独立 SDK 浏览器在准备阶段取得 217 字节冷图块，进入目标不重复请求已准备媒体，11 个模拟媒体 lease 全部释放，未来音频播放次数为零。Node/Bun 外部 realm 缓冲区、iframe URL 与源突变检查通过。固定源码审查和完整日志见 `.temp/goal/runs/G-049/manager-acceptance/` 与对应 `main-acceptance/2745b87/`。

该 SDK 切片为 `0.1.0-alpha.1`、SHA-256 `2fae09fba6fffcbfefb395c524a030ed6d0c320634035586e99167574307344e`、2,275,288 字节，见 `.temp/delivery/vn-preparation-release.json`；与旧切片身份分别保留，CI 待推送后核对。CLI 工作保留后由 manager 完成释放、实时预算、源身份及集成修正。最终参考宿主接线、实际媒体和 VN A/B 联合旅程、完整固定包交付由 [G-051](tasks/G-051.md) 继续，尚未宣称完整 VN2/VN3。

独立参考宿主的按需分发基线已在 `b2e40d99df320939fad87f1e48435ae990b71d3e` 测量：首屏 1 个 1,040 字节剧情块与 539 字节清单，背景、两个人物及当前音乐/语音共 8,816,923 字节；仅加载 1 个 reader Wasm，作者编译器保持懒加载。进入大厅后才增加 337 字节剧情块和大厅背景请求。该记录使用固定作者 API 切片，尚未包含候选预加载；原始响应身份、哈希与分阶段测量见 `.temp/goal/runs/vn-reference-installed/author-player-distribution-baseline/distribution.json`，最终接线由 [G-051](tasks/G-051.md) 验收。

manager 为 `codex-01a187ba-9af7-71c1-b110-0ee55db54795-web-vn`。模型与 effort 按任务简报选择，Codex worker 优先使用 gpt-6-luna/high 或 max；实时派发、认领与进程以 `task goal -- status` 为准。

REZICS 的媒体、授权和上传能力仍只列为[宿主能力请求](../../integrations/rezics.md#web-vn-后续需要的宿主能力)。七项 Gamebook 请求仍为“提出”，npm registry 未发布，真实 REZICS 联合验收尚未执行；单仓库 VN0 验收不代表 VN 产品上线。

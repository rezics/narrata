# Nix/NixOS 与 Nickel 对 Narrata 的启发

状态：已决定
日期：2026-09-01

## 结论

Nix/NixOS 对 Narrata 的时间旅行和存档架构有直接启发，但不应成为 runtime 依赖。Nickel
适合作为**可选的编译期配置前端**，用于组合和校验 manifest、capability、host profile
或作者工具配置；它不适合执行剧情状态机，也不应读取或迁移玩家存档。

| 能力 | Nix/NixOS | Nickel | Narrata 决定 |
| --- | --- | --- | --- |
| 不可变历史 | store object + generation | 不提供历史存储 | 采用不可变 `Commit` 与可变 `Ref` |
| 快速 rollback | profile 指针切换 | 不提供 | 移动 cursor/ref，不反向执行 |
| 分支时间线 | profile 基本为线性 generation | 无时间概念 | 增加单父提交 DAG 与 branch ref |
| 依赖保留 | GC root 保留 closure | package lock 仍实验 | save root 固定 program/content closure |
| 声明式组合 | NixOS typed option merge | 对称 record merge + metadata | 仅用于 authoring/config，不用于事件顺序 |
| 数据校验 | module option type | gradual type + lazy contract | runtime 边界仍由 Rust parser/schema 验证 |
| 外部 mutable state | NixOS rollback 明确不覆盖 `/var` | 不处理外部状态 | 外部 effect ledger 不随剧情回滚 |

## 从 Nix/NixOS 采用的模式

### 1. 对象、generation 与当前指针分离

Nix 先产生新的不可变 store object，再创建 generation，最后原子切换 profile 的链接。
Rollback 只是把链接移回旧 generation，而不是把一次安装逐文件反向执行。官方
[Profiles 手册](https://nix.dev/manual/nix/stable/package-management/profiles)和
[`profiles.cc`](https://github.com/NixOS/nix/blob/4750701db3802868445276c1a09c2f065a5a4bc6/src/libstore/profiles.cc)
都体现了这个分层。

Narrata 对应为：

```text
immutable RuntimeState blob
        ↓
immutable Commit(parent, state, program, cause)
        ↓
mutable Ref(name, revision, commit_id)
```

创建存档永远先写对象和 commit，确认完整后才以 compare-and-swap 更新 Ref。崩溃最多留下
尚未引用的对象，不能让 Ref 指向半个存档。

### 2. 回滚能力来自 root 与 closure

Nix 的 GC root 不仅保留 root 本身，还保留它引用的依赖闭包。旧 generation 一旦删除，
对应对象才有资格被 GC。

Narrata 的 root 包括：

- 用户 save slot；
- timeline branch head 与当前 active cursor；
- 明确 bookmark；
- 完整时间线启用后的 Catalog Head/Archive root；
- 尚未结束的 external effect ledger entry；
- 有期限的 debugger/autosave pin。

从这些 root 必须遍历 `Commit → parent/state/program/transition/content lock`；Catalog/Archive
root 还必须遍历目录事件和其中引用的所有 Commit。只保留 snapshot 而清掉其精确 Program
Artifact，会制造无法恢复的“有效存档”。

### 3. 临时对象到永久 root 的交接必须与 GC 同步

Nix `createGeneration` 的实现专门防止 GC 在 temporary root 转成 permanent root 的窗口中
删掉新构件。Narrata 的等价约束是：

- SQLite/IndexedDB adapter 在一个事务里写对象、commit 并更新 Ref；或
- 非事务对象存储先建立有租约的 temporary pin，写完全部对象后 CAS Ref，最后释放 pin。

GC 与 Ref 更新不能各自“最终一致”而没有交接协议。

### 4. exact identity 与语义身份不能混用

Nix store path 是不可变对象的 opaque identity；具体 digest 规则依 store object 类型而异，
不能简单概括成“所有路径都是文件内容哈希”。Narrata 需要两个独立命名空间：

- `StableId`：作者语义身份，跨编辑和重编译尽量保持，例如 `FlowId`、`InstructionId`；
- `ArtifactId` / `ObjectId`：某一份精确字节或规范化结构的加密摘要。

前者用于迁移和 continuation relocation，后者用于完整性、去重和锁定精确版本。即使底层
都是 32 字节，也必须使用不同 newtype，禁止互换。

### 5. 依赖锁必须进入 Program Artifact

Flake lock 固定依赖图的精确输入。Narrata 同样需要两层锁：`BuildProvenance` 固定 source、
compiler、导入模块与可选 Nickel evaluator/package；runtime `ContentLock` 固定尚未完全嵌入
构件的 capability schema 和外部内容 revision。存档引用精确 `ProgramArtifactId`，而不是
只保存 `1.2.0` 这样的展示版本。

### 6. NixOS 的限制正好说明 host state 边界

NixOS 官方说明 rollback 可以回到旧配置，但 `/var` 等 mutable state 不会随之恢复。这与
Narrata 完全同构：剧情状态可以回到旧 Commit，已经写入服务器、扣除库存或完成支付的事实
不会自动消失。

因此不可逆效果的 ledger 必须单调地存在于时间线之外，并选择以下策略之一：

- 回滚屏障：玩家不能越过；
- 宿主提供同一 `EffectId` 的幂等去重；
- 宿主提供显式 compensation，但 compensation 是新的外部事实，不是假装旧事实未发生。

### 7. NixOS module system 的诊断比“最后一个覆盖”更值得借鉴

NixOS option type 决定如何合并定义；相同优先级不能合并时给出定义来源。Narrata 的作者
配置也应保存每个 definition 的 source span，并只在 schema 明确允许时执行 merge。
Stable ID、transition target 和 capability version 的冲突必须报错，不能使用文件顺序静默
覆盖。

## Nickel 可以承担的工作

Nickel 的优势不是存档，而是声明式配置的组合与校验：

- record merge 是对称的；相同优先级冲突会失败，`default`、数字 priority 与 `force` 显式
  表达覆盖意图；
- field metadata 可以同时携带 contract、文档、optional、priority 与 `not_exported`；
- partial record 可以先保留缺失定义，合并完整后再 export；
- recursive record 会在输入被覆盖后重新计算依赖字段；
- 当前仓库有面向嵌入的 `nickel-lang` Rust facade，可深度求值并转换成 Serde 类型。

这些能力适合：

```text
Narrata host profile
capability manifest
compiler/package options
editor defaults and policy
test scenario matrix
deployment-specific content resolver config
```

建议的边界是：

```text
.ncl sources + pinned imports
          ↓  isolated build-time evaluation
deep evaluation + all contracts forced
          ↓
Rust AuthoringManifest parser/validation
          ↓
canonical Narrata Program Artifact
```

Nickel export 不是权威 Schema。权威数据契约由 Narrata 维护，并生成或契约测试 Nickel
contract，防止两份手写定义漂移。

## Nickel 不应承担的工作

### 不执行 runtime 状态迁移

Nickel merge 是对称、无时间顺序的配置组合。状态机 transition 则有明确的前态、输入、
顺序、失败和后态。把 `RuntimeState` merge 成下一状态会丢失因果关系，也无法表达 effect
提交和 run-to-completion。

### 不直接验证不可信存档

Nickel contract 是 lazy runtime check。未被访问或 export 的字段可能尚未检查；Nickel 文档
也明确区分静态 type 与延迟 contract。存档 decoder 必须先执行大小、深度、版本、枚举和
引用完整性检查，再构造 Rust 的可信 domain type，不能把“Nickel 程序能求值”当作恢复证明。

### 不进入游戏 runtime 或 FFI 热路径

Nickel 是 Turing-complete；作者代码可能不终止或消耗过多资源。package management 当前也
仍标为实验。若采用，只在 CLI/构建进程中运行，固定 evaluator 版本、限制 import root、
时间、内存与输出大小；native/Wasm runtime 不链接 Nickel。

### 不用 `force` 掩盖身份冲突

`force` 很适合部署 policy，但不适合两个同名 Flow、同一 Stable ID 的不同节点或互斥迁移
规则。这些冲突必须由 Narrata compiler 显式拒绝。

## 与 Nix 不同、Narrata 必须新增的机制

1. **分支 DAG**：Nix profile generation 通常是线性的；从旧剧情点继续会产生新未来。
2. **完整 continuation**：存储 instruction、frame、queue、RNG、等待交互，而不只是构件。
3. **Transition Receipt**：记录输入、Effect Response 和状态摘要，用于重放校验。
4. **外部效果账本**：不可逆事实不能跟随可回滚 Commit 一起消失。
5. **Program migration**：旧 continuation 指向新 Program 时必须显式 relocation。
6. **安全点**：只能在完整 macrostep、等待交互或完成状态创建持久 Commit。

## 采用决定与验证门槛

### Nix/NixOS

- 作为架构参照和可选开发/发布环境采用；
- 不要求最终用户安装 Nix；
- 不把 `/nix/store` 当玩家存档后端；
- 不把 Nix expression evaluator 放入 runtime。

### Nickel

先执行一个不阻塞 runtime 的 optional spike。满足以下条件才增加 `narrata-nickel` 工具：

1. 一个 `.ncl` capability/host manifest 能经 `eval_deep_for_export` 转换为 Rust 类型；
2. 缺字段、额外字段、错误 Stable ID、超限集合都产生带 source span 的诊断；
3. 同一锁定输入重复构建产生相同的 canonical Program Artifact hash；
4. evaluator 在独立进程中有资源上限，不能拖死 editor/server；
5. Nickel contract 从权威 schema 生成，或有自动 drift test；
6. 关闭 Nickel feature 后，core、save、FFI 与 Wasm 的依赖图完全不包含 Nickel。

任一条件失败都只意味着继续支持 JSON/程序化 Rust manifest，不影响 Narrata 核心交付。

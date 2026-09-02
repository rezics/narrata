# ADR 0010：Stage 5 不发布 Nickel adapter

状态：Accepted

## 背景

可选 spike 复查了固定的 Nickel commit
[`454e0ae37272`](https://github.com/nickel-lang/nickel/commit/454e0ae372721ca7d531c678b042f8ebdafc6e1b)、
`eval_deep_for_export`、contract/merge 语义和仍属实验能力的 package management。深度 export 与
Serde facade 足以证明“可以转换 manifest”，但不等于 Stage 5 optional Gate 全部成立。

## 决策

不创建或发布 `narrata-nickel` crate/feature，继续使用 strict JSON 与 Rust typed manifest。原因是
当前 spike 没有同时证明以下生产约束：

- Windows、macOS、Linux 上统一且可审计的独立进程时间与内存上限；
- import root 之外的文件访问和网络访问被强制禁止；
- 权威 Rust schema 与 Nickel contract 的自动生成或 drift test；
- evaluator/package lock、所有 import 与限制参数完整进入 Build Provenance。

这些是 optional Gate 的必要条件，缺任一项都不能把研究 facade 提升为可发布 frontend。仅靠
线程、超时 future、约定 import 路径或 lazy contract 不能提供所需隔离。

## 后果

- G5 不被阻塞；Runtime、Store、C ABI 与 Wasm 的默认及完整依赖图都不含 Nickel。
- `.ncl` 不是 v1 可重复构建的唯一来源，也不能读取或迁移玩家存档。
- 后续若重新开启 spike，必须作为独立 build-time tool 提交上述四类可执行测试；通过前不得改动
  core canonical model 或 save format。

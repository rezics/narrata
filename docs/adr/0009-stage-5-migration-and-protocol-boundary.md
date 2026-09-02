# ADR 0009：Stage 5 migration 与跨语言协议边界

状态：Accepted

## 决策

存档升级采用显式、可信的 `MigrationDescriptor` 链，而不是让新 Program 猜测旧 continuation。
每一步必须声明 source/target Artifact、可接受的 Snapshot schema、Stable ID relocation 与有损
recovery point。迁移成功产生以 source Commit 为 parent、cause 为 `MigrationId` 的新 Commit；
对象写入和最终 Ref CAS 属于同一事务。失败、歧义路径或缺失映射都不发布 Ref，也不修改旧对象。

Program、Snapshot、Object Envelope 和跨语言 protocol 分别 dispatch 自己的 version。Protobuf
只承载不可信 transport DTO，不进入 Program/Snapshot/Receipt/Commit 的 canonical hash。DTO 必须
经过 Rust checked constructor 才能成为运行时值，unknown required version、超限消息和非法 ID
在边界返回 typed diagnostic。

C ABI 只暴露同步 opaque handle、owned buffer、status code 与显式 free；进程内 registry 串行化
调用并阻止 Rust panic 穿越边界。C# 使用 `SafeHandle` 和 source-generated P/Invoke；Wasm 调用同一
`ProtocolEngine`；TypeScript 与 C# DTO 都从同一 `.proto` 生成。Unity native 适配器只在非 WebGL
平台编译，WebGL 使用独立 `.jslib`/Wasm bridge。

## 后果

- 普通 decoder 升级不等于 Program semantic migration；二者不能用同一个 version 或 fallback。
- pending Effect relocation 会改变幂等身份，必须显式确认；丢弃 continuation、跨 barrier recovery
  也分别需要确认并进入报告。
- 冻结 corpus 是只读兼容证据。修改 expected bytes 必须另立格式 ADR 或 migration，不能批量更新。
- bindings 只包装 pull protocol，不在 C#/JS continuation、callback 或 async task 中保存权威状态。
- IndexedDB adapter 用一个 transaction 写 immutable objects 与 CAS refs；浏览器资源上限在写入前
  检查。

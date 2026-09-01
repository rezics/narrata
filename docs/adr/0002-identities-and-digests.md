# ADR 0002：Identity 与 digest 分离

状态：Accepted

## 决策

作者/Host 提供的 identity 使用不可互换的 16-byte newtype；内容衍生 identity 使用不可互换的
32-byte SHA-256 newtype。binary wire 是固定长度 byte string，CLI 文本使用 namespace prefix。

`ProgramArtifactId`、`StateDigest`、`InputPayloadDigest` 与 `ReceiptDigest` 对各自 canonical payload
做 domain-separated digest。`InteractionId` 额外包含 execution、parent state、input payload、origin
instruction、occurrence 与 interaction kind。

## 后果

编译器阻止 `FlowId`/`ChoiceId` 等误传；不同 schema domain 中相同 bytes 不会产生可互换 digest。


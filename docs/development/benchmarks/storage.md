# 存储历史增长基线

| 运行条件 | 值 |
| --- | --- |
| 日期 | 2026-10-05（UTC+08:00） |
| 提交 | `47e34ef`（存储实现基于 `266cfd7`） |
| 主机 | Windows 11 Pro for Workstations；Intel Core i9-14900HX；32 逻辑 CPU；约 32 GB RAM |
| 工具 | Rust 1.98.0；Criterion 0.7.0；rusqlite 0.37.0；release profile |
| SQLite | 本 worktree 的 `.temp/storage-history/`；文件数据库；WAL；synchronous=FULL |
| 深度 N | N 次派发 + 1 个 Genesis；Standard recording；每步 Say 后循环；固定大小运行时状态 |
| commit | 每次从深度 N 的 Checkpoint Bundle 导入独立后端，再派发第 N+1 个输入；导入、打开与销毁不计时；历史输入索引不随 Bundle 恢复 |
| open | 原始生成会话的 `SessionCoordinator::open`；恢复至可继续状态；程序预加载；同一已打开后端，热缓存；不计文件连接建立 |
| checkpoint | 原始生成会话的 `CheckpointBundle::export` + `to_bytes`；receiver_has 为空 |
| 延迟统计 | 10 个 Flat 样本的单次操作均值取中位数；warm-up 目标 1 s；measurement 目标 1 s（慢样本延长墙钟时间）；`new/estimates.json` 的 `median.point_estimate` |
| 字节统计 | 序列化 Bundle 长度；同深度固定值 |
| 构建限额 | 600 s；构建时间不计入三项延迟 |

| 后端 | N | commit 中位数（ms） | open 中位数（ms） | checkpoint 中位数（ms） | checkpoint（B） |
| --- | ---: | ---: | ---: | ---: | ---: |
| MemoryStore | 100 | 0.702783 | 0.068749 | 0.387609 | 110489 |
| MemoryStore | 1000 | 7.469550 | 0.749339 | 4.874510 | 1100768 |
| MemoryStore | 10000 | 124.218800 | 8.498231 | 81.191525 | 11009768 |
| SqliteStore | 100 | 17.800550 | 201.386750 | 1.434360 | 110489 |
| SqliteStore | 1000 | 103.733000 | 20631.988850 | 15.304979 | 1100768 |
| SqliteStore | 10000 | 未测 | 未测 | 未测 | 未测 |

| 后端 | N | 累计构建实测（s，单次） |
| --- | ---: | ---: |
| MemoryStore | 10 | 0.000741 |
| MemoryStore | 100 | 0.036241 |
| MemoryStore | 1000 | 3.626823 |
| MemoryStore | 10000 | 507.618232 |
| SqliteStore | 10 | 0.077184 |
| SqliteStore | 100 | 0.711150 |
| SqliteStore | 1000 | 41.514692 |

## 未测组合与外推

| 组合 | 项目 | 未测；外推约 | 依据 |
| --- | --- | ---: | --- |
| SqliteStore / 10000 | 累计构建 | 4151.469 s（69.2 min） | 41.514692 s × (10000 / 1000)² > 600 s，跳过 |
| SqliteStore / 10000 | commit | 604.506 ms | M(1000)² / M(100)；实测 100→1000 倍率 5.827517 |
| SqliteStore / 10000 | open | 2113738.684 ms（35.2 min） | M(1000)² / M(100)；实测 100→1000 倍率 102.449584 |
| SqliteStore / 10000 | checkpoint | 163.308 ms | M(1000)² / M(100)；实测 100→1000 倍率 10.670246 |
| SqliteStore / 10000 | checkpoint 字节 | 11009768 B | 同输入 MemoryStore / 10000 实测值；100 与 1000 两后端字节数及 Commit 摘要相同 |

M(N) 为对应项目在深度 N 的实测中位数。

## 复现

```powershell
task goal -- slot --heavy -- cargo bench -p narrata-store --bench history -- --noplot
# 全部持久化与内核基准：
task goal -- slot --heavy -- task bench:g2
```

```powershell
Get-ChildItem target/criterion/history_* -Recurse -Filter estimates.json |
  Where-Object { $_.Directory.Name -eq 'new' } |
  ForEach-Object {
    $estimate = Get-Content -Raw $_.FullName | ConvertFrom-Json
    '{0}: {1:F6} ms' -f $_.FullName, ($estimate.median.point_estimate / 1000000)
  }
```

标准输出包含 `build_seconds`、`checkpoint_bytes`、目标 Commit 摘要和跳过项的外推依据。

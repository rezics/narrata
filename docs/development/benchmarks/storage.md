# 存储历史增长基准

## ADR 0014 存档引擎

| 运行条件 | 值 |
| --- | --- |
| 日期 | 2026-10-05（UTC+08:00） |
| 提交 | `71cdb7b`（[ADR 0014](../../adr/0014-save-engine-key-layout.md) 的引擎） |
| 主机、工具 | 同下方基线 |
| 后端 | `Store<MemoryBackend>`；`Store<SqliteBackend>`：本 worktree 的 `.temp/storage-history/`，文件数据库，WAL，synchronous=FULL，按行读写 |
| 深度 N | 同基线：N 次派发 + 1 个 Genesis；同一会话依次构建到 100、1000、10000 |
| commit | 在深度 N 的会话上：回退到深度 N 的头（不计时），再派发一个新输入（计时）；每次计时得到一个深度 N+1 的提交，同一后端跨样本累积这些兄弟提交与分叉分支（内存 181621 个，SQLite 1505 个，三个深度合计）；程序缓存为热 |
| open | 同基线：同一已打开后端上的 `SessionCoordinator::open`，热缓存 |
| checkpoint | 同基线：`CheckpointBundle::export` + `to_bytes`，receiver_has 为空 |
| 延迟统计 | 同基线：10 个 Flat 样本，`new/estimates.json` 的 `median.point_estimate` |

| 后端 | N | commit 中位数（ms） | open 中位数（ms） | checkpoint 中位数（ms） | checkpoint（B） |
| --- | ---: | ---: | ---: | ---: | ---: |
| 内存 | 100 | 0.030363 | 0.004294 | 0.331349 | 110489 |
| 内存 | 1000 | 0.030158 | 0.004275 | 3.841846 | 1100768 |
| 内存 | 10000 | 0.032194 | 0.004432 | 54.010150 | 11009768 |
| SQLite | 100 | 2.080838 | 0.013330 | 0.898441 | 110489 |
| SQLite | 1000 | 2.144886 | 0.013693 | 10.638275 | 1100768 |
| SQLite | 10000 | 2.244790 | 0.014325 | 126.277200 | 11009768 |

| 后端 | N | 累计构建实测（s，单次） |
| --- | ---: | ---: |
| 内存 | 100 | 0.002818 |
| 内存 | 1000 | 0.025768 |
| 内存 | 10000 | 0.264290 |
| SQLite | 100 | 0.207523 |
| SQLite | 1000 | 2.119024 |
| SQLite | 10000 | 22.620298 |

- commit 与 open 从 100 到 10000 都在同一数量级：内存 commit 约 0.03 ms、open 约 0.004 ms；
  SQLite commit 约 2.1–2.2 ms（每批次一次 synchronous=FULL 的 WAL 提交），open 约 0.014 ms。
  构建时间随深度线性增长，SQLite 构建 10000 次提交用时 22.6 s（基线外推约 69 min）。
- 深度无关由测试断言而非计时推断：`crates/narrata-store/tests/read_costs.rs` 在两个后端上
  比较深度 10 与 1000 的后端调用计数，加载头提交读 2 个对象，`SessionCoordinator::open` 读
  3 个键与 2 个对象，提交下一回合读 6 个键与 3 个对象并写 1 个批次，对象扫描均为零。
- checkpoint 仍随深度线性增长，因为 bundle 携带目标提交的全部祖先；字节数与基线逐深度相同，
  两个后端在每个深度的目标提交摘要相同，导出字节没有增长。

## 基线：整库读写的适配器（ADR 0014 之前）

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

### 未测组合与外推

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

标准输出包含 `build_seconds`、`checkpoint_bytes`、目标 Commit 摘要和 `measured_commits`。

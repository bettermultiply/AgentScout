# 高权限验证：`scripts/priv-check.sh`

## 运行方式

以**普通用户**运行（不要 `sudo ./scripts/priv-check.sh`）：

```bash
cd /path/to/AgentScout-suit/AgentScout
./scripts/priv-check.sh
```

脚本用 `sudo` 执行 btrfs 子卷操作、必要的目录创建、初始 user namespace 的 `lower=/`
检查、特权 Rust 测试以及状态和 workspace 的清理。`cargo` 编译和普通进程测试由调用用户执行，
避免 root 在 `target/` 下留下属主为 root 的文件。共有 3 个 `priv_` 测试以 root 运行。

## 环境变量

| 变量 | 含义 |
|---|---|
| `ASCOUT_PRIV_STATE_DIR` | btrfs 上已存在的基目录。脚本在其中用 `mktemp -d` 创建 `.ascout-priv-check-state.XXXXXXXX`，测试的 `state` 位于该子目录下。 |
| `ASCOUT_PRIV_WORKSPACE_DIR` | 另一个可写挂载上已存在的基目录。脚本在其中用 `mktemp -d` 创建 `.ascout-priv-workspace.XXXXXXXX`，测试的 workspace 位于该子目录下。 |

脚本解析基目录的实际路径，再尝试以调用用户创建子目录。必要时通过 `sudo` 创建，
并将子目录的属主和属组设为调用用户的 UID 和 GID。

两个变量都可省略。省略时脚本按下述规则自动选择；不会为了“能跑”而降级成
状态目录与 workspace 同挂载、或状态目录位于被隔离挂载内的布局。

```bash
ASCOUT_PRIV_STATE_DIR=/path/on/btrfs \
ASCOUT_PRIV_WORKSPACE_DIR=/path/on/another-writable-mount \
  ./scripts/priv-check.sh
```

## 目录选择规则

### 状态目录

1. 必须是可写的 btrfs 目录。
2. 必须位于后端要隔离的挂载点之外。后端隔离的是 `real_writable_mounts()` 中
   能通过 overlay 探测的挂载；`/` 被排除，因此通常可以直接把状态目录放在
   btrfs 文件系统的根挂载下（例如 `/.ascout-priv-check-state.XXXXXXXX`）。
3. 脚本用 `findmnt` 做近似判断：除 `/` 和伪文件系统外，可写挂载都视为会被
   隔离。找不到合法挂载时打印 `NO_VALID_STATE_LAYOUT` 并失败，不 SKIP。
4. 特权测试会再次用后端的 `state_overlap_mounts()` 断言，并用默认配置
   （`allow_overlapping_state_directory=false`）调用 `establish`；如果布局仍然
   重叠，测试打印状态目录、workspace、各自 mount 和被隔离挂载列表后失败。

因此脚本不会再把 `/var/tmp` 这类自身会被隔离的 btrfs 挂载误选为状态目录；
它检查的是实际创建目录所在的挂载，而不是“挂载点自身是否可写”。

### 工作区目录

1. 必须与状态目录位于不同的挂载。
2. 必须是后端会隔离的可写挂载，这样 workspace 的 upper/lower 布局才有意义。
3. 未设置 `ASCOUT_PRIV_WORKSPACE_DIR` 时，按 `/var/tmp`、`/tmp`、`/home`、`/srv`
   的顺序寻找符合条件的基目录，并在第一个可成功创建子目录的候选处创建临时目录。
   有状态目录但所有候选均不可用时，打印 `NO_VALID_WORKSPACE_MOUNT` 并失败。

## 脚本覆盖的检查清单

脚本开头打印：

1. 环境与必需工具；
2. 非重叠的 btrfs 状态目录 + 另一个挂载上的 workspace；
3. btrfs 原始能力：子卷创建、只读快照、`send -p | receive --dump`、严格子卷删除；
4. 初始 user namespace 的 `lower=/` overlay；
5. 以调用用户构建 Rust 测试二进制；
6. 进程组冻结测试：`runtime::process` 与 `filesystem::tests::process`；
7. 3 个特权 Rust 测试。

结尾打印覆盖汇总表，列出各项的状态（`PASS`、`FAIL` 或 `UNRUN`）和 Rust 测试实际执行数。
一次成功运行的示例如下。

```text
===== coverage summary =====
CHECK                        RESULT DETAIL
environment                  PASS   uid=1000 gid=1000
layout                       PASS   state=<...> workspace=<...>
btrfs-raw                    PASS   create/snapshot/send-dump/strict-delete
direct-root-overlay          PASS   lower=/ in initial userns
rust-build                   PASS   lib tests
process-freeze               PASS   4/4 executed; 4/4 passed; 0 ignored; mechanism=cgroup
privileged-rust              PASS   3/3 executed; 3/3 passed; 0 ignored
process tests executed       4      freeze mechanism: cgroup
process tests passed         4      ignored: 0
privileged tests executed    3      expected 3
privileged tests passed      3      ignored: 0
SUMMARY: ALL PRIVILEGED CHECKS PASSED
```

任何一项缺失、失败、输出 `SKIP`、或没有执行，都会让脚本返回非零，并且不会
打印 `ALL PRIVILEGED CHECKS PASSED`。

## 各步骤预期

### 1. btrfs 原始能力

原始 btrfs 检查在状态运行目录的 `raw/` 下执行。增量发送和接收的 stderr 分别写入
`send.err` 和 `receive.err`，管道失败时两者都会打印到终端。

- `btrfs subvolume create` → `PASS`
- 两次只读快照 → `PASS`；只读快照创建后不再递归 `chown`
- `btrfs send -p snap1 snap2 | btrfs receive --dump` → `PASS`，打印前 80 行；
  期望包含 `modified.txt`、`base.txt`、`renamed`
- `btrfs subvolume delete` → `PASS`

### 2. 初始 user namespace 的直接 `lower=/`

测试使用 workspace 运行目录下的 `overlay/upper` 和 `overlay/work`。root 下在
`unshare --mount --propagation private` 中运行 bwrap，普通用户通过 `sudo unshare ...`
执行。预期 `PASS: direct lower=/ overlay`。缺少 root 权限且无法使用 sudo 时明确失败。

### 3. 进程组冻结测试

以调用用户身份运行：

```text
runtime::process::tests::
runtime::backends::filesystem::tests::process::
```

两个过滤器分别 `--list` 精确核对，共 4 个测试必须全部 `... ok`，且不出现 `SKIP`。
运行使用 `--show-output --format=pretty --color=never --test-threads=1`，保留捕获
日志并保证 libtest 的结果行不被测试内日志打断。测试打印固定标记（沿用
`FreezeMechanism` 的名字）：

```text
ASCOUT_FREEZE_MECHANISM=cgroup test=process_freeze_thaw_stops_and_resumes_writer
```

`cgroup` 与 `process_group` 都可通过，汇总表写出自动选择的机制。每个测试必须恰好
打印一条带自身测试名的有效标记。`none`、缺少标记或自动选择结果不一致均失败。
`process_group_fallback_freezes_and_reports_mechanism` 强制使用进程组信号，其标记
不参与判断自动选择的机制。

### 4. 特权 Rust 测试

先以 `--list` 精确比对以下三个测试，全部位于
`runtime::backends::filesystem::tests::privileged` 模块下。数量或名称不符即失败。

```text
priv_btrfs_chain_layout
priv_btrfs_flatten_layout
priv_btrfs_mixed_generations
```

再以 `--show-output --format=pretty --color=never --test-threads=1` 运行。
`check-test-output.py` 交叉核对命令退出码、精确逐项结果、running 数和最终摘要，
缺失、重复、额外测试、ignored、计数不一致或任何位置的 `SKIP` 均失败。汇总分别
报告执行数（通过+失败）、通过数和 ignored；组内有错不打印该组 PASS。这些测试
在 `ASCOUT_PRIV_TESTS=1` 打开后前置条件不满足会直接断言失败。

- `priv_btrfs_chain_layout`（`fork_flatten_entry_limit=0`）
  - 默认配置（不打开 `allow_overlapping_state_directory`）建立状态目录与
    workspace 分离的环境；
  - fork 后子挂载的 lower 比父多一层，子 upper 为空；
  - 子进程真实修改、删除、新建文件；diff 报告这三项；
  - Chain 的 delta 扫描还会报告为写入路径 copy-up 的祖先目录（结构项）；
    辅助函数把这些目录也纳入精确集合，任何额外的文件或目录仍然失败；
  - accept 回 root 后 root 真实视图可见；
  - discard 后子环境根、快照层和 trash 清空。
- `priv_btrfs_flatten_layout`（`fork_flatten_entry_limit=usize::MAX`）
  - 同样的场景，但子 lower 与父相同，子 upper 含父文件；
  - 验证 Flatten 下 diff/accept 走 `btrfs_delta` 分支；精确预期也包含父目录变化。
- `priv_btrfs_mixed_generations`
  - 使用 `fork_flatten_entry_limit=32`，大 root upper 先 Chain，小 child upper 再 Flatten；
  - 真实启动 grandchild，精确 diff（含父目录）、accept 回 child1，父 root 视图不变。

## 诊断与清理

脚本在 `/var/tmp/ascout-priv-check.XXXXXXXX` 中暂存构建日志和测试的列表及运行输出。
终端会显示检查结果、测试输出、失败时的构建日志以及最终汇总。

退出时，无论检查成功还是失败，脚本都会依次尝试清理 workspace 运行目录、状态运行目录和
诊断目录。收到 `INT`、`TERM` 或 `HUP` 并进入退出流程时也会执行清理。前两个目录按需用
`sudo` 清理，诊断目录由调用用户清理。清理只针对本次创建的目录，基目录和其中原有的文件
不在清理范围内。

清理会拒绝以符号链接作为清理目标，也会拒绝清理自身是挂载点或含有挂载点的目录。
对需要提权清理的 btrfs 目录，先从最深处删除子卷，再用 `rm -rf --one-file-system`
删除目录。某项清理失败时会打印对应路径，继续尝试清理其他目录，最终返回非零。

需要留档或对比复跑结果时，在运行前将标准输出和标准错误一起写入自行指定的日志。
`pipefail` 让管道保留脚本失败时的非零状态。每次复跑都会创建新的运行目录，可沿用相同的基目录设置。

```bash
(set -o pipefail; ./scripts/priv-check.sh 2>&1 | tee priv-check.log)
```

## 无特权脚本回归

```bash
python3 scripts/test-priv-check.py
```

该回归用本地假命令模拟 btrfs、提权、`findmnt` 和测试二进制，并用 `rustc --test`
构建真实 libtest fixture，先证明 `--nocapture` 会打断结果行，再验证脚本通过。
无需真实 root 或 btrfs 环境。每个场景的工作目录和脚本的临时日志目录都创建在 `/var/tmp`
下，由 Python 在退出相应的临时目录上下文时清理。若回归抛出异常，清理前会打印已收集的场景日志。

用 `--reproduce` 复跑全部场景时，每个场景会打印退出码、是否出现成功摘要、测试二进制的
调用记录和临时日志路径。此模式仍检查目录清理，但跳过常规的场景结果断言。要保留输出，
可沿用上文的 `tee` 方式，并为本次复跑另选日志文件名。

```bash
python3 scripts/test-priv-check.py --reproduce
```

下表列出常规回归检查的场景。

| 场景 | 断言 |
|---|---|
| `success` | 退出 0、出现 `ALL PRIVILEGED CHECKS PASSED`、汇总表完整、3/3 特权测试、4/4 进程测试、`mechanism=cgroup` |
| `nonroot` | 通过假 `sudo` 走提权分支，仍然成功 |
| `freeze-pg` | 通过，汇总表明确写 `mechanism=process_group` |
| `freeze-none` | 非零，汇总写 `none` |
| `zero` | 特权测试列表为 0，非零 |
| `missing-test` | 缺少一个固定特权测试名，非零 |
| `skip` | 特权测试输出 `SKIP`，非零 |
| `build-fail` | 编译失败，非零且不运行任何测试 |
| `raw-fail` | 第二次只读快照失败，非零，仍清理本次运行目录 |
| `nonbtrfs` | 状态目录不是 btrfs，非零 |
| `no-layout` | 找不到 btrfs 非重叠布局，打印 `NO_VALID_STATE_LAYOUT`，非零 |
| `overlap-layout` | 状态目录在会被隔离的挂载内，非零 |
| `cleanup-fail` | workspace 清理失败，报告路径并返回非零。状态和诊断目录仍清理 |
| `interleaved` | 真实 libtest 的测试内输出交错；脚本捕获模式仍精确通过 |
| `missing-process` / `extra-test` / `duplicate-test` | 缺进程测试、额外测试、重复测试均失败 |
| `list-fail` / `process-fail` / `priv-exit-fail` | 即使输出貌似通过，命令非零也失败 |
| `priv-failed` | 3/3 执行、1/3 通过，非零且无该组 PASS |
| `ignored` / `skip-inline` | ignored 不算执行；行内 SKIP 同样失败 |
| `bad-summary` / `incomplete-run` | 摘要或逐项记录不完整、不一致均失败 |

常规回归中，每个失败场景都断言不出现 `ALL PRIVILEGED CHECKS PASSED`。除故意让
workspace 清理失败的 `cleanup-fail` 外，各场景都检查脚本创建的临时目录已删除。
使用显式基目录的场景还检查原有文件完好。

## 已知边界

- 脚本不会自动安装 btrfs-progs / bubblewrap / sudo。
- 需要 root 的步骤失败时会明确标为 `FAIL` 或 `NEEDS_ROOT`，不会伪装成 `PASS`。
- 普通用户无法在没有 sudo 的情况下把状态目录放到 `/`；此时脚本明确失败并
  说明需要什么。
- 三项 btrfs 场景的 accept 使用 `target_mount_pid=None`，直接写目标 upper，随后
  真实启动确认结果；不等于运行中 live target 的 accept 验证，也不确定其偶发超时根因。
- 本次不覆盖全部内核生成的 redirect/opaque/metacopy、特殊文件/ACL/ownership，
  不代表物化已实现；这些边界见 `current-state.md`。

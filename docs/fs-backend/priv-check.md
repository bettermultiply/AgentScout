# 高权限验证：`scripts/priv-check.sh`

## 运行方式

以**普通用户**运行（不要 `sudo ./scripts/priv-check.sh`）：

```bash
cd /path/to/AgentScout-suit/AgentScout
./scripts/priv-check.sh
```

脚本内部只对 btrfs 子卷操作、状态目录创建、初始 user namespace 的 `lower=/`
检查和成功后的清理使用 `sudo`；`cargo` 编译和普通测试始终由调用用户执行，避免
root 在 `target/` 下留下属主为 root 的文件。只有 4 个 `priv_` 测试以 root 运行。

## 环境变量

| 变量 | 含义 |
|---|---|
| `ASCOUT_PRIV_STATE_DIR` | btrfs 上的可写基目录。脚本在其中创建唯一运行子目录；测试的 `state` 位于该子目录下。 |
| `ASCOUT_PRIV_WORKSPACE_DIR` | 另一个可写挂载上的基目录。脚本在其中创建唯一运行子目录，测试的 workspace 位于其下。 |

两个变量都可省略。省略时脚本按下述规则自动选择；不会为了“能跑”而降级成
状态目录与 workspace 同挂载、或状态目录位于被隔离挂载内的布局。

```bash
ASCOUT_PRIV_STATE_DIR=/path/on/btrfs \
ASCOUT_PRIV_WORKSPACE_DIR=/tmp/ascout-workspace \
  ./scripts/priv-check.sh
```

## 目录选择规则

### 状态目录

1. 必须是可写的 btrfs 目录。
2. 必须位于后端要隔离的挂载点之外。后端隔离的是 `real_writable_mounts()` 中
   能通过 overlay 探测的挂载；`/` 被排除，因此通常可以直接把状态目录放在
   btrfs 文件系统的根挂载下（例如 `/.ascout-priv-check-state-<pid>`）。
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
3. 未设置 `ASCOUT_PRIV_WORKSPACE_DIR` 时依次尝试 `/tmp`、`/var/tmp`、
   `/home`、`/srv` 下的唯一临时目录；都不可用时打印
   `NO_VALID_WORKSPACE_MOUNT` 并失败。

## 脚本覆盖的检查清单

脚本开头打印：

1. 环境与必需工具；
2. 非重叠的 btrfs 状态目录 + 另一个挂载上的 workspace；
3. btrfs 原始能力：子卷创建、只读快照、`send -p | receive --dump`、严格子卷删除；
4. 初始 user namespace 的 `lower=/` overlay；
5. 以调用用户构建 Rust 测试二进制；
6. 进程组冻结测试：`runtime::process` 与 `filesystem::tests::process`；
7. 4 个特权 Rust 测试。

结尾打印覆盖汇总表，每一项 `PASS`/`FAIL`，并给出 Rust 测试实际执行数：

```text
===== coverage summary =====
CHECK                        RESULT DETAIL
environment                  PASS   uid=1000 gid=1000
layout                       PASS   state=<...> workspace=<...>
btrfs-raw                    PASS   create/snapshot/send-dump/strict-delete
direct-root-overlay          PASS   lower=/ in initial userns
rust-build                   PASS   lib tests
process-freeze               PASS   7/7 executed; mechanism=cgroup
privileged-rust              PASS   4/4 executed
process tests executed       7      freeze mechanism: cgroup
privileged tests executed    4      expected 4
SUMMARY: ALL PRIVILEGED CHECKS PASSED (diagnostics: <log-dir>)
```

任何一项缺失、失败、输出 `SKIP`、或没有执行，都会让脚本返回非零，并且不会
打印 `ALL PRIVILEGED CHECKS PASSED`。

## 各步骤预期

### 1. btrfs 原始能力

每步保留 stderr：

- `btrfs subvolume create` → `PASS`
- 两次只读快照 → `PASS`；只读快照创建后不再递归 `chown`
- `btrfs send -p snap1 snap2 | btrfs receive --dump` → `PASS`，打印前 80 行；
  期望包含 `modified.txt`、`base.txt`、`renamed`
- `btrfs subvolume delete` → `PASS`

### 2. 初始 user namespace 的直接 `lower=/`

root 下在 `unshare --mount --propagation private` 中运行 bwrap；非 root 时通过
`sudo unshare ...`。预期 `PASS: direct lower=/ overlay`。没有 root/sudo 时明确
失败，不伪装成成功。

### 3. 进程组冻结测试

以调用用户身份运行：

```text
runtime::process::tests::
runtime::backends::filesystem::tests::process::
```

共 7 个测试必须全部 `... ok`，且不出现 `SKIP`。测试在 `--nocapture` 下打印固定
标记（沿用 `FreezeMechanism` 的名字）：

```text
ASCOUT_FREEZE_MECHANISM=cgroup test=terminate_uses_cgroup_or_group_kill_and_cleanup_removes_cgroup
```

`cgroup` 与 `process_group` 都算通过，但汇总表明确写出实际机制，让用户知道
cgroup 路径有没有被真实走到。`none` 视为失败；没有标记也视为失败。
`process_group_fallback_freezes_and_reports_mechanism` 强制进程组信号，脚本
在判断“本次实际机制”时忽略这个强制 fallback 的标记。

### 4. 特权 Rust 测试

先以 `--list` 精确比对固定列表，数量不对或缺任何一个都失败：

```text
priv_btrfs_chain_layout
priv_btrfs_flatten_layout
priv_btrfs_mixed_generations
priv_direct_root_overlay_is_available_in_initial_userns
```

再以 `--nocapture --test-threads=1` 运行，逐个确认 `... ok`，并拒绝任何
`SKIP`。这些测试在 `ASCOUT_PRIV_TESTS=1` 打开后不会再跳过：前置条件不满足
直接断言失败。

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
  - 验证 Flatten 下 diff/accept 走 `btrfs_delta` 分支。
- `priv_btrfs_mixed_generations`
  - 大 root upper 先 Chain，小 child upper 再 Flatten；
  - 真实启动 grandchild，精确 diff、accept 回 child1，父 root 视图不变。
- `priv_direct_root_overlay_is_available_in_initial_userns`
  - root + `unshare --mount` 中直接 `--overlay-src / ... /` 成功。

## 诊断与清理

完整诊断输出保存在 `${TMPDIR:-/tmp}/ascout-priv-check.*`，开始和结尾都会打印
实际目录，包括 `check.log`、构建日志、测试列表/运行日志、btrfs 命令输出。

- 全部检查通过：删除脚本通过 sudo 创建的状态目录、workspace 目录及其中的
  btrfs 子卷（先递归 `btrfs subvolume delete`，再 `rm -rf`）。
- 任何检查失败：保留这两个目录并打印路径，方便事后检查；不会吞掉清理错误。
- 清理本身失败也会让脚本返回非零。

## 无特权脚本回归

```bash
python3 scripts/test-priv-check.py
```

该回归用本地假命令模拟 btrfs、提权、`findmnt` 和测试二进制，不要求实际
root/btrfs 环境。临时工作目录使用 Python `tempfile` 的默认位置（支持
`TMPDIR`），诊断日志保留在 `priv-check-regression-*` 下；可用
`ASCOUT_PRIV_CHECK_TEST_LOG_DIR` 指定日志目录。

场景：

| 场景 | 断言 |
|---|---|
| `success` | 退出 0、出现 `ALL PASSED`、汇总表完整、4/4 特权测试、7/7 进程测试、`mechanism=cgroup` |
| `nonroot` | 通过假 `sudo` 走提权分支，仍然成功 |
| `freeze-pg` | 通过，汇总表明确写 `mechanism=process_group` |
| `freeze-none` | 非零，汇总写 `none` |
| `zero` | 特权测试列表为 0，非零 |
| `missing-test` | 缺少一个固定特权测试名，非零 |
| `skip` | 特权测试输出 `SKIP`，非零 |
| `build-fail` | 编译失败，非零且不运行任何测试 |
| `nonbtrfs` | 状态目录不是 btrfs，非零 |
| `no-layout` | 找不到 btrfs 非重叠布局，打印 `NO_VALID_STATE_LAYOUT`，非零 |
| `overlap-layout` | 状态目录在会被隔离的挂载内，非零 |
| `cleanup-fail` | 清理失败，非零且不出现 `ALL PASSED` |

每个失败场景都断言不出现 `ALL PRIVILEGED CHECKS PASSED`。

## 已知边界

- 脚本不会自动安装 btrfs-progs / bubblewrap / sudo。
- 需要 root 的步骤失败时会明确标为 `FAIL` 或 `NEEDS_ROOT`，不会伪装成 `PASS`。
- 普通用户无法在没有 sudo 的情况下把状态目录放到 `/`；此时脚本明确失败并
  说明需要什么。
- 本机没有 Btrfs 数据面权限，所以 Chain/Flatten/mixed 三个 btrfs 测试只有在
  用户实际运行本脚本时才第一次进入真实 btrfs send/receive 路径。

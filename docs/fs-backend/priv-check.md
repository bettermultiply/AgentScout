# 高权限验证：`scripts/priv-check.sh`

## 运行方式

以**普通用户**运行（不要 `sudo ./scripts/priv-check.sh`）：

```bash
cd /path/to/AgentScout-suit/AgentScout
./scripts/priv-check.sh
```

脚本内部只对 btrfs 子卷操作和初始 user namespace 的 `lower=/` 检查使用
`sudo`；`cargo` 编译和测试二进制选择始终由调用用户执行，避免 root 在
`target/` 下留下属主为 root 的文件。

可用 `ASCOUT_PRIV_STATE_DIR` 指定一个位于 btrfs 上的可写目录：

```bash
ASCOUT_PRIV_STATE_DIR=/path/on/btrfs/ascout-priv ./scripts/priv-check.sh
```

未指定时脚本用 `findmnt` 找第一个可写的 btrfs 挂载，并使用其下的
`.ascout-priv-check`。找不到时打印
`NO_WRITABLE_BTRFS_MOUNT: set ASCOUT_PRIV_STATE_DIR ...` 并让汇总失败，而不是
逐项报一堆无意义的 btrfs 失败。

## 脚本步骤与预期

1. 打印 `id`、`cargo`/`bwrap`/`btrfs`/`findmnt`/`sudo` 是否存在。
2. 选择状态目录并打印 `state fs type: btrfs`。
   - 预期：`PASS: state directory is btrfs`
3. btrfs 原始能力（每步保留 stderr）：
   - `btrfs subvolume create` → `PASS`
   - 两次只读快照 → `PASS`；快照创建后保持只读，所有权只在可写的 `sub`
     创建完成后设置，不对只读快照递归执行 `chown`
   - `btrfs send -p snap1 snap2 | btrfs receive --dump` → `PASS`，并打印前 80 行
     dump；预期包含 `modified.txt`、`base.txt`、`renamed`（create/modify/delete/
     rename）。
   - `btrfs subvolume delete` → `PASS`
4. 初始 user namespace 的直接 `lower=/`：
   - root 下在 `unshare --mount --propagation private` 中运行 bwrap；
   - 非 root 时通过 `sudo unshare ...`；
   - 预期：`PASS: direct lower=/ overlay`。若提示 `NEEDS_ROOT`，说明当前环境
     无法进入初始 userns，这不是静默成功。
5. 以当前用户执行 `cargo test --lib --no-run --message-format=json`，从 JSON
   `compiler-artifact` 中取 lib 测试可执行文件。
6. 运行：

脚本实际筛选的测试路径为：

```text
runtime::backends::filesystem::tests::privileged::priv_
```

普通用户运行时脚本会在需要时为该命令加上 `sudo`，并先用 `--list` 确认恰好
两个预期测试，再执行它们。

预期两个测试实际执行（不是 SKIP）：

- `priv_btrfs_fork_diff_accept_discard`
  - 断言数据面为 `btrfs`、`fast_delete=true`；
  - fork 后在子挂载写 `child.txt`，diff 文档必须包含该路径；
  - accept 后目标 upper 必须出现 `child.txt`；
  - discard 后 child 环境根目录被移除。
- `priv_direct_root_overlay_is_available_in_initial_userns`
  - root + `unshare --mount` 中直接 `--overlay-src / ... /` 成功。

脚本将完整诊断输出保留在 `${TMPDIR:-/tmp}/ascout-priv-check.*` 下，并在开始和
汇总时打印实际目录；其中包括 `check.log`、构建日志、测试列表/运行日志以及
btrfs 命令的输出文件。

最后打印：

```text
SUMMARY: ALL PRIVILEGED CHECKS PASSED (diagnostics: <log-dir>)
```

或

```text
SUMMARY: PRIVILEGED CHECKS FAILED (diagnostics: <log-dir>)
```

## 无特权脚本回归

```bash
python3 scripts/test-priv-check.py
```

该回归用本地假命令模拟 btrfs、提权和测试二进制，包括只读快照上的 `chown`
失败，不要求实际 root/btrfs 环境。临时工作目录使用 Python `tempfile` 的默认
位置（支持 `TMPDIR`），诊断日志默认保留在同一临时目录根下的
`priv-check-regression-*` 目录中；可用 `ASCOUT_PRIV_CHECK_TEST_LOG_DIR` 指定日志
目录。每次运行都会打印实际日志位置；每种场景有汇总日志，内部脚本的完整
诊断目录也保留在该位置下。

## 已知边界

- 脚本不会自动安装 btrfs-progs / bubblewrap / sudo。
- 若 btrfs 挂载不可写，需设置 `ASCOUT_PRIV_STATE_DIR`。
- 需要 root 的步骤失败时会明确标为 `FAIL` 或 `NEEDS_ROOT`，不会伪装成 PASS。

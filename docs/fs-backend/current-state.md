# 文件系统 Backend 当前实现状态

本文只记录实现事实，不定义目标语义。末尾逐条标注“实测”或“推断”。

## 已实现

- **overlay fork（独立 upper 副本）**
  - 只复制父当前 upper，不复制 lower 链和 Base。
  - 逐文件优先 FICLONE reflink；不支持时普通复制。保留目录、whiteout 字符设备、
    overlay xattr、权限、符号链接和 mtime；不使用硬链接。
  - 子 lower = 父 lower + S，子 upper 为空。父 upper、lower 和运行中挂载不变。
  - S 由子（以及 mark/diff 基准）引用，按引用计数 GC。
  - 快照层/副本层先写入 staging 目录，完成后原子 rename 到最终路径并 sync 父目录；
    没有完成 rename 的 `.tmp-*` 在下次启动清理，不被视为有效层。复制文件不再
    逐文件 `sync_all`。
  - `filesystem_fork_cost=btrfs_snapshot|reflink|copy`。选择顺序：
    **方案 A（btrfs 增量）→ 方案 2 + btrfs 快照副本 → reflink → copy**。
  - 当状态目录在 btrfs 且 `probe_btrfs_snapshot_copies` 成功（创建/只读快照/
    overlay lower/子卷删除均可用）时，每个挂载的 upper 置于一个 btrfs 子卷的
    `upper` 目录中，fork/mark 用只读快照产生层，成本 O(1)；子 upper 是新的空
    子卷，flatten 情形用可写快照。启用探测要求创建/只读快照/overlay lower/
    btrfs-aware 递归删除成功；`fast_delete` 只影响后台删除成本，不阻止启用。
  - FS Backend 自身不冻结写者。经 BaseRuntime 调用时，进程在 fork/mark/accept
    前由 ProcessBackend 冻结，成功后 `writers_quiesced=true`；直接调用 FS API 或
    冻结机制为 `none` 时仍为 false，并发写可能导致副本跨文件交错。
- **btrfs fork**
  - 每个挂载点的 upper 是一个独立 btrfs 子卷；fork 对每个选中挂载的父 upper
    分别做只读快照 S_child，再为 child 建可写子卷（含状态目录的挂载用 S_child
    的可写快照展平，避免 lower 重叠）。
  - 父 upper/lower 和挂载不变。
  - btrfs 只有在 `incremental_diff=true` 时才被选为数据面；探测真实执行
    `btrfs send -p <snapshot> <snapshot> | btrfs receive --dump`。失败则降级
    overlay 并在日志/能力中说明。btrfs diff/accept 已实现：diff 解析 send dump，
    accept 用同一 dump 构建增量视图后走 live-mount/openat2 重放。本机无权限，
    btrfs 测试 SKIP；高权限验证见 `scripts/priv-check.sh`。
- **btrfs 快照副本（overlay 方案 2）**
  - 探测 `probe_btrfs_snapshot_copies`：建子卷、写文件、只读快照、把快照作为
    overlay lower 真实 bwrap 挂载，再走 btrfs-aware 递归删除路径确认能删净。
    创建/快照/挂载/删除探测全部成功才启用；`fast_delete` 不再作为门槛，只影响
    后台删除成本，`filesystem_fast_delete=false` 与 `fork_cost=btrfs_snapshot`
    可以同时出现。
  - 每个挂载 upper 位于一个 btrfs 子卷的 `upper` 子目录；fork 用只读快照作为
    lower 层，子 upper 为新空子卷；flatten 情形用可写快照。mark 用只读快照。
  - 选择顺序：方案 A → 方案 2 + btrfs 快照 → reflink → copy。
  - 本机普通用户 `btrfs subvolume delete` EPERM，但 `property set ro false`
    + 递归删除可删净；因此本机现在也启用 btrfs 快照副本，`fast_delete=false`
    只表示后台删除走 O(文件数) fallback。
  - 测试：`btrfs_snapshot_copy_fork_is_end_to_end`（本机实际运行并通过，含
    snapshot xattr、whiteout、后台删除清空断言）。

- **mark**
  - overlay：逐选中挂载独立复制当前 upper，不改 live upper。
  - overlay + btrfs_snapshot：逐选中挂载对 upper 所在子卷做只读快照，O(1)。
  - btrfs：逐选中挂载只读子卷快照，不改 live upper。
- **diff**
  - overlay：扫描分支 upper 增量，不读内容。
  - 对 in-lower 的 fork 基准，扫描 base 之后的 delta 层和当前 upper。
  - 对独立 mark 副本，比较当前 upper 与 mark 副本元数据，O(upper entries)。
  - btrfs：send dump 解析 create/modify/delete/rename/chmod/xattr，whiteout
    （mknod c 0 0）翻译为 delete；opaque/redirect/metacopy 通过读取 upper 的
    overlay xattr 保留。只有 `incremental_diff=false` 时不被选中。
- **accept（live mount 重放）**
  - 三方冲突检测保留；ours 逐路径读取目标 lower+upper，不构建完整视图。
  - `target_mount_pid=None`：调用方确认无 live mount，直接写目标 upper，删除写
    whiteout。
  - `target_mount_pid=Some(inner_pid)`（由 ProcessBackend 提供）：
    - 校验 `/proc/<pid>/ns/mnt` 等于该环境启动时由 bwrap `--json-status-fd`
      记录的 mount namespace，且不等于宿主 mount namespace；
    - 以 `/proc/<pid>/root` 的 dirfd 为根，用
      `openat2(RESOLVE_IN_ROOT | RESOLVE_NO_MAGICLINKS)` + 最终组件 `O_NOFOLLOW`
      执行 mkdir/unlink/write/symlink；
    - 已实测：目标进程内立即可见合并结果，删除产生 whiteout，宿主 Base 未改。
  - 重放操作日志：每个会覆盖或删除目标内容的操作前，把目标 upper 的旧状态
    保存到 `state/replay-journal/`（优先 reflink；不支持时复制，单文件超过
    64MiB 会记录告警）。失败时按逆序恢复全部已记录路径；回滚失败会列出未恢复
    路径并进入 `cleanup_failures`。成功后删除日志区。
  - 重放是“可回滚、非原子”：并发读者可能在中间态读到部分结果，不宣称原子；
    失败时 `published=false`。
  - lower 链 accept 只对 source 相对 base 的增量路径探测 ours；不再读取完整
    target upper。rename/metacopy 才构建 source 视图。计数测试
    `accept_work_does_not_scale_with_unrelated_upper_size` 证明 5000 个无关文件
    不会增加扫描项。
- **状态目录重叠检测**
  - 启动时列出状态目录所在的 overlay 挂载点，写入能力字段
    `state_directory_overlaps_mount`，并打印
    `strategy=flatten fork_cost=O(upper) diff_cost=O(upper)`。
  - 部署建议见 `docs/fs-backend/deployment.md`：生产环境应把状态目录放在不被
    overlay 的独立挂载点。
  - 测试：`overlapping_state_directory_is_reported_and_flattened`、
    `non_overlapping_state_directory_uses_lower_chain`。

- **LaunchFragment / ProcessBackend**
  - FilesystemBackend 只产出 `LaunchFragment` 中的挂载条目，不再 spawn，也不
    记录 pid/mnt namespace。
  - ProcessBackend 负责把 MountSpec 翻译成 bwrap 参数、spawn、cgroup/freeze、
    `--json-status-fd`，并对外提供 `process_handle(env)`。
- **删除**
  - discard 热路径 rename 到 trash，O(1)；后台删除。
  - 严格 btrfs 子卷删除可用时 `fast_delete=true`，否则 false 并走 O(文件数)
    后台删除。能力字段如实上报。
- **持久化/幂等**
  - 环境、快照、operation 记录持久化并可重启读回。
  - operation id + generation 重放返回同一结果；更小 generation 为 stale。
  - fork/mark 重放验证快照和元数据存在，否则报错。
  - operation 记录持久化失败时，已生效操作返回
    “applied but operation record was not persisted” 错误，不静默忽略。
  - suspend/resume/terminate 先持久化候选状态，成功后才替换内存状态；失败保持
    原状态。全局 state 锁外执行持久化写盘。
  - 损坏的 `operations.json` 会改名为 `.corrupt-<pid>` 并告警，不静默当作空。
  - 副本层使用 staging 目录 + 原子 rename + 目录 sync；没有完成 rename 的
    `.tmp-*` 在下次启动清理，不视为有效层。
  - `btrfs send | receive --dump` 并发 drain send stderr，避免大输出死锁。

## 未实现 / Phase 2

- accept selector、依赖闭包、控制面逐条变更列表。
- `diff(target=Some(_))` 明确返回未实现。
- accept 重放已支持修改/删除/创建/目录替换的逆序回滚，但仍非原子；并发读者
  可能观察到中间状态。
- 有特权时直接 `lower=/`：只做了探测/高权限测试，未作为默认路径。
- 跨 Backend 冻结/快照/解冻编排、cgroup 归属、进程停止归属、mount ns 是否拆成
  独立 Backend：待用户决定。
- **ProcessBackend 已实现**：`src/runtime/process/` 负责 spawn、MountSpec、bwrap
  `--json-status-fd`、进程组、cgroup 创建/加入/冻结/解冻/kill、内层 pid 与 mnt ns。
  FilesystemBackend 只产出 `LaunchFragment`。
- **真实 freeze 已实现**：cgroup v2 `cgroup.freeze`；不可用时 fallback 进程组
  SIGSTOP/SIGCONT；能力字段 `freeze_mechanism`。`BaseRuntime::suspend/resume`
  已改为真实 freeze/thaw，`terminate` 优先 `cgroup.kill`。
- 仍然待办：逃出进程组的后代冻结（process_group fallback 无法覆盖）；
  cgroup 归属和跨 Backend 冻结编排的进一步统一。

## 已知偏差

- overlay fork 的副本不是冻结快照：并发写时各文件可能来自不同时刻；是否冻结
  由 BaseRuntime/ProcessBackend 在操作窗口内负责。
- btrfs 低权限下通常不满足 `incremental_diff`，会降级 overlay；btrfs fork 代码
  路径在本机未被选中，未做端到端验证。
- accept 直接写 upper / live mount 重放均非原子。
- 并行 `clone_upper` 未实现：reflink/快照已覆盖数据拷贝，剩余主要成本是目录项
  遍历；并行目录/xattr/whiteout 写入顺序风险未在本轮引入。
- 当前挂载含状态目录时，fork 采用“upper 展平 + 快照仅作 diff 元数据”，不再把
  该快照层作为 lowerdir；这是为了避免 OverlayFS 的
  `Overlay directories may not overlap`。
- 本机普通用户 `btrfs subvolume delete` 返回 EPERM，`fast_delete=false`；快照
  副本仍可启用，后台删除走 recursion fallback（已验证可删净），只是成本更高。

## 未验证 / 需高权限

- **btrfs fork/diff/accept 成功路径**：本机 send/subvolume 无权限，探测降级
  overlay；需要高权限环境验证。测试在不满足条件时打印 `SKIP`。运行
  `scripts/priv-check.sh`，见 `docs/fs-backend/priv-check.md`。
- **严格 btrfs subvolume delete ioctl**：本机 `fast_delete=false`；fallback 的
  O(文件数) 后台删除未做大数据量性能验证。
- 真实内核生成的 redirect/opaque/metacopy 只用合成 xattr 测试；whiteout 字符
  设备已在真实 bwrap 测试中验证。
- live replay 的 `openat2` 符号链接逃逸测试覆盖了绝对符号链接；其它特殊文件/
  ACL/ownership 未验证。

高权限验证入口：普通用户运行 `./scripts/priv-check.sh`（可用
`ASCOUT_PRIV_STATE_DIR` 指定 btrfs 目录）；脚本会以普通用户编译测试，再用 sudo
运行 `ASCOUT_PRIV_TESTS=1 <test-binary> filesystem::tests::priv_ --nocapture`。
live mount 重放不要求高权限，本机实际运行并通过。当前本机 btrfs 测试会打印 SKIP。

## 逐挂载 overlay（无特权默认，实测）

- **隔离单元是挂载点**。`Environment.mounts: BTreeMap<absolute target, MountUnit>`
  记录 `{target, upper, work, lower 链, layer_key}`；workspace 与 `/tmp` 只是
  普通挂载点，没有单独的 workspace/tmp upper。
- `establish` 读取 `/proc/self/mountinfo`，过滤伪文件系统、vfat/squashfs/overlay/
  fuse 等，候选逐挂载做 bwrap overlay probe 并缓存；`launch_fragment` 启动前
  补齐新出现的可 overlay 挂载。
- fork/mark/diff/accept 遍历同一组 `MountUnit`：父 upper 复制到
  `layers/<snapshot>/mounts/<hash>/upper`，子 lower 链按挂载点追加；accept 按
  挂载点分别合并且重放到目标 upper 或 `/proc/<pid>/root`。
- diff 文档中的路径带绝对挂载点前缀（例如 `/var/tmp/foo`），不会与另一个
  挂载点的相对路径冲突。
- 不能 overlay 的可写挂载在 sandbox 内 ro-bind；`passthrough_rw` 可显式直通
  rw（能力字段 `filesystem_passthrough` 列出）。
- `PathScope` 只是可选过滤器：空或同时含 workspace+tmp = 全部挂载；只传
  Workspace/Tmp 时按 cwd/`/tmp` 所在挂载过滤。
- 状态目录在 overlays 之后用 `--tmpfs` + `--remount-ro` 遮蔽；测试
  `state_directory_is_hidden_inside_sandbox` 检查空、不可写、`..` 和
  `/proc/self/root` 均无法列出内容。
- **状态目录位于被 overlay 的挂载内时的实测问题**：fork 生成的快照层位于该
  挂载下，若作为 lowerdir 会得到
  `OverlayFS: Overlay directories may not overlap`。复现：cwd 与 state 都在
  `/home`，fork 后启动子进程即报错。解决：该挂载 fork 时把父 upper **展平**
  到 child upper，快照层只用于 diff/accept 元数据，启动时 lower 只用宿主挂载
  点。其它挂载仍使用 lower 链。
- 嵌套挂载仍受 bwrap 参数层限制：父挂载与子挂载分别 probe；不能在纯 bwrap
  参数中保存子挂载 lower 后再重挂。失败候选会被 ro-bind，不静默直通。
- 实测测试：`extra_mount_fork_diff_accept_discard_round_trip`（`/var/tmp`：
  父写 a、fork 子见 a、子写 b + 删除宿主 lower、diff 报告 b/delete、accept
  目标可见、mark 覆盖额外挂载、discard 清理 upper）、
  `home_cache_and_config_writes_stay_in_home_upper`、`writes_outside_cwd_...`。

### 数据面覆盖矩阵（按挂载点）

| 操作 | 覆盖范围 | 实现 | 验证 |
|---|---|---|---|
| fork | 选中（默认全部）挂载 | 每挂载复制 upper / 追加 lower；含状态目录的挂载展平 | `extra_mount_...`、`backend_overlay_...`、`overlay_fork_isolates...` |
| mark | 选中挂载 | 每挂载独立快照副本，记录每挂载 lower | `extra_mount_...`（mark diff） |
| diff | 选中挂载 | 每挂载 delta 或 upper 副本比较，路径加挂载前缀 | `extra_mount_...`、`real_overlay_whiteout...` |
| accept | 选中挂载 | 每挂载三方合并 + 目标 upper/live root 重放 | `extra_mount_...`、`live_target_accept...` |
| discard | 整个环境/标记 | rename 环境根或快照层到 trash，覆盖全部挂载 | `extra_mount_...`、`round_trip` |
| 引用计数/GC | 每挂载 lower 链 + base + marks | `refresh_references` 遍历 mount.lower | `round_trip`（GC 清理）、`extra_mount_...`（discard 后 upper 消失） |
| 持久化 | 每挂载 `MountUnit` | environment/snapshot metadata；旧 scope 元数据迁移 | `legacy_scope_metadata_is_migrated_to_mount_units` |

## Authority 执行对比（实测）

`authority_surface_comparison` 在宿主和 sandbox 跑同一脚本：

| 项 | 宿主 | sandbox | 结论 |
|---|---|---|---|
| cwd 写入 | ok | ok | 一致 |
| /tmp 写入 | ok | ok | 一致（进 branch upper） |
| /var/tmp 写入 | ok | ok | 一致（进 branch upper） |
| ~/.cache 写入 | ok | ok | 一致（/home 整体 overlay，进 branch upper） |
| mv lower dir | ok | ok | 一致（overlay redirect/metacopy 由内核处理） |
| hardlink / symlink | ok | ok | 一致 |
| O_TMPFILE / mmap | ok | ok | 一致 |
| flock / fsync | ok | ok | 一致 |
| xattr / chmod / utimes | ok | ok | 一致 |
| stat st_dev/st_ino | btrfs inode | overlay st_dev + 变化 inode | 部分一致；overlay 正常语义 |
| statfs | btrfs | overlayfs | 不一致（机制差异） |
| id / HOME / pwd | 当前 uid/gid、$HOME、base | 同 | 一致 |
| /dev/shm | ok | ok | 一致 |
| hostname | betmul | betmul | 一致 |
| ps 计数 | 466 | 467 | 部分一致（多 bwrap/子进程） |
| inotify | ok | ok | 一致（lower 文件被 upper 修改后收到事件） |

权威写入留在 branch upper，宿主观察者看不到；何时物化回真实 workspace 仍未实现，
属于设计已知点。

## 成本表（实测/推断）

| 操作 | 方案 A btrfs 增量 | 方案 2 + btrfs 快照 | 方案 2 + reflink | 方案 2 + copy |
|---|---|---|---|---|
| fork | O(1) 快照（推断） | 快照命令 O(1)，本机实测 100k 约 1.2–1.4s（仍随 N 增长） | O(目录项数) | O(目录项数 + 数据) |
| mark | O(1) 快照 | 快照命令 O(1)，本机实测 100k 约 0.6–1.25s | O(upper 目录项数) | O(upper 数据) |
| diff lower 链 | O(增量 + send) | O(增量) | O(增量) | O(增量) |
| diff flatten | O(upper) | O(upper) | O(upper) | O(upper) |
| accept lower 链 | O(增量) | O(增量) | O(增量) | O(增量) |
| accept flatten | O(upper) | O(upper) | O(upper) | O(upper) |
| discard 热路径 | O(1) rename | O(1) rename + 后台删除 | O(1) rename | O(1) rename |

`accept` 的 O(增量) 已由本轮计数测试和最终基准验证；flatten 仍为 O(upper)。

## 性能实测

环境：kernel `7.2.14-zen1-1-zen`，CPU `AMD Ryzen 7 5800H with Radeon Graphics`。
`ASCOUT_BENCH_ITERS=20`；N=100/10_000/100_000，每文件 4KiB；child 只修改 10 个文件。
命令：

```bash
cargo run --release --example fs_bench
```

`reflink-flatten`：状态目录与 workspace 都在 `/home`（btrfs），`/home` 重叠 →
flatten。`reflink-lower`：状态目录在 `/var/tmp`（btrfs），workspace 在 `/home` →
`/home` 使用 lower 链；`/var/tmp` 自身重叠并 flatten。`tmpfs-flatten`：状态与
workspace 都在 `/tmp` tmpfs，flatten。

| case | N | operation | p50_ms | p95_ms | fork_cost | mechanism | overlaps |
|---|---|---|---|---|---|---|---|
| btrfs-snapshot-flatten | 100 | accept | 7.931 | 8.933 | btrfs_snapshot | overlay | /home |
| btrfs-snapshot-flatten | 100 | diff | 6.702 | 8.922 | btrfs_snapshot | overlay | /home |
| btrfs-snapshot-flatten | 100 | discard | 5.684 | 7.955 | btrfs_snapshot | overlay | /home |
| btrfs-snapshot-flatten | 100 | fork | 161.881 | 200.599 | btrfs_snapshot | overlay | /home |
| btrfs-snapshot-flatten | 100 | mark | 97.697 | 116.500 | btrfs_snapshot | overlay | /home |
| btrfs-snapshot-flatten | 10000 | accept | 273.469 | 322.370 | btrfs_snapshot | overlay | /home |
| btrfs-snapshot-flatten | 10000 | diff | 399.681 | 418.566 | btrfs_snapshot | overlay | /home |
| btrfs-snapshot-flatten | 10000 | discard | 7.087 | 7.825 | btrfs_snapshot | overlay | /home |
| btrfs-snapshot-flatten | 10000 | fork | 620.334 | 1260.640 | btrfs_snapshot | overlay | /home |
| btrfs-snapshot-flatten | 10000 | mark | 727.592 | 893.439 | btrfs_snapshot | overlay | /home |
| btrfs-snapshot-flatten | 100000 | accept | 2801.237 | 2833.775 | btrfs_snapshot | overlay | /home |
| btrfs-snapshot-flatten | 100000 | diff | 4320.630 | 4448.689 | btrfs_snapshot | overlay | /home |
| btrfs-snapshot-flatten | 100000 | discard | 7.319 | 10.330 | btrfs_snapshot | overlay | /home |
| btrfs-snapshot-flatten | 100000 | fork | 1176.917 | 1338.662 | btrfs_snapshot | overlay | /home |
| btrfs-snapshot-flatten | 100000 | mark | 1245.006 | 1366.863 | btrfs_snapshot | overlay | /home |
| btrfs-snapshot-lower | 100 | accept | 8.176 | 10.083 | btrfs_snapshot | overlay | /var/tmp |
| btrfs-snapshot-lower | 100 | diff | 6.435 | 8.208 | btrfs_snapshot | overlay | /var/tmp |
| btrfs-snapshot-lower | 100 | discard | 5.644 | 5.913 | btrfs_snapshot | overlay | /var/tmp |
| btrfs-snapshot-lower | 100 | fork | 215.267 | 257.893 | btrfs_snapshot | overlay | /var/tmp |
| btrfs-snapshot-lower | 100 | mark | 127.254 | 159.610 | btrfs_snapshot | overlay | /var/tmp |
| btrfs-snapshot-lower | 10000 | accept | 14.977 | 21.231 | btrfs_snapshot | overlay | /var/tmp |
| btrfs-snapshot-lower | 10000 | diff | 12.465 | 24.407 | btrfs_snapshot | overlay | /var/tmp |
| btrfs-snapshot-lower | 10000 | discard | 5.702 | 6.230 | btrfs_snapshot | overlay | /var/tmp |
| btrfs-snapshot-lower | 10000 | fork | 370.658 | 406.673 | btrfs_snapshot | overlay | /var/tmp |
| btrfs-snapshot-lower | 10000 | mark | 171.111 | 260.384 | btrfs_snapshot | overlay | /var/tmp |
| btrfs-snapshot-lower | 100000 | accept | 5.483 | 16.055 | btrfs_snapshot | overlay | /var/tmp |
| btrfs-snapshot-lower | 100000 | diff | 3.031 | 41.187 | btrfs_snapshot | overlay | /var/tmp |
| btrfs-snapshot-lower | 100000 | discard | 4.690 | 5.581 | btrfs_snapshot | overlay | /var/tmp |
| btrfs-snapshot-lower | 100000 | fork | 1379.413 | 1517.485 | btrfs_snapshot | overlay | /var/tmp |
| btrfs-snapshot-lower | 100000 | mark | 583.268 | 932.236 | btrfs_snapshot | overlay | /var/tmp |
| reflink-flatten | 100 | accept | 8.801 | 13.433 | reflink | overlay | /home |
| reflink-flatten | 100 | diff | 6.143 | 8.564 | reflink | overlay | /home |
| reflink-flatten | 100 | discard | 0.614 | 1.191 | reflink | overlay | /home |
| reflink-flatten | 100 | fork | 29.781 | 45.363 | reflink | overlay | /home |
| reflink-flatten | 100 | mark | 19.407 | 27.606 | reflink | overlay | /home |
| reflink-flatten | 10000 | accept | 272.239 | 287.042 | reflink | overlay | /home |
| reflink-flatten | 10000 | diff | 271.157 | 286.988 | reflink | overlay | /home |
| reflink-flatten | 10000 | discard | 0.575 | 0.761 | reflink | overlay | /home |
| reflink-flatten | 10000 | fork | 1732.160 | 1819.097 | reflink | overlay | /home |
| reflink-flatten | 10000 | mark | 904.401 | 1023.580 | reflink | overlay | /home |
| reflink-flatten | 100000 | accept | 2707.857 | 2995.416 | reflink | overlay | /home |
| reflink-flatten | 100000 | diff | 2705.498 | 2810.918 | reflink | overlay | /home |
| reflink-flatten | 100000 | discard | 0.680 | 0.853 | reflink | overlay | /home |
| reflink-flatten | 100000 | fork | 21334.920 | 23178.252 | reflink | overlay | /home |
| reflink-flatten | 100000 | mark | 7219.313 | 8422.537 | reflink | overlay | /home |
| reflink-lower | 100 | accept | 5.100 | 5.225 | reflink | overlay | /var/tmp |
| reflink-lower | 100 | diff | 3.107 | 3.218 | reflink | overlay | /var/tmp |
| reflink-lower | 100 | discard | 0.406 | 0.460 | reflink | overlay | /var/tmp |
| reflink-lower | 100 | fork | 16.153 | 17.086 | reflink | overlay | /var/tmp |
| reflink-lower | 100 | mark | 10.881 | 11.608 | reflink | overlay | /var/tmp |
| reflink-lower | 10000 | accept | 5.249 | 5.601 | reflink | overlay | /var/tmp |
| reflink-lower | 10000 | diff | 3.174 | 3.245 | reflink | overlay | /var/tmp |
| reflink-lower | 10000 | discard | 0.421 | 0.469 | reflink | overlay | /var/tmp |
| reflink-lower | 10000 | fork | 644.411 | 653.522 | reflink | overlay | /var/tmp |
| reflink-lower | 10000 | mark | 11.298 | 11.529 | reflink | overlay | /var/tmp |
| reflink-lower | 100000 | accept | 6.366 | 15.214 | reflink | overlay | /var/tmp |
| reflink-lower | 100000 | diff | 4.434 | 12.126 | reflink | overlay | /var/tmp |
| reflink-lower | 100000 | discard | 0.418 | 0.506 | reflink | overlay | /var/tmp |
| reflink-lower | 100000 | fork | 6817.275 | 7208.904 | reflink | overlay | /var/tmp |
| reflink-lower | 100000 | mark | 16.736 | 51.573 | reflink | overlay | /var/tmp |
| tmpfs-flatten | 100 | accept | 2.564 | 2.582 | copy | overlay | /tmp |
| tmpfs-flatten | 100 | diff | 1.662 | 1.714 | copy | overlay | /tmp |
| tmpfs-flatten | 100 | discard | 0.108 | 0.122 | copy | overlay | /tmp |
| tmpfs-flatten | 100 | fork | 6.493 | 6.534 | copy | overlay | /tmp |
| tmpfs-flatten | 100 | mark | 3.393 | 3.451 | copy | overlay | /tmp |
| tmpfs-flatten | 10000 | accept | 163.562 | 171.390 | copy | overlay | /tmp |
| tmpfs-flatten | 10000 | diff | 162.097 | 166.976 | copy | overlay | /tmp |
| tmpfs-flatten | 10000 | discard | 0.142 | 0.161 | copy | overlay | /tmp |
| tmpfs-flatten | 10000 | fork | 617.501 | 628.632 | copy | overlay | /tmp |
| tmpfs-flatten | 10000 | mark | 315.190 | 317.426 | copy | overlay | /tmp |
| tmpfs-flatten | 100000 | accept | 1665.411 | 1715.608 | copy | overlay | /tmp |
| tmpfs-flatten | 100000 | diff | 1667.799 | 1816.359 | copy | overlay | /tmp |
| tmpfs-flatten | 100000 | discard | 0.149 | 0.168 | copy | overlay | /tmp |
| tmpfs-flatten | 100000 | fork | 6167.011 | 6366.283 | copy | overlay | /tmp |
| tmpfs-flatten | 100000 | mark | 3148.152 | 3249.308 | copy | overlay | /tmp |

### 结论与瓶颈（如实）

- **lower 链 accept 已做到 O(增量)**：`reflink-lower` 的 accept 在
  100 / 10k / 100k 下 p50 约 5.1 / 5.2 / 6.4ms，基本不随 upper 总大小增长。
  这是本轮任务 1 的实测结果。
- **lower 链 diff 仍是 O(增量)**：约 3.1 / 3.2 / 4.4ms。
- **lower 链 mark 接近 O(1)**：约 10.9 / 11.3 / 16.7ms。
- **fork 仍随 N 增长**：reflink lower 16ms → 644ms → 6.82s。reflink 只省数据
  拷贝，仍要遍历 N 个目录项；btrfs 快照副本本机已启用，100k fork 约 1.2–1.4s，
  明显优于 reflink，但仍非严格 O(1)。
- **flatten 路径仍 O(upper)**：`reflink-flatten` fork 29.8ms → 1.73s → 21.3s，
  mark 19.4ms → 904ms → 7.22s，diff/accept 也随 N 增长。状态目录与 workspace
  同挂载时无法使用 lower 链增量。
- **tmpfs 复制路径随 N 增长**：100k fork 6.17s、mark 3.15s、accept 1.67s。
- **discard 热路径仍 O(1)**：所有 case 约 0.1–0.7ms。
- 本轮基准未再出现 prior background removal 的 NotFound 告警。
- **btrfs 快照副本已在本机启用**（`fast_delete=false` 仅表示后台 fallback
  为 O(文件数)）。100/10k/100k 的 fork p50 约 162/620/1177ms（flatten）与
  215/371/1379ms（lower），比 reflink 的 29.8ms/1.73s/21.3s（flatten）明显更好，
  但仍随 N 增长，不是严格 O(1)：本机 btrfs 快照命令对 100k 文件仍有约 1s 级
  首次元数据/提交成本，且每个 fork 对每个挂载发一次 snapshot 命令。mark 也有
  类似成本。diff/accept flatten 仍 O(upper)。

### 结论与瓶颈（如实）

- **flatten 路径随 N 线性增长**：`reflink-flatten` 的 fork 从 162ms 增到 16.18s，
  mark 从 213ms 增到 6.77s，diff 从 54ms 增到 2.59s，accept 从 260ms 增到 5.14s。
  `tmpfs-flatten` 同样线性增长。原因是 fork/mark 复制整个 upper、diff 比较完整
  upper，属于本轮已知成本。
- **lower 链 diff 确实是 O(增量)**：`reflink-lower` 的 diff 在 100、10k、100k
  下分别约 3.8ms、4.3ms、4.2ms，几乎不随 N 增长。
- **lower 链 mark 接近 O(1)**：约 15ms，不随 N 增长（child upper 为空）。
- **lower 链 accept 仍随 N 线性**：22ms → 约 235ms → 2.16s。当前 accept 在冲突检测
  前读取完整 `target_mount.upper`，并构建完整 source maps，因此不是 O(增量)。
  这是明确的优化点，本轮未改。
- **fork 即使 reflink 也随 N 增长**：reflink 只省数据拷贝，仍要遍历/复制 N 个
  目录项；`reflink-lower` fork 19.5ms → 约 679ms → 6.57s。
- **discard 热路径接近 O(1)**：所有 case 约 0.1–0.6ms。
- **复制型 tmpfs 明显更慢**：100k 文件 fork 6.15s、mark 3.13s、accept 3.32s。
- 基准结束清理阶段出现一次后台删除竞态：
  `background removal ... failed: No such file or directory`；属于清理/后台删除
  路径已不存在时的竞态，已记录在 `docs/refactor-notes.md`，本轮未修。

### 第八轮新增测试证据

| 项 | 证据 |
|---|---|
| accept 不扫描无关 upper | `accept_work_does_not_scale_with_unrelated_upper_size`（read_layer entry 计数） |
| staged clone 崩溃恢复 | `failed_clone_and_orphan_staging_are_not_valid_layers` |
| 持久化失败返回错误 / 锁外写盘 | `persistence_failures_are_reported_and_locks_are_released` |
| opaque 删除失败中止重放 | `opaque_removal_error_aborts_replay` |
| SafeRoot mkdir mode | `safe_root_mkdir_preserves_source_mode` |
| btrfs send stderr drain | `send_dump_drains_large_send_stderr` |
| 损坏 operations.json 隔离 | `corrupt_operations_metadata_is_isolated` |
| btrfs 快照 fork | `btrfs_snapshot_copy_fork_is_end_to_end`（本机通过） |

## P4 调研结论：btrfs 增量

本机实测：

```text
btrfs subvolume find-new <path> <gen>   -> Operation not permitted
btrfs send -p <snapshot> <subvolume>    -> Operation not permitted
btrfs subvolume show <path>             -> Operation not permitted
```

因此探测增加 `incremental_diff`：真实尝试
`btrfs send -p <snapshot> <snapshot> | btrfs receive --dump`。成功才选择 btrfs；
实现已启用 send dump diff/accept。结构是否改为 delta 链待用户决定，本轮未改
btrfs fork 结构。

## D. 冻结（简单版）调研

- 旧实现（已修正）：`BaseRuntime::suspend_with_context` 曾只改状态并委托
  inner/filesystem，没有真正的 freeze。现在改为 ProcessBackend 真实 freeze/thaw。
- **cgroup v2**：本机 `/sys/fs/cgroup` 根不可写，但当前进程所在的 delegated
  `.scope` 可创建子 cgroup、写入 `cgroup.procs`、写 `cgroup.freeze=1`，并观察到
  `cgroup.events` 中 `frozen 1`。实测成功（无需 sudo）。
- **SIGSTOP/SIGCONT**：BaseRuntime 已在 `pre_exec` 设置独立进程组，终止用
  `kill(-pgid, SIGKILL)`；同样的进程组原语可扩展为 `SIGSTOP/SIGCONT`，但当前
  没有实现。限制：Agent 自建新 session/进程组后后代不受该组信号影响。
- 本轮已实现 cgroup v2 freeze，作为 ProcessBackend 的 freeze/thaw；cgroup 不可用
  时 fallback 到进程组 SIGSTOP/SIGCONT。`BaseRuntime` 在 fork/mark/accept 前冻结、
  之后解冻，只有真实冻结+解冻成功才把 `writers_quiesced` 置 true。
- `writers_quiesced` 不由 FS Backend 写 true；FS 侧始终 false，由 BaseRuntime
  依据 ProcessBackend 结果覆盖。

## P5 调研结论：整根 overlay

### 实测结果

- overlay `/home`、`/srv`、`/var/log`、`/var/tmp`、`/extra`，状态 upper/work 在
  `/tmp`（不同文件系统）：全部成功。
- overlay `/tmp`，状态 upper/work 在 `/home`（不同文件系统）：成功。
- overlay `/tmp` 且 upper/work 就在 `/tmp` 下：成功。所以 upper/work 位于 lower
  路径之下本身不是 EINVAL 原因。
- overlay `/`：`fsconfig(FSCONFIG_CMD_CREATE) = -1 EINVAL`（strace）。
- `mount --bind / <dir>`：`open_tree(AT_FDCWD, "/", OPEN_TREE_CLONE) = -1 EINVAL`。
- `mount --rbind / <dir>` 能成功，但以该 rbind 路径为 lower 仍 EINVAL。
- 按挂载点：`/home`、`/tmp`、`/srv`、`/var/log`、`/var/tmp`、`/extra` OK；
  `/run`、`/dev`、`/proc`、`/sys` 失败；`/run/credentials/*`（叶子 tmpfs）OK；
  `/run/user/1000` 失败（它自身有子挂载 `/run/user/1000/doc`）；
  `/boot/efi`（vfat）失败，原因不是锁定子挂载，而是 vfat 不支持作为 overlay lower。
- 用 pivot_root 切到一个新 tmpfs 根，再在 userns 内创建 `/usr`、`/lib`、`/etc`、
  `/tmp` 等子挂载（这些挂载由本 userns 创建，不是从父 userns 继承的 locked 挂载），
  然后 `lowerdir=/` 的 overlay 成功。这直接证明“lower 是命名空间根”或
  “upper 在 lower 下”都不是 EINVAL 原因。

### 真实原因

strace 只显示 `fsconfig`/`open_tree` 返回 EINVAL；内核路径在 `fs/namespace.c`：

```c
bool has_locked_children(struct mount *mnt, struct dentry *dentry)
{
    list_for_each_entry(child, &mnt->mnt_mounts, mnt_child) {
        if (!is_subdir(child->mnt_mountpoint, dentry))
            continue;
        if (child->mnt.mnt_flags & MNT_LOCKED)
            return true;
    }
    return false;
}
```

- `clone_private_mount()`（overlay lower/upper 都会调用）：
  `if (has_locked_children(old_mnt, path->dentry)) return ERR_PTR(-EINVAL);`
- `__do_loopback()` 的非递归 bind/open_tree 路径：
  `if (!recurse && has_locked_children(old, old_path->dentry)) return ERR_PTR(-EINVAL);`
- 进入新的 userns/mount namespace 时，从父命名空间继承的挂载树会在
  `lock_mnt_tree()` 中被打上 `MNT_LOCKED`（非 root 的子挂载）。
- 宿主 `/` 有 `/proc`、`/sys`、`/dev`、`/home`、`/tmp` 等子挂载；这些继承了
  `MNT_LOCKED`。因此 `/` 的 `clone_private_mount` / `open_tree(CLONE)` 返回 EINVAL。
- `/home`、`/tmp` 等叶子挂载没有 locked 子挂载，所以 overlay 成功。
- `/run`、`/run/user/1000` 有子挂载；`/run/credentials/*` 是叶子所以成功。

### 整根 overlay 能否成立

- **无特权、直接 lower=/：不可行**（根继承的 locked 子挂载）。
- **无特权、新根/新挂载树：可行**。实验：pivot_root 到 userns 内新建的 tmpfs 根，
  子挂载也由 userns 创建（非 locked），`lowerdir=/` overlay 成功。
- **有特权初始 userns：预计直接 lower=/ 可行**，因为继承挂载不再是 MNT_LOCKED；
  本机无 sudo/初始 userns 权限，未实测，标记推断。
- **备选：逐挂载点 overlay**：可行，但必须处理 locked 子挂载和嵌套挂载。

### 嵌套/相邻挂载

- 相邻挂载：bwrap 同时 overlay `/home` 和 `/tmp`，两边内容都正确，实测 OK。
- 嵌套挂载：父挂载 overlay 后，子挂载会被隐藏。实测顺序必须是：
  1. 先把子挂载的原 lower bind 到一个稳定路径（`mount --bind <child> <saved>`）；
  2. overlay 父挂载；
  3. 在父 overlay 内重建子挂载点；
  4. 用 saved lower 在子挂载点做 child overlay。
  实测父子两个 tmpfs 同时 overlay 后，父文件与子文件都可见。
- 禁止对同一 target 或互为祖先的两个 target 各自独立 overlay 而不做上述重挂。

### 性能注意

- 本机真实可写挂载约 13 个（`/`, `/home`, `/srv`, `/root`, `/var/cache`,
  `/var/log`, `/var/tmp`, `/extra`, `/tmp`, `/boot/efi`, `/run`,
  `/run/user/1000`, docker overlay）。逐挂载 overlay 会产生同等数量的 overlay
  mount；启动/清理成本随挂载数线性增长。
- 实测 200 次 mount+umount overlay（lower=/home）约 634 ms，即 ~3.2 ms/次
  （含 mount 进程开销）。
- 元数据开销实测（200000 次 stat，lower=/home）：
  - 直接 lower：1.807 us/stat
  - overlay：2.422 us/stat（约 +34%）
- 每个 overlay 只用一个 lower 层时，路径查找只多一层；不要把大量 lower 层叠在
  一个 overlay 上。挂载数增加主要影响 namespace 建立、dentry/inode 缓存和清理，
  不会让每次 stat 按挂载数线性变慢。

### 推荐部署形态

- **首选（有特权）**：原生 mount namespace + CAP_SYS_ADMIN，状态目录放在独立
  挂载点（tmpfs 或专用盘），直接 `lower=/`、`upper=<state>/upper`、
  `work=<state>/work`、`target=/`；根在初始 userns 中，子挂载没有 MNT_LOCKED。
- **无特权**：不要直接 lower=/。要么：
  - 用 pivot_root/新根，把所有需要的挂载由本 userns 重新创建为 unlocked 挂载，
    再 overlay `/`；或
  - 逐挂载点 overlay：跳过有 locked 子挂载的父挂载，先保存子挂载 lower，
    overlay 父挂载后，在父 overlay 内按 child saved lower 重放子 overlay。
- **状态目录位置**：放在一个不会被覆盖的独立挂载点（例：独立 tmpfs `/run/ascout`
  或专用磁盘），不要放在被 overlay 的父/子挂载内部，避免自递归和挂载后不可见。

## 陈述证据表

| 陈述 | 证据 |
|---|---|
| `/proc/<pid>/root` 用错 pid 会写宿主 Base | **实测**：外层 bwrap pid A 的 ns 与宿主相同，用 A 删除写到了宿主 Base |
| 内层 sandbox pid 可无特权重放 | **实测**：`--json-status-fd` 给 child-pid B；用 B 删除在 upper 产生 0/0 whiteout，Base 不变 |
| bwrap json-status-fd 提供 child-pid/mnt-namespace | **实测**：`{ "child-pid": 55547, "mnt-namespace": 4026533064 }` |
| `openat2(RESOLVE_IN_ROOT)` 阻止绝对符号链接逃逸 | **实测**：`live_replay_blocks_absolute_symlink_escape` 通过，host sensitive 未写 |
| btrfs find-new/send 无权限 | **实测**：上述命令输出 EPERM |
| 跨文件系统 overlay /home、/tmp 可行 | **实测**：bwrap 命令 exit 0 |
| upper/work 在 lower 路径之下本身不会 EINVAL | **实测**：lower=/tmp、upper/work 在 /tmp 下成功 |
| 根 `/` overlay EINVAL 来自 locked 子挂载 | **实测 + 源码**：open_tree(CLONE) on / = EINVAL；clone_private_mount/__do_loopback 的 has_locked_children 返回 EINVAL；pivot 到新根后 lower=/ 成功 |
| 无特权新根（子挂载由 userns 创建）可 lower=/ overlay | **实测**：pivot_root 新 tmpfs 根，lower=/ overlay rc=0 |
| 初始 userns 特权下 lower=/ 可行 | **推断**：基于 MNT_LOCKED 只在跨 userns 继承时设置；本机无 sudo，未实测 |
| 相邻挂载同时 overlay | **实测**：bwrap 同时 overlay /home 与 /tmp，exit 0 |
| 嵌套挂载可同时 overlay（先保存 child lower，再 parent，再 child） | **实测**：父子 tmpfs overlay 后两个文件都可见 |
| 逐挂载 overlay 的挂载数/元数据成本 | **实测**：13 个可写挂载；200 次 mount+umount ~634ms；stat +34% |
| btrfs fork/diff/accept 端到端 | **未验证（需高权限）**：send/`find-new` 本机 EPERM，测试 SKIP；代码与 `scripts/priv-check.sh` 已就绪 |
| ProcessBackend 启动/冻结/解冻 | **实测**：`process_freeze_thaw_stops_and_resumes_writer` 冻结期间 counter 不变，thaw 后继续 |
| 进程组 SIGSTOP/SIGCONT fallback | **实测**：`process_group_fallback_freezes_and_reports_mechanism`，能力上报 `process_group` |
| BaseRuntime writers_quiesced | **实测**：`base_runtime_freezes_and_marks_writers_quiesced` 有进程时 true，无进程时 false |
| 状态目录在 sandbox 内不可见 | **实测**：`state_directory_is_hidden_inside_sandbox` |
| 默认逐挂载写入 /tmp、/var/tmp、/srv | **实测**：`writes_outside_cwd_reach_branch_uppers_not_host`，宿主不变、branch upper 出现 |
| authority 行为对比 | **实测**：`authority_surface_comparison`；~/.cache、/tmp、/var/tmp 写入一致，inotify 一致 |
| 额外挂载 fork/diff/accept/discard | **实测**：`extra_mount_fork_diff_accept_discard_round_trip`（/var/tmp） |
| ~/.cache、~/.config 写宿主不变 | **实测**：`home_cache_and_config_writes_stay_in_home_upper` |
| 旧 scope 元数据迁移 | **实测**：`legacy_scope_metadata_is_migrated_to_mount_units`；缺 workspace_root 明确报错 |
| 冻结机制 none 不阻塞 FS 操作 | **实测**：`base_runtime_without_freeze_mechanism_still_runs_operations` |
| freeze 失败 / thaw 失败 | **实测**：`freeze_failure_fails_before_filesystem_operation`、`thaw_failure_is_reported_as_cleanup_failure` |
| 进程已退出不挂死 | **实测**：`exited_process_does_not_block_filesystem_operation` |
| cgroup.kill 与 cgroup 目录清理 | **实测**：`terminate_uses_cgroup_or_group_kill_and_cleanup_removes_cgroup` |
| fallback O(文件数) 后台删除成本 | **推断**：实现为递归删除；未做大数据量性能测量 |
| overlay work 目录 mode 000 需要 force 删除 | **实测**：测试残留复现并修复 |
| accept 回滚覆盖修改/删除/创建/目录替换 | **实测**：`accept_replay_rolls_back_every_injected_failure`、`replay_directory_replacement_rolls_back`；回滚失败见 `accept_replay_reports_rollback_failure` |
| cgroup v2 在当前 delegated scope 可 freeze | **实测**：创建子 cgroup、写 cgroup.procs、写 cgroup.freeze=1，cgroup.events 显示 frozen 1 |

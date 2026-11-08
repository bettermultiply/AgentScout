# 文件系统 Backend 当前实现状态

本文只记录实现事实，不定义目标语义。末尾逐条标注“实测”或“推断”。

## 已实现

- **overlay fork（独立 upper 副本）**
  - 只复制父当前 upper，不复制 lower 链和 Base。
  - 逐文件优先 FICLONE reflink；不支持时普通复制。保留目录、whiteout 字符设备、
    overlay xattr、权限、符号链接和 mtime；不使用硬链接。
  - 子 lower = 父 lower + S，子 upper 为空。父 upper、lower 和运行中挂载不变。
  - S 由子（以及 mark/diff 基准）引用，按引用计数 GC。
  - `filesystem_fork_cost=reflink|copy`，启动时在状态目录文件系统上试 FICLONE。
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
- **mark**
  - overlay：逐选中挂载独立复制当前 upper，不改 live upper。
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
  - 路径级操作日志：记录已应用的创建操作；失败时逆序删除已创建文件/目录/符号
    链接。修改、删除、opaque 替换没有保存旧内容，**不可逆**；这类操作会在错误
    消息中报告数量，`published=false`。因此仍非原子，但不宣称原子。
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

## 未实现 / Phase 2

- accept selector、依赖闭包、控制面逐条变更列表。
- `diff(target=Some(_))` 明确返回未实现。
- accept 对“修改/删除/opaque”没有旧内容备份，不能完整逆序回滚；当前只回滚已创建
  的文件/目录/符号链接，仍明确非原子。
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
- 当前挂载含状态目录时，fork 采用“upper 展平 + 快照仅作 diff 元数据”，不再把
  该快照层作为 lowerdir；这是为了避免 OverlayFS 的
  `Overlay directories may not overlap`。

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
| id / HOME / pwd | 1000/1000, /home/betmul, base | 同 | 一致 |
| /dev/shm | ok | ok | 一致 |
| hostname | betmul | betmul | 一致 |
| ps 计数 | 466 | 467 | 部分一致（多 bwrap/子进程） |
| inotify | ok | ok | 一致（lower 文件被 upper 修改后收到事件） |

权威写入留在 branch upper，宿主观察者看不到；何时物化回真实 workspace 仍未实现，
属于设计已知点。

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
| accept 只回滚创建类操作 | **实测/代码事实**：路径日志 + 逆序删除创建；修改/删除/opaque 不可逆 |
| cgroup v2 在当前 delegated scope 可 freeze | **实测**：创建子 cgroup、写 cgroup.procs、写 cgroup.freeze=1，cgroup.events 显示 frozen 1 |

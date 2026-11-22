# 文件系统 Backend 当前实现状态

本文只记录实现事实，不定义目标语义。末尾逐条标注“实测”或“推断”。

## Authority 物化实现与验收边界（2026-11-14）

Pi 会话宿主不再持有 Authority overlay，Branch 内建文件工具按对应 Runtime 计划
建立独立挂载。shell 使用私有 PID namespace 的 PID 1 supervisor 等待所有后代；
同 Branch 工具操作串行使用 upper/work。常驻 Pi 不作为活跃写者，其他直接受管进程
仍保留物化安全门槛。成功后创建新 upper/work，不清空 live upper。

恢复允许普通宿主 lower，使用恢复时内容，不保存整个挂载；受管增量、候选、
Agent/Policy 状态独立保存，保存代和代码/配置/依赖校验保留。

新建环境不复制宿主挂载或建立全量物化基准。物化从增量层选取路径，使用只读
overlay 内核视图导出受影响节点；opaque/redirect 才展开对应子树。所有挂载先导出，
再逐路径写回；失败保留包与已成功路径回执，不检测或报告冲突。

定向回归覆盖启动无挂载副本、最终视图权限/删除/目录/符号链接边界、
重复选择子树不会覆盖导出数据、部分失败重试不重复清理已完成 opaque 目录、
过期增量子路径不会沿最终视图的祖先符号链接读取挂载外内容。
`cargo check --all-targets` 通过。验证数据全部位于 `/tmp/opencode`，未调用真实模型。
物化以 `O_PATH|O_NOFOLLOW` 固定目录 inode，因此可遍历/修改但不可读的已有目录仍可
作为路径父级并设置权限位；不临时放宽权限。目录子树清理仍需读取目录项，缺少读权限时
会失败并保留已完成回执，修复访问条件后可重试。符号链接仍不作为目录跟随。
普通用户回归还验证了 `0300` 宿主根和嵌套父目录上的文件写入，以及后续失败/重试期间
这些目录权限不被提前改动；重试跳过已有文件回执。

`tests/authority-filesystem.mjs` 覆盖后台写者等待、两个 task 不重放、文件工具、取消、
部分失败重试与 SIGINT。`tests/recovery-filesystem.mjs` 当前宿主 lower 加独立 upper
恢复通过。FS 候选暂停、恢复、晋级、旧父清理后工具写入及再次保存通过，证据
`/tmp/opencode/authority-active-final5.log` 与 `/tmp/opencode/authority-e2e-final6.log`。
saved_base 最后引用及 snapshot 下层链纳入回收；计划切换失败保留旧代和回执由库
回归覆盖，`/tmp/opencode/authority-lib-final5.log` 为 161/161（包含环境条件跳过的项，
不代表原生 btrfs 特权验收）。原生 btrfs incremental 本机未通过探测。
最终联合验证 `node --test --test-concurrency=1 tests/authority-filesystem.mjs tests/recovery-filesystem.mjs tests/recovery-active.mjs`
7/7 通过，日志 `/tmp/opencode/authority-e2e-final7.log`。证据分别为
`authority-filesystem-tSXSIp`、`recovery-active-GmHpEZ`、`recovery-filesystem-fyfaLL`
（均位于 `/tmp/opencode`）。取消与 SIGINT 夹具等待实际写入标记，不以固定延时
假设工具已开始。`cargo check --all-targets`、`cargo build --bins`、`git diff --check`
通过。任意扩展宿主代码、深层候选组合及所有属性/链接组合不在此验收覆盖范围。

## 已实现

- **overlay fork（逐挂载 Chain / Flatten）**
  - 空 upper 完全跳过（见下），非空 upper 为每个挂载单独选择一次布局。
  - **Chain**：对父 upper 做只读快照/复制得到 S；子 lower = 父 lower + S，
    子 upper 为空。fork 便宜，子 upper 只含子自己的改动，diff/accept 成本只随
    改动量增长；每次 Chain fork 都会让该挂载的 lower 链加深一层。
  - **Flatten**：父 upper 整个复制（或可写快照）成子 upper；子 lower = 父 lower，
    不加入 S。链不增长，但子 upper 混着父的内容，diff/accept 要拿子 upper 与
    基准 S 比较，成本随 upper 条目数增长。fork 时需要复制父 upper，冻结窗口内
    的复制成本更高。这里只是不增加本次 fork 的层，不会压平或移除已继承的 lower 链。
  - 逐挂载布局规则：状态目录位于该挂载内（只可能由内部开关
    `allow_overlapping_state_directory=true` 制造）时强制 Flatten；否则用
    `upper_entry_count` 封顶计数，条目数 < `DEFAULT_FORK_FLATTEN_ENTRY_LIMIT`
    走 Flatten，否则走 Chain。计数的成本是 O(min(N, 阈值))。
  - 逐文件优先 FICLONE reflink；不支持时普通复制。保留目录、whiteout 字符设备、
    overlay xattr、权限、符号链接和 mtime；不使用硬链接。
  - 子挂载的 lower 只有在 Chain 布局下才追加 S；Flatten 不追加。父 upper、lower
    和运行中挂载不变。
  - `SnapshotRecord.lower`（快照的视图链）无论子挂载采用哪种布局，都显式记录
    **父 lower + S**（该挂载有产出时）。diff/accept 的三方合并以它为基准视图；
    若直接取子挂载 lower，Flatten 下会缺少 S 的内容。
  - S 由子（以及 mark/diff 基准）引用，按引用计数 GC。
  - 快照层/副本层先写入 staging 目录，完成后原子 rename 到最终路径并 sync 父目录；
    没有完成 rename 的 `.tmp-*` 在下次启动清理，不被视为有效层。复制文件不再
    逐文件 `sync_all`。
  - `filesystem_fork_cost=btrfs_snapshot|reflink|copy`。选择顺序：
    **方案 A（btrfs 增量）→ 方案 2 + btrfs 快照副本 → reflink → copy**。
  - 当状态目录在 btrfs 且 `probe_btrfs_snapshot_copies` 成功（创建/只读快照/
    overlay lower/子卷删除均可用）时，每个挂载使用父 volume 子卷，其中 upper/work
    都是普通子目录，fork/mark 用只读快照产生层。Chain 布局创建子 volume，再建
    空 upper/work；Flatten 布局用 S 的可写快照作为子 volume。启用探测要求创建/只读快照/
    overlay lower/btrfs-aware 递归删除成功；`fast_delete` 只影响后台删除成本，
    不阻止启用。
  - **空 upper 跳过**：fork/mark 先对每个挂载用 `read_dir` 取第一个条目（O(1)）
    判断 upper 是否为空。空 upper 不建层、不复制，子的该挂载直接得到空 upper，
    lower 链沿用父链。投机分支通常只有 workspace 挂载被写，其余系统挂载全部走
    这条路径。
  - **按挂载自适应生产者**：非空 upper 先递归计数到 `SNAPSHOT_CLONE_ENTRY_LIMIT`
    （256）即停止，成本 O(min(N, 256))。条目数 < 256 用 reflink（不支持时普通
    复制），≥256 用 btrfs 快照；记录在 `MountUnit::volume_is_subvolume=false`
    的挂载（空挂载产生的普通目录）只能继续复制，不能快照。这个阈值只决定
    “S 怎么产生”（复制还是快照），与下面的 fork 布局阈值是两件不同的事，不要
    混用。
  - **逐挂载布局阈值**：`DEFAULT_FORK_FLATTEN_ENTRY_LIMIT = 256`，可由
    `FilesystemConfig::fork_flatten_entry_limit` 覆盖（测试与基准使用；
    `0` = 总是 Chain，`usize::MAX` = 总是 Flatten）。它决定本次 fork 把父
    upper 放进子 upper（Flatten）还是放进子 lower 链（Chain）。
  - **链深度上限**：`MAX_LOWER_DEPTH = 96`。若本次 Chain 会让子 lower 超过该
    上限，`fork` 在创建任何目录或元数据之前返回 `BackendError`，说明当前深度、
    上限和原因；不静默降级为 Flatten，也不合并整条链。本机实测 bwrap 在 231
    个 lower 源时成功、232 个时 `Invalid argument`；bwrap 会把每个源转换成
    `/proc/self/fd/N`，所以限制不是底层路径长度。96 低于实测最大值的一半并留
    余量。
  - **有界并行**：多个非空挂载的层/子卷创建最多用 4 个线程；同一挂载的“层→
    子 upper”保持顺序。任一挂载失败仍整体回滚 child root 与 staging，错误按挂载
    顺序返回（与串行循环一致）。
  - fork/mark 只持久化本操作新增的 snapshot 元数据，不再重写全部历史 snapshot；
    fork 的元数据持久化成本因此不再随历史分支数增长。
  - 经 Backend 接口调用 fork/mark/accept 时，FilesystemBackend 冻结环境写者后执行操作。确认写入已静止时报告 `writers_quiesced=true`。底层文件操作需由调用者保证一致性。
- **btrfs fork**
  - 每个挂载使用 `mounts/<hash>/volume` 子卷，其中 upper/work 都是普通子目录。
    fork 对每个选中且非空的父 upper 所在 volume 做只读快照 S_child。Chain 布局
    创建空的 child volume 并把 S_child 的 upper 子树加入子 lower；Flatten 布局
    用 S_child 的可写快照作为 child volume，子 lower 沿用父 lower。空挂载也创建
    child volume，再建立 upper/work。含状态目录的挂载（仅内部开关）强制 Flatten。
  - upper/work 必须同设备、同 btrfs 子卷；创建、发现新挂载、fork 和启动前检查
    `st_dev`。元数据恢复还检查 btrfs volume/upper 结构，不兼容布局返回含路径的
    错误并保留原数据，不自动搬迁；应保留状态目录并在新目录重建环境。
  - 父 upper/lower 和挂载不变。
  - 链布局下 accept 优先走 `delta_since`（子 upper 只含本分支改动）；只有
    Flatten 才用 `btrfs_delta`（拿 S 全量与子 upper 比较）。否则 Chain 会把
    父内容全部误判为删除。用户 2026-11-10 第四轮验证已覆盖两种布局与 mixed 的成功路径，
    包括真实进程写入、精确 diff、accept 后真实视图及 discard 清理。
  - btrfs 只有在 `incremental_diff=true` 时才被选为数据面；探测真实执行
    `btrfs send -p <snapshot> <snapshot> | btrfs receive --dump`。失败则降级
    overlay 并在日志/能力中说明。btrfs diff/accept 已实现：diff 解析 send dump，
    accept 用同一 dump 构建增量视图后走 live-mount/openat2 重放。增量 diff 快照
    父 volume，dump 排除内部 work 和 volume 元数据，只解释 upper 子树并保留
     其中父目录变化。普通用户无 send 权限时会 SKIP 完整 btrfs 测试；无特权真实
     create/snapshot/rename/mark/reload 与 dump 解析已有回归。用户第四轮特权运行
     已验证 Chain/Flatten/mixed 的 fork/diff/accept/discard，能力为
     `incremental_diff=true`、`fast_delete=true`。
- **btrfs 快照副本（overlay 方案 2）**
  - 探测 `probe_btrfs_snapshot_copies`：建子卷、写文件、只读快照、把快照作为
    overlay lower 真实 bwrap 挂载，再走 btrfs-aware 递归删除路径确认能删净。
    创建/快照/挂载/删除探测全部成功才启用；`fast_delete` 不再作为门槛，只影响
    后台删除成本，`filesystem_fast_delete=false` 与 `fork_cost=btrfs_snapshot`
    可以同时出现。
  - 每个挂载使用父 volume 子卷，upper/work 是其普通子目录；fork 的快照生产者
    快照 volume，lower 使用其中 upper 子树，Chain 创建空 child volume，Flatten
    用可写快照。mark 同样快照 volume。
  - 空 upper 同样跳过；小 upper（<256 条目）用 reflink/复制而不是快照，只有
    ≥256 条目且 volume 为 btrfs 子卷的挂载才走快照；复制得到的普通目录继续复制。
  - 选择顺序：方案 A → 方案 2 + btrfs 快照 → reflink → copy。
  - 本机普通用户 `btrfs subvolume delete` EPERM，但 `property set ro false`
    + 递归删除可删净；因此本机现在也启用 btrfs 快照副本，`fast_delete=false`
    只表示后台删除走 O(文件数) fallback。
  - 测试：`btrfs_snapshot_copy_fork_is_end_to_end`（本机实际运行并通过，含
    snapshot xattr、whiteout、后台删除清空断言）。

- **mark**
  - overlay：逐选中挂载独立复制当前 upper，不改 live upper；空 upper 不产生层。
  - overlay + btrfs_snapshot：非空挂载按与 fork 相同的阈值选择快照或复制；空
    upper 直接记录为空视图。
  - btrfs：逐选中挂载只读子卷快照，不改 live upper。
  - 独立 mark 的层只表示该时刻的增量；空层在 diff/accept 中按空视图处理。
- **diff**
  - 只表示当前环境相对 fork 基准（或指定快照基准）的变化，不支持比较另一个环境；
    accept 的目标环境参数不受此限制。
  - overlay Chain：扫描分支 upper 增量，不读内容；Flatten 比较当前 upper 与基准副本。
  - 对 in-lower 的 fork 基准，扫描 base 之后的 delta 层和当前 upper。
  - 对独立 mark 副本，比较当前 upper 与 mark 副本元数据，O(upper entries)。
  - btrfs：send dump 解析 create/modify/delete/rename/chmod/xattr，whiteout
    （mknod c 0 0）翻译为 delete；opaque/redirect/metacopy 通过读取 upper 的
    overlay xattr 保留。只有 `incremental_diff=false` 时不被选中。
- **accept（live mount 重放）**
  - 三方冲突检测保留；ours 逐路径读取目标 lower+upper，不构建完整视图。
  - `target_mount_pid=None`：调用方确认无 live mount，直接写目标 upper，删除写
    whiteout。
  - `target_mount_pid=Some(inner_pid)`（由 FilesystemBackend 的进程句柄提供）：
    - 校验 `/proc/<pid>/ns/mnt` 等于原生 holder 就绪消息记录的 mount namespace，
      且不等于宿主 mount namespace。PID 是宿主侧可见的常驻 holder PID。
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
- **状态目录重叠检测与默认拒绝启动**
  - `establish` 与 `launch_fragment` 在创建环境根、upper 目录或环境元数据之前，
    检查状态目录是否位于本次要 overlay 的任一挂载点内。
  - 默认拒绝：返回 `BackendError`，错误包含状态目录路径、冲突的挂载点路径，以及
    修复建议“把状态目录放到被隔离挂载点之外，例如独立挂载的文件系统”。拒绝时
    不创建环境根、upper 目录或环境元数据，不留半成品。
  - 只有内部开关 `allow_overlapping_state_directory=true`（仅测试与诊断）保留
    flatten 降级；此时行为与之前一致：含状态目录的挂载展平，快照层只作 diff
    元数据。该开关属于代码配置，不能由外部配置文件设置，默认 false。
  - 能力字段 `state_directory_overlaps_mount` 仍如实上报；启动日志说明默认拒绝
    启动，不再把 flatten 描述为正常策略。
  - 测试：`overlapping_state_directory_is_rejected_by_default`、
    `overlapping_state_directory_is_reported_and_flattened`、
    `non_overlapping_state_directory_uses_lower_chain`、
    `default_config_starts_when_state_directory_is_outside_isolated_mounts`。

- **环境启动**
  - FilesystemBackend 把挂载计划交给 Runtime Adapter 同目录的 `ascout-fs-holder`。
    原生进程建立 user、mount 和 PID namespace，在完成挂载并监听 `cmd.sock` 后通过
    管道发送宿主侧可见的 PID 与 mount namespace。Adapter 校验并持有 namespace 句柄。
  - 派生环境在同步完成 fork 和 holder 目录准备后返回。父环境此后的写入不会改变
    这次 fork 的文件系统基准。后台任务完成实际 holder 启动。首次工具操作、冻结、
    排空写入、live-mount accept 和保存等待就绪并保留启动错误。终止及清理等待
    启动收敛后再回收进程和目录。
  - `examples/fs_holder_bench.rs` 分别计时数据层准备、分支返回、首次读取完成。
    本机 release 构建在 `/var/tmp` 上跑 30 轮，派生返回 p50 为 0.694 ms、p95 为 6.992 ms，
    首次读取完成 p50 为 4.306 ms、p95 为 10.896 ms。该基准直接调用 FilesystemBackend，
    不含控制面传输、Pi 会话初始化和模型请求。
  - 支持时使用 `clone3(CLONE_INTO_CGROUP)` 直接把 holder 建在目标 cgroup 中。
    Linux 需提供 `mount_setattr`，用于递归设定宿主 bind mount 为只读。
- **删除**
  - discard 热路径 rename 到 trash，O(1)；后台删除。
  - 后台删除由常驻工作者执行：首次需要物理删除时发信号；工作者持续扫描 trash
    并删除，直到为空后静默阻塞等待下一次信号，多次信号合并。启动时若 trash
    非空，发一次信号回收崩溃或被杀进程遗留的目录。
  - 删除失败记录日志、保留条目，并在下一次信号或有界延迟后重试，不忙等。
    扫描跳过瞬态或半创建项，不删除尚未完成 rename 的条目。
  - 工作者不持有 backend 强引用；backend 被 drop 后工作者退出，未删完的目录
    留在 trash 供下次启动回收。
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
  - 损坏的 `operations.json` 改名为 `.corrupt-<pid>` 并大声告警，随后以空记录
    继续；若改名隔离失败，加载返回明确错误并保留坏文件，不以空记录继续，也不
    覆盖坏文件。运行期元数据和副本层通过同一文件系统中的临时文件或 staging 目录
    加 `rename` 发布，使用内核页缓存。环境和快照元数据无法读取时跳过并记录日志。
    未完成 rename 的 `.tmp-*` 在下次启动清理。
  - `/pause` 同步独立保存代的文件、目录及其元数据。最终物化同步宿主目标文件系统
    和新的环境元数据。
  - `btrfs send | receive --dump` 并发 drain send stderr，避免大输出死锁。

## 未实现 / Phase 2

- accept selector、依赖闭包、control plane 逐条变更列表。
- accept 重放已支持修改/删除/创建/目录替换的逆序回滚，但仍非原子；并发读者
  可能观察到中间状态。
- 有特权时直接 `lower=/`：只做了探测/高权限测试，未作为默认路径。
- **环境进程实现**。`src/runtime/process/` 管理进程组、cgroup 和 namespace 句柄。FilesystemBackend 持有各环境的 `ascout-fs-holder`。临时探测与物化仍使用 bubblewrap。
- **真实 freeze 已实现**：cgroup v2 `cgroup.freeze`；不可用时 fallback 进程组
  SIGSTOP/SIGCONT；能力字段 `freeze_mechanism`。`BaseRuntime::suspend/resume`
  已改为真实 freeze/thaw，`terminate` 优先 `cgroup.kill`。
- 仍然待办：逃出进程组的后代冻结（process_group fallback 无法覆盖）；
  cgroup 归属和跨 Backend 冻结编排的进一步统一。

## 已知偏差

- overlay fork 的副本不是冻结快照：并发写时各文件可能来自不同时刻；是否冻结
  由 FilesystemBackend 在操作窗口内负责。
- btrfs 低权限下通常不满足 `incremental_diff`，会降级 overlay；用户第四轮特权
  日志已选中真实 btrfs，并通过 Chain/Flatten/mixed 的端到端场景。
- accept 直接写 upper / live mount 重放均非原子。
- 并行 `clone_upper` 未实现：reflink/快照已覆盖数据拷贝，剩余主要成本是目录项
  遍历；并行目录/xattr/whiteout 写入顺序风险未在本轮引入。
- 当前挂载含状态目录时，默认拒绝启动；只有内部测试开关
  `allow_overlapping_state_directory=true` 才采用“upper 展平 + 快照仅作 diff
  元数据”，不再把该快照层作为 lowerdir；这是为了避免 OverlayFS 的
  `Overlay directories may not overlap`。非重叠挂载的 Flatten 是正常的逐次布局
  选择，不是这个开关造成的。
- overlay Flatten 的 diff 用“当前 upper 副本 vs 基准层副本”比较。目录的 size/mtime 会
  因子项增删而变化，而子项变化已经单独报告；因此目录比较只看 mode、kind 和
  overlay 标记，不把父目录时间戳变化算成额外 diff 项。文件仍比较 size/mtime。
- Chain 布局的 lower 链每代加深一层；`MAX_LOWER_DEPTH=96` 只负责明确报错，
  不实现整链合并。达到上限后需要调整分支策略或在新基准上重建环境；搬移状态目录
  不会缩短既有链，小 upper Flatten 也只是不再追加当前层。
- Chain 的 diff 走 `delta_since`，会报告子 upper 中因写入而 copy-up 的祖先
  目录；btrfs Flatten/mixed 的 send dump 也报告父目录元数据变化。生产 diff 保留
  这些目录；测试按数据面和布局精确期待文件及父目录，重复、额外项或异常 old_path
  都失败。overlay Flatten 的既有目录比较行为未改。
- live-target 的偶发超时尚未复现、原因未确定。测试增加“目标已看到 x”的标记和
  spawn/观察/发布/退出计时，共用原有 30 秒期限，未延长等待。一次定向运行约
  37/37/57/107ms；这不能证明或排除高负载假设。status pipe 首行仍为无超时阻塞
  读取，挂载轮询期限不覆盖该阶段；此前的全局计数污染、EBUSY 不作为此次超时解释。
- 本机普通用户 `btrfs subvolume delete` 返回 EPERM，`fast_delete=false`；快照
  副本仍可启用，后台删除走 recursion fallback（已验证可删净），只是成本更高。

## 特权验证结果与剩余边界

用户 2026-11-10 第四轮验证已通过 `scripts/priv-check.sh`：四个指定测试
逐项 ok，4 passed、0 failed、0 ignored；脚本退出 0，全部检查 PASS。
状态目录在根 btrfs 挂载，workspace 在独立 `/tmp` tmpfs 挂载，默认
非重叠配置；三项 btrfs 测试均为 `incremental_diff=true`、`fast_delete=true`。

- `priv_btrfs_chain_layout` / `priv_btrfs_flatten_layout`：分别强制 Chain / Flatten，
  断言 lower/upper 布局，真实进程修改、删除、新建文件，精确 diff 含父目录、无
  重复或异常 old_path；accept 无冲突且 published，随后启动的真实进程看到结果。
  discard 后环境、快照、层及 trash 清空。
- `priv_btrfs_mixed_generations`：阈值 32，大 root upper Chain、小 child upper
  Flatten；grandchild 真实视图和精确 diff 正确，accept 回 child1 后可见且 root
  视图不变；discard 后环境、快照与 trash 清空。
- `priv_direct_root_overlay_is_available_in_initial_userns`：初始 user namespace
  下 root + `unshare --mount` 的直接 `lower=/` bwrap 命令成功。
- 原始 btrfs create、只读 snapshot、增量 send dump、严格 subvolume delete 均通过。
  7 项进程测试全部通过、0 ignored：自动选择 cgroup，强制 `process_group` fallback
  也通过。详细名单与运行入口见 `docs/fs-backend/priv-check.md`。

剩余边界：

- 上述 accept 调用的 `target_mount_pid=None`，验证直接写目标 upper 后的真实视图，
  不代表本轮验证了运行中 live target 的重放或偶发超时根因。
- `fast_delete=true` 与严格删除已在此次特权环境验证；普通用户仍可能为 false。
  fallback 的 O(文件数) 后台删除未做大数据量性能验证。
- 真实内核生成的 redirect/opaque/metacopy 只用合成 xattr 测试；whiteout 字符
  设备已在真实 bwrap 测试中验证。
- live replay 的 `openat2` 符号链接逃逸测试覆盖了绝对符号链接；其它特殊文件/
  ACL/ownership 未验证。

高权限验证入口：普通用户运行 `./scripts/priv-check.sh`（可用
`ASCOUT_PRIV_STATE_DIR` 指定 btrfs 基目录，用 `ASCOUT_PRIV_WORKSPACE_DIR`
指定另一个挂载上的 workspace 基目录）；脚本以普通用户编译测试并运行进程组
冻结测试，再用 sudo 运行 `ASCOUT_PRIV_TESTS=1 <test-binary>
runtime::backends::filesystem::tests::privileged::priv_ --show-output
--format=pretty --color=never --test-threads=1` 的 4 个测试。
状态目录必须位于后端隔离的挂载之外，脚本和测试都会断言这一点。live mount
重放不要求高权限，本机实际运行并通过。普通用户缺少 btrfs send/receive 权限时
会打印 SKIP，不作为特权通过证据；特权通过结果来自用户 2026-11-10 第四轮验证，
测试名、结果与范围保留在本文及 `priv-check.md`，原始根目录日志已按用户要求删除。

`check-test-output.py` 交叉核对精确发现名单、命令退出码、逐项结果和摘要，
执行数、通过数、ignored 分开汇总，任何 SKIP 或组内错误均不打印该组 PASS。
`scripts/test-priv-check.py` 保留假命令失败场景，增加真实 libtest 交错输出回归，
共 24 场景；交错场景已先 red 后 green。

本轮完整测试另暴露两处短命令竞态：已退出但未回收进程的 mountinfo 真实返回
EINVAL；读取成功但目标未齐时，轮询遗漏检查 child 退出。两项回归均先失败后通过，
后一项用独立存活进程保持 mountinfo 可读，固定复现遗漏分支，不依赖 PID 复用。
修复后 `cargo test --lib --bins` 为库 140 项、bin 1 项通过，0 失败；其中无权限
而自行 SKIP 的 btrfs/特权项仍不算真实特权验证。

## 逐挂载 overlay（无特权默认，实测）

- **隔离单元是挂载点**。`Environment.mounts: BTreeMap<absolute target, MountUnit>`
  记录 `{target, upper, work, lower 链, layer_key}`；workspace 与 `/tmp` 只是
  普通挂载点，没有单独的 workspace/tmp upper。
- `establish` 读取 `/proc/self/mountinfo`，过滤伪文件系统、vfat/squashfs/overlay/
  fuse 等，候选逐挂载做 bwrap overlay probe 并缓存；`launch_fragment` 启动前
  补齐新出现的可 overlay 挂载。
- fork/mark/diff/accept 遍历同一组 `MountUnit`：父 upper 复制到
  `layers/<snapshot>/mounts/<hash>/upper`；Chain 布局把该层追加到子 lower，
  Flatten 布局把父 upper 复制进子 upper、不追加 lower；accept 按挂载点分别
  合并且重放到目标 upper 或 `/proc/<pid>/root`。
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
  `/home`，fork 后启动子进程即报错。现在默认在 `establish`/`launch_fragment`
  创建环境根、upper 和元数据之前拒绝启动，并报告状态目录与冲突挂载点。只有内部
  测试开关 `allow_overlapping_state_directory=true` 才保留原有降级：该挂载 fork
  时强制 **Flatten**，把父 upper 复制到 child upper，快照层只用于 diff/accept
  元数据，启动时 lower 只用宿主挂载点；其它挂载按逐次 fork 的布局规则选择
  Chain 或 Flatten。
- 嵌套挂载当前未实现。当前 bwrap 参数层做不到不等于实际做不到，之后可以另找
  实现手段；父挂载与子挂载分别 probe；现有参数无法保存子挂载 lower 后再重挂。
  失败候选会被 ro-bind，不静默直通。
- 实测测试：`extra_mount_fork_diff_accept_discard_round_trip`（`/var/tmp`：
  父写 a、fork 子见 a、子写 b + 删除宿主 lower、diff 报告 b/delete、accept
  目标可见、mark 覆盖额外挂载、discard 清理 upper）、
  `home_cache_and_config_writes_stay_in_home_upper`、`writes_outside_cwd_...`。

### 数据面覆盖矩阵（按挂载点）

| 操作 | 覆盖范围 | 实现 | 验证 |
|---|---|---|---|
| fork | 选中（默认全部）挂载 | 每挂载按 upper 大小选 Chain（追加 lower）或 Flatten（复制进 upper）；含状态目录的挂载强制 Flatten（仅内部开关） | `extra_mount_...`、`backend_overlay_...`、`overlay_fork_isolates...`、`fork_layout_...`、`mixed_chain_and_flatten_...` |
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

当前权威写入留在 branch upper，宿主观察者看不到。系统最终必须物化到现实；
正常完成须在任务完成、没有存活候选且任务相关写入完成后物化并重置基准，
不为物化暂停执行。取消、失败、主动退出立即进入物化处理，但写回仍须满足无候选
及任务相关写入完成。暂停任务保留环境等待继续；旧环境与基准无人引用时释放。
Agent 完成本轮工作并开始等待用户输入时，task 完成；一个 session 对应一个
AScoutAgent 和一棵树。Full 接受后胜者 session 使用 AuB 合并后的环境，Part 由 AuB session 继续。Agent 常用 Ctrl-C 或退出均终止任务；暂停由 AScout 独立接口提供，
保留树、候选和环境。已有退出保存入口尚未对齐，恢复能力保留；接口名称及 UI 待定。
宿主写回按 Authority 增量最终视图覆盖，不做宿主与共同祖先比较或冲突报告，
不新增全量哈希，不复制整个挂载。运行期间不编辑宿主目录仅为
使用约定；不为被覆盖的宿主版本保留可恢复副本。目标写回覆盖工作目录及任务
允许修改的其他挂载，只落实 Authority 相对基准的改动；属性变更首版仅支持权限位，
其他属性不新增修改支持，也不允许破坏原文件未改属性或链接结构；
暂允许中间态，失败保留已写部分并重试剩余，
不承诺回滚；失败阻止任务成功报告。写入完成后正常退出可见，不新增掉电保证。
这些物化要求及基准重置、引用保护和资源释放仍待实现验证，不能以 accept 或退出
恢复已有实现代替。Runtime 向 control plane 报告物化完成／失败，物化失败等同操作失败，
control plane 必须等物化成功后才能宣告任务成功；具体报告与协调路径仍待实现核查，
本轮不新增接口。具体边界见
[物化提案](materialize-proposal.md)。

## 成本表（实测/推断）

| 操作 | 方案 A btrfs 增量 | 方案 2 + btrfs 快照 | 方案 2 + reflink | 方案 2 + copy |
|---|---|---|---|---|
| fork | O(1) 快照（推断） | 空 upper 跳过；非空挂载按 256 条目阈值选快照或 reflink/复制；快照本身 O(1)，首次快照的 btrfs 事务提交随 N 约 0.2s/100k | O(目录项数)，小 upper 固定成本低于首次 btrfs 快照 | O(目录项数 + 数据) |
| mark | O(1) 快照 | 空 upper 跳过；非空挂载同上；独立 mark 层为空时不复制 | O(upper 目录项数) | O(upper 数据) |
| fork/mark 元数据 | O(1) | 只写本操作新增 snapshot + environment，不再重写全部历史 snapshot | 同左 | 同左 |
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
ASCOUT_BENCH_CASES=<case> ASCOUT_BENCH_ITERS=20 ASCOUT_BENCH_SIZES=<N> \
  target/release/examples/fs_bench
```

每个 N 单独起进程、使用独立状态目录。基准本身复用 `case.state`；在 btrfs 上
后台回收尚未结束时直接跑下一个 N 会污染结果（甚至触发残留状态错误），因此
最终表把 N 分开跑。btrfs 前台延迟也会随后台回收积压波动；下表取回收空闲时的
一次运行，并在结论里给出另一轮的范围。

`reflink-flatten`：状态目录与 workspace 都在 `/home`（btrfs），`/home` 重叠 →
flatten。`reflink-lower`：状态目录在 `/var/tmp`（btrfs），workspace 在 `/home` →
`/home` 使用 lower 链；`/var/tmp` 自身重叠并 flatten。`tmpfs-flatten`：状态与
workspace 都在 `/tmp` tmpfs，flatten。所有基准用例的状态目录都位于被隔离挂载内，
因此 `fs_bench` 显式打开内部测试开关 `allow_overlapping_state_directory`；生产
部署默认拒绝这种布局。这些历史 case 现在用
`FilesystemConfig::fork_flatten_entry_limit`（`0` 或 `usize::MAX`）显式强制
Chain/Flatten，以免默认阈值改变后与旧数据不可比。

| case | N | operation | p50_ms | p95_ms | fork_cost | mechanism | overlaps |
|---|---|---|---|---|---|---|---|
| btrfs-snapshot-flatten | 100 | accept | 8.457 | 15.890 | btrfs_snapshot | overlay | /home |
| btrfs-snapshot-flatten | 100 | diff | 6.200 | 16.894 | btrfs_snapshot | overlay | /home |
| btrfs-snapshot-flatten | 100 | discard | 4.367 | 4.881 | btrfs_snapshot | overlay | /home |
| btrfs-snapshot-flatten | 100 | fork | 22.723 | 45.308 | btrfs_snapshot | overlay | /home |
| btrfs-snapshot-flatten | 100 | mark | 15.448 | 57.617 | btrfs_snapshot | overlay | /home |
| btrfs-snapshot-flatten | 10000 | accept | 281.687 | 293.363 | btrfs_snapshot | overlay | /home |
| btrfs-snapshot-flatten | 10000 | diff | 385.794 | 392.249 | btrfs_snapshot | overlay | /home |
| btrfs-snapshot-flatten | 10000 | discard | 6.759 | 7.603 | btrfs_snapshot | overlay | /home |
| btrfs-snapshot-flatten | 10000 | fork | 57.368 | 88.220 | btrfs_snapshot | overlay | /home |
| btrfs-snapshot-flatten | 10000 | mark | 39.825 | 58.957 | btrfs_snapshot | overlay | /home |
| btrfs-snapshot-flatten | 100000 | accept | 2878.982 | 2969.998 | btrfs_snapshot | overlay | /home |
| btrfs-snapshot-flatten | 100000 | diff | 4194.562 | 4352.346 | btrfs_snapshot | overlay | /home |
| btrfs-snapshot-flatten | 100000 | discard | 6.697 | 7.957 | btrfs_snapshot | overlay | /home |
| btrfs-snapshot-flatten | 100000 | fork | 81.772 | 186.928 | btrfs_snapshot | overlay | /home |
| btrfs-snapshot-flatten | 100000 | mark | 43.859 | 100.698 | btrfs_snapshot | overlay | /home |
| btrfs-snapshot-lower | 100 | accept | 5.689 | 6.387 | btrfs_snapshot | overlay | /var/tmp |
| btrfs-snapshot-lower | 100 | diff | 3.827 | 4.308 | btrfs_snapshot | overlay | /var/tmp |
| btrfs-snapshot-lower | 100 | discard | 4.070 | 4.280 | btrfs_snapshot | overlay | /var/tmp |
| btrfs-snapshot-lower | 100 | fork | 15.801 | 18.257 | btrfs_snapshot | overlay | /var/tmp |
| btrfs-snapshot-lower | 100 | mark | 9.232 | 10.392 | btrfs_snapshot | overlay | /var/tmp |
| btrfs-snapshot-lower | 10000 | accept | 5.471 | 5.665 | btrfs_snapshot | overlay | /var/tmp |
| btrfs-snapshot-lower | 10000 | diff | 3.582 | 4.369 | btrfs_snapshot | overlay | /var/tmp |
| btrfs-snapshot-lower | 10000 | discard | 4.406 | 4.812 | btrfs_snapshot | overlay | /var/tmp |
| btrfs-snapshot-lower | 10000 | fork | 27.078 | 31.605 | btrfs_snapshot | overlay | /var/tmp |
| btrfs-snapshot-lower | 10000 | mark | 8.481 | 8.725 | btrfs_snapshot | overlay | /var/tmp |
| btrfs-snapshot-lower | 100000 | accept | 6.947 | 10.157 | btrfs_snapshot | overlay | /var/tmp |
| btrfs-snapshot-lower | 100000 | diff | 3.813 | 7.353 | btrfs_snapshot | overlay | /var/tmp |
| btrfs-snapshot-lower | 100000 | discard | 4.645 | 4.880 | btrfs_snapshot | overlay | /var/tmp |
| btrfs-snapshot-lower | 100000 | fork | 53.323 | 67.251 | btrfs_snapshot | overlay | /var/tmp |
| btrfs-snapshot-lower | 100000 | mark | 8.510 | 20.188 | btrfs_snapshot | overlay | /var/tmp |
| reflink-lower | 100 | accept | 5.577 | 5.858 | reflink | overlay | /var/tmp |
| reflink-lower | 100 | diff | 3.785 | 3.884 | reflink | overlay | /var/tmp |
| reflink-lower | 100 | discard | 0.396 | 0.442 | reflink | overlay | /var/tmp |
| reflink-lower | 100 | fork | 17.160 | 57.068 | reflink | overlay | /var/tmp |
| reflink-lower | 100 | mark | 9.188 | 12.999 | reflink | overlay | /var/tmp |
| reflink-lower | 10000 | accept | 6.622 | 77.521 | reflink | overlay | /var/tmp |
| reflink-lower | 10000 | diff | 4.615 | 80.638 | reflink | overlay | /var/tmp |
| reflink-lower | 10000 | discard | 0.454 | 0.546 | reflink | overlay | /var/tmp |
| reflink-lower | 10000 | fork | 640.847 | 665.698 | reflink | overlay | /var/tmp |
| reflink-lower | 10000 | mark | 12.527 | 134.242 | reflink | overlay | /var/tmp |
| reflink-lower | 100000 | accept | 5.936 | 9.481 | reflink | overlay | /var/tmp |
| reflink-lower | 100000 | diff | 3.988 | 5.778 | reflink | overlay | /var/tmp |
| reflink-lower | 100000 | discard | 0.437 | 0.503 | reflink | overlay | /var/tmp |
| reflink-lower | 100000 | fork | 6620.377 | 6921.664 | reflink | overlay | /var/tmp |
| reflink-lower | 100000 | mark | 9.731 | 16.037 | reflink | overlay | /var/tmp |
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

### 逐挂载布局阈值实测（2026-11-10，本机）

同一非重叠布局下强制 Chain 与强制 Flatten 对比。状态目录在 `/var/tmp`（btrfs），
workspace 在仓库 `target/`（`/home`，btrfs）；每个改动的 child 只写 10 个文件。
命令：

```bash
ASCOUT_BENCH_CASES=nonoverlap ASCOUT_BENCH_ITERS=5 \
ASCOUT_BENCH_SIZES=16,64,256,1024,4096 \
  timeout 600 target/release/examples/fs_bench
```

`reflink-chain-nonoverlap` 用 `fork_flatten_entry_limit=0` 强制 Chain，
`reflink-flatten-nonoverlap` 用 `usize::MAX` 强制 Flatten。p50 毫秒：

| N | Chain diff | Chain accept | Chain diff+accept | Flatten diff | Flatten accept | Flatten diff+accept | Chain fork | Flatten fork |
|---|---|---|---|---|---|---|---|---|
| 16 | 3.875 | 5.765 | 9.640 | 6.704 | 8.387 | 15.091 | 11.635 | 21.740 |
| 64 | 7.121 | 8.328 | 15.449 | 6.973 | 10.636 | 17.609 | 24.513 | 29.118 |
| 256 | 7.104 | 8.262 | 15.366 | 13.132 | 14.540 | 27.672 | 37.901 | 54.864 |
| 1024 | 7.119 | 8.563 | 15.682 | 28.591 | 29.831 | 58.422 | 86.173 | 150.147 |
| 4096 | 6.493 | 7.621 | 14.114 | 121.910 | 92.360 | 214.270 | 283.889 | 524.862 |

选择准则：取 Flatten 的 diff+accept p50 开始明显高于 Chain（超过约 10ms）的
最小条目数，再向下取 2 的幂。N=16、64 的差分别约 5.5ms、2.2ms；N=256 的差
约 12.3ms，是第一个超过 10ms 的点，且 256 已是 2 的幂。因此
`DEFAULT_FORK_FLATTEN_ENTRY_LIMIT = 256`：条目数 < 256 用 Flatten，≥256 用
Chain。Chain 的 fork p50 在所有 N 下都低于 Flatten（它只产生一次 S，子 upper
为空；Flatten 还要把父 upper 复制进子 upper），所以小 upper 用 Flatten 换
diff/accept，大 upper 用 Chain 换 fork 和链增长，两个方向都不吃亏。

这是本机（kernel `7.2.14-zen1-1-zen`、Ryzen 7 5800H、btrfs）测得的值；换文件
系统、CPU 或状态目录所在挂载后需要重测。阈值只影响布局选择，不是正确性边界。

### 第十轮：快照模式 fork/mark 前后对比

第九轮数字来自提交 `6de74d5` 的实测；第十轮为同一台机器、同一 case、20 次中位数。

| case | N | 第九轮 fork p50 | 第十轮 fork p50 | 第九轮 mark p50 | 第十轮 mark p50 |
|---|---|---|---|---|---|
| btrfs-snapshot-lower | 100 | 215.267 | **15.801** | 127.254 | **9.232** |
| btrfs-snapshot-lower | 10000 | 370.658 | **27.078** | 171.111 | **8.481** |
| btrfs-snapshot-lower | 100000 | 1379.413 | **53.323** | 583.268 | **8.510** |
| btrfs-snapshot-flatten | 100 | 161.881 | **22.723** | 97.697 | **15.448** |
| btrfs-snapshot-flatten | 10000 | 620.334 | **57.368** | 727.592 | **39.825** |
| btrfs-snapshot-flatten | 100000 | 1176.917 | **81.772** | 1245.006 | **43.859** |

btrfs 的前台延迟对后台回收积压敏感。同一配置在另一轮（前一个 100k 回收尚未
结束时）测得 flatten 100/10k/100k fork p50 为 34.5 / 212.9 / 138.6ms，lower
100/10k/100k 为 136.0 / 190.1 / 258.6ms；上表是回收空闲时的一轮。即使取较差
的一轮，100k lower（258.6ms）仍明显优于第九轮 1379ms，但会超过 200ms 目标。
结论按空闲轮给出，同时如实保留这个范围。

### 自适应阈值 K 的依据

对同一 lower case 临时把阈值设为 0（全部快照）或很大（全部复制），各跑
2–4 次取 p50：

| N | clone fork p50 | snapshot fork p50 | 选择 |
|---|---|---|---|
| 100 | 16.8ms | 18.8ms | clone |
| 200 | 25.2ms | 39.7ms | clone |
| 300 | 33.6ms | 25.9ms | snapshot |
| 500 | 50.3ms | —（约 20ms 量级） | snapshot |
| 1000 | 84.7ms | 24.3ms | snapshot |

交叉点在 200–300 条目之间，取 2 的幂 **K = 256**。快照的固定成本包含 btrfs
事务提交，因此交叉点会随文件系统脏页/回收状态移动；K 只决定“小 upper 不触发
btrfs 命令”，不是精确的性能分界。

### 分阶段计时（`ASCOUT_FS_PROFILE=1`）

改造前（第九轮代码，N=100 lower，11 个挂载全部被快照）：

```text
fork snapshot_layer=216.323ms(11) create_child=34.375ms(20) snapshot_child=12.011ms(1)
     prepare_dirs=0.302ms(1) finalize=5.933ms(1) persist=71.761ms(1)
mark snapshot_layer=448.525ms(11) finalize=14.689ms(1) persist=153.741ms(1)
```

改造后（每个 N 独立进程，`sync` 后第一次 fork，含首次 btrfs 事务提交）：

```text
N=100   fork clone_layer=7.786ms(1) prepare_dirs=1.425ms(12) finalize=26.599ms(1) persist=106.004ms(1)
N=10k   fork snapshot_layer=90.945ms(1) create_child=2.057ms(1) prepare_dirs=1.238ms(11) finalize=25.950ms(1) persist=105.085ms(1)
N=100k  fork snapshot_layer=464.139ms(1) create_child=1.834ms(1) prepare_dirs=1.144ms(11) finalize=25.767ms(1) persist=103.659ms(1)
```

冷启动首轮会把 setup 写入的脏元数据一次提交，所以 persist/finalize 明显高于
20 次中位数（例如 N=100 的 p50 fork 为 15.8ms）。关键变化是**调用次数**：
`snapshot_layer` 从 11 次降到 1 次，`create_child` 从 20 次降到 0–1 次，
`persist` 只写本操作新增的 snapshot 而不是全部历史 snapshot。固定成本主要来自
“每个挂载一次 btrfs snapshot + 一次子卷创建 + 一次持久化”，不是 `prepare_dirs`；
随 N 增长的部分来自首次快照的 btrfs 元数据提交和 flatten 的 upper 复制。

### 结论与瓶颈（如实）

- **lower 链 accept/diff 是 O(增量)**：btrfs-snapshot-lower 的 accept p50
  约 5.7 / 5.5 / 6.9ms，diff 约 3.8 / 3.6 / 3.8ms，不随 upper 总大小增长。
- **lower 链 mark 接近 O(1)**：btrfs-snapshot-lower 约 9.2 / 8.5 / 8.5ms；
  空 upper 不产生层，fork 出来的 child upper 通常为空。
- **lower 链 fork 已大幅下降**：btrfs-snapshot-lower 从第九轮
  215 / 371 / 1379ms 降到 **15.8 / 27.1 / 53.3ms**；N=100 与 reflink 的
  17.2ms 基本持平，N=100k 低于 200ms 目标。reflink 仍随 N 线性
  （17.2ms → 641ms → 6.62s）。
- **flatten 仍 O(upper)**：状态目录与 workspace 同挂载时 fork/mark 要复制整个
  upper，diff/accept 也比较完整 upper。btrfs-snapshot-flatten 的 fork p50 为
  22.7 / 57.4 / 81.8ms（优于第九轮 162 / 620 / 1177ms），但 diff/accept 在
  100k 仍约 4.2s / 2.9s。`reflink-flatten` 100k fork 仍约 21.3s。
- **tmpfs 复制路径随 N 增长**：100k fork 6.17s、mark 3.15s、accept 1.67s。
- **discard 热路径仍 O(1)**：所有 case 约 0.1–15ms（btrfs rename 到 trash）。
- **剩余 O(N) 部分（未消除）**：
  1. 对一个含 N 个条目的 upper 做**首次**只读快照时，btrfs 需要提交元数据；
     100k 冷启动首轮约 0.2–0.5s。这是 btrfs 事务成本，不是我们的遍历。
  2. `fast_delete=false` 时 discard 的后台回收仍需递归删除 N 个条目；热路径
     是 rename。多个后台删除已进程内串行化，避免与下一次 fork 争用事务。
  3. flatten 的 fork/mark 复制整个 upper，diff/accept 也比较完整 upper，仍
     O(upper)；这是“状态目录与 workspace 同挂载”的固有成本。

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
| 隔离 rename 失败时失败关闭 | `corrupt_operations_metadata_fails_closed_when_isolation_fails` |
| 启动回收崩溃遗留 trash | `startup_reclaims_trash_left_by_a_previous_run` |
| 空闲工作者被下一次 discard 唤醒 | `idle_worker_wakes_for_a_later_discard` |
| 连续 discard 信号合并后 trash 为空 | `repeated_discards_leave_trash_empty` |
| btrfs 快照 fork | `btrfs_snapshot_copy_fork_is_end_to_end`（本机通过） |
| 逐次布局选择（小 flatten / 大 chain） | `fork_layout_flattens_small_upper_and_chains_large_upper` |
| 混合链/展平多代 diff、真实启动、accept | `mixed_chain_and_flatten_layout_diff_and_accept_preserve_parent_view` |
| 混合布局冲突不写入 | `mixed_layout_accept_reports_conflict_and_writes_nothing` |
| 超过链深度上限时 fork 前失败、无半成品 | `fork_refuses_chain_deeper_than_max_lower_depth_before_creating_anything` |
| bwrap lower 层数上限 | 本机实测 231 成功、232 `Invalid argument`；`MAX_LOWER_DEPTH=96` |

## P4 调研结论：btrfs 增量

本机实测：

```text
btrfs subvolume find-new <path> <gen>   -> Operation not permitted
btrfs send -p <snapshot> <subvolume>    -> Operation not permitted
btrfs subvolume show <path>             -> Operation not permitted
```

因此探测增加 `incremental_diff`：真实尝试
`btrfs send -p <snapshot> <snapshot> | btrfs receive --dump`。成功才选择 btrfs；
实现已启用 send dump diff/accept；fork 按逐挂载 Chain/Flatten 规则决定是否追加
lower 层，upper/work 位于同一 volume 子卷。

## D. 冻结（简单版）调研

- FilesystemBackend 的暂停和恢复操作冻结或解冻环境进程。
- **cgroup v2**：本机 `/sys/fs/cgroup` 根不可写，但当前进程所在的 delegated
  `.scope` 可创建子 cgroup、写入 `cgroup.procs`、写 `cgroup.freeze=1`，并观察到
  `cgroup.events` 中 `frozen 1`。实测成功（无需 sudo）。
- **SIGSTOP/SIGCONT**：底层启动代码在 `pre_exec` 设置独立进程组，终止用
  `kill(-pgid, SIGKILL)`；环境进程实现提供 `SIGSTOP/SIGCONT` 回退，
  由 `process_group_fallback_freezes_and_reports_mechanism` 验证。受管命令另建会话时，进程组信号可能无法覆盖其后代。
- FilesystemBackend 在 fork/mark/accept 的操作窗口内冻结运行中的 holder，操作后恢复原执行状态。已暂停的 holder 保持暂停。

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
- **有特权初始 userns：直接 lower=/ 可行**，用户 2026-11-10 第四轮验证的
  脚本检查和 `priv_direct_root_overlay_is_available_in_initial_userns` 均通过。
- **备选：逐挂载点 overlay**：可行，但必须处理 locked 子挂载和嵌套挂载。

### 嵌套/相邻挂载

- 相邻挂载：bwrap 同时 overlay `/home` 和 `/tmp`，两边内容都正确，实测 OK。
- 嵌套挂载当前未实现；以下是原始 mount 实验结论，不是当前 bwrap 参数实现。
  当前 bwrap 参数层做不到不等于实际做不到，之后可以另找实现手段。
  父挂载 overlay 后，子挂载会被隐藏。实测顺序必须是：
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
| 初始 userns 特权下 lower=/ 可行 | **用户实测**：2026-11-10 第四轮脚本检查和 `priv_direct_root_overlay_is_available_in_initial_userns` 通过 |
| 相邻挂载同时 overlay | **实测**：bwrap 同时 overlay /home 与 /tmp，exit 0 |
| 嵌套挂载在原始 mount 实验中可同时 overlay（先保存 child lower，再 parent，再 child） | **实测**：父子 tmpfs overlay 后两个文件都可见；当前 bwrap 参数实现未接入 |
| 逐挂载 overlay 的挂载数/元数据成本 | **实测**：13 个可写挂载；200 次 mount+umount ~634ms；stat +34% |
| btrfs fork/diff/accept/discard 端到端 | **用户实测**：2026-11-10 第四轮 Chain/Flatten/mixed 全通过，真实 btrfs `incremental_diff=true fast_delete=true`；accept 无 live target，随后真实启动确认视图，含清理断言 |
| upper/work 同 btrfs 子卷 | **实测**：旧创建路径 work→upper rename 稳定 errno 18，修复后真实 create/snapshot/rename/mark/reload 回归通过；恢复拒绝不兼容布局并保留数据 |
| 测试结果校验不受日志交错影响 | **实测**：真实 libtest 交错输出先 red 后 green；24 个脚本场景覆盖非零退出、精确名单、逐项结果、摘要、ignored、SKIP |
| 全部 overlay 目标就绪 | **实测**：`readiness_requires_every_overlay_and_decodes_mount_paths` 先 red 后 green；不延长轮询期限 |
| 短命令退出与挂载就绪轮询 | **实测**：真实未回收退出进程的 EINVAL、可读但目标未齐的 mountinfo 两项回归先 red 后 green；受管 child 确认退出才接受完成，退出码保留 |
| live-target 偶发超时根因 | **未确定**：增加观察标记与分段计时，一次约 107ms 正常结束，不能据此归因高负载；status pipe 首行仍阻塞 |
| 环境进程启动、冻结和解冻 | **实测**：`process_freeze_thaw_stops_and_resumes_writer` 冻结期间 counter 不变，thaw 后继续 |
| 进程组 SIGSTOP/SIGCONT fallback | **实测**：`process_group_fallback_freezes_and_reports_mechanism`，能力上报 `process_group` |
| FilesystemBackend 写入静止确认 | **实测**：`filesystem_freezes_and_marks_writers_quiesced` 有进程时 true，无进程时 false |
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

# 文件系统 Backend 部署建议

## 状态目录必须在被隔离挂载点之外

`FilesystemBackend` 会把状态目录放在自己内部（environments、layers、snapshots、
replay journal）。当状态目录位于某个会被 Agent overlay 的挂载点内时，OverlayFS
不允许把该状态目录下的快照层同时作为同一挂载的 lowerdir
（`Overlay directories may not overlap`）。

当前实现检测到这种重叠会**默认拒绝启动**：`establish` 和 `launch_fragment` 在
创建环境根、upper 目录或环境元数据之前返回明确错误，错误包含状态目录路径、冲突
的挂载点路径，以及“把状态目录放到被隔离挂载点之外”的修复建议。拒绝时不会留下
半成品。

针对“状态目录重叠”的 flatten 降级仍保留在代码中，但只能由内部开关
`allow_overlapping_state_directory=true`（仅测试与诊断）启用；它不属于外部配置
文件，默认 false。启用时：

- 该重叠挂载强制 Flatten：有 btrfs 快照副本时 child upper 用可写快照，O(1)；
  没有快照副本时 fork 把父 upper 整个复制到 child upper，成本 O(upper 大小)；
- diff/accept 用 child upper 与快照基准比较，而不是只扫描增量；
- 启动日志和能力字段仍列出重叠挂载。

这与正常部署里的逐次布局选择是两回事。即使状态目录不重叠，每次 fork 也会按
该挂载 upper 的条目数选择布局：

- 条目数 < `DEFAULT_FORK_FLATTEN_ENTRY_LIMIT`（默认 256）→ Flatten：父 upper
  复制进 child upper，子 lower = 父 lower；
- 条目数 ≥ 阈值 → Chain：新增只读层 S 到子 lower，子 upper 为空。

阈值可由 `FilesystemConfig::fork_flatten_entry_limit` 覆盖（测试与基准；
`0` = 总是 Chain，`usize::MAX` = 总是 Flatten），不进入外部配置文件。Chain
每次会让该挂载的 lower 链加深一层；达到 `MAX_LOWER_DEPTH`（96，低于本机实测
bwrap 上限 231/232 的一半）时 fork 返回明确错误，不会静默改成 Flatten。

生产部署要求把状态目录放在**不会被 Agent overlay 的独立挂载点**上，例如：

```json
{
  "filesystem": {
    "state_directory": "/var/lib/ascout/fs-state",
    "bubblewrap_path": "/usr/bin/bwrap"
  }
}
```

要求：

1. `/var/lib/ascout/fs-state` 位于独立挂载（如单独的 btrfs subvolume、tmpfs 或
   专用磁盘），不要位于 `/home`、`/tmp`、`/var/tmp`、`/srv` 等工作区挂载内；
   状态目录必须在该次运行会被 overlay 的所有挂载点之外。
2. 该目录必须与 Agent 工作区挂载点不重叠；目录本身不需要对 Agent 可见，沙箱内
   会用 `--tmpfs` + `--remount-ro` 遮蔽。
3. 状态目录所在文件系统应支持 reflink（btrfs/XFS）或 btrfs 快照副本，以获得
   O(1) 的 fork/备份；否则 fork 和 accept 回滚备份会退化为复制。
4. btrfs 快照副本模式要求创建子卷、只读快照、快照作为 overlay lower、以及
   btrfs-aware 递归删除（先 `property set ro false`）探测成功。`fast_delete`
   只影响后台删除成本，不再是启用门槛；`filesystem_fast_delete=false` 与
   `fork_cost=btrfs_snapshot` 可以同时出现。

## 副本生产者选择顺序

1. **方案 A**：btrfs volume 子卷（内含 upper/work）+ 有特权的增量 send diff/accept（最高优先级）。
2. **方案 2 + btrfs 快照**：状态目录在 btrfs，且创建/只读快照/overlay lower/
   btrfs-aware 删除探测全部成功。fork/mark 为 O(1) 子卷快照；删除热路径是
   rename，后台 fallback 为 O(文件数)。
3. **方案 2 + reflink**：支持 FICLONE。fork 仍需遍历 N 个目录项，数据 O(1)。
4. **方案 2 + copy**：逐文件复制。

状态目录与 workspace 不同挂载且不重叠时，每个挂载按 upper 条目数逐次选择
Chain（diff/accept O(增量)）或 Flatten（diff/accept O(upper)）；重叠时默认
拒绝启动，只有内部测试开关启用时才强制 flatten。阈值与实测数据见
`docs/fs-backend/current-state.md`。

## btrfs upper/work 布局与恢复

每挂载使用 `mounts/<hash>/volume` 子卷，其中 `upper` 与 `work` 都是普通子目录。
fork/mark 快照整个 volume，lower 与 diff 只使用其 upper 子树。两目录必须同设备、
同 btrfs 子卷，才能完成 OverlayFS copy-up 的临时文件 rename；仅在同一 btrfs
文件系统上并不够。

创建、发现新挂载、fork 和启动前均检查该约束。恢复不兼容布局时会返回含路径的
明确错误并保留数据；当前不自动迁移，应保留旧状态目录、处理完其中环境后，在
新的状态目录重建环境。不要仅编辑元数据路径来绕过检查。

## 验证

启动后检查能力字段：

```json
{
  "state_directory_overlaps_mount": []
}
```

空列表表示状态目录不在任何被隔离挂载内；每个挂载仍按 upper 条目数逐次选择
Chain 或 Flatten，不会因为不重叠就一律使用 lower 链。

若看到：

```text
filesystem: state_directory_overlaps_mount=["/home"] startup=refused_by_default unless internal allow_overlapping_state_directory=true; flatten fallback retained for tests/diagnostics (fork_cost=O(upper) diff_cost=O(upper))
```

说明状态目录与 `/home` 重叠。默认启动会返回错误并拒绝创建环境；应把状态目录移到
被隔离挂载点之外，而不是打开内部测试开关。

当前测试覆盖：

- `overlapping_state_directory_is_rejected_by_default`
- `overlapping_state_directory_is_reported_and_flattened`
- `non_overlapping_state_directory_uses_lower_chain`
- `default_config_starts_when_state_directory_is_outside_isolated_mounts`
- `fork_layout_flattens_small_upper_and_chains_large_upper`
- `mixed_chain_and_flatten_layout_diff_and_accept_preserve_parent_view`
- `mixed_layout_accept_reports_conflict_and_writes_nothing`
- `fork_refuses_chain_deeper_than_max_lower_depth_before_creating_anything`

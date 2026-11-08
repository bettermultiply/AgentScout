# 文件系统 Backend 部署建议

## 状态目录必须尽量放在独立挂载点

`FilesystemBackend` 会把状态目录放在自己内部（environments、layers、snapshots、
replay journal）。当状态目录位于某个会被 Agent overlay 的挂载点内时，OverlayFS
不允许把该状态目录下的快照层同时作为同一挂载的 lowerdir
（`Overlay directories may not overlap`）。

当前实现会自动检测这种重叠，并对重叠挂载使用 **flatten**：

- fork 时把父 upper 整个复制到 child upper，成本 O(upper 大小)；
- diff 用完整 upper 与快照比较，而不是只扫描增量；
- 启动日志和能力字段会列出重叠挂载。
- 非重叠挂载仍使用 lower 链增量路径。

生产部署建议把状态目录放在**不会被 Agent overlay 的独立挂载点**上，例如：

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
   专用磁盘），不要位于 `/home`、`/tmp`、`/var/tmp`、`/srv` 等工作区挂载内。
2. 该目录必须与 Agent 工作区挂载点不重叠；目录本身不需要对 Agent 可见，沙箱内
   会用 `--tmpfs` + `--remount-ro` 遮蔽。
3. 状态目录所在文件系统应支持 reflink（btrfs/XFS）以获得 O(1) 的 fork/备份；
   不支持时仍可用，但 fork 和 accept 回滚备份会退化为复制。

## 验证

启动后检查能力字段：

```json
{
  "state_directory_overlaps_mount": []
}
```

空列表表示所有挂载都走 lower 链增量路径。

若看到：

```text
filesystem: state_directory_overlaps_mount=["/home"] strategy=flatten fork_cost=O(upper) diff_cost=O(upper)
```

说明状态目录与 `/home` 重叠，该挂载会使用 flatten 策略，fork/diff 成本随 upper
大小线性增长。

当前测试覆盖：

- `overlapping_state_directory_is_reported_and_flattened`
- `non_overlapping_state_directory_uses_lower_chain`

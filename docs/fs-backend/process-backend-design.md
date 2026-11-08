# ProcessBackend 设计

## 职责

`ProcessBackend`（`src/runtime/process/`）是唯一启动、冻结、终止 Agent 进程的组件。

- 接收 `CompositeRuntime`/`BaseRuntime` 汇总的 `LaunchFragment`。
- 把 `MountSpec` 翻译成 bubblewrap 参数并 spawn。
- 管理进程组、cgroup v2 子 cgroup、`cgroup.freeze`/`cgroup.kill`。
- 通过 `--json-status-fd` 记录内层 sandbox pid 与 mount namespace。
- 对外提供 `process_handle(env) -> {inner_pid, mount_namespace, cgroup_path, process_group}`。
- 清理时删除 cgroup 目录；cgroup 不可用时用进程组 SIGSTOP/SIGCONT。

FilesystemBackend 不再 spawn、不再记录 pid/namespace，只产出 `LaunchFragment`
中的挂载条目。隔离单元是挂载点（`MountUnit`），不是固定 Workspace/Tmp scope；
启动片段按挂载点生成 overlay，fork/mark/diff/accept 也按同一组挂载点处理。

当后端状态目录位于某个被 overlay 的挂载点下时，该挂载点的快照层不能再作为
lowerdir（OverlayFS 报 `Overlay directories may not overlap`）。此时 fork 把父
upper 展平进 child upper，快照层只作为 diff/accept 的元数据；启动时该挂载点
只用宿主挂载点作 lower。其他挂载点仍使用 lower 链。

## 接口与组合

`Backend` 增加：

```rust
fn launch_fragment(&self, context, launch) -> Result<LaunchFragment, BackendError>
```

- `CompositeRuntime::launch_fragment` 按 Operate 路由选择 Backend，合并各片段。
- `BaseRuntime` 合并 inner 与 FilesystemBackend 的片段，交给 `ProcessBackend`。
- `LaunchFragment { mounts: Vec<MountEntry>, chdir, env }`。

`ProcessBackend` 不是单独注册的 Backend 连接，而是 BaseRuntime 的进程执行部件；
这样保持现有 Composite/Adapter 契约不变，也不把 FS 专属逻辑放进 Adapter。

冻结机制选择：优先 cgroup v2（当前 delegated scope 可创建子 cgroup）；否则退化为
进程组 SIGSTOP/SIGCONT；能力字段 `freeze_mechanism = cgroup|process_group|none`。
进程组 fallback 无法覆盖 Agent 自行创建的新进程组/session，列为已知限制。

## 有序协议

`BaseRuntime::filesystem_operation` 在调用 FS Backend 前：

1. `FilesystemFork` 冻结父环境；`FilesystemMark` 冻结被标记环境；
   `FilesystemAccept` 冻结目标环境。
2. 冻结成功且目标有 ProcessHandle 时，把 `inner_pid`/`mount_namespace` 填入
   `BackendContext`，供 FS live accept 使用。
3. 执行 FS 操作。
4. 无论成功/失败都解冻；解冻失败追加到 `cleanup_failures` 并返回错误。
5. `writers_quiesced` 只由 BaseRuntime 在“冻结真实成功且解冻成功”后置 true；
   FS Backend 永远不写 true。没有 ProcessBackend/没有进程/冻结机制为 `none`
   时保持 false，且 FS 操作照常执行。
6. 进程已经退出但 handle 仍存在时，`is_running=false`：跳过 freeze，操作继续，
   不等待空 cgroup。有机制但 freeze 失败才让操作失败。

`BaseRuntime::suspend/resume` 改为真实 freeze/thaw；freeze/thaw 失败则状态不变并
返回错误。`terminate` 用 `cgroup.kill`（可用时）或进程组 SIGKILL；`cleanup` 删除
cgroup 目录。

## 失败回滚

- freeze 成功、后续操作失败：仍必须在返回前 thaw；thaw 失败进入 cleanup_failures。
- 冻结机制为 `none`：不调用 freeze/thaw，操作继续，`writers_quiesced=false`。
- cgroup freeze 写入 `1` 后等待 `frozen 1` 失败时回写 `0`，避免留下部分冻结。
- resume 时先 thaw；inner/filesystem resume 失败则尝试重新 freeze，保持状态闭环。
- cleanup 先 stop + 删除 cgroup，再清理 inner/filesystem；任何一步失败合并上报。
- cgroup 目录删除失败不伪造成功。

## 已知限制

- 进程组 SIGSTOP/SIGCONT 不能冻结已脱离进程组的后代。
- cgroup 创建/加入存在极小的 spawn 后窗口；当前用 pre_exec 写 `cgroup.procs` 缩小
  该窗口，但没有重新验证每条 exec 路径。
- 当前 ProcessBackend 是 BaseRuntime 内部组件，不是独立 Backend 连接；未来若需
  独立注册，可把它的方法提升为 Backend 实现。

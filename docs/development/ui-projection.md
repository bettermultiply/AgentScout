# UI Projection 与系统运行日志实现

已确认语义见 [control plane](../design/control-plane.md)。HTTP 和终端读取均经 Gateway 接入 UI Projection；Projection 按 Ledger 已保存记录的序号补齐可丢弃缓存，持有连接读取位置，连接释放时移除位置。Ledger 不再保存完整观察事件内存历史，写入失败不发布事件、不推进序号；已移出的 Branch 不放回 Tree。普通写入错误由协调循环使用进程持有的 SystemLog 报告历史不完整；Branch 归档最终失败由 Ledger 在释放记录前报告，报警经 Projection 转发，HTTP 用独立 SSE `system-warning` 消息，终端显示于 notice，不成为 Ledger 观察事件。

启动读取 `log_path` 并创建父目录、打开追加文件；打不开就拒绝启动。日志固定 UTF-8、UTC 毫秒时间、一行一条，说明中的 CR/LF 转为空格；只可配置路径，不做轮转。写入失败后的 Ledger 拒绝普通追加，避免把新记录接在可能损坏的尾部上。已退出 Tree 的 Branch 归档首次失败时，记录并检查失败前的完整行边界，截除失败尾部、解除 failed 标志后实际重试一次；若先前普通记录已失败，也必须先安全恢复该边界。恢复失败则明确说明恢复及重试无法完成。最终失败由 Ledger 在 BranchRecord 释放前调用 SystemLog，包含 session id、Branch id、首次及重试或恢复失败原因；报警沿用 Projection 通道，系统日志失败则 stderr 保留身份与原因。不保存 unarchived 集合，不恢复控制状态，不承诺 fsync。

当前按序号读取仍扫描 JSONL 文件并过滤序号，尚无文件偏移索引；缓存需要重建时重新读取文件。完整历史读取不持有追加使用的索引锁。Projection 的缓存不承担记录保存责任。进程持有本次运行的报警副本用于界面转发。

本次归档回归验证：先运行 `cargo test --lib archive_retries_once_and_repairs_partial_line`，旧实现因首次 `injected append failure` 直接返回而失败（退出码 101）。修复后 `cargo test --lib` 183 项、`cargo test --test ledger_projection` 2 项、`cargo test --bin ascout-control-plane` 3 项及 `cargo check --all-targets` 通过；包含 4 项新增归档回归，覆盖零字节／部分写入、两次真实写入后失败、先前 failed 标志恢复及无法恢复。系统日志 `/dev/full` 失败场景仍可经 Projection 读取完整身份与原因，stderr 回退沿用；代码顺序保证报警在 BranchRecord 释放前，移除 unarchived 所有权与退出错误路径。仍有既有 `IsTerminal` 未使用警告。未调用真实模型。

验证：`cargo check --all-targets`；Ledger 写入 `/dev/full` 不产生内存事件、Projection 丢弃缓存与重新打开 Ledger 的序号检查共 2 项；Gateway 现有 2 项；control plane 现有 3 项；Authority HTTP 生命周期现有测试（本地伪模型）1 项；Web UI TypeScript 与构建。配置检查使用私有 HOME，覆盖缺省路径、自定义路径、打不开文件时拒绝启动、拒绝格式配置。未调用真实模型。编译仍有既有 `IsTerminal` 未使用警告；Web 构建提示 bundle 大于 500 kB。

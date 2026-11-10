# 控制面观察接口（2026-11-10）

本阶段只实现后端及现有页面/TUI 的兼容接线。前端已选择 React + React Flow、Vite 构建、产物嵌入 Rust 二进制、开发代理；该前端尚未实施。

## 当前接口

仅绑定 loopback，继续校验 Host。所有观察接口只读；`POST /api/request` 的 Prompt、Interrupt、Shutdown 以及同源 JSON 校验保持原样。

- `GET /api/observation/state-event-set`：`{sequence, events}`。events 是从第 1 号到 sequence 的完整有序公共事件读取，称为状态事件集；客户端自行折叠得到展示状态，不包含对话正文。
- `GET /api/observation/events?after=N`：SSE，只补发 `sequence > N` 的事件，再持续等待新事件；`id` 与事件 sequence 相同，`data` 是事件 JSON。自动重连的 `Last-Event-ID` 优先于初始 after 参数。缺省从 0 开始；非法序号返回 400，超前序号返回 409，需重新取得状态事件集。15 秒空闲发送注释心跳。每个连接独立线程，不占住其他请求。
- `GET /api/branches/{branch_id}/conversation`：Branch id 按 URL segment 编码；未知 Branch 返回 404。响应 `{inherited, messages, streaming, unverified}`；Candidate 一律 `unverified=true`，已归档 Branch 的历史仍可读取。

公共事件为 `{sequence, timestamp_ms, task_id, event, ...内容}`。timestamp_ms 是控制面记录事实的 Unix 毫秒时刻，不冒充源侧实际发生时间。`branch_created` 的时间就是其公共 timestamp_ms。枚举值使用现有代码的 snake_case 序列化。

事件名与内容：

- `branch_created`：branch_id、parent_branch_id、role、edge_id、speculation_kind。
- `authority_changed`：branch_id、previous_authority。
- `status_changed`：branch_id、status、reason。
- `environment_changed`：branch_id、state、independent_instance、parent_environment、configuration。
- `environment_cleaned`：branch_id、succeeded、reason。
- `conversation_updated`：branch_id、unverified，不含正文。
- `model_usage`：branch_id、action_id、model、input_tokens、output_tokens、cache_read_tokens、cache_write_tokens、total_tokens、cost、duration_ms。
- `tool_finished`：branch_id、action_id、status，不公开工具结果或参数。
- `settlement`：branch_id、hit、time_ms、money、currency；两个维度均为 `{value, measured}`，value 为有符号数。只定义公共类型，没有无数据的生产器。
- `run_status`：ready、running、notice、connected，维持现有交互状态。

## 已核实的数据与差距

当前 Agent 协议实际已有 `Usage`，Pi `message_end` 已报告 model 和 SDK usage。因此 `model_usage` 按已知 SDK 字段映射 token 和 cost.total；字段未提供时返回 null，不补零。工具完成来自现有 `ActionFinished(ToolExecution)`。没有上报调用耗时，也没有可靠串行基线和结算归因，因此 duration_ms 为 null，不产生 settlement。

下一轮需要讨论 Agent 接入契约中的“模型调用结束/工具调用结束”上报项：稳定 action 关联、源侧起止/耗时、token 与费用的完整字段、币种及实测/估计来源。现有 Usage 不能当作该契约已完整实现；不从文本输出推断收益。

Tree 尚无 Edge identity，edge_id 为 null；speculation_kind 在创建入口按父 action 的现有 ActionKind 填写，无法关联时为 null。没有 Runtime 嵌套身份报告，parent_environment 为 null，不能拿逻辑 Parent 冒充环境嵌套。configuration 仅公开 shared_environment 能力摘要，不公开本地配置文件路径或凭据。Failed 不足以判断独立 Instance 是否仍存在，返回 null；Cleaned 表示该 Branch 已不拥有独立 Instance。

Tree 的 insert_branch、change_status、change_authority、record_runtime_state / emit_environment 是创建、生命周期、角色与环境事件入口。入口修改状态时将事实放入 Tree 的 pending_observations，控制循环通过 drain_observations 取走并交给 Ledger，不依赖 HTTP 或文件格式。状态转换入口提供原因；Discard Applied 后依次记录 Discarded、Cleaning，不遗漏同一次处理内的中间状态。对话提示、模型用量、工具完成与 run_status 保持原有来源，通过 Ledger 追加。清理成功来自 Runtime Cleaned；Runtime Failed 只发布失败状态，尚不能从现有状态区分 terminate、cleanup 或其他操作失败，不伪造 environment_cleaned 失败结果。

Ledger 复用 events.jsonl：观察事件记录为 {message: "observation", observation: 公共事件}，与 record 的原始事实和 archive_branch 的归档记录区分。采用同一文件便于一并检查控制事实和观察事实，保留 record / archive_branch 的职责；不逐次保存完整树，不设计崩溃恢复。append_observation 在追加时分配连续序号；文件写入失败仍保留内存事件并返回错误，报告历史不完整，不承诺部分文件写入回滚。

Ledger::reader 返回只读 LedgerReader；Ledger 和句柄均提供 state_event_set、events_after。web.rs / tui.rs 直接接收该句柄，不经 Interface 转交事件。内存有序历史用不可变 Arc 链保存，读取只在短锁内复制末端句柄和序号，遍历、复制和折叠均在锁外；追加不等待完整历史读取。events_after 在同一个序号锁和 Condvar 上检查并等待新事件，追加和断开事件唤醒所有等待连接。销毁历史采用迭代释放，避免长链递归释放栈溢出。

日志包含完整运行前缀，不截断；状态事件集和 TUI 按事件折叠，尚未优化长运行的折叠与内存成本，也未实现折叠后的缓存状态。序号范围是单次控制面进程运行，重启后需重新取得状态事件集，不支持跨重启续传。归档不删除观察历史或对话。

新增代码名和接口名需主 Agent 向用户确认，未将它们扩展为新的领域术语。

本轮待确认代码名：StateEventSet / state_event_set / `/api/observation/state-event-set`（状态事件集直译）；LedgerReader / reader（Ledger 只读句柄）；append_observation（Ledger 观察追加入口）；pending_observations / drain_observations（Tree 待发队列与取走入口）；change_status / change_authority / emit_environment / resolve_candidate_with_reason（内部转换及明确原因入口）；flush_observations（控制循环交接）；ObservationBlock / ObservationIndex（私有不可变事件链及末端索引）；observation.rs（公共事件类型模块）；文件记录 message 的 observation 值（区分观察记录）。ObservationEvent、ObservationContent、ObservationAmount、run_view 和现有公共事件名沿用原名，仅移动职责或调用位置；没有新增公共事件名。

## 本轮验证

复核补充：默认并行测试首轮 141 项通过；后一次复跑中 btrfs_snapshot_copy_fork_is_end_to_end、extra_mount_fork_diff_accept_discard_round_trip 出现 overlay 挂载就绪超时。未修改 Runtime 或测试等待逻辑，随后 `cargo test --lib --bins -- --test-threads=1` 全部 141 项通过。临时 smoke 另验证同一次 Discard Applied 处理内的 Discarded → Cleaning 两条事件、等待新事件唤醒，以及 POST 请求继续返回既有的 202 Accepted；临时程序最初误期待 200，修正检查预期后通过。

`cargo build` 通过；`cargo test --lib --bins` 为 140 项库测试与 1 项控制面测试通过，没有新增测试；既有展示选择测试直接使用折叠后的观察类型，避免构造界面事件保存者。临时程序 `/tmp/opencode/observation-smoke.rs` 使用真实 Ledger、Tree、ui.rs 与 web.rs，验证状态事件集、连续序号、SSE 补发、Last-Event-ID 优先、新事件投递、断开唤醒、对话读取、未知 Branch 404、非法序号 400、超前序号 409 及文件观察记录。不运行真实模型、浏览器/TUI 人工交互或完整 Firecracker E2E；不处理该 E2E 既有 Unix socket 路径长度问题。

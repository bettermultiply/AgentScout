# 控制面观察接口（2026-11-10）

本阶段只实现后端及现有页面/TUI 的兼容接线。前端已选择 React + React Flow、Vite 构建、产物嵌入 Rust 二进制、开发代理；该前端尚未实施。

## 当前接口

仅绑定 loopback，继续校验 Host。所有观察接口只读；`POST /api/request` 的 Prompt、Interrupt、Shutdown 以及同源 JSON 校验保持原样。

- `GET /api/observation/snapshot`：`{sequence, events}`。events 是截至 sequence 的完整公共事件前缀，客户端从头折叠得到快照，不包含对话正文。
- `GET /api/observation/events?after=N`：SSE，只补发 `sequence > N` 的事件，再持续等待新事件；`id` 与事件 sequence 相同，`data` 是事件 JSON。自动重连的 `Last-Event-ID` 优先于初始 after 参数。缺省从 0 开始；非法序号返回 400，超前序号返回 409，需重新取得快照。15 秒空闲发送注释心跳。每个连接独立线程，不占住其他请求。
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

Tree 尚无 Edge identity，edge_id 为 null；speculation_kind 只在父 action 仍可关联时填现有 ActionKind，否则为 null。没有 Runtime 嵌套身份报告，parent_environment 为 null，不能拿逻辑 Parent 冒充环境嵌套。configuration 仅公开 shared_environment 能力摘要，不公开本地配置文件路径或凭据。Failed 不足以判断独立 Instance 是否仍存在，返回 null；Cleaned 表示该 Branch 已不拥有独立 Instance。

当前在每个控制事件处理后、progress 前后及归档前比较受限观察事实并追加日志；不会整体复制或公开内部 Tree。一个树方法内部连续经过的瞬时中间状态可能没有独立可见时点；若需要每一次内部状态转换的精确原因和源时间，需要下一轮将观察追加接到树的统一状态转换入口。清理成功来自 Runtime Cleaned；Runtime Failed 只发布失败状态，尚不能从现有状态区分 terminate、cleanup 或其他操作失败，不伪造 environment_cleaned 失败结果。

日志目前仅保存在该进程内，包含完整运行前缀，不截断、不跨进程恢复；快照和 TUI 按事件折叠，尚未优化长运行的折叠与内存成本。序号范围是单次控制面进程运行，重启后需重新取得快照，不支持跨重启续传。归档不删除观察历史或对话。

新增代码名和接口名需主 Agent 向用户确认，未将它们扩展为新的领域术语。

## 本轮验证

`cargo build` 通过；`cargo test --lib --bins` 为 140 项库测试与 1 项控制面测试通过，没有新增测试。临时 HTTP smoke 验证快照、SSE 重放、Last-Event-ID 优先、按需对话、旧路径 404 与非法/超前序号错误；未调用模型。全量 `cargo test` 的 8 项真实 Firecracker E2E 失败，多项报当前仓库下生成的 Unix socket 路径过长，另有环境未建立后的连带失败；不声称全量测试通过，也未修改 Firecracker 来绕过失败。没有执行真实模型或浏览器/TUI 人工交互验收。

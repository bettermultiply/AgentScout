# 控制面观察接口（2026-11-10）

本阶段已实现后端观察接口、TUI 兼容接线与正式 WebUI。WebUI 使用 React + React Flow、Vite 构建、轻量状态管理，构建产物嵌入 Rust 二进制，开发时通过 Vite 代理访问控制面 API；启动、开发与构建说明见 [控制面 WebUI](web-ui.md)。

## 当前接口

仅绑定 loopback，继续校验 Host。所有观察接口只读；`POST /api/request` 的 Prompt、Interrupt、Shutdown 以及同源 JSON 校验保持原样。

- `GET /api/observation/state-event-set`：`{sequence, chances, events}`。events 是从第 1 号到 sequence 的完整有序公共事件读取，称为状态事件集；`chances` 是由这些事件折叠出的 Chance 当前视图（id、父 Branch/父 Chance、operation、tool、stage、authority_state、proposal_ids、branch_ids），客户端可据此直接显示机会状态。客户端自行折叠 events 得到展示状态，不包含对话正文。
- `GET /api/observation/events?after=N`：SSE，只补发 `sequence > N` 的事件，再持续等待新事件；`id` 与事件 sequence 相同，`data` 是事件 JSON。自动重连的 `Last-Event-ID` 优先于初始 after 参数。缺省从 0 开始；非法序号返回 400，超前序号返回 409，需重新取得状态事件集。15 秒空闲发送注释心跳。每个连接独立线程，不占住其他请求。
- `GET /api/branches/{branch_id}/conversation`：Branch id 按 URL segment 编码；未知 Branch 返回 404。响应 `{inherited, messages, streaming, role, status}`；`role` 与 `status` 是该 Branch 当前的 Branch 角色和生命周期状态，前端据此显示候选的投机身份，不再依赖单独的标志字段。已归档 Branch 的历史仍可读取。

公共事件为 `{sequence, timestamp_ms, task_id, event, ...内容}`。timestamp_ms 是控制面记录事实的 Unix 毫秒时刻，不冒充源侧实际发生时间；所有事件（包括 `branch_created`）都遵循这一含义。枚举值使用现有代码的 snake_case 序列化。`branch_created`、`model_usage` 与 `tool_finished` 中的源侧时间字段是接入侧测量值，与事件公共 timestamp_ms 含义不同。

事件名与内容：

- `branch_created`：branch_id、parent_branch_id、role、chance_id、speculation_kind、speculation_started_at_ms。chance_id 是这个 Branch 所属 Chance；初始权威 Branch 为 null。speculation_started_at_ms 只对由准入请求创建的投机候选填写，值为对应准入请求的接入侧时间戳，表示投机起点；其他 Branch 为 null。
- `chance_created`：chance_id、parent_branch_id、parent_chance_id、operation_id、tool_name、stage、authority_state。`admit` 通过时产生，此时 stage 为 `pending`、authority_state 为 `not_requested`。
- `chance_stage_changed`：chance_id、stage、reason。stage 取 `pending`、`running`、`resolving`、`{"accepted": proposal_id}` 或 `{"discarded": reason}`；reason 为描述性文字。
- `authority_state_changed`：chance_id、state。state 取 `not_requested`、`fetching`、`arrived`、`unavailable`。
- `proposal_added`：chance_id、proposal_id、payload。每次 `predict` 生成一个 Proposal 时发布。
- `authority_changed`：branch_id、previous_authority。
- `status_changed`：branch_id、status、reason。status 取 `created`、`running`、`{"blocked": reason}`、`finished`、`promoting`、`promoted`、`discarding`、`discarded`、`cleaning`、`cleaned`；Blocked 的 reason 取 `extent_reached`、`rebalanced` 或 `effect_gate`，同一时刻只记最近一次挡住它的 Decision。
- `environment_changed`：branch_id、state、independent_instance、parent_environment、configuration。
- `environment_cleaned`：branch_id、succeeded、reason。
- `conversation_updated`：branch_id，不含正文。
- `model_usage`：branch_id、action_id（无法与模型 action 配对时为 null）、model、input_tokens、output_tokens、cache_read_tokens、cache_write_tokens、total_tokens、cost、started_at_ms、finished_at_ms、duration_ms；started_at_ms 与 finished_at_ms 是接入侧测量的模型请求开始与结束时刻，duration_ms 在两者可得时由二者相减得到。
- `tool_finished`：branch_id、action_id、tool_name、status、started_at_ms、finished_at_ms、duration_ms，不公开工具结果或参数；tool_name 来自同一 action 的 ActionStarted，某 action 只有准入请求而尚无 ActionStarted 时为 null；started_at_ms 同样只来自接入侧真实的 ActionStarted；finished_at_ms 来自 ActionFinished；duration_ms 在起点和终点时间都可得到时填写。
- `settlement`：branch_id、hit、time_ms、money、currency；time_ms 与 money 都是 `{measured, estimated}`，两个部分各自是带符号数字或 null，缺少实测时为 null、缺少估计依据时为 null，二者不相加、也不互相填充；currency 只在金钱维度有数值时给出。每次已准入投机恰好产生一次；命中对应 Branch 进入 Promoted，未命中对应 Branch 进入 Discarded。累计值由读取方分别累加实测与估计部分，不另发统计事件。
- `run_status`：ready、running、notice、connected，维持现有交互状态。

## 已核实的数据与差距

Agent 协议现在要求 `ActionStarted`、`ActionFinished` 与 `SpeculativeAdmissionRequest` 携带接入侧测量的 Unix 毫秒时间戳；`Usage` 应关联 `action_id`，并携带接入侧的开始与结束时间；接入侧无法配对时仍上报用量，`action_id` 为 null。准入请求的 `timestamp_ms` 只表示投机起点：控制面登记 action 身份以便与后续结果配对，不再用该时间合成 action 的开始时间；工具真正开始的时间只来自接入侧真实的 `ActionStarted`。`model_usage` 按已知 SDK 字段映射 token 和 cost.total，字段未提供时仍返回 null，不补零；`started_at_ms` / `finished_at_ms` 来自 Usage 的接入侧起止时刻，`duration_ms` 在两者可得时由二者相减得到。工具完成来自 `ActionFinished(ToolExecution)`，`tool_name` 与 `started_at_ms` 都来自同一 action 的 `ActionStarted`，`finished_at_ms` 来自 `ActionFinished`；某 action 只有准入请求而尚无 `ActionStarted` 时工具名与起点为 null，`duration_ms` 在起点缺失或终点早于起点时不填写。

## 一次投机的结算（2026-11-10）

Tree 在 `insert_branch` 收到准入投机的起点时建立内部 `SpeculationRecord`（投机起点、投机类型、是否已结算）；只有由 `SpeculativeAdmissionRequest` 创建的投机候选会有这条记录。统一状态转换入口 `change_status` 在 Branch 进入 `Promoted` 或 `Discarded` 时调用 `queue_settlement`：命中只结算一次，未命中同样只结算一次；后来被替换并进入 `Discarded` 的旧权威不会重复结算。事件进入 Tree 的待发队列，由协调循环的 `flush_observations` 取走并追加到 Ledger。成功写入文件前的内存事件仍保留。

公共形状：`time_ms` 与 `money` 都是 `{measured, estimated}`。两个部分各自独立，缺少实测时为 null，缺少估计依据时为 null；实现不把两者相加，也不用其中一部分填充另一部分。币种只在金钱维度至少有一个数值时写 `"USD"`，否则为 null。

规则与实现位置：

- 工具投机命中：`time_ms.measured` = 权威侧同一 `operation_id` 的 `ActionFinished` 接入侧 `timestamp_ms` − 投机起点，`estimated` 为 null；`money` 没有可归属的模型费用差额，两部分都是 null，`currency` 为 null。控制面为此在 `handle_action_finished` 中按 Branch 和 action 保留已完成工具 action 的接入侧时刻，不改 action 生命周期。
- 模型投机命中且有权威模型校验：快速侧取候选 Branch 的模型用量（同一 `operation_id` 下有记录优先，否则只有候选恰好一条模型用量时取它；多于一条且无法对应时视为无数据），权威侧取父 Branch 同一 `operation_id` 的用量。两侧都有 `duration_ms` 时 `time_ms.measured` = 权威耗时 − 快速耗时；两侧都有 `cost.total` 时 `money.measured` = 权威费用 − 快速费用；任一数据缺失时该维度两部分都为 null，不退化为估计。
- 模型投机命中但没有权威侧同 operation 用量：按“只靠简单校验”处理。被避免的权威调用只能以同一快速调用的接入侧耗时和费用作为估计依据，因此 `time_ms.estimated` = 快速模型耗时、`money.estimated` = 快速模型费用，实测部分为 null。Pi SDK 的 `cost.total` 按美元计，金额存在时币种写 `"USD"`。
- 任何未命中：`time_ms` 的实测与估计部分都是 null。原因见下方数据缺口：未命中的时间项要记的是投机给权威路径造成的额外延迟，而不是候选存活时长或快速模型自身的耗时。`money.measured` = −(该候选 Branch 所有已记录模型用量的费用之和)；没有任何用量记录或任一已记录用量缺少 `cost.total` 时两部分为 null，不把部分成本当成完整成本。
- 其他 `ActionKind` 没有定义结算公式，两个维度都不猜测，保持 null。

当前数据缺口（因此这些情况不产生伪造值）：

- 未命中的时间项没有可靠测量。已知延迟来源是：权威路径在工具 Hook 处可能多等一次本地 Policy 求值（沙箱内 Python 子进程），以及所有报告共用一条串行发送队列、控制面读取慢时会拖住后续报告。二者都没有接入侧时间戳，控制面也不记录“候选决定丢弃”之外的可靠对照，因此不输出候选存活时长，也不输出 0。
- “简单校验失败而必须再跑权威模型时，时间为负的估计值（浪费 = 快速模型耗时 + 校验耗时）”尚未实现：校验耗时没有测量，只用快速模型耗时会遗漏一半浪费，也属于本设计明确不统计的“白耗”；并且模型投机当前还没有生产者。接入校验计时后才能实现该规则。
- 模型投机还没有生产者：Pi 的模型推理前 Hook 只执行本地 Decision，不发送 `SpeculativeAdmissionRequest`，控制面当前只从工具准入创建候选。上面两条模型命中规则按已有数据结构实现，但暂无运行路径会触发。
- 模型快速侧与权威侧没有显式的投机关联。“有权威模型校验”只能以父 Branch 是否存在同一 `operation_id` 的 `Usage` 近似判断；候选同时有多条模型用量且无法对应到被投机的 operation 时，快速侧视为无数据。未来接入模型投机时应补显式关联。
- 权威工具的 `ActionFinished` 可能晚于 Promote 到达；此时命中结算已经发出，工具命中时间为 null，不回填。
- 权威模型的 `Usage` 也可能晚于 Promote 到达；此时按“没有权威模型校验”只写估计部分，之后不回填为实测。
- 候选的 `Usage` 可能晚于 Discarded 到达；此时未命中结算已经发出，金钱维度为 null，不回填。
- Discard 命令未确认（例如只在途或超时）时 Branch 尚未进入 `Discarded`，因此这次投机没有结算事件；不以“决定丢弃”冒充已丢弃。

## events.jsonl 的三类记录

同一个 `events.jsonl` 同时保存三类记录，读取时按顶层 `message` 字段区分，不能把每一行都当作观察事件：

- 原始事实：控制面与 Runtime 运行事实、Agent 消息引用、Policy 记录和调度记录。transcript 只保留 Branch id、`session_entry_id` 与 role，正文存于权威 session 文件；其他 action／Runtime 事实仍可能含完整工具参数与结果。`message` 是事实名（例如 `agent_adapter` 来源的消息、`runtime_event`、`candidate_created`、`policy_error`），可能带 `source` 与运行内相对时刻 `t_ms`；这些行不是公共观察事件。
- Branch 归档：`{"message":"branch_archived","branch_id":...,"branch":...}`，保存 Branch 移出活动 Tree 时的记录；它不是观察事件，也不参与界面折叠。
- 观察事件：`{"message":"observation","observation":{sequence,timestamp_ms,task_id,event,...}}`。只有这一类的 `observation` 对象进入状态事件集和 SSE；`sequence` 是同一控制面进程内连续递增的观察序号。

排查问题时可以混合检索三类记录；按事件重放界面状态时必须只使用第三类。

## 模型 action 与 usage 的配对

Pi 接入在每个 Branch 内保存模型 action 队列：模型推理前把 `action_id` 和开始时刻入队，并发送 `ActionStarted`；assistant 的 `message_start` 标记队列中下一个尚未见到消息的 action，`message_end` 取走该 action 并发送 `Usage`。Pi 会在 `message_end` 之前用 `stream.result()` 结算模型流并调用 `beforeModelInference` 的完成回调，因此完成回调只发送 `ActionFinished`，不把仍可能收到消息的 action 出队；只有完全没有产生消息的请求（流创建前就抛错）才在完成回调中移除，避免它与后续消息错配。`message_start` / `message_end` 本身是 SDK 输出事件，不是推理前 Hook。

如果一条 assistant 消息找不到被标记的 action（例如该调用没有经过 Hook），接入侧仍发送 `Usage`，`action_id` 为 null；控制面的 `model_usage` 原样使用接入侧给出的 `action_id`，不另行推断配对，也不伪造身份。同一 Branch 出现并发模型请求时，消息与 action 的对应仍以 `message_start` 的标记顺序为准；无法确定时上报 null，不猜测。

时钟实现：接入侧用 `Date.now()` 产生 Unix 毫秒墙钟时间戳；模型结束时间优先用 SDK 以单调时钟测得的 `durationMs` 锚定在请求开始时刻，缺少时才退回结束时的墙钟。Action 耗时使用两个墙钟时间戳相减，因此墙钟回退时不填写而不是给出负值。上述时间表示接入侧观察到的请求开始、结果到达或提交时刻，不是控制面收到消息的时刻。

当前已知限制（属于原型实现，不是目标语义）：权威路径在工具 Hook 处可能需要等待一次本地 Policy 求值；该求值在沙箱内启动 Python 子进程，所有报告共用 Pi 接入侧的一条串行发送队列，控制面读取较慢时会拖住后续报告。对话正文仍由接入事件实时带来；持久化时 transcript 保存消息引用，正文由权威 session 文件保存，继承关系由记录父链表达。文件位置与引用是内部记录，不直接加入公共观察事件。原生 compaction 可随 SessionManager 汇总保留，未持久化的扩展上下文改写不能还原，见[记录与回放](replay.md)。

Tree 在 `admit` 通过时创建 Chance；`branch_created` 的 `chance_id` 与 `chance_created` 使用同一个不透明 id，初始权威 Branch 为 null。speculation_kind 在创建入口按父 action 的现有 ActionKind 填写，无法关联时为 null。没有 Runtime 嵌套身份报告，parent_environment 为 null，不能拿逻辑 Parent 冒充环境嵌套。configuration 仅公开 shared_environment 能力摘要，不公开本地配置文件路径或凭据。Failed 不足以判断独立 Instance 是否仍存在，返回 null；Cleaned 表示该 Branch 已不拥有独立 Instance。

Tree 的 insert_branch、change_status、change_authority、record_runtime_state / emit_environment 是创建、生命周期、角色与环境事件入口。入口修改状态时将事实放入 Tree 的 pending_observations，控制循环通过 drain_observations 取走并交给 Ledger，不依赖 HTTP 或文件格式。状态转换入口提供原因；Discard Applied 后依次记录 Discarded、Cleaning，不遗漏同一次处理内的中间状态。对话提示、模型用量、工具完成与 run_status 保持原有来源，通过 Ledger 追加。清理成功来自 Runtime Cleaned；Runtime Failed 只发布失败状态，尚不能从现有状态区分 terminate、cleanup 或其他操作失败，不伪造 environment_cleaned 失败结果。

Ledger 复用 events.jsonl：观察事件记录为 {message: "observation", observation: 公共事件}，与 record 的原始事实和 archive_branch 的归档记录区分。采用同一文件便于一并检查控制事实和观察事实，保留 record / archive_branch 的职责；不逐次保存完整树，不承诺完整 Tree 崩溃恢复。Promote 与消息引用可以提供有限消息定位依据，但恢复工具尚未实现。append_observation 在追加时分配连续序号；文件写入失败仍保留内存事件并返回错误，报告历史不完整，不承诺部分文件写入回滚。

Ledger::reader 返回只读 LedgerReader；Ledger 和句柄均提供 state_event_set、events_after。Gateway HTTP 与 TUI 直接接收该句柄，不经 Interface 转交事件。内存有序历史用不可变 Arc 链保存，读取只在短锁内复制末端句柄和序号，遍历、复制和折叠均在锁外；追加不等待完整历史读取。events_after 在同一个序号锁和 Condvar 上检查并等待新事件，追加和断开事件唤醒所有等待连接。销毁历史采用迭代释放，避免长链递归释放栈溢出。

日志包含完整运行前缀，不截断；状态事件集和 TUI 按事件折叠，尚未优化长运行的折叠与内存成本，也未实现折叠后的缓存状态。序号范围是单次控制面进程运行，重启后需重新取得状态事件集，不支持跨重启续传。归档不删除观察历史或权威 session 文件中的消息旁支。

新增代码名和接口名需主 Agent 向用户确认，未将它们扩展为新的领域术语。

本轮待确认代码名：StateEventSet / state_event_set / `/api/observation/state-event-set`（状态事件集直译）；LedgerReader / reader（Ledger 只读句柄）；append_observation（Ledger 观察追加入口）；pending_observations / drain_observations（Tree 待发队列与取走入口）；change_status / change_authority / emit_environment / resolve_candidate_with_reason（内部转换及明确原因入口）；flush_observations（控制循环交接）；ObservationBlock / ObservationIndex（私有不可变事件链及末端索引）；observation.rs（公共事件类型模块）；文件记录 message 的 observation 值（区分观察记录）。ObservationEvent、ObservationContent、ObservationAmount、run_view 和现有公共事件名沿用原名，仅移动职责或调用位置；没有新增公共事件名。上一轮新增待确认代码名：Agent 消息字段 `timestamp_ms`（接入侧事件时刻）、`started_at_ms` / `finished_at_ms`（Usage 的接入侧起止时刻），对话响应字段 `role` / `status`，以及 `tool_finished` 的 `duration_ms`。2026-11-10 树形界面轮新增待确认字段：`tool_finished.tool_name`（来自 ActionStarted.name，缺失为 null）。本轮新增待确认代码名：公共事件 `branch_created` 的 `speculation_started_at_ms`（投机起点）、`model_usage` 与 `tool_finished` 的源侧字段 `started_at_ms` / `finished_at_ms`、Tree 内部入口 `register_action`（登记 action 身份，不记录开始时间）及 `ToolPending` 的同名投机起点字段。`model_usage` 的 `action_id` 沿用原名，正常情况为模型 action id，无法配对时为 null。2026-11-10 结算轮新增公共字段：`time_ms` 与 `money` 的 `measured` / `estimated`（替代原 `value` / `measured`）；内部名：`SpeculationRecord`、`ModelUsageFact`、`queue_settlement`、`fast_model_usage` 与模块 `settlement.rs`，`currency` 沿用原名但改为可空。

## Pi 接入的 EOF 与退出

接入侧使用 `socket.iterator({destroyOnReturn:false})` 读取命令，而不是默认 async 迭代器：默认迭代器在循环结束（干净 EOF）时会销毁 socket，可能在仍有 `report` 写入时产生 `AbortError`，表现为退出码非零或 stderr 非空。循环结束后，接入侧先排空串行发送队列、完成会话取消、报告 `agent_released`，再显式 `socket.end`。

显式保留 socket 后又暴露出另一个退出问题：当取消发生在一次本地 Policy 求值期间，Pi 的求值器在 `child.kill` 后立即拒绝，求值子进程的 stdio 句柄可能仍留在事件循环中，使接入进程在 EOF 后不退出。该求值器位于 Pi 仓库，本轮不修改；接入侧在完成上述收尾后显式 `process.exit`（有传输错误时为 1，否则为 0），因为此时已没有本进程需要完成的工作。`tests/pi-access.mjs` 的 admission 用例正是通过“客户端 EOF 后接入进程及时退出”检查这一点。

## 本轮验证

`cargo build` 通过；`cargo test --lib --bins` 为 140 项库测试与 1 项控制面测试通过，没有新增常驻测试。`/usr/bin/node --test tests/pi-access.mjs` 的 4 项接入回归通过；修复前用带 `destroyOnReturn:false` 的半成品连续运行 20 次有 2 次 admission 失败（均为 “integration did not exit after EOF”，不是 `AbortError`），修复后 100 次仅 admission 用例与 20 次完整测试均为 0 失败。用 HEAD 的默认迭代器连续运行 80 次 admission 用例未复现 `AbortError`，说明它是低频竞态而不是稳定失败；其机制由默认迭代器在干净 EOF 时销毁 socket 解释，修复保留 `destroyOnReturn:false` 并加上显式退出。

模型用量漏报修复：`tests/pi-access.mjs` 新增一个伪造 SDK 用例，模拟 Pi 先结算模型流并调用完成回调、再发出 `message_end` 的真实顺序，另有一轮不经过 Hook 的 assistant 消息。修复前该用例稳定超时，收到的消息里只有 `action_started`、`action_finished`、`transcript` 与 `turn_completed`，没有 `usage`；修复后完成回调不再提前出队，配对用例的 `action_id` 与 `ActionStarted` 一致，未配对用例仍发送 `usage` 且 `action_id` 为 null。`cargo build`、`cargo test --lib --bins`（140 项库测试与 1 项控制面测试）通过；`node --test tests/pi-access.mjs` 的 5 项接入回归连续运行 20 次无失败。全程使用伪造模型与手工事件，没有真实调用模型。

临时序列化检查确认 `branch_created` 含 `speculation_started_at_ms`，`model_usage` 的 `action_id` 正常为字符串、无法配对时为 null，并携带 `started_at_ms` / `finished_at_ms`，`tool_finished` 在起点缺失时输出 null 且不计算 `duration_ms`。结算临时检查确认工具命中输出 `time_ms: {"measured":123.0,"estimated":null}`、`money` 两部分为 null 且币种为 null；模型未命中输出 `time_ms` 两部分为 null、`money: {"measured":-0.5,"estimated":null}`、币种 `USD`；同一 Branch 命中后再次进入 Discarded 不重复结算。

不运行真实模型、浏览器/TUI 人工交互或完整 Firecracker E2E；不处理该 E2E 既有 Unix socket 路径长度问题。

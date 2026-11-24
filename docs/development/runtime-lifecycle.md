# Runtime 生命周期协议

## 当前范围：执行承载、资源清理与恢复

2026-11-16 确定：暂不要求程序异常退出后重新启动并继续原任务；已有主动保存、加载和恢复功能保留，范围见[主动退出恢复](recovery.md)。异常退出后的受管进程停止、残留资源归属确认和清理仍需处理，不因暂不要求任务恢复而取消，也不据此要求持久化全部运行状态。

正常运行中的执行承载与派生以 [Branch 语义](../design/branch-semantics.md) 和 [Runtime 契约](../design/runtime-adapter-contract.md) 为准，不重新讨论是否要求保留原进程。能力声明的充分性归 Discussion 第 17 项，Branch 与 Runtime 执行上下文对齐归第 23 项，跨步骤与嵌套的整体验证归第 8、33 项；不能把已有基本执行闭环笼统列为尚未实现。

服务效果插件的目标语义见[效果管理](../design/effect-management.md)，实现范围见 [Runtime 接入](runtime-adapter.md)，相关 Policy 差距归第 28 项。输入传播与输入消费边界分别归第 24、26 项，Agent 输入接入范围见[接入实现](agent-access.md)；历史持久化与主动恢复归第 31 项。未说明具体能力缺口的“完整资源租约”不作为独立交付要求；资源归属与清理责任随第 30 项处理。是否采用 pidfd 等机制留到具体清理方案选型时决定，本轮不作选择。

## 2026-11-16：Runtime 创建身份

Gateway 提供 `establish()` 和 `branch(parent_environment_id, agent)`；协议中的 branch 不接收子环境身份，创建结果的 `environment_id` 指向新环境。Runtime 内部 `Backend::create_environment` / `create_child_environment` 分配身份，再调用内部 Backend 组合。分配器使用进程、时间与原子序号并检查当前状态，不接收 Branch 或 execution 身份。

`BranchRecord.environment_id` 和待交付的启动命令使用可选绑定；协调完成事件收到 Runtime 返回值后绑定 Branch 并填入启动命令。创建失败已产生环境时，返回该身份并保留实际 Backend 参与者供清理；没有环境时不发终止或清理调用。保存恢复校验非空、唯一绑定，区分创建中、已绑定和已释放记录，恢复显式 load 后沿用原身份。

无模型回归覆盖重复 establish、独立 sibling、后续操作定位、拒绝预定身份、组合 Runtime 部分失败后的身份和清理，以及 control plane 失败绑定的保存恢复。`cargo test --lib` 为 170/170；`cargo fmt -- --check`、`cargo check --all-targets`、`cargo build --bins` 通过。真实 Firecracker 集成调用方已对齐返回身份，本轮未运行真实 VM 或真实模型集成测试。协议版本仍为原型版本 1，升级时 control plane 和 runtime proxy 必须一起更新，不保证混用旧二进制。

本文集中说明一个 Runtime 实例在运行期如何建立、操作、派生、提升、静止、恢复、终止和清理环境，以及多 Backend 组合下这些操作如何按固定顺序执行、如何暴露部分失败、如何聚合状态和如何划分清理责任。它是开发阶段的实现说明，对应代码 `src/runtime/`、`src/runtime/backend.rs`、`src/runtime/proxy/`、`src/runtime/agent_supervisor.rs`、`src/runtime/backends/filesystem/` 和 `src/bin/ascout-runtime-adapter.rs` 的当前语义；正式接入契约见 [runtime proxy 契约设计](../design/runtime-adapter-contract.md)，Branch 的树结构与裁决语义见 [Branch 语义设计](../design/branch-semantics.md)。

## 1. 当前 Runtime 的责任边界

`src/bin/ascout-runtime-adapter/config.rs` 读取 Runtime JSON，将旧顶层 `backend` 和 `filesystem` 配置规范化为登记项及有序路由，再用独立的连接配置装配 `RuntimeProxy`。`BackendRegistry` 保存实现入口、接口和能力声明，`BaseRuntime::from_registry` 接收 Runtime 身份及路由策略，检查路由后创建并持有实例。直接提供实例和路由策略的 Rust 调用方使用 `BaseRuntime::with_policy`。`RuntimeProxy` 直接使用 `Arc<BaseRuntime>`，负责连接、请求身份、队列、输出、取消和回执。具体 backend 管理自己的执行资源。节点配置支持内置实现与[独立可执行程序](executable-backend.md)。热加载、多节点接口及独立能力验证仍待实现。

`AgentSupervisor` 为整个 run 启动、检查并回收唯一的 Agent 与 Instrumentation 宿主。Runtime 先准备启动参数并请求 supervisor 启动或复用宿主，再通过 backend 接口启动目标环境的执行资源。宿主句柄锁在调用环境后端之前释放。环境启动失败时保留仍存活的宿主，允许新的操作重试环境启动。同一 `request_id` 重发仍由请求缓存返回原来的结果。

EOF 时 runtime proxy 关闭队列和 Instrumentation 执行接入并发送取消。`BaseRuntime` 关闭操作受理、回收宿主，再等受理中的操作结束后处理环境。环境执行启动在 Runtime 内受理，已经受理而在关闭中才完成的启动仍进入回收范围。保存和载入在尝试前即设置保留模式，失败后退出也只停止执行资源。载入要求无其他在飞操作。恢复文件的预检查失败保留当前环境，载入或参与者校验失败仍走恢复自身的回滚。关闭等待后端返回，耗时取决于后端能否及时响应取消。

Agent Session 的复制、恢复和切点由 Instrumentation 管理。`FilesystemBackend` 管理文件视图、namespace holder、冻结与恢复、挂载轮换和清理。

本轮验证覆盖宿主复用、环境启动失败后重试、关闭中晚到资源的回收、接入取消和请求去重。Pi 接入及保存恢复使用本地伪模型，包含保存失败重试、继续、多个 backend、文件系统恢复和打开已结束会话。Firecracker 集成沿用此前验证，本轮未重跑。

## 2. 请求身份与环境身份

一次操作请求带两套身份，不能互相冒充：

- **请求身份**：`OperationRequest` 里的 `request_id`、`task_id`。`request_id` 用于幂等；Runtime 普通操作只接收 `environment_id` 与 operation，不接收 Branch 身份。
- **环境身份**：`environment_id` 由 Runtime 创建并返回，control plane 按 Branch 保存独占绑定。Runtime 不接收 Branch id，也不从环境推导 Branch；内部 Backend 使用 Runtime 交付的环境身份组织资源。

`BackendContext` 传递 `task_id`、`environment_id`、`parent_environment_id` 及操作身份；Backend 只使用 runtime proxy 明确传入的局部身份，不自行推断全局结构。Runtime 内部以独立 `backend_id` 标识 Backend instance，并维护 Environment 与 Backend instance 的关联；control plane 不传该 id。

## 3. 生命周期操作

固定生命周期操作由契约统一规定，各 Backend 可覆盖也可沿用默认实现：

| 操作 | 完成条件 |
|---|---|
| `establish`（建立） | 环境已经可用才返回完成；多个 Backend 共同建立时按固定顺序逐个建立，失败时回滚已建立的 Backend 并上报 |
| `operate`（环境内操作） | 返回实际执行的结果；可携带 `agent` 请求启动或复用可信宿主，并启动对应环境的执行资源 |
| `branch`（派生） | 派生出与父环境、与其他 sibling 互不影响的子环境；失败时回滚已派生的 Backend |
| `accept_environment`（接受） | 目标已暂停，来源自己的写者已收敛。回执报告 full、已实际移交的 part 结果 ID 或失败 |
| `suspend`（静止） | 阻止新的环境操作，并说明未完成工作 |
| `resume`（恢复） | 让原环境重新可用，保持已有状态和隔离边界 |
| `terminate`（终止） | 不可恢复地停止 Runtime 管理的执行 |
| `cleanup`（清理） | 回收进程、资源、临时状态、句柄和租约 |

Backend 接口同时提供不带上下文的旧方法和带 `BackendContext` 的新方法；旧方法默认委托到带上下文方法（反向也是），因此既有单 Backend 实现不需要重写。runtime proxy 一律调用带上下文的方法，把请求身份透传下去。

## Full 与 part 接受

`Promote` 请求全量接受。`Salvage` 请求按结果 ID 或受管路径移交部分状态。接受裁决中的结果 ID 引用候选已成功完成的工具动作，由控制面解析文件依赖。也可按 `workspace` 或 `tmp` 挂载范围选择，或传入规范化绝对路径。Policy 可通过 `context.candidate_execution`、`context.candidate_results` 和 `context.candidate_proposal` 判断交付内容。

Pi 先阻止 parent 和 candidate 启动新调用，回报已在执行边界暂停。control plane 随即要求 Runtime 暂停目标环境。暂停确认后，Pi 等待 candidate 自己已经发出的模型调用收敛。Runtime 再关闭移交期间双方的写入准入，等待来源自己的受管写者，并将获准状态三方合并到目标。Full 成功要覆盖全部受管挂载。文件级 part 按结果检查依赖，仅交付 Runtime 回执 `part_results.results` 列出的结果。每个入选结果的全部文件必须可移交，冲突结果的通知不会送入目标 session。共享状态的 Backend 不重复写入。Runtime 完成移交并恢复目标环境后，Pi 仍在执行边界等待，直到回执应用且 control plane 确认才继续。

A → B → C 中，内层验证采用 B 的 Chance、真实结果（或 Policy 明确允许的尚未执行等待点）及执行进展。C 只能向 B 的隔离环境移交，且继承未决 Proposal 必须与 B 一致。Full 保留 C 的 session 和后代，将 C 的后续调用绑定到 B 的环境，并由 C 承接 B 的外层 Chance。Part 保留 B 的 session 和路由，向它交付实际接受的结果，随后抛弃 C 和后代。A 的权威 session 和环境不受内层接受影响。外层拒绝丢弃仍依赖 B 的延续，外层接受则再次验证并移交到 A。原来源环境保留到使用它的延续和后代释放为止。

原型对已经开始不同调用的 parent 保守拒绝接受。Firecracker 和独立可执行 Backend 尚无隔离状态合并实现，多个独立隔离 Backend 也无联合提交。本实现只覆盖受管调用和挂载。内存、外部服务以及未受管写入不在移交范围内。文件系统重放可报告回滚失败，回执丢失和进程崩溃仍可能留下待清理或待核查状态。主动保存不跨未确认的移交。长期写者可能超过现有 Runtime 请求超时。

## 4. 顺序与中断

- 每个环境一条执行队列，按到达顺序串行执行；`suspend` / `terminate` 插到队首并打断在飞操作。
- 在飞操作通过 `CancelHandle` 协作式中断：每次调用 Backend 前检查，已取消就返回 `interrupted`，并把已完成、未执行的 Backend 和已产生的输出如实带上。
- 多 Backend 的**执行顺序是固定的**（Policy 给出有序列表），不会因为“存在多个连接”就广播。清理按相反顺序执行，先释放依赖其他 Backend 的外层资源。

## 5. 超时与重试

- control plane 侧 `call_with_context` 等待结果有 30 秒超时；超时返回错误，不假装已完成。
- 同一 `request_id` 重发按重试处理：runtime proxy 用 `seen` 集合去重，不重复执行。
- 本文不规定 Backend 内部的超时；是否支持超时、如何收敛由具体 Backend 实现承担，未完成就不能返回完成。

## 6. 结果与事件

- 每次操作返回一个 `OperationResult`，含 `status`、`environment_state`、`backend_states`（每个参与 Backend 的状态快照）、`outputs`（全部可见输出）、`output`（旧单值主输出）和 `reason`。
- **多个输出必须全部保留在 `outputs` 里**，不会因为 `output` 只有一个字段而静默丢弃其他 Backend 的输出。`output` 只有两种情况下才有定义：恰好只有一个 Backend 输出，或 Policy 用 `output_backend` 明确选择了主输出。否则 `output` 为空，`status` 降为 `PartiallyCompleted`，提示配置显式主输出。
- 状态变化通过事件上报：环境状态意外变化时发 `EnvironmentStateChanged`，区分“意外终止”和一般状态变化。

## 7. 多 Backend 组合

### 7.1 选择来自 Policy，不来自 Runtime

Backend 选择完全由 `BackendPolicy`（配置注入）决定：

- `default`：没有规则匹配时的有序 Backend 列表。
- `routes`：按 `operation` / `task_id` / `environment_id` 匹配的规则；未填字段表示“任意”。多条规则同时匹配时，约束字段最多的规则胜出；同等具体度出现歧义就返回明确错误，不允许靠排列顺序静默覆盖。

Runtime 只按给定规则选择、按明确顺序执行、如实汇报。它不会因为“恰好有多个连接”就广播；多 Backend 配置必须显式给出 `default` 或至少一条规则，否则装配失败。

### 7.2 执行顺序与部分失败

逐个 Backend 按固定顺序执行。任一 Backend 失败时，错误里同时携带：

- `completed`：已经完成的有序 Backend 标识；
- `remaining`：因此没有执行到的 Backend 标识；
- `cleanup_failures`：回滚或清理仍未完成的 Backend 标识；
- `outputs`：失败前已经产生、不能丢弃的输出。

这些信息通过 `OperationResult` 的 `resources` / `remaining` 和 `outputs` 如实上报，control plane 据此区分“已完成 / 未执行 / 需要清理”，不会静默覆盖多个输出。

### 7.3 Environment 与 Backend instance 的关联

control plane 独占绑定 `Branch -> Environment`，因此不同 Branch 不能共享 Environment。Runtime 对外只接受 Environment identity；内部每个 `BackendConnection.id` 标识一个独立 Backend instance，且每个 Environment 记录实际参与建立或派生的 Backend instance 集合。不同 Environment 可以选择不同组合，例如 `env1` 使用 `fs1/process1`，`env2` 使用 `fs1/process2/network1`；这种共享只表示 Backend 内部资源可以共享，不表示 Environment 生命周期共享。

内置和外部 backend 走同一条登记和参与路径。建立或派生成功后，操作只能使用 Policy 选出的、且属于该 Environment 参与集合的 Backend；缺少参与记录、选出未绑定 instance 或清理未参与 instance 都必须失败。Runtime 保存并恢复有序参与者记录，并由 Runtime 校验 Backend instance 与 Environment 的实际组合；control plane 恢复只验证每个存活 Environment 至少存在于一个保存组件，不要求所有 Backend 都含有它。

### 7.4 状态聚合

多 Backend 共同提供同一个环境时，聚合状态以参与过该环境 `establish` / `branch` 的 Backend 集合为准：

- 参与者全部报告了状态且一致 → 返回该状态；
- 任一参与者没有报告状态，或状态不一致（例如一个 `Available`、一个 `Suspended`，或建立到一半失败）→ 返回 `PartiallyAvailable`。
- 没有参与者记录（未知环境或不完整恢复）→ 返回未知，不观察或操作未绑定的 Backend。

`PartiallyAvailable` 只表示聚合层无法把环境归入单一生命周期状态，它不是一个可以继续执行普通操作的状态；`can_accept_operations` 会为假，环境不会在部分失败后伪装成 `Available`。当一次成功返回的操作后环境实际为 `PartiallyAvailable` 时，结果 `status` 报告为 `PartiallyCompleted` 并给出 `reason`，而不是 `Completed`。

### 7.5 清理责任

- 建立 / 派生失败时，组合层回滚已经完成的 Backend（按相反顺序清理），并记录哪些回滚也失败了。
- `cleanup` 会尽力覆盖所有选中的 Backend，不因单个失败就跳过后续；失败以 `cleanup_failures` 如实上报，`remaining` 保持为空（因为确实都尝试了）。
- 单个 Backend 的清理责任由它自己承担；组合层只负责协调顺序和汇总失败。

## 相关文档

- [runtime proxy 契约设计](../design/runtime-adapter-contract.md)：正式接入契约与验证要求
- [runtime proxy 接入指南](runtime-adapter.md)：runtime proxy 实现与接入方法
- [Runtime 语义整理](default-runtime-semantics.md)：Runtime 执行承载的方向性语义
- [Branch 语义设计](../design/branch-semantics.md)：Branch 结构、生命周期与 resolution
- [Runtime 设计](../design/runtime.md)：Runtime 的动作承载、状态边界与可插拔能力

执行地址随创建或恢复后的环境启动结果交付。Instrumentation 按其 Branch 映射选择 Environment。执行代理按 Environment 与 operation 关联终态和单操作取消，Environment drain 的 generation 和请求去重窗口也各自独立。

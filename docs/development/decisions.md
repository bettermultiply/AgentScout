# 开发决策

本文记录已接受的实现架构与验证决策。目标语义归 `docs/design/` 所有。

## 2026-11-07：首版通用文件系统 Backend

文件系统隔离实现为独立 Backend，仅拥有 Overlay upper/work 生命周期和 Agent
启动时的文件系统视图；不拥有网络、内存、资源或 Agent Session。BaseRuntime
在该 Backend 提供的 bubblewrap 命令中启动 Agent，mount/user namespace 由
启动入口继承到全部后代进程。Linux namespace、bubblewrap 或 OverlayFS 不可用
时必须拒绝启动，不得回退成普通 cwd 启动。Branch 使用父 upper 的只读 lower
视图，promote 为环境侧 no-op，discard/cleanup 删除 child upper/work；首版不
自动合并文件变更，也不把文件写入自动提升为权威效果。

启用 `filesystem` 配置时，即使没有填写 `base_runtime`，装配器也自动创建
`BaseRuntime`，避免 Backend 建立成功但 Agent 启动绕过隔离。BaseRuntime 负责
Agent 进程组和停止，文件系统 Backend 只负责 namespace/Overlay 视图；进程
主动创建新 session 后不在进程组停止保证内。

cleanup 的固定顺序是先停止 BaseRuntime 管理的 Agent 进程组，再清理 inner
和 filesystem；停止失败会保留 AgentProcess 句柄，禁止删除 upper/work。即使
inner terminate 失败，terminate 仍先尝试停止 Agent，避免其继续污染 Overlay，
并合并报告所有失败。BaseRuntime 的无 context 方法统一委托对应 context 方法，
防止绕过 filesystem。

组合 Runtime 在 establish/branch 的 inner Backend 中途失败时，不能直接调用
filesystem cleanup（其合法前置状态是 Terminated）。现在回滚顺序固定为
filesystem terminate -> filesystem cleanup；原始错误的 completed/remaining 保留，
terminate 或 cleanup 失败追加到 cleanup_failures。若回滚仍失败，环境 metadata
保留以便后续收敛，不伪造已清理。

BaseRuntime 组合 filesystem 后，generic operate、tool_context 和 invoke_tool
仍由 inner Backend 执行。filesystem 不再执行 generic operate，也不返回自己的
命令输出；它只在这些调用前检查环境是否为 Available，并在 Agent 启动时提供
可由后代进程继承的 bubblewrap namespace/Overlay 视图。这样 NLE/Firecracker
等已有业务 Backend 不会被静默绕过，且同一命令不会因组合而执行两次。

本轮范围审查确认 Adapter 的生命周期门槛是 Runtime Adapter 契约的通用语义，
不是为 filesystem 临时增加的限制：operate/promote/suspend 需要 Available，
branch 允许 Available 或 Suspended（暂停只阻止父环境内的新操作，派生读取
父状态仍是合法操作），resume 需要 Suspended，terminate 允许 Available 或
Suspended，cleanup 需要 Terminated。保留这些 Adapter 门槛不会改变未启用
filesystem 的 NLE/Firecracker 行为；filesystem 专属的状态检查仍只在
FilesystemBackend 和配置了 filesystem 的 BaseRuntime 中执行。

## 2026-11-06：通用 Agent 事实协议

废弃 `ToolPending`、`ParentOutcome`、`CandidateCompleted` 和含义混合的 `CleanupCompleted` Agent 消息，以及 `Hello` Agent 握手命名。分别使用 `ActionStarted`（等待边界由 result_pending 表达）、`ActionFinished`（带 action kind 与终态）、`AgentSessionReleased` / `BranchReleased`、`AgentConnected`；Runtime 握手不受此命名变更影响。Transcript 不再由 Adapter 声明 Authority/Candidate 角色。线协议与 Pi 接入同步切换，不保留旧消息兼容分支。

`CommandResult` 扩充 Accepted/Started，只有终态消费 pending command；Applied 仍只表示 Agent Adapter 已落实。Runtime Adapter 发出操作开始/完成事件，保留已有 OperationResult 请求配对；控制面记录实际 Runtime 事件。环境建立失败由 Runtime 结果驱动 Candidate 丢弃，不再构造 Agent BranchFailed。目标事实类型的完整定义见 `docs/design/agent-integration.md`；原型接入覆盖范围见 `reference-workload.md`。

普通 `ActionStarted` 只登记 action 事实。只有接入侧本地 Decision 明确产生 `SpeculativeAdmissionRequest` 时，控制面才会登记投机操作并调度 Policy 预测。Pi 工具 Hook 已执行下发 artifact 中的 `admit`，普通工具不自动准入，Policy 允许时才产生投机准入请求；validator/evidence-only 路径与完整全局 rebalance 仍未实现，代码必须保留准确的未启动状态。

本原型现已把 Proposal 独立保存在 `SpeculativeTree.proposals` 中；Branch 只保存 `proposal_id` 和 `assumption_set`。当前实现为每个候选建立一个直接 Proposal，AssumptionSet 仍按 Proposal ID 表达，但尚未实现任意多 Proposal 依赖的传播、确认和重挂载。控制面下发编译后的本地 Decision artifact，Pi 在真实工具 Hook 中求值；`extent` 由接入侧按 Branch 隔离的 Hook 次数求值，Hook 次数不是 `steps_completed` 的完成步骤数。`launch=false` 会保留 Proposal 并记录 validator/evidence-only unsupported，不启动 Authority Invoke。控制面 `BranchPhase` 只记录控制面实际推进的阶段：显式准入请求进入 `select`，随后按实际路径更新 `predict`、`launch`、`effect`、`match`、`dispose`；本地 `admit` 不单独占用控制面阶段。完整全局 `rebalance` 仍未实现；本地 extent stop 已可在 Hook 边界暂停，并支持取消、discard、cleanup 和合法恢复。

## 2026-11-04：Runtime 整体边界与首阶段范围

已收敛：AScout 自身支持一套完整 Runtime，承载 Agent 一步动作，使其正常、正确运行，并把对真实世界或 Authority 的影响限制在容忍范围内。Runtime Adapter 与 Effect Coordinator 是这个整体中的接入和效果管理职责，不是平级的独立运行系统。Agent Session 内部管理仍归接入侧，Runtime 负责动作承载和外部效果边界；状态可以复制、共享、写时复制、重建或恢复，优先低成本方式。

能力可插拔、组合或缺失，但必要保证不能缺失。第一阶段先实现覆盖 Agent 及其派生进程文件操作的 Overlay 文件系统，不要求 VM；Overlay 本身不是 Runtime 的固定必需项。后续才展开 Runtime 与 Policy 的协调、协商和运行时检测。该决定最初只更新文档；文件系统 Backend 其后已实现 overlay/btrfs 数据面、逐挂载 overlay、live mount accept 与进程 Backend，当前覆盖范围和偏差见 [文件系统 Backend 当前实现状态](../fs-backend/current-state.md)。

**废弃记录**：不再采用“Runtime 仅是 AScout 不拥有的外部后端”“Runtime Adapter 是独立一级侧面”“外部效果执行只属于控制面而不属于 Runtime”的架构定义。此前四侧路径中的 Adapter 现在作为 Runtime 的接入边界；效果协调归入 Runtime 整体责任。这里改变的是职责定义，不预定 crate、进程、transport 或部署方式。既有 Branch、状态可见性、commit/discard 和 single-winner 规则保持不变。

目标说明见 `docs/design/runtime.md`；首阶段与未实现能力见 `docs/development/runtime-adapter.md`。本节取代本文其他历史记录中关于上述架构边界的表述。

## 2026-11-02：Candidate 最终失败不再等同于立即丢弃

问题：Candidate 已进入 `Promoting` 时，模型返回 429，Pi 立即报告 `branch_failed`；控制面把它当作普通 Candidate 失败并丢弃，而 Pi 已先切换 authority，造成两侧权威分叉。

现在的行为：

- Promote 仍是一步决策；Candidate 的模型续跑使用 `AgentSession.continue()`，保留 Pi 的自动重试、压缩和 settle 流程。
- Pi 只在重试耗尽等最终失败后报告 `branch_failed`，并记录失败/取消状态；此后的迟到 Promote 被拒绝，不切换 authority。
- 控制面在 `Running` 收到失败时丢弃 Candidate；在 `Promoting` 时暂存失败，等 Promote 结果；Promote 已应用后失败属于权威失败，显式结束 run，而不是当 Candidate 丢弃。
- Runtime 增加固定生命周期操作 `promote_environment`，在 Agent Promote 前调用；共享环境实现可以 no-op。Runtime promote 失败则丢弃 Candidate，不切换 Agent authority。

验证：新增 Rust 状态机回归测试和 TypeScript `AgentSession.continue()`／PiAdapter 晚到 Promote 测试；`cargo check`、`cargo test --lib --bins`、`npm run check` 通过。未运行真实 NLE。

## 2026-11-02：Pi 模型参数直接传入接入入口

AScout 启动的是 Pi 的 `agent-access` 接入入口，不是 Pi 交互式 CLI。此前该入口没有解析模型参数，只能回退到 Pi 配置中的默认模型；用户配置里的默认模型过期时，会错误地落到不可用模型。

现在 `agent-access` 接受 `--model <provider>/<model>` 和 `--thinking <level>`，通过 Pi 的 `ModelRuntime` 与 `resolveCliModel` 解析后传给 `createAgentSession`。控制面把这两个参数原样转发给接入进程；Authority 使用显式模型，Candidate 继续继承 Parent 的模型与 thinking level。认证仍使用 Pi 默认配置，不再需要复制独立的 `PI_CODING_AGENT_DIR`。

同时把 NLE 演示依赖改为可复现的相对布局：`requirements-nle.txt` 固定 `nle==1.3.0` 与 `gymnasium==1.2.0`，`nle-runtime.json` 使用 `../.venv-nle/bin/python` 和 `examples/nle-runtime-bridge.py`。Runtime 配置由 Policy 的 `runtime` 字段选择，控制面只在需要覆盖时读取 `ASCOUT_RUNTIME_CONFIG`。

验证：`npm run check`、`cargo build`、`cargo test --lib --bins` 通过；使用 `--model deepseek/deepseek-flash --thinking low` 和默认 Pi 认证运行 NLE，12 次动作中 11 次 Promote、1 次 Discard、0 次 `branch_failed`，模型调用均为 `deepseek-flash`。

## 2026-11-02：NLE 游戏内容留在 Runtime

- `nle_action` 的 `result`（位置、生命、消息、地图摘要）和 `evidence` 的构造移到 `examples/nle-runtime-bridge.py`；`NleBackend` 只转发，不解析 `blstats`、`chars`、`message`。
- 控制面只解析通用的 `{result, evidence}` 外层；Pi 只把 `result` 文本交给 Agent；只有 Policy 读取结构化 evidence 做游戏语义判断。
- 完整 NetHack 3.6 Guidebook 来自 NLE 1.3.0 源码包，放在 `examples/nle/Guidebook.txt`；Runtime 通过 `nethack_guide` 工具按章节提供，不再把手工摘要当作完整规则。

## 手动试用后的展示修正

历史方案：曾讨论以独立 PTY 与终端模拟器嵌入 Pi 原生渲染，但该方案已被放弃，不是当前目标。当前只通过 Pi 公开 SDK 获取 AgentSession 的消息、思考、工具调用、结果和事件，由 AScout UI 自行排版；不修改 Pi 源码、不调用 Pi 内部展示组件，也不捕获 Pi 原生终端画面。

当前限制：不接管 Pi 完整 InteractiveMode、原生编辑器或命令菜单；输入使用 AScout 的 Prompt/Abort 路径。图片和其他尚未映射的消息内容会明确标记为未展示，不伪造为普通文本。

验证：零模型调用的真实 PTY 检查覆盖原生 Markdown、中文、工具结果、分支内容分隔、窗口 resize、socket 断开退出；控制面检查覆盖 Tree、非法编号、Branch 查看、Tab 运行事实、Esc 返回及 Ctrl-D。Pi 完整 `PATH=/usr/bin:$PATH npm run check` 通过。Promote 后连续流式展示未做真实模型验收。清理确认后释放 PTY 文件描述符，只保留模拟器的最后画面；未变化画面不重复刷屏。

- Runtime Adapter 部署形式暂不变更；先修正 `/ascout` 无效编号、Branch 上下文和工具展示。
- 展示代码放在 `src/bin/ascout-control-plane/view.rs`，只格式化已有事实，不改变 Branch 生命周期或输入路由。
- 分支详情使用结构化文本，分别显示运行事实、创建时继承的展示上下文、分支自身输出；候选创建后不再跟随 Parent 的后续输出。
- Pi 接入侧不再把工具事件拼接成 Output 文本；控制面根据已有 Transcript 显示工具调用和返回。Output 只承载实时模型正文和明确错误，Ledger 的原始事实记录保持不变。
- NLE 任务知识放在外部示例 `examples/nle-task.md`，由用户提供给 Agent；动作表基于实际 `NetHackScore-v0` 环境核对，不把游戏规则写入核心。
- 当前只做基础配色与内容层次，完整 Pi 风格 Markdown、工具折叠、多行输入及宽字符排版仍未完成。

## 2026-11-02：Runtime 启动 Agent 接入侧，运行目录与 Ledger 归位

已确认三项归属，并按此修改了控制面：

- **Agent 接入进程由 Runtime 启动**。控制面（用户）决定用哪个 Agent 产品、哪个版本，把命令、参数、工作目录和日志路径作为 `operate_environment` 的启动参数交给 Adapter；Adapter 在环境内启动它，并在自己的进程退出时回收。控制面不再 `Command::new("node")`，也不再持有 Agent 进程句柄。
- **运行目录由调用方创建**。控制面原本自建临时目录，既放日志又当工作目录；这属于实验脚手架。现在第一个命令行参数就是调用方创建好的运行目录，控制面只使用它，不再删除或回收。
- **事件记录归 Ledger**。新增 `src/control_plane/ledger.rs`，控制面通过 `Ledger::open` / `record` 追加事实，不再自己 `writeln!` 到文件。Ledger 只记录，不参与决策。

附带约束：当时 `ASCOUT_RUNTIME_CONFIG` 是必需的，因为环境和 Agent 都由 Runtime 建立；后来 Policy 的可选 `runtime` 字段可以提供同一路径，环境变量只用于覆盖。

Agent 接入侧仍然是唯一控制 Agent 会话的地方：创建会话、克隆上下文、发送 prompt、中断、Promote/Discard、清理都在接入侧执行，控制面只发命令并接收事实。

## 2026-11-02：控制面不再持有环境状态，结果判定交给 Policy

已确认三项职责归属，并按此修改了 NLE 测试路径：

- **控制面只发命令，不保存环境状态**。原先控制面里有一个包装结构，自己记着“哪个环境是权威”、每个环境的坐标和最后观测，并用坐标差算出 `moved`/`blocked`。这些职责归 Runtime Adapter：控制面按 Branch 名字发操作，Adapter 负责对应到环境；需要观测时向 Runtime 要一次 `observe`。控制面现在只持有 Adapter 进程句柄和协议会话，没有专门的包装名词。
- **观测缓存只有一份，在环境自己那里**。`NleBackend` 原先自己存一份最后观测，绕过了 bridge 已经实现的 `observe`；控制面又存了第二份。两处都删了，`observe` 直接转发给 bridge。
- **真实结果的判定属于 Policy**。Runtime 只交回动作前后的两份公开观测；`predict` 从观测产生预测，`match_proposal` 从 `blstats` 读出实际是 `moved` 还是 `blocked` 再与预测比较。控制面不再出现坐标、格子或移动这类词。

工具通道现在由 Runtime 生成 `result` 文本（可读摘要）和 `evidence` 结构化事实；控制面只转发，不解释游戏字段。早期 `{"is_error", "observation"}` 的结果形态不再使用。

仍未落实：Agent 产品的选择由控制面（用户）决定，但启动 Agent 进程属于 Runtime Adapter，路径和版本作为参数由控制面传入。Pi 接入的启动已按 2026-11-02 决策移入 Runtime Adapter（`src/runtime/adapter/agent.rs`），控制面只传入启动路径与版本。

## 2026-11-01：一次 run 只提供一个工作目录，不再按 Branch 复制目录

已确认：控制面不再为每个 Branch 创建和复制工作目录，一次 run 只有一个工作目录，所有 Branch 共用。Branch 是同一执行状态的别名，不各自持有目录。

原因：Pi 把会话工作目录的绝对路径写进系统提示，而系统提示位于对话最前面。每个 Branch 一个目录时，该路径每开一个候选就变化一次，导致整段对话的前缀缓存在每次新建候选时全部失效。48 个动作的实测中 87% 的花费来自这类整段重读。

影响与边界：

- Candidate 在当前实现中不执行工具（工具调用暂存到 Promote 才放行），共享目录不扩大它的实际能力。
- 目录级隔离属于 Runtime 职责，不能靠命名不同的 Branch 目录冒充。需要并行执行工具的候选时，由 Runtime 提供隔离。
- 已废弃：按 Branch 复制工作目录（`copy_workspace`）、清理时删除 Branch 目录、`BranchRecord.workspace` 字段。控制面在收到 `CleanupCompleted` 时只更新 Branch 状态。

## 2026-11-01：Branch 非干扰、环境形态与产品接入解耦

已确认以下边界：

- **Branch 的固定要求是非干扰**：一个 Branch 不得影响权威路径，也不得影响其他 Branch。独立执行环境只是实现非干扰的一种方式，不是 Branch 的定义，也不是所有 Runtime 的前提。
- **`branch_environment` 只表示派生独立子环境**。不能派生独立环境的后端不得用共享别名伪装成功；该操作是可选能力，不是所有合格 Adapter 都必须具备的固定操作。
- **共享环境 + 作用前阻止**是另一类非干扰实现。NLE 采用这种方式，但它属于 NLE 专用 Runtime／测试 workload 的描述，不属于 AScout 核心契约。AScout 只要求 Adapter 声明并兑现所选非干扰机制的范围与限制。
- **NLE 只是测试用例，不属于 AScout 实现**。NLE 的 bridge、策略和运行环境作为外部 workload／适配保留，不进入核心 crate 的能力边界；核心契约不得出现 NLE 专用语义。
- **AScout 不与 Pi 耦合**。Pi 是首个接入实现，不是架构组成；设计文档不得把 Pi 写成 AScout 的前提，控制面也不应把 Pi 的路径、工具或会话结构固化进核心协议。

当前实现差距：`src/runtime/backends/nle.rs` 仍在核心 crate 中。控制面已不再对 NLE 调用 `branch_environment`，也不再识别 `nle_action`；它按 Runtime 声明的共享环境和环境工具调用 `tool_context`／`invoke_tool`。Pi 接入的启动已按 2026-11-02 决策移入 Runtime Adapter。

## 2026-10-31：NLE 投机改为动作边界暂存，废弃进程级 fork 分支

已确认环境不可修改：不为适配 AScout 在 NLE 或任何外部 workload 侧增加克隆、分支或回放能力。先前在 `examples/nle-runtime-bridge.py` 中用 `os.fork()` 复制环境进程、用 socketpair 建私有通道、失败后反向移动回退的方案废弃。废弃原因：

- fork 让 workload 脚本承担了分支语义，而隔离策略本应属于 Runtime；
- 它只能从根环境分支，从已 Promote 的候选再分支会得到没有游戏对象的空壳；
- 子进程异常退出时会掉回父进程主循环，产生重复响应；
- 单世界下反向移动仍消耗回合，怪物会行动，“可逆”只是位置可逆。

CRIU 一类进程快照同样不采用：需要特权、镜像 I/O 和 FD 重绑，每步一条分支不划算。

现行实现：环境单实例；`branch` 只是同一世界的别名，控制面不再使用它；Candidate 只在预测结果下推演一步，工具调用暂存到 Promote；动作前固定延迟（`action_delay_seconds`，当前配置 1 秒）；预测由固定程序从公开观测枚举，模型不负责预测。旧内容不再出现在正式文档。这里的 `branch` 是 NLE 专用测试适配的逻辑视图，不是 AScout 的 `branch_environment`；边界见 2026-11-01 条。

## 2026-10-25：随机双 Candidate 与 Parent 不等待裁决

用户确认 Workload 与 Policy 分离：Parent 真值随机取 example/true/false，Policy 用非固定随机抽取两个不同预测；候选执行 Predicted 命令，Parent 默认继续 spec fail。Parent 结果交付不等待比较，只有 Promote 才取消旧 Parent。等待点按 Parent 和 operation 归属，不因一条消息有多个工具而整组拒绝；其他工具结果需真实配对，不允许重复其效果。Promote 后取消旧 Parent 下其余候选，保持 single-winner。工具准入不写入 Pi；由 Policy、AScout 和 Runtime 的能力边界共同决定。

废弃先前 afterToolCall 等待 ResolvePending、单候选容量、固定 example 预测及以单工具数量作为准入条件的实现。等待点准入和结果匹配在示例 Policy；Pi 不按工具名或命令格式筛选 Candidate 工具。当前接入把布尔预测和字符串枚举预测转换为 Agent 工具所需的文本表示，复杂结果转换仍有实现差距，见 reference-workload.md。非标准上下文仅记录，暂缓处理。

随机场景位于独立脚本模型接口，不是控制面业务逻辑，也不是大模型成功证据。已有脚本命中实测确认两个候选先执行、Parent 真值返回、单一 Promote 和 loser/Parent 清理；既有在飞输出验证与本次随机场景验证应分开描述。

## 2026-10-25：TUI 与实际投机过程观察优先

先实现稳定的 AScoutAgent TUI，由 AScout 接收 Agent 输出和运行事件并呈现，输入与中断经对应 Agent 接入实现传递；产品差异留在接入实现，不依赖 Pi 原有主界面。增加当前 AScoutAgent 整棵 Tree 的只读观察，可选择具体 Branch 深入查看。先使用简单环境，不把接通完整 Runtime Adapter 环境管理作为前置条件；不据此声称已实现完整环境隔离。实际启动、执行、预测比较、Policy 处置与清理必须有事实依据，不能用两个独立模型请求或发送命令代替完成证据。当前已完成自有 TUI 与受限单工具真实等待点续跑，具体可用范围以 reference-workload.md 为准。

用户已批准 `/ascout` 为 AScout TUI 内的整棵 Tree 观察操作，选择 Branch 只改变观察对象，不切换主对话或触发生命周期操作；命令由 AScout 处理，不作为提示词发送给 Agent。该名称与含义记录于术语表。

废弃助手提出的“复用 Pi 主界面”的实现方向；目标是整合原 Agent 的体验，而非把 AScout 用户端绑定到 Pi TUI。首版仍接入 Pi，不同时实现其他 Agent 的适配。

首步实现：现有 `ascout-control-plane` 开发入口改为 AScout 自有终端界面，保持 socket 接入、控制面 Tree 和连续 Pi AgentSession，加入流式输出与只读 Tree/Branch 观察。废弃该入口原先在 Branch 启动时直接发起无共同等待点的候选、把模型最终文本作为 ParentOutcome 的演示路径；接入侧不再产生该占位结果。退出保留运行目录和诊断文件，不声称已确认所有工具后代及环境清理。

本轮连接真实投机：ToolPending 报告后让真实工具继续一次，复用既定 ParentOutcome 名称表示真实工具结果，不另引入 ToolOutcome。Candidate 使用独立目录与独立内存 SessionManager，保存共同上下文与预测 toolResult 后 continue，不以新用户提示词伪造续跑。控制面求值 admit/select/predict/launch/match/dispose，在合法 Promote 后发布缓冲输出并保持活跃 Candidate，不命中回退 Parent。未验证 Candidate 的工具调用由 AScout 的 Branch 处置控制：Pi 只等待控制命令，不按工具名或命令格式判断安全性；Runtime 能力与 Policy 决定仍需由控制面接入。默认示例不自动开启投机，新增可显式选择的 pi-tool.yaml。实测使用本地脚本模型接口加真实 Pi/bash，不能作为真实大模型效果证据。

废弃 D-031-I 中“用户不可直接观察候选”的限制；保留主对话只属于权威路径、候选不建立用户对话、用户不直接裁决 Branch 的边界。只读观察明确标记候选内容未验证，不向其他 Branch 反馈这些内容。

## D-026-I：抽象札记不重复维护已收敛语义

2026-09-03 决定：`docs/development/spec-exec-abstraction.md` 的原 §1（Edge 生命周期与核心实体）和原 §3（Assumption DAG 组合语义）不再保留第二份规范定义。两节改为指向 `docs/design-model.md` 与 `docs/design/policy-decisions.md` 的下沉说明；该开发札记仅保留论文调研、候选方案和实现分析。同步更新 glossary 中受影响的术语引用，避免把开发札记当作语义来源。

## D-027-I：效果管理规范只保留单一来源

2026-09-03 决定：`spec-exec-abstraction.md` §4 保留论文对比和研究动机；EffectClass、StateDomain、效果处理模式、效果边界阻塞及审计等规范性内容以 `docs/design/effect-management.md` 和 `docs/design/policy-decisions.md` 为唯一来源，不在开发札记重复维护。同步更新 glossary 的 EffectClass/StateDomain 引用。

## D-028-I：嵌套投机章节只保留调研与假设

2026-09-03 决定：`spec-exec-abstraction.md` §9 不再重复定义 Edge、Branch、AssumptionSet 或父子生命周期语义；正式规则以 `docs/design-model.md`、`docs/design/policy-decisions.md` 和 `docs/design/branch-semantics.md` 为唯一来源。§9 保留论文覆盖情况、成本/收益分析、实验假设和候选实现方向；未经确认的概率公式、深度限制和接口形态不得写成通用机制约束。

## D-029-I：废弃单 Edge 即时启动与固定 frontier 策略

2026-09-04 决定：废弃由 `admit` 精判直接决定投机是否立即启动的七槽决策面，现由 `admit` 决定机会是否进入待选集合，独立的 `select` 在同一 task / AScoutAgent 的全部待选 Edge 间选择当前启动项。也废弃将 best-first、固定的 `P(存活) × 期望延迟节省 / 资源成本` 排序、浅层分叉/深层单链或强制 `survival` 字段写成 AScout 机制规则的表述；这些机制作为 Policy 可选策略继续受支持。当前语义见 `docs/design/policy-decisions.md`、`docs/design/control-plane.md` 与 `docs/design/decisions.md` D-043。

## D-030-I：废弃 Scheduler 固定暂停与抢占策略

2026-09-04 决定：废弃“所有投机工作固定低优先级”以及 Scheduler 根据内置价值排序自行暂停或抢占投机工作的表述。Scheduler 继续拥有运行队列、资源和硬约束，但暂停、恢复和价值驱动的抢占由独立 `rebalance` Policy Decision 选择；抢占后的处置由 `dispose` 决定。当前语义见 `docs/design/policy-decisions.md` §2.9 与 `docs/design/decisions.md` D-044。

## D-033-I：Speculation Coordinator 吸收 Scheduler 的控制面职责

2026-09-04 决定：在控制面组件模型中，不再将 Scheduler 作为独立组件。其待选机会、运行调整、预算和硬约束执行职责并入 Speculation Coordinator；`select` 与 `rebalance` 仍由 Policy Decision 决定，Speculation Coordinator 在统一的全局投机视图上负责激活、复核和执行这些意图。D-030-I 中关于 Scheduler 的描述保留为历史决策记录，当前组件结构以本条和 `docs/design/control-plane.md` 为准。

## D-031-I：废弃端用户直接参与投机 Branch 裁决

2026-09-04 决定：废弃通过专门投机 UI 让端用户直接观察、选择或裁决内部 Branch 的表述。端用户只通过 AScoutAgent 视图操作当前有效的权威执行主线，并只能看到该路径的内容、状态与交互；人作为 Authority 提供输入或中断时，也通过该主线进入系统。Branch 接受、丢弃、重新挂载与清理在内部完成，不要求用户选择或重绑定 Branch。当前语义见 `docs/design/agent-integration.md`、`docs/design/branch-semantics.md` 与 `docs/design/decisions.md` D-045。

## D-032-I：前提变更暂不兼容或修复 Branch

2026-09-04 决定：废弃在用户操作改变 Proposal 前提后继续保留、挂起、兼容判断、修复或重新对齐受影响 Branch 的候选方向。现阶段所有依赖已变更前提的 Branch 及其 subtree 一律剪枝；未依赖该前提的 Branch 不受无关输入影响。当前语义见 `docs/design/branch-semantics.md` §6、`docs/design/policy-decisions.md` §4 与 `docs/design/decisions.md` D-046。

## D-034-I：抽象札记收敛为论文证据

2026-09-14 决定：`spec-exec-abstraction.md` 只保留论文覆盖、实验结果、负结果、效果处理现状、shadow/replay 方法学、接入与隔离事实、嵌套投机事实及待验证问题。已经在 `docs/design/` 正式呈现的生命周期、Policy Decision、Effect、架构、组件职责、契约和指标方案从该文删除；该文不再作为目标语义或实现结构的来源。

## D-015-I：首个真实 Workload 使用 Delayed-Choice Coding

首个真实端到端 workload 通过 Product Integration 的观察和拦截接缝接入 Agent，使用真实仓库。不要求 Agent 主动调用 AScout，也不开放显式生命周期协作。具体产品和接入方式尚未选择。Sequential 与 speculative run 使用相同 Agent、任务、outcome 和预算。

**废弃记录（2026-09-17）**：原先使用 `ascout codex`、Command + Skill，由 Agent 主动调用命令并先验证显式路径的路线错误，已废弃。确定性测试 Agent 专用于 conformance tests 的初期测试安排也不再采用，初期测试范围见 D-035-I。

Workload contract 与指标见 [reference-workload.md](reference-workload.md)。

## D-035-I：控制面最小闭环与初期实现约束

2026-09-17 确定：先实现控制面最小闭环。保持代码简洁、干净，避免为尚未需要的扩展增加抽象和占位实现；能以简短清晰代码解决的问题不引入冗长框架。

2026-10-23 更新测试约束：原型阶段默认不编写测试。只有实际遇到可复现、且今后很可能再次出现的问题时，才针对该问题编写测试；先让测试稳定复现问题，再修复代码直到测试通过。已有测试不因此自动删除，真实执行验证也不等同于必须新增自动化测试。

**废弃记录（2026-10-23）**：不再采用“设计初期先定少量端到端测试、须经用户同意并在业务逻辑实现前定稿”的普遍门槛。早先等待 test 通过／失败的两个 Proposal、确定性 FollowUp 及两个控制面场景的首批测试安排不再作为实现前提。

早期 `tests/test_outcome.rs` 及其确定性替代实现已经删除，不再作为当前正确性证据。当前 Runtime Adapter 端到端测试直接启动真实 Firecracker，验证 microVM 建立、guest 内操作、物理暂停与恢复、终止与清理、请求重试去重、在飞操作打断、意外停止事件，以及 snapshot/restore 分支和 sibling 隔离；SSH 诊断同样使用真实 virtio-net 与 guest sshd。当前已实现 task 身份校验、Proposal 独立存储和待选集合；仍未实现全局唯一 Proposal identity、九类 Decision 的完整接通、fallback、嵌套树、效果处理和主动环境验证。当前 Rust 接口仍是实验接口，不是稳定对外契约。

Adapter Registry 的主动环境验证延后实现，初期先检查接入接口与契约的对应关系。这是原型限制，不改变目标契约：接口检查不构成隔离保证的实际验证，相关能力不得标记为已验证。尚未实现的差距是 Registry 自主建立环境、执行验证场景并形成验证证据。

## D-047-I：重做由控制面拥有 Agent 状态的 Pi 最小接入

2026-10-24 确定：此前位于 Pi `packages/coding-agent/src/core/ascout.ts` 的 `SpeculativeTree`、`BranchRecord`、`ControlState` 和根目录一次性 `ascout.ts` 驱动不符合责任边界，统一作为废弃实验材料，不在其上继续扩展。它们把 AScout 的 Tree、Branch 生命周期、Policy 落实和用户入口编排放进 Agent 接入侧，不能作为正式系统或端到端完成度证据。

后续实现由 AScout 控制面拥有每个 Task 的 Speculative Tree 和 Branch 状态，控制面创建、登记、推进、接受、丢弃和清理 Agent 执行；Pi 只通过最小 Product Integration／Interposer 接入，报告 Agent 行为事实和 outcome，执行控制面返回的命令，不维护 AScout Tree、Policy 状态或生命周期裁决。控制面与 Pi 之间的通信必须传递受契约约束的事实、命令、结果和事件，不能以 Pi 的本地对象作为控制面状态。

第一版不等待完整 Runtime Adapter：候选 Branch 使用 AScout 运行管理建立的独立目录作为最低程度的文件系统隔离，目录创建、可见范围、清理和 Branch 归属由控制面／运行管理落实。仅修改 Pi 的 cwd 或在 Pi 内创建目录不构成已实现的隔离。

根目录一次性启动程序不得作为系统入口。测试可以是 AScout 内部测试，或从外部模拟用户行为的端到端脚本；测试脚本不能承载正式 Tree、Policy 或 Agent 管理职责，也不能把固定 `choice` 场景包装成通用系统能力。当前旧原型的真实模型运行只保留为历史实验记录。

首个实现步骤采用已确认的最小名称 `AgentMessage`、`AgentCommand`、`SpeculativeTree`、`BranchRecord`、`ControlState`、`BranchRole`、`BranchStatus` 和 `AgentController`。它们位于 AScout 控制面代码中：控制面创建 Branch 工作目录、处理 Pi 报告的启动和 outcome，并生成 Promote、Discard、Cleanup 命令。内部 Rust 测试验证控制面状态转移和目录隔离；随后已通过 `ascout-control-plane` 与 Pi `agent-access.ts` 的 Unix socket 连接完成真实跨进程验证。

已完成的下一步（2026-10-24）：旧 Pi-local 实现和一次性驱动已直接删除。Rust 新增 `ascout-control-plane` 二进制，Pi 新增最小 `agent-access.ts` 接入；二者通过长度前缀 JSON Unix socket 通信。控制面负责调用 Policy 的 `admit`、`select`、`launch`、`predict`、`match` 和 `dispose`，并且只有 `dispose` 的 Promote 意图才会触发 Tree Promote。真实运行已验证两个跨进程 Pi AgentSession、真实 `deepseek/deepseek-flash` 模型请求、Candidate 独立目录、控制面 Tree 更新和旧 Authority 清理。

该实现随后被发现仍有三项边界错误，不能作为正确闭环继续扩展：Pi 接入由用户手工启动而非由控制面启动；控制面把目录直接当作独立执行环境；控制面用 Candidate output 而不是 Parent 的真实 outcome 作为 `match` evidence。后续实现必须由 AScout 用户入口向控制面提交请求，由控制面启动 Pi 和 Agent 接入；由 Runtime Adapter 创建并报告抽象的独立 Candidate 环境，当前目录只作为该 Adapter 的实现细节；并由 Parent outcome 与 Candidate hypothesis 完成 Policy 匹配。

## D-036-I：Policy DSL 首版编译为 Python 脚本

2026-09-17 确定：PolicyDefinition 只通过 Policy DSL 定义；九类 Decision 都需要根据各自的输入作出规定输出，每个位置必须具有明确行为。固定取值和常用规则只是决策逻辑的快捷表达，例如 Eager 编译为恒定返回 `true` 的 `launch` 决策。

首版 Policy Engine 将 DSL 编译为可交付的 Python 脚本，并通过运行脚本对已编译的 Policy Decision 求值、作出本次决定。控制面可以直接调用 Policy Engine，也可以按需查询编译结果并将脚本交付给 Agent 接入侧，由接入侧以相同语义求值；Policy Engine 不负责落实决定、选择运行位置或传输。脚本是内部编译产物，不是另一种用户入口，也不能包含控制面内存对象或仅在编译进程内有效的引用。

2026-10-23 确定编写形式：YAML 描述 Policy 的结构、各 Decision 的行为和配置，并通过文件路径引用独立的算法源码。首版算法语言仅支持 Python，九类 Decision 暂时分别显式指定；不能通过省略某类 Decision 静默补齐行为。YAML 和被引用源码共同组成一个 PolicyDefinition，不增加独立算法加载入口，也不将自定义策略限制为内置算法菜单。源码语言与编译产物是独立选择，具体库范围和调用方式仍待确认。

2026-10-23 确定参数作用域：用户可在 YAML 中为各 Python 函数分别指定算法参数，传给对应函数；首版不支持全局共享参数，不设全局／函数配置的继承、合并或覆盖规则。参数与系统提供的运行时数据、授权能力分开。具体字段布局和调用约定仍待确认。

系统按每类 Decision 的约定提供数据，算法只在规定的数据及授权能力范围内计算，返回值经检查后由状态拥有者落实。源码路径在编译时解析并固定所用代码、明确依赖；运行期使用编译结果，跨侧交付不能只传作者机器上的原始路径。这里确定的是职责和交付边界，尚未确定具体 YAML 字段、函数入口或依赖打包方式。YAML、参数限制和静态检查都不能替代真实执行权限边界。

这一选择用于优先验证 Policy DSL、九类 Decision 的输入输出契约和跨侧运行路径，不把 Python 固化为长期执行载体。后续更换执行形式仍须保持已选语言语义和 Decision 接口，不能假定源码自动具备跨后端可移植性。

2026-10-23 用户授权开始实际实现，并确认 `policy`、`PolicyEngine`、`CompiledPolicy` 和 `Decision` 的代码命名。现有实现已打通 YAML 检查、独立源码固定、各函数参数传递、可序列化产物与按 Decision 导出 Python 脚本、隔离加载检查、实际求值、基本输入输出检查和失败返回。部署使用 bubblewrap namespace、用户 import 前 seccomp、rlimit、限量临时空间及受信调用侧超时监督，不使用普通 Python 子进程冒充隔离。实际遇到 bubblewrap 不支持 `--preserve-fds` 后，先新增真实环境的回归测试复现，再修复管道启动方式。

尚未实现的内容包括完整领域 schema 和 Contract／Fidelity 检查、外部模型等能力调用、第三方库环境固定、标准库内容身份、增量预测、score 规则、多数效果与处置计划检查、异步取消、cgroup 总量统计、生产级交付认证以及状态拥有组件和端侧真实接入。未知 YAML 字段和不支持的结果会被拒绝，不静默降级或伪造成功。具体 API、运行方式、数据清单、验证与缺口见 [Policy Engine 原型](policy-engine.md)。

整体方案与讨论仍保存在仓库根目录 `Discussion.md` 的 Policy DSL 小节；开始实现不表示其全部候选能力已完成。代码中实际采用的原型接口以开发文档为准，不把尚未实现的草案能力写成可用能力。

## D-016-I：默认 Runtime 优先高效隔离

默认 Runtime 面向可信 Child Agent 与工具，优先低开销环境隔离，不提供 adversarial containment。它必须强制维护声明的环境、资源和效果边界，以及停止和 cleanup。Agent Session、执行实体的本地上下文、operation 归属和 capability 路由由 Agent 接入侧管理。

**废弃记录（2026-11-04）**：此前曾以“Runtime 只提供环境能力”为由，把外部效果实际处理移到控制面。此边界已被本文顶部的 Runtime 整体边界取代；Session 仍由接入侧管理，环境与效果执行归 Runtime，Tree 与全局裁决归控制面。

需要 adversarial containment 时，用户选择另行适配的 sandbox/hardened Runtime。AScout 不协商 security guarantee；当前首个实现为文件系统 Backend（overlay/btrfs）与进程 Backend，见 [文件系统 Backend 当前实现状态](../fs-backend/current-state.md)。

## D-017-I：新 Rust 实现不继承旧代码

新实现继续使用 Rust。旧 `src/**`、Cargo manifests、tests、examples、C header、构建产物和实验文件已经删除，未读取、迁移或复用。新实现只能依据已接受的设计与开发决策建立。

## D-024-I：实现组织服从 AScout 可控边界

当前实现组织遵循 Agent 接入侧、控制面与 Runtime 的责任边界。Runtime Adapter 是 Runtime 的接入边界，Effect Coordinator 属于 Runtime 的效果协调职责；这些职责不直接推出 crate、进程或部署数量。

Execution、Workspace、Input、Effect、Outcome 与 Interposer 的职责语义不绑定具体 crate、模块、进程或服务边界。

后续 crate、模块、进程和服务边界必须依据责任、依赖方向、权限、替换频率与部署需求决定，不能由“一侧一模块”或“一组件一 crate”直接推出。具体 Adapter transport、discovery、连接生命周期和 Runtime backend 属于 Adapter/部署实现选择。

**废弃记录**：此前先要求“实现组织服从四侧一级架构”，随后又把 Runtime 定义为 AScout 不拥有的外部后端、把 Effect Coordinator 放在控制面。两种解释均已由 2026-11-04 的 Runtime 整体边界取代；不再以“四侧”推导源码或文档拓扑。

## D-025-I：项目名称统一为 AScout

2026-08 决定：项目现行名称统一为 **AScout**（Agent Scout），相关现行复合名称使用 `AScoutAgent`。正式设计、架构、使用和开发文档不再使用 `ASCout` 作为现行名称；本条仅保留旧拼写用于历史追溯。该名称变更不改变既有生命周期、Policy、Runtime 或接口语义。
## 2026-11-02：独立 Agent 接入核心的第一步

已确认并开始落实四层边界：控制面继续拥有 Speculative Tree、全局 Policy 和 Branch 生命周期；TypeScript 接入包承载 Agent 无关的协议类型、命令传输、命令去重、能力声明和局部执行 Policy；具体 Agent 通过 Adapter 提供会话与工具切点能力；Runtime 继续负责任务环境和工具效果约束。

第一步已加入 `pi/packages/ascout-agent-access`。公共协议只使用 `branch_id`、`operation_id`、`tool_call_id` 和不透明的 `CheckpointHandle`，不暴露 Pi transcript 或 session 类型。`CheckpointHandle` 只能由产生它的 Adapter 解释，公共层只保存和转发。

本步保留现有 Pi 接入主循环以保持行为不变，并让它复用公共协议、传输函数和能力类型；尚未宣称完成全部 session 编排迁移。下一步应把 `agent-access.ts` 的会话表、pending operation 和工具前置拦截逐步移入 `AgentAdapter`，再移除控制面中的 NLE 特例。

## 2026-11-02：Pi Adapter 与 Runtime 工具声明

已完成第二步：

- `AgentAccessCore` 真正驱动 Agent 连接和命令分发，并缓存重复命令的结果；同一 `command_id` 携带不同命令时返回 `rejected` 命令结果，而不是把协议冲突误报成 Branch 失败。
- `agent-access.ts` 只保留入口连接与生命周期，Pi 的 session、transcript、checkpoint、工具拦截和候选续跑全部移入 `PiAdapter`。
- Runtime Adapter 在 `Hello` 中声明 `RuntimeCapabilities`：可执行的 `RuntimeTool` 列表、是否共享底层环境、是否支持独立 `branch_environment`。
- 控制面不再识别 `nle_action`。它按 Runtime 声明的工具名调用 `tool_context` 和 `invoke_tool`，用 Runtime 返回的 `result` 更新 Agent 工具通道，用 `evidence` 交给 Policy 匹配。
- NLE Runtime 声明 `shared_environment: true`、`branch_environment: false` 和 `nle_action` 工具；Candidate 不再创建虚假的 `branch_environment` 视图，环境动作只发生在权威环境。
- Pi Adapter 从 `ASCOUT_RUNTIME_TOOLS` 读取 Runtime 工具描述并动态注册；它不知道 NetHack、动作编号或 NLE 语义。
- 控制面在 ToolPending 后立即发送一次 `resolve_pending`；候选处置阶段只决定 Candidate 生命周期，不再重复发送同一 `command_id` 的 resolve 命令。

验证：`npm run check`、`cargo check`、`cargo test --lib --bins` 通过。本地脚本模型加真实 Pi 工具完成一次 Promote 和旧 Parent 清理，并在 Promote 后继续下一轮。真实模型 DeepSeek V4.1 Flash 加真实 NLE 完成 10 次动作：9 次 Promote、1 次 Discard、10 次 Discard／Cleanup 命令 applied、0 次 `branch_failed`，总花费约 $0.0060；候选 Promote 后继续执行，旧 Parent 被清理。

## 2026-11-03：Ledger 接管后的归档失败所有权

废弃此前“不新增 Ledger 失败语义、错误直接返回调用方”的表述：它只说明了错误如何传出，没有说明已经移出活动 Tree 的 Branch 由谁保存。按值移交后若追加失败，Branch 会随调用返回被释放，历史在任何组件都不负责的情况下消失。

现行规则：Branch 只有在 `Cleaned`、非当前 Authority、无未完成控制命令且无子 Branch 引用时才从活动 Tree 移交给 Ledger。Ledger 接管后，写入成功才按约定释放记录；写入失败时继续持有该 Branch，并由控制面报告这次运行的历史不完整。写入成功仍不包含强制 `flush`，失败后也不对已写入的部分内容作回滚或去重承诺，不引入自动重试或恢复队列，不把 Branch 放回活动 Tree。

实现：`Ledger` 保留未写入的 Branch，`archive_branch` 失败时不再丢弃记录；控制面退出时列出未能归档的 Branch 并返回错误。

### 2026-11-05：控制状态归树与消息触发

用户确认：task 创建时建立树，具体投机状态归树；Controller 不持有 task 状态，可协调多棵树。Policy 为决策函数，树的方法负责合法转换和后续操作要求。消息／定时器触发同一 task 顺序处理，耗时操作异步返回。Gateway 的统一范围暂未决定。

废弃实现：AgentController 持有 ControlState 和编号，及主循环每 100 毫秒扫描、同步调用 Policy／Runtime 的组织方式。此前 start_candidates、advance_candidates 与同步回调包装由树的事件处理和无状态协调替代。未经用户同意不提交。
# 2026-11-09：物化责任边界与 Firecracker 清理

控制面只报告操作间隙，Runtime 自行决定内部整理；控制面不持有物化专用语义。
撤回 materialize 提案中控制面触发 `FilesystemMaterialize`、新增物化状态/确认
字段、由控制面选择物化冲突策略的建议。本轮只同步文档，不实现新接口；宿主锁、
冲突处理及环境保留仍待后续明确。

Firecracker 停止未确认不标记 Terminated、不丢 Child、不拆资源；清理失败保留环境
和网络所有权供重试，全部清理成功才释放。启动失败回滚适用同一规则。生命周期
操作暂串行化，环境锁覆盖同环境操作，避免清理、重建与旧状态更新竞态。

# 2026-11-09：物化可选、宿主写回要求与后台回收

用户确认以下实现边界：

- 物化是可选能力，不在关键路径上。运行阶段物化后重置 authority 的基准；任务
  结束即结束 authority 的使命。
- 冲突处理倾向让 authority 覆盖，但尚未最终确认。当前暂不为被覆盖的宿主版本
  保留可恢复副本；此前 materialize 提案中的 journal/备份恢复方案不作为现行
  要求。
- 写宿主文件的要求是：写前逐文件与基准比对，再原子替换。实现时必须检查性能，
  先比较文件大小和修改时间，仅在不一致时再计算哈希。这是待实现的要求。
- 嵌套挂载当前未实现，可以另找实现手段；当前 bwrap 参数层做不到不等于实际做
  不到，不把该能力写成不可能。

后台删除改为信号驱动：首次 discard 需要物理删除时向工作者发信号；工作者持续
扫描 trash 并删除，直到为空后静默阻塞等待下一次信号，多次信号合并。启动时若
trash 非空发一次信号回收崩溃遗留。删除失败不得静默丢弃：记录日志并保留条目，
在下一次信号或有界延迟后重试，不忙等。工作者不持有 FilesystemBackend 强引用；
backend 被 drop 时工作者退出，未删完的目录留在 trash 供下次启动回收。btrfs
递归删除、只读属性处理、`BACKGROUND_DELETE_LOCK` 与 `fast_delete` 语义保持不变。

损坏的 `operations.json` 改为失败关闭：改名隔离成功时以空记录继续并大声告警；
改名失败时加载返回包含路径与底层错误的明确错误，不以空记录继续，也不覆盖或
删除坏文件。元数据写入是 temp + `sync_all` + rename 原子写，因此“写到一半
崩溃”不是 `operations.json` 的损坏原因；损坏来源包括磁盘错误、人为编辑和版本
不一致。环境/快照元数据读不出来仍跳过并记录日志。

**废弃记录**：移除 filesystem diff 的“比较另一个环境”参数及其未实现分支。
diff 现在只表示该环境相对其 fork 基准的变化，行为不变；accept 的 target（目标
环境）是另一个概念，不受影响。移除前检查了 protocol contract、Filesystem
Backend 实现与分发、测试和文档，没有调用者传入该参数。

# 2026-11-10：状态目录重叠默认拒绝启动

决定：Filesystem Backend 的状态目录必须位于本次要 overlay 的挂载点之外。
`establish` 和 `launch_fragment` 在创建环境根、upper 目录或环境元数据之前检查
状态目录是否落在被隔离挂载点内；重叠时返回明确错误，错误包含状态目录路径、
冲突挂载点路径，以及修复建议“把状态目录放到被隔离挂载点之外，例如独立挂载的
文件系统”。

理由：此前的 flatten 降级会掩盖部署错误，并让 fork/diff/accept 退化为
O(upper)。flatten 路径保留，但只允许通过内部开关
`allow_overlapping_state_directory` 启用；该开关仅用于测试与诊断，默认 false，
不属于外部配置文件。

**废弃记录**：不再把“检测到重叠后自动 flatten”作为默认策略。能力字段
`state_directory_overlaps_mount` 与启动日志保留上报，日志措辞改为说明默认拒绝
启动、除非内部开关开启。

# 2026-11-10：fork 的逐挂载 Chain / Flatten 布局与链深度上限

决定：fork 时每个有产出的挂载（upper 非空）单独选择一次布局，不再用
“状态目录是否在该挂载内”这种对挂载恒定的判断。

- **Chain**：对父 upper 做只读快照/复制得到 S，S 加入子 lower（子 lower =
  父 lower + S），子 upper 为空。fork 便宜；子 upper 只含子自己的改动，
  diff/accept 成本随改动量增长；代价是每次 fork 让链加深一层。
- **Flatten**：父 upper 整个复制（或可写快照）成子 upper，子 lower = 父 lower，
  不加入 S。链不增长；代价是 fork 复制父 upper，diff/accept 随 upper 条目数增长。

选择规则：状态目录位于该挂载内（只可能由内部测试开关
`allow_overlapping_state_directory` 制造）时强制 Flatten；否则用
`upper_entry_count` 封顶计数，条目数 < `DEFAULT_FORK_FLATTEN_ENTRY_LIMIT` 用
Flatten，否则用 Chain。新配置字段 `FilesystemConfig::fork_flatten_entry_limit`
供测试与基准覆盖（`0` = 总是 Chain，`usize::MAX` = 总是 Flatten），默认取常量。
它与 `SNAPSHOT_CLONE_ENTRY_LIMIT` 是两件事：后者决定 S 用复制还是 btrfs 快照，
前者决定 S/父 upper 放在子 lower 还是子 upper。

快照基准：`SnapshotRecord.lower` 无论布局都显式记录“父 lower + S”（该挂载有
产出时），作为 diff/accept 三方合并的基准视图。Flatten 下不能直接取子挂载
lower，否则基准会缺 S。

diff/accept：`in_lower` 只判断 mount.lower 是否包含 base 路径（Chain 为真，
Flatten 为假）。Flatten 走现有的 upper 副本比较（overlay：
`changes_from_upper_copy` / `upper_delta_from_base`；btrfs：`btrfs_delta`）。
accept 中 Chain 的 `delta_since` 必须排在 btrfs 分支之前；否则用 S 全量与只含
本分支改动的子 upper 比较，会把父内容全部误判为删除。`needs_source_maps`
不再按 btrfs/flatten 只读子 upper，而是 lower 为空时只读 upper，否则读取
`read_chain(lower)` + upper。

链深度保护：新增 `MAX_LOWER_DEPTH = 96`。本机无特权实测 `bwrap --overlay-src`
叠层：231 个 lower 源成功，232 个 `Invalid argument`；bwrap 把源转成
`/proc/self/fd/N`，限制与底层路径长度无关。取低于实测上限一半并留余量的 96。
超过时 fork 在创建任何目录或元数据之前返回 `BackendError`，说明当前深度、上限
和原因；不静默降级为 Flatten，也不实现整链合并。

阈值标定：在同一非重叠布局下强制 Chain/Flatten，用 `examples/fs_bench.rs`
（ITERS=5，N=16/64/256/1024/4096）。Flatten 的 diff+accept p50 在 N=256 首次
比 Chain 高约 12.3ms（此前 N=16/64 分别约 5.5/2.2ms），向下取 2 的幂得到
`DEFAULT_FORK_FLATTEN_ENTRY_LIMIT = 256`。数据与命令写入
`docs/fs-backend/current-state.md`；这是本机测量，换环境需重测。

用户已批准 `ForkLayout`、`fork_flatten_entry_limit`、`MAX_LOWER_DEPTH`、
`allow_overlapping_state_directory` 命名；对应含义分别为逐次布局、Flatten 阈值、
lower 深度上限、内部允许状态目录重叠开关。父目录变化保留，不通过生产过滤修改
diff；测试必须精确计入对应数据面会报告的目录。

**验证边界**：用户第三轮 `log-3.txt` 已进入真实 btrfs Chain/Flatten/mixed，但
Chain/mixed 因 EXDEV、Flatten 因缺父目录预期失败；第四轮 `log-4.txt` 已验证修复后
三项成功路径，详见下方第四轮记录。

**废弃记录**：`FilesystemBackend::mount_uses_flattening` 及“flatten 只由状态目录
重叠决定”的规则被删除；`launch_lowers` 不再因该判断忽略 lower 层，始终使用
`[挂载点] + mount.lower` 的真实链。重叠场景仍强制 Flatten，但由逐次布局逻辑
显式判断，不再有对挂载恒定的辅助函数。

# 2026-11-10：第三轮特权失败修复与证据

此前 send dump 修复历史：基线 `a37e75c` 的 `log.txt` 中宿主原始能力检查通过，
Rust btrfs 测试却在 diff 断言失败、未到 accept。解析器未去掉 stream snapshot
根（实际 `./upper/`）、只接受箭头 rename 而非真实 `dest=`、输出 `modify` 而非
契约 `change`。修复按 snapshot 根和 inode 路径生成最终增量，临时创建/rename
不产生伪删除；真实目录 rename 只扫描目标子树补足未修改子项。真实 dump 格式
回归先失败后通过，accept 共用增量视图同步修复。随后 `log-2.txt` 319–324 行的
两个旧特权测试通过，仅覆盖当时 Flatten 与 direct-root，不外推其它布局。

- btrfs 挂载存储使用父 volume 子卷，内部含 upper/work。OverlayFS copy-up 的
  work→upper rename 必须位于同一子卷；普通用户真实回归已复现旧布局 errno 18，
  修复后覆盖建立、新挂载、空子挂载、Chain、Flatten、mark 与恢复。fork/mark/diff
  快照父 volume，send dump 只解释 upper 子树；其中父目录变化仍保留。
- 旧的 upper-as-subvolume 布局恢复时明确拒绝并保留原数据、元数据和 work，不做
  自动搬迁；upper/work 的同设备检查也在创建/启动前执行，防止再次延迟到 copy-up
  才暴露 EXDEV。
- 特权脚本使用 `--show-output --format=pretty --color=never`，避免 nocapture
  打断 libtest 结果行。`check-test-output.py` 分别验证精确名单、命令退出码、结果
  行、running 数和最终摘要；执行数=通过+失败，ignored 不算执行，任何 SKIP 失败。
  真实 libtest 交错回归先 red 后 green，保留原假命令场景，共 24 场景。
- launch 的挂载就绪要求每个 overlay 目标都存在（解码 mountinfo 路径），不再
  接受任意一个。目标未齐时每轮检查 owned child 的退出状态；只有确认它已退出才
  接受短命令完成，覆盖 mountinfo 可读但不完整、ENOENT 和退出未回收时的 EINVAL。
  其他读取错误上报，活进程仍须满足全部目标，保留 400 × 5ms 期限和真实退出码。
  两项退出回归先 red 后 green；完整测试最终库 140 项、bin 1 项通过、0 失败，
  其中无权限自行 SKIP 的 btrfs/特权项不作特权成功证据。
  live-target 增加初始观察标记与分阶段计时，共用原 30 秒期限；无证据将偶发超时
  归因为高负载。status pipe 首行仍阻塞，作为已知边界记录。

**废弃记录**：btrfs upper 本身为独立子卷、work 位于其外部的布局被父 volume
布局取代；从 `--nocapture` 日志精确 grep `test NAME ... ok` 的检查被捕获输出和
摘要交叉核对取代。`log-3.txt` 中 4 执行/1 通过与 0/7 误判不是实际只执行了
1 个或没有执行进程测试。

此修复轮未运行 sudo/特权挂载，当时保留的三份用户日志未改；后续用户第四轮
运行完成下述验证。历史上的“修复后待特权重跑”状态已结束。

# 2026-11-10：第四轮特权验证通过与范围

用户 `log-4.txt` 784–787 行为 Chain、Flatten、mixed、direct-root 四测试逐项 ok，
816 行为 4 passed、0 failed、0 ignored；三项 btrfs 场景均选中真实 btrfs，
`incremental_diff=true fast_delete=true`。状态目录位于根 btrfs 挂载，workspace 在
独立 `/tmp` tmpfs，默认非重叠布局成立。7 项进程测试全通过、0 failed、0 ignored，
自动机制为 cgroup，强制 process_group fallback 也通过。原始子卷创建、只读快照、
增量 send dump、严格删除及初始 userns 直接 lower=/ 均通过；脚本 830 行退出 0，
835–848 行全部 PASS，最终 `ALL PRIVILEGED CHECKS PASSED`。

亲读 `tests/privileged.rs` 与 `tests/support.rs` 确认：Chain/Flatten 检查实际布局、
真实进程修改/删除/新建、含父目录的精确 diff、accept 发布及后续真实视图与清理；
mixed 检查两代 Chain→Flatten、accept 回 child1 且 root 不变及清理。accept 传入
`target_mount_pid=None`，不是运行中 live-target 重放验收；不外推全部内核生成的
overlay 元数据、特殊文件/ACL/ownership、偶发超时根因或物化完成。

废弃的 `priv_btrfs_fork_diff_accept_discard` 已由 Chain/Flatten/mixed 三项替代，
不再作为当前测试名单；先前失败与修复过程留在本文件，正式状态以第四轮证据为准。
本次仅同步文档，不改代码、不重跑 Rust/特权测试、不提交；所有用户 `log*.txt` 保留。

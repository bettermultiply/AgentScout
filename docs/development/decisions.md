# 开发决策

## 2026-11-23：确立 AScout 编码哲学

经评估，完整 TigerStyle 将安全置于性能之上，并依赖高密度断言，与 AScout 的目标不合。DOD 与 Plan 9 也不作为项目的通用编码准绳。

AScout 确立自身的编码哲学，优先考虑性能并避免过早优化，保持通用性与可扩展性，通过深模块收拢职责，消除无意义特例，让同类操作同构、成对操作对称。纯内存推进限于控制面核心，以清晰的数据结构和不可逆边界的校验替代高密度断言。相关原则独立成文，并由 `AGENTS.md` 和文档导航引用，用于后续代码实现与审查。详见 [AScout 编码哲学](coding-philosophy.md)。

## OpenTelemetry 与执行记录

追踪由 control plane、Pi Adapter 和 Runtime Adapter 批量发送。Ledger 继续负责执行事实、恢复和 SSE 重放，仅在已有的 `task_started` 增加可选 `trace_id`。Trace Context 作为内部传输元数据，不参与命令、工具请求或 Runtime 请求的去重。暂停目录独立保存可选上下文，缺失时仍可恢复。追踪分别记录 Policy 决策和 Agent 实际应用的晋升。接收失败不改变执行结果，用户可见警告仍走原有渠道。部署与验证见 [OpenTelemetry 追踪](../usage/tracing.md)。

## 本地候选按需绑定执行环境

本地候选遇到首个工具调用时，Pi Adapter 提交内部环境请求并等待绑定。控制面将 Runtime 执行状态推进到 `EnvironmentRequested`，随后调度环境建立，绑定后放行工具。等待期间公开 Branch 状态保持 `Running`，整轮执行结束时才记为 `Finished`。Policy 报告的阻塞仍使用 `ExtentReached`、`Rebalanced` 和 `EffectGate`。

## 工具投机的裁决栅栏（方案 R2）

工具真实结果立即交给 Parent。只要这次操作还有存活候选待裁决，Parent 就在下一次模型推理前等待。全部候选被丢弃或 Part 保留 Parent 时，control plane 按 operation 放行一次。Full 接替时，旧 Parent 走原有淘汰流程。

选择 R2 是为了避免父分支启动下一轮推理后，进展相容性检查拒绝原本可以提交的候选，同时避免多发一次模型请求。方案 R1 允许中止已开始但尚未输出的推理，可作为后续权衡，它能减少候选都被丢弃时的等待，但需要处理请求费用、取消以及输出竞态。

带结果的工具失败属于已交付的权威结果。事件保留 `failed`，候选仍由 `match` 和 `dispose` 裁决。权威结果不可用时回收存活候选并释放父分支的继续执行等待。

## Policy 模型授权归属与通用 Decision 事实补齐

选定 Policy 的 `policy_model_grant` 随编译结果留档并纳入摘要。两侧宿主按当前 Decision 和该声明强制执行模型授权。凭证和 provider 配置归 Pi 宿主持有。未获得授权的 Decision 只使用本次传入的数据。无授权字段的既有编译产物继续使用原摘要。

候选分支在可管理动作开始前检查 `extent`，以完成通知更新计数。权威分支只执行准入等适用 Decision。准入读取分支历史统计，`predict` 与 `match` 按声明接收 `history` 和 `outcome`，`dispose` 可读取已经报告的工具结果及详情。晋升期间到达的准入请求由接收暂停保护，获准在暂停结束后执行。实现与示例见 [Policy Engine 开发说明](policy-engine.md)。

## 纯表达式内存求值

编译后的纯表达式树供 Rust 和 Node 共用。两侧校验输入和输出。`admit.opportunity_pass` 必须编译出纯表达式树。静态无法编译时返回 `InvalidInput`，并且该 Decision 不可请求模型能力。其他 Decision 在无法纯内存求值时可使用 Python 沙箱。表达式表是编译派生数据，保留既有 executable digest，并随完整 artifact 进行来源与完整性检查。

## Filesystem 运行期写入与 holder 启动

运行期环境与快照使用普通磁盘目录，通过同文件系统的原子 rename 发布元数据和 staging 层。`/pause` 同步独立保存代，最终 materialize 同步宿主目标和环境元数据。默认 btrfs 子卷创建、快照使用内核 ioctl。原生 `ascout-fs-holder` 在挂载与 Unix socket 监听后通过管道报告宿主侧 holder PID 与 mount namespace，供 live-mount accept 校验和重放。完整冷启动可用 `cargo run --example fs_holder_bench` 测量。本机 release 构建在普通磁盘目录上，建立和派生小环境后读首个文件的 p50 分别为 3.10 ms、3.05 ms。100 文件的数据层测试仍有逐文件克隆开销，链式 reflink 的 fork 和 mark p50 分别为 10.26 ms、0.97 ms。

## 候选启动与 Pi 会话历史

派生环境同步完成文件系统 fork 并准备 holder 地址。Runtime 返回执行访问后，Pi 可以先推理。工具执行、冻结、合并和保存需要 holder 时等待同一个启动结果，清理等待启动收敛后停止进程。

Pi 候选在内存中建立独立的会话索引，与源会话共享已完成的消息和原生记录。Checkpoint 仅复制消息列表。晋升、通知确认和 `/pause` 按原有持久化契约写盘。最终权威会话仍将胜出候选接到真实工具结果上，候选自己的历史保持原样。

工具准入与后续三个 Decision 均可用端侧已有数据求值时，空闲的权威分支可放行单个工具调用并并行启动一个候选。候选需要调用工具时才请求绑定环境。若已通过 Full 裁决且候选尚未获得独立环境，则在确认晋升时把父环境的使用权交给候选。控制面继续负责结果裁决、单一胜者和结算。

## Runtime 的 full 与 part 接受

Policy 通过 `Promote` 请求全量移交，采用候选会话与后代。Runtime 将相对分叉基线的候选增量写入目标，同路径以候选为准，保留其余权威改动。`Salvage` 保留父会话，交付 Runtime 回执确认的结果并回收候选，选择性移交仍检查冲突。接受裁决计入命中、占用唯一赢家名额并清理同级候选。未接受的裁决允许提交失败验证通知，须组合 `Discard`，保留整体未命中结算。

`predict` 用 `abstain` 显式放弃预测，批量预测必须至少提供一个 Proposal。控制面保留 Proposal 来源及可选的 `model`、`thinking`、`candidate_guidance`，并向 `dispose` 透传 `match` 对象供处置参考。Pi 使用同一套模型恢复路径处理工具投机与 `branch_model` 的晋升。

`dispose` 可单独返回 `{"intent": "Yield", "results": [...]}`。控制面按候选的工具 `action_id` 去重，仅把通知送入父分支的会话，并先交付通知再放行等待点。候选继续在隔离环境中执行，不暂停或中止在途操作。父分支越过分叉点后，候选只能继续 `Yield`，不能晋升。候选运行中积累的工具成果在权威下一次推理前交付。已准入的推理使用现有放行命令，Pi 本地未准入的推理只在有活跃 `yield_only` 候选时报告推理边界并等待。停止后、任务结束时或因资源压力被抢占时回收。编码 Policy 默认启用 `yield_in_flight_verification`，允许在尚无结果时以空列表保留在途验证。

目标会话暂停后，Pi 会中止 `Salvage` 候选正在运行的推理，并立即确认准备完成。`Promote` 仍等待候选推理结束。Authority 给出当前步骤的工具调用后，编码 Policy 仅交付已完成的文件读取或搜索结果。若 Authority 当前的只读调用已覆盖该结果的 60% 或以上，则跳过该结果。候选模型初始与后续步数上限默认均为 4，默认关闭只读步骤中的 Authority 推理流水线，两项均可配置。每条通知最多 4,500 字符，总计最多 14,000 字符。通知注明内容来自当前工作区文件，并显式标记被截断的输出。

只读信息比较在双方路径能够确定唯一工作区前缀时对齐绝对路径和相对路径。由 `head` 或 `tail` 限制的搜索和列表可以用于累计信息比较，首步匹配仍保留输出限制。验证识别允许受限的 `cd`、`mkdir -p` 前置命令、标识符形式的环境变量赋值、末尾 `2>&1` 和单个 `head`／`tail` 管道。比较 Python 脚本验证时，将脚本路径按 `cd` 归一化。`type`、`command -v` 和 `which` 可以作为只读查询。源码修改后的广泛验证仍可预测通过和失败两种结果，使用 `pytest -k` 或仅指定测试节点时只预测一种结果，有过验证失败则预测失败。

启用快速模型投机时，编码 Policy 在 Authority 首步最多执行四次快速模型推理。后续候选先执行一次快速模型推理，只有该步至少完成一个工具调用且所有工具调用均成功、只读时，才可再并行执行一次 Authority 模型推理。`extent` 使用候选启动时的父分支进度和候选自己的工具执行记录。

## Agent 协议与 Speculative Tree 分离

原 `src/control_plane/agent_control/` 已迁移至 `src/control_plane/speculative_tree/`。Agent 消息、命令、握手能力、线封装及其共享值类型归 `src/control_plane/gateway/agent/protocol.rs`；Tree 在 `speculative_tree/messages.rs` 检查并应用消息，继续持有状态、未决命令和恢复转换。这样 Gateway 能接入并交付消息而不依赖 Tree 实现，Tree 仍决定状态如何变化。仅迁移 Rust 模块与定义，不更改消息编码、恢复快照格式或执行行为。

## 前台 control plane

当前入口采用前台单 run 生命周期。此前常驻 control plane 与 CLI 自动启动后台的决定由本条取代，是否采用后台常驻延后再定。多 run 调度独立于进程部署方式，保留 Coordinator 可服务多棵 Tree 的边界。从暂停记录继续和从已结束会话打开下一轮的规则沿用。

## 删除 `CompositeRuntime` 兼容入口

已删除旧类型、构造函数、模块及导出。六处既有测试构造直接使用 `BaseRuntime::with_policy`，保留原 backend 实例、路由策略与断言。每个 Environment 可关联多个 backend，目标是让各 backend 通过同一机制参与协调。本轮交付范围为入口迁移及文档清理。

本轮 `cargo check --all-targets`、Runtime、Proxy 环境身份、NLE 与文件系统路由的既有定向测试均通过。

## 2026-11-18 Tree 异步操作归属

`src/control_plane/speculative_tree/actions.rs` 负责登记操作、识别过期反馈、应用完成结果和推进 `progress`。Tree 与 `SpeculationCoordinator` 共用 `ControlOperation`、`ControlResult`、`ControlActions` 和 `ToolPending`。Coordinator 继续组织 Policy 求值及 Runtime 操作。Tree 内部维护操作编号、反馈指纹、Policy 上下文和推进状态。`coordination` 保留共享类型的既有导入路径，Tree 快照沿用原字段、枚举名称与序列化规则。

Tree 在重开已结束 run 时初始化 run 与 task 身份并恢复 Policy、Runtime、capability 和本地 Policy 绑定，也负责校验暂停 task 与 run id 的关联。恢复层负责读取 Ledger 和 Pi 引用及附件，并校验这些记录与 Tree 的关联。

CLI 调用启动连接与 run 操作，Coordinator 顺序推进每棵 Tree。当前入口服务一个 run。自动启动后台是期望的使用方式，目前尚未实现。

### 正常入口的运行顺序

`coordination/execution.rs` 接管正常入口的事件循环和交付顺序。它先持久化 `task_started`，再把 Prompt 放入交付队列。异步操作和物化回到同一事件循环处理结果，命令交付继续使用独立的工作线程、取消标志及回执。协调侧负责清理、归档、Agent 事实和观察事件的发布。可执行入口负责装配、转交进程信号及管理界面。界面接收正文、流式文本和输入状态用于展示，UI Projection 继续从 Ledger 读取已保存的观察事件。

本次 `cargo check --all-targets`、`cargo build --bins`、`git diff --check` 通过。`cargo test --lib --bins --test ledger_projection -- --test-threads=1` 通过 224 项。正常入口的 Authority、观察 SSE、交付回执及恢复脚本合计通过 15 项。另用临时检查验证了实时流式正文、`task_started` 先于 Agent 的用户消息引用、Ledger 不含对话正文，以及工具名称和耗时字段。模型响应均来自本地伪模型。验收日志为 `/tmp/ascout-coordinator-rust.log`、`/tmp/ascout-coordinator-node.log` 和 `/tmp/ascout-coordinator-streaming.log`。

### 此前的 Tree 归属回归

实跑通过 `cargo check --all-targets`、`cargo build --bins`、`cargo test --lib control_plane::`（59 项）及 `cargo test --lib recovery`（8 项，含与前一组重叠的用例）。使用本地伪模型的 `tests/recovery-active.mjs`、`tests/recovery-filesystem.mjs` 和 `tests/resume-ended.mjs` 共通过 8 项，覆盖活跃任务暂停、恢复失败重试和重开已结束的会话。

## 完整操作的双向二进制传输

完整操作在既有 socket 上发送有界 JSON 首行和独立字节帧。Rust 和目标 backend 先接收完整上传，再执行原子写入或有序修改。请求指纹覆盖内容字节，输出暂存直到终态确认。目录、声明工具结果和 Control Gateway 继续按各自契约返回。缓存归属、内存需求、磁盘限制和真实 VM 证据见 [Runtime Adapter 开发记录](runtime-adapter.md#完整操作与实际限制)。

## 完整操作阶段：有界读取、目标 cwd 与独立队列验证

本阶段将 Pi offset／limit／50 KiB 输出预算下推到同一次通用 read_file；Runtime 的 binary_fallback 只按标准图片签名选择完整二进制返回，文本返回 actual_range／has_more，不扫描 total_lines。FS 使用已有 backend workspace 映射，VM 使用既有 /root；可信宿主 Agent launch cwd 不能当作 VM cwd，客户端 bound_cwd 不可覆盖绑定。不分配新 Environment，不改变授权、generation／重放或取消策略。

Pi session 的宿主 cwd 仅用于会话。文件工具将原始路径交给绑定的 Environment，`bash` 省略 cwd。非零退出码作为 Runtime 命令结果成功交付，由 Pi 转为 `isError: true`，保留输出和退出码。验证见 [Runtime Adapter 开发记录](runtime-adapter.md#完整操作与实际限制)。

独立 holder 和 VM 使用 FIFO 打开作为确定性持锁屏障，第二环境在第一环境释放前完成相同 canonical path 的写入；同环境 alias 另以锁获取／释放事件验证确实等待同一锁。FS 与 VM 的统一入口均覆盖相对读写、替换、目录项及 exec 默认／显式 cwd。定向测试命令和结果见 [runtime-adapter.md](runtime-adapter.md)。

## 完整 Runtime 操作替代 SDK 工具注入

用户批准撤销下方历史 SDK reuse／mutation 注入方案：Pi Integration 独立适配 schema、参数与结果，一次完整调用对应一次 Runtime read_file、write_file、replace_text、list_directory 或 exec。目标 backend 完成编辑及 canonical path 串行，不经过宿主 read→edit→write；相邻 Pi edit/write 已移除仅为旧方案留下的改动，其他恢复／接入改动保留。Runtime／backend 无 Pi SDK／Node 工具依赖，NLE 声明调用生命周期保留。单操作取消按 Pi 原语义尽力终止进程组；用户明确拒绝为全部 daemonized 后代添加 cgroup、supervisor 或 PID namespace，该限制不作为验收阻碍。通信使用现有 socket 上的有界帧，输入完整接收及验证后才修改文件；实际资源限制与验证范围见 runtime-adapter.md。checkpoint、四启动路径、promotion、一次回退及恢复新执行授权语义不变。

## 2026-11-18：Pi 原生工具与 Runtime 通用环境操作

定向补验发现并修复 guest 请求超限无明确错误、读取文件无内存界限及 CompositeRuntime 漏转发 backend writer drain；保留现有 JSON byte 数组协议，不新增传输方案。后续按明确授权，同时修改相邻 Pi SDK 的 supported operations：edit/write 可注入 mutation 的路径解析与共享 scope，原生默认 realpath、队列登记顺序和取消中在飞写入锁保持不变。Product Integration 经通用 `canonicalize` 在目标 Environment 中解析路径，按执行端点及 Environment 身份共享 scope，独立环境不互相串行；scope 延迟读取当前绑定，不把恢复前地址或 Branch 名称当环境身份。真实 Pi SDK → AscProxy → Runtime → Firecracker 无模型用例以 guest `/root` 路径先复现宿主 EACCES，再通过，并覆盖相冲突的 host/guest symlink。逐操作 detached descendant 取消仍未解决；所有候选机制仅研究，须用户选择，不宣称整体完成。确切范围、上限及候选对比见 [Runtime 接入验证](runtime-adapter.md#2026-11-18-通用操作的定向验证与限制)。

按已批准方案，Pi Product Integration 在已有 Agent 宿主复用 SDK 原生 read/edit/write/bash，通过 supported operations 注入调用 `read_file/write_file/stat/mkdir/exec`。文件系统 holder 与 Firecracker guest agent 各自原生执行；删除 Runtime 内的 Pi 工具模块及逐工具 Node 启动，不新增传输或组件。ASC-Proxy 与 Runtime 接收端复用既有执行／输出／取消／确认及重放窗口；SDK 子操作保留 tool_call_id 和原取消信号，独立请求编号不产生缺口。NLE 业务工具仍由 backend 负责。checkpoint 保持原有 snapshot cloning，不改变四条工具／模型 eager／speculative-first 路径或控制面所有权。

集成验证实际复现：执行、drain 和 materialize 重新生成 launch fragment 会重置仍在使用的 OverlayFS work 目录，导致后续文件访问 ENOENT。三条路径改为读取当前内部 holder 元数据，不再准备已挂载的 work 目录。既有 Authority／恢复测试先失败后通过，不放宽断言。恢复 barrier fixture 保留执行函数新增参数，避免把通用操作误当空名称 NLE 工具。测试、证据和验证限制见 [接入实现](agent-access.md)。无提交／推送，无真实模型请求。

## Ledger 恢复审查：共享写入与控制快照

按本轮明确授权，同一路径的 Ledger 打开操作复用进程内原写入器，追加、sync、失败回退与失败状态不可分离；等待观察事件不持写入锁。恢复直接使用 recovery_complete.control 重建 Tree，取消必需 tree.json 副本，继续检查 Agent／Runtime 附件和 Pi 引用；回退不得跨过 task_completed。兼容索引替换失败只写系统日志警告。直接附件及目录同步不代表 Runtime 外部资源与 Pi 引用的全链路断电保证。复现、回归和未完成的 session 打开入口见 [恢复开发记录](recovery.md)。未委派、提交或请求真实模型。

## 2026-11-18：Interposer API 命名对齐

用户明确授权将 `instrumentation/core/index.ts` 的 `createAgentAccess` 改名为 `createInterposer`，导出类型 `AgentAccess` 改名为 `Interposer`，使通用 Instrumentation API 与已确认的 Interposer 术语一致。生产调用、测试和当前接口映射同步更新；下文旧工厂名仅表示当时的历史名称。本次仅改名，不改变行为，不保留兼容包装，不新增概念名称，不涉及 backend 重设计；保留既有文档清理及恢复相关未提交改动。不调用子 Agent、真实模型或 Firecracker，不提交或推送。

验证：通用层 `tsc --noEmit --strict --target es2022 --module nodenext --moduleResolution nodenext --skipLibCheck instrumentation/core/index.ts` 通过（使用 `../pi/node_modules/.bin/tsc`）；`node --test --test-concurrency=1 tests/predictor-fact.mjs tests/pi-access.mjs` 共 10 项，8 通过、2 失败，其中 Pi SDK double 6/6、predictor 2/4。失败均为留档 Policy 的模型／工具准入求值报 `Policy evaluation failed`；使用任务开始时保存的原始 core 和测试副本在临时目录复跑，仍为相同 2 项失败、2 项通过，未在改名任务内修复。`git diff --check` 通过。当前接口文档删除不存在的 `.predict` 调用说明，按现有代码描述 `Interposer.hasPredictor` 读取已交付事实；未改变预测实现。

## 2026-11-18：Instrumentation 对齐后的文档清理

核对 `1c6eec7` 后的实际代码，清除旧代理形式、控制面代执行工具及结果注入表述。ASC-Proxy 为进程内受管 Unix socket 客户端；Branch 身份直接传递，Agent session 不另作通信对端。Instrumentation 的执行访问信息沿 Runtime → control plane → Instrumentation 交付，内部工具执行实现已由上节通用环境操作取代。握手、绑定和请求窗口不视为完整凭证认证保证；能力缺口留在开发文档。`launch` 放行原调用；暂停普通消息排队，只有 `/continue` 解除暂停。该文档清理任务本身未改代码、运行模型、提交或推送，未写入 `Discussion.md`。

## 2026-11-17：Instrumentation 最终定向修正

按已授权语义修正，不新增组件或 Policy 名称：Session 管理属于 Product Integration；`createAgentAccess` 对应 Interposer，`AscProxy` 对应 ASC-Proxy。工具／模型 speculative-first 保持父调用执行前等待，候选独立运行，接受后 Promote 候选并取消旧父调用，不向旧父调用填入候选结果；全部拒绝时原调用放行一次，eager 则并行启动权威调用。以手工事件先复现空／失败预测未放行、held 候选无法验证、失败预测后模型结果丢失及等待释放前回收问题，复用现有授权、Policy 裁决和生命周期机制修复。具体证据及测试边界见 [接入实现](agent-access.md#本轮验证与能力边界)。未运行真实模型、Firecracker 或已完成的整组检查；未提交或推送。

## 暂停只能用 `/continue` 解除

2026-11-17 复核收敛：Branch 取消在 Runtime 准入锁内推进 execution generation，等待操作与写者结束后才开放新 generation；drain 失败保持执行关闭，但清理本身可重试。恢复 candidate setup 必须重新绑定当前 Load 下发的执行访问方式。活跃恢复夹具使用真实 checkpoint 后的执行边界，不在生产 backend 固化工具编号顺序；continue 后等任务完成再保存。Firecracker cleanup 成功按 backend 回执返回 Cleaned，不依赖已被删除的状态记录。验证范围见 [Instrumentation 通用层](agent-access.md)。

目标语义已定：暂停期间的普通消息先留着，不解除暂停；`/continue` 之后再按原顺序交给模型。`/continue` 本身不生成模型用户消息。

实现已改为暂停时只排队普通输入；显式 `/continue` 解开暂停后，control plane 依次投递队列内容。`/continue` 自身不作为模型输入。

Runtime 在环境结果中返回命令端点访问信息，由 control plane 随 Branch 启动及恢复命令交给 Instrumentation。Instrumentation 通过统一 JSON-lines 执行/取消/终态确认接口发送 Branch、operation、tool、arguments 和 context；control plane 不再执行 Runtime 工具。当前 Pi 原生工具在 Product Integration 中通过 SDK operations 注入调用通用环境操作，Filesystem holder 原生执行文件／命令，不再使用内部 Pi 辅助进程。缺少 execution access 时直接失败，不回退为 control-plane 结果注入，也不执行宿主文件工具。取消必须等端点接受命令后发送，并等待 backend 终态才确认。

2026-11-17 实现：ASC-Proxy 统一拥有控制 Gateway socket 与独立执行连接；control plane 转交目标 Environment 的执行地址。端点复用 `req-N` 有界滑动身份窗口，确认释放正文，窗口外身份拒绝重放；执行连接传递增量 output。NLE 工具模拟延迟在 backend 内可取消等待，以独立动作锁维持共享世界动作串行，不占住 bridge 阻塞候选环境创建；请求进入 bridge 后转发取消并等待终态，已经进入同步游戏调用的动作等待调用返回。Environment 取消在活跃操作终态后执行 holder `drain(cancel:true)`，清理已完成调用留下的写者。恢复重新发布当前连接状态；失败模型的合成消息保留 action 关联。验证过期检查区分模型与真实工具动作；保存时已有 checkpoint 结果的工具不再因 transcript 尚未追加结果而重复登记恢复调用。具体验证及未完成项见 [Instrumentation 通用层](agent-access.md)。

## 异步反馈与 Gateway 接收确认（待复核）

按用户授权补原请求范围校验、第一份终态保护和接收回执。Agent 与 Runtime 发送成功须有对端接收证据，重试复用原标识，写失败关闭可能含残帧的连接。当前取消竞态、Agent 发送失败回 Coordinator、完整冲突日志仍有缺口，详见[实现及未完成范围](feedback-delivery.md)。本轮不改暂停、中断、恢复语义或 Ledger 恢复用途。

## 固定路径与 backend 检查修正

Policy 保持 `policy.yaml` 与 `decisions.py`，所需 backend 身份只来自 Policy 已引用的 Runtime 配置，不新增 Policy 字段或改变摘要。启动只接受配置中的 Policy 名称，session 的 `policy.json` 只用于该 session 恢复。fixture 必须在 Runtime 启动前完成 workspace 复制。Rust 常驻 Policy worker 由所有者 Drop 清理，不能使用随创建线程退出触发的 `--die-with-parent`。线程退出回归先复现 Broken pipe 再修正；指定 Rust 检查和两项恢复脚本均通过，证据见 [Policy Engine](policy-engine.md)。

## Pi 接入与可信宿主

Agent 与随附 Instrumentation 在 Branch 隔离环境外运行，仍由 Runtime 管理启动、监视和回收。Branch 文件访问、shell、受管子程序和效果留在对应 Environment；挂载切换和物化不重启宿主。Firecracker Guest Agent 继续作为 guest 内 Runtime 命令服务。

本轮保留 SDK 宿主：原生 RPC `fork`/`clone` 切换当前 session，没有在同一宿主并行管理独立候选和恢复待决工具的协议，也没有当前 `AgentSession.continue` 的对应控制命令。扩展 `context` 能等待并调用 `ctx.abort()` 取消，但不提供 Agent 或候选 session 控制接口；不以扩展吞异常作为否定依据。原生 inline extension 的公开 `getCommands()` 已替代生产命令发现对 runner 的直接读取，正常资源加载保留。`--models` 仍为当前集成的 provider JSON 参数，不宣称原生 CLI 模型列表兼容。

SDK 模型完成回调现在在终态事件及 `result()` 交付前等待，避免原先 detached Promise 的完成顺序缺陷；保留 SDK 原有认证路由及 stream 身份，不使用外部 `markRuntimeStreamFunction` 包装。fake OAuth 摘要、取消与完成等待由相邻 Pi 的 native verification 测试覆盖。产品扩展的任意 Node 文件/进程访问无法自动改写成 Branch 视图；隔离保证只适用于已经适配的文件工具和 Runtime 工具，不声称所有插件都被自动隔离。


## Policy 常驻求值：已确认边界与当前差距

已确认每个 worker 启动时建立一次 bwrap namespace 并持续复用，不能移除文件、网络、环境和 fd 隔离。`model_complete` 必须由宿主按当前请求身份、Decision、模型允许列表、额度和取消状态校验，凭证留在宿主；终态撤销授权，真实模型请求不自动重试。Node 显式授权入口已接 Pi `Models.complete`，实际 runner 假 provider 往返和一次 deepseek/deepseek-flash 请求通过；Rust 新增显式 grant 回调入口。跨 Branch 恶意模块状态隔离、Rust 同步 provider 强制取消及正常启动配置的 grant 接线仍有差距，不能宣称完整验收。细节与验证记录见 `docs/development/policy-engine.md`。

Rust 和 Pi 本地均已接入有界常驻 worker 池，同一编译脚本的请求通过 IPC 串行求值；超时及协议失败后杀死并回收，后续重建。Python 每次重新导入声明模块，不把命名空间当安全边界。删除会跨请求累计的 CPU 限额，保留逐次 wall deadline。Pi 保留原监督修复并提供显式释放和空闲回收。没有发出真实模型请求，不能宣称完整任务或 capability 验收已完成。验证与具体限制见 [Policy Engine](policy-engine.md)。

## 2026-11-16：信号唤醒与物化错误补验

信号 socket 只承担唤醒提示，线程每 20ms 重新检查 release/acquire pending 标志，避免字节已消费或非阻塞写入失败后标志无人处理；退出关闭写半部，不依赖可能丢失的结束字节。同类标准信号可以合并，不引入信号次数语义，SIGINT 仍终止 task 并留 session，SIGTERM 仍请求 Shutdown。Authority 验收不再通过外部 release 结束待取消写者，改用无限 heartbeat 写者并检查停止。加强测试在已有停止路径上即通过，本轮不冒认该路径为新修复。缺失 authority／Branch／Environment 进入已有可见、可重试的物化失败状态，不伪造成功，不自动重建损坏状态。实际失败→通过证据、并发挂载用例的未定位失败与剩余限制见[恢复](recovery.md)。

## 2026-11-16：物化等待不占用控制事件循环

按既定 Ctrl-C 终止 task 并保留 session 的验收要求，物化及其后台写者等待改为工作线程执行，控制循环继续接收 Interrupt。只维护一个在飞物化，结果与 Abort 等命令的终态回执全部收齐后完成 task；失败保留收尾与重试门槛，不能提前启动下一 task。Abort 复用已有命令编号、代数及 command_result，不以普通 turn_completed 代替取消回执；控制面不查看或操作 Runtime 内部 PID。holder 被动 drain 与取消 drain 串行确认，避免关闭 holder 抢在取消回复之前。具体证据及限制见 [恢复](recovery.md)。此前同步物化导致无限后台写者无法响应 Ctrl-C 的限制已修复，不再作为当前免责边界。

## 2026-11-16：终止、暂停与明确继续入口实现

SIGINT 与 SIGTERM 分离：SIGINT、TUI Ctrl-C、Web Interrupt 共用 task 终止路径，SIGTERM 与显式 Shutdown 保留退出语义。`/continue` 走独立控制命令，不生成模型用户消息；当时保留普通消息解除暂停的兼容行为，该行为已由 2026-11-17“普通消息只排队，须 `/continue`”的决定取代。重复终止不重复发出 Abort，终止期间 Pause 不阻塞收尾；保存晚到回执不重新发布终止中的运行。

核查发现持久环境允许工具返回后保留后台进程，而原物化直接结束环境持有进程，后台写者可能被提前终止。环境持有进程通过 Linux subreaper 接管脱离 shell 的后代；物化前确认其拥有的写者结束，取消通过同一环境停止这些写者，不把宿主无关进程视为任务写者。相关实际验证与剩余限制见 [恢复](recovery.md)。此前“SIGINT 仍映射 Shutdown”和“/continue 仅定名未接入”的阶段记录保留为历史，不再代表当前实现。

本文件按日期倒序记录已经接受的实现架构选择、当前实现差距与废弃记录。目标语义以 `docs/design/` 与 `docs/architecture.md` 为准，本文不替代正式设计文档。日期表示记录时间，不重排历史编号；同一天的多条记录保持原有先后。早期 `D-0xx-I` 条目保留为历史，集中在本文件末尾。

## 2026-11-16：Gateway 随控制面统一启停

用户批准 Gateway 属于控制面、不独立部署、由控制面统一启动和停止；目标以[控制面 §2.1](../design/control-plane.md#21-gateway)为准。废弃将 Gateway 部署与启动继续列为未决问题的记录，关闭 Discussion 第 14 项；此前“Adapter 长期必须独立部署”的表述不再成立，Adapter 部署及配置组织继续由第 15 项承接。统一生命周期不推导必须同进程或自动打开浏览器。实现及验证见 [Gateway 核查](gateway.md)。

## 2026-11-16：暂停后的明确继续命令定为 /continue

用户确认 AScout `/pause` 暂停后的明确继续命令为 `/continue`，用于恢复原任务执行，保留原树、候选和环境关系。当时普通消息是否解除暂停尚未确定；此历史待议项已由 2026-11-17“普通消息只排队，须 `/continue`”的决定取代。Ctrl-C 终止当前 task 后保留程序和 session、留在对话中的已定语义保持不变。

此决定取代下文“暂停后的继续命令名称未定”的阶段记录，独立暂停入口的具体 UI 仍未定。目标同步至 [Runtime](../design/runtime.md)、[Agent 接入](../design/agent-integration.md)及上层文档；现有普通输入继续仅属实现现状，`/continue` 命令接入待核查／实现，见[恢复](recovery.md)。本轮仅同步文档，不编码、不测试、不提交。

## 2026-11-16：Ctrl-C 终止 task 后留在对话

用户确认 Ctrl-C 仅终止当前 task，保留程序和 session，在同一对话中等候下一轮；须先结束候选，等待相关写入及物化处理完成。物化失败显示失败，不能宣告成功。这样将结束本轮工作与关闭程序分开；不修改退出命令语义。独立 `/pause` 仍保留树、候选和环境，具体 UI 与暂停后的继续命令名称未定。

此决定取代下文“Ctrl-C 是否同时关闭整个程序未定”的阶段记录。目标见 [Runtime](../design/runtime.md) 与 [Agent 接入](../design/agent-integration.md)；现有 SIGINT／SIGTERM → Shutdown 仍关闭程序，与目标的差距及 TUI 路径待核查范围见[恢复](recovery.md)。本轮仅同步文档，不编码、不测试、不提交，不把现有信号处理认作已修复。

## 2026-11-16：Runtime 生命周期文档改名

`docs/development/runtime-discussion.md` 改为 `docs/development/runtime-lifecycle.md`。该文件的 H1 与内容一直是「Runtime 生命周期协议」的开发阶段实现说明，不以讨论记录为职责；正式接入契约仍归 [Runtime Adapter 契约设计](../design/runtime-adapter-contract.md)。同步更新索引与两处引用，章节锚点不变。

## 2026-11-16：拆除执行承载与资源身份混合待议项

删除 Discussion 原第 36 项：它混合了已有执行语义、实现缺口与未选定机制，不能作为单一设计问题；各项归属见 [Runtime 生命周期](runtime-lifecycle.md#当前范围执行承载资源清理与恢复)。暂不要求异常退出后重新启动并继续原任务，不取消异常清理责任，也不删除已有主动保存恢复功能。

## 2026-11-16：Environment 身份归 Runtime

废弃控制面根据 execution_id、Branch generation 或 Branch 名称生成 Environment id 的实现。Runtime 建立或派生后返回环境身份，控制面只保存 Branch 绑定；创建前用无绑定状态，不保留占位身份。内部 Backend 组合仍属于 Runtime，单世界 workload 共享不等于 Environment 生命周期共享。

## 2026-11-16：撤销 NLE branch 世界复制

废弃本轮将 NLE `branch` 改为 seed/action 重放并复制世界的实现：该修改未经批准，且把 Environment 独占身份错误地扩展成内部世界独占。恢复既定单世界引用方案；save/load 的世界重建继续保留，Environment 生命周期分别保存，最后一个引用释放后关闭世界。旧 `shared_environment` 能力字段及其生产消费者已删除，不保留恒 false 或 AND 聚合替代。

## 2026-11-16：恢复绑定沿用 Pi 的目录解析

核对相邻 Pi 的公开 coding-agent 入口后，恢复绑定不再自行用 `homedir()` 推测 agent 目录；它改用 Pi 导出的 `getAgentDir()`。因此显式 `PI_CODING_AGENT_DIR` 仍由 Pi 正常尊重，未设置时仍使用宿主 `~/.pi/agent`，恢复路径不复制 credentials，也不修改用户配置。

恢复仍使用 Pi 导出的 `DefaultPackageManager` 与 `SettingsManager` 做资源解析，并把缺失源回调固定为错误：恢复前只解析已存在的资源，禁止安装或执行扩展。资源指纹和配置变化检查保留，因为它们是恢复一致性校验而不是重复的 Pi 加载器。

## 2026-11-16：共性沿用已有路径，真实差异集中

本轮代码整理遵循用户确认的偏好：相同语义沿用同一条已有处理路径，真实差异集中在各自边界，不增加新的命名或绕路层，并以保持现有能力优先。控制面 `AgentController::perform` 的 Runtime 成功／失败判定改为复用已有 `runtime_call`，保留原错误文本和外部签名；未把 Adapter 的 run 级 Agent 启动槽位套到 Branch 启动上。

核查结论：`Runtime Adapter` 的 `launch_agent` 使用单个 run 级 `Option<Child>`；已有进程时会直接返回，因此不能用它代替每个 Branch holder 的启动。`BaseRuntime` 则按 `environment_id` 保存 launch template 和 namespace-holder，并由各环境的 Runtime 生命周期负责启动、重建和停止。控制面不再以 `shared_environment` 把环境映射到 `authority`；它始终沿用每个 Branch 独占的真实 `environment_id`。NLE 的内部 workload 世界不改变控制面的 Branch→Environment 独占绑定；组合 Runtime 的 Environment→Backend instance 关系由 Runtime 内部维护。建立、恢复、物化和清理均沿用这个物理环境语义。Save/Load 保护状态继续共用同一处理。

本轮还删除了 `examples/ascout.ts`：调用链核对确认仓库没有引用它，当前控制面入口是 `src/bin/ascout-control-plane.rs`，Pi 接入入口是 `agent-access/pi/main.ts`；开发决策中已将该一次性 Pi 驱动记录为废弃实验材料。`agent-access/core`、`session-tree`、文件工具、NLE bridge、`fs_bench` 和 `tools/replay` 均仍有生产代码、测试或文档调用，因此未删除。

## 2026-11-15：单一 Pi 宿主与每枝 namespace-holder，控制面持有唯一逻辑 transcript

用户确认：Pi 是宿主上的单一进程，在同一进程内托管多个 Branch session。每个 Branch（Authority 也一样）拥有自己的 Runtime 环境和低成本常驻 namespace-holder；holder 只持有该 Branch 的 mount namespace，并通过最小 IPC 代执行 Branch 工具命令。环境 cleanup/discard 时 holder 一并结束，Pi 不进入 Branch namespace。

对话历史归属采用方案一：控制面持有唯一逻辑 transcript，各 Branch session 的历史与 checkpoint 由控制面归属；Promote 后被提升分支的 transcript 成为权威历史，旧 Authority 环境及其 holder 一并结束，其它分支的记录不获得发布权。候选的 fork、暂停、恢复、Discard 与 cleanup 都以“该分支环境及其 holder”为单位进行，不依赖父子 Agent 进程共享内存状态。

凭据与主目录不作为分支文件视图：`$HOME`（至少 `~/.pi`）直接读写，Pi 继续使用默认 `~/.pi/agent` 路径；不设置 `PI_CODING_AGENT_DIR`，不复制 credentials。宿主会出现 Pi 正常运行产生的锁与 `models-store.json`，不再要求宿主文件不变。分支文件中只有任务 workspace 需要 overlay。

当前实现：

- `agent-access/pi/main.ts` 是宿主上的单一进程，托管 authority 与全部 candidate；Runtime 只启动一次宿主 Pi，不为每个 Branch 启动第二个 Pi。
- 每个环境有一个常驻的 `ascout-fs-holder`（`src/bin/ascout-fs-holder.rs`），在该 Branch 的 mount namespace 内，通过 Branch 专属 Unix socket 代执行文件工具命令。holder 不持有 transcript 或 Policy，Pi 留在宿主进程。
- holder 的连接各自独立处理，不引入全局串行队列；一条命令使用独立 session/process group，直接 shell 退出即返回结果，后台后代继续留在 Branch namespace。取消或超时只结束该命令的 process group；环境结束时通过 cgroup/process group 与 mount namespace 扫描结束 holder 和残留后代。
- 物化会发布 Authority 增量并轮换 execution generation，因此先结束旧 holder，发布后按新环境重新建立 holder；旧 holder 的 mount 不再是新 generation 的视图。
- `/app` 映射由 FilesystemBackend 的 `isolated_paths: [{path, target}]` 表达：host 目录仍是 overlay lower 与 publication 目标，Branch 内看到的是 target。tool plan 同时携带 host/target，Pi 文件工具在 Branch 内使用 sandbox path。
- 控制面持有逻辑 transcript，并在新建 Candidate 时继承父 Branch 的逻辑 transcript；Pi 的 Branch session 仍保留原生记录，用于最终单一权威 session 汇总与原生 checkpoint 续跑。

本地脚本验收：`tests/benchmark-task.mjs` 的四个场景（match／mismatch／authority_first／fast_failure）使用真实 FS/Pi/控制面与本地 scripted SSE provider 全部通过；其中在 `materialization_started` 前检查 host workspace 未被工具写入，确认工具确实经 Branch overlay 执行，而不是落在宿主工作目录。`tests/fixtures/recover-obfuscated-files` 的原始 prompt 与验收文件未改。最终一次证据目录为 `/tmp/opencode/benchmark-task-xQJYmR`、`/tmp/opencode/benchmark-task-g1it8i`、`/tmp/opencode/benchmark-task-2KU6c2`、`/tmp/opencode/benchmark-task-eWZPCJ`。

验证还包括：`cargo fmt -- --check`、`cargo check --all-targets`、`cargo test --lib`（167 项）、Pi TypeScript `tsc --noEmit` 与语法检查、`node --test tests/pi-access.mjs`（6 项），以及新增的 namespace-holder 并发/父子视图/后台清理测试和 materialize 后 holder 重建测试。

真实 DeepSeek Flash 的最终验证为 `/tmp/opencode/deepseek-flash-final-FTGsuO`：在当前代码与本地脚本全部通过后只提交一次原始 instruction prompt，未设置 `PI_CODING_AGENT_DIR`、未复制 credentials，使用宿主默认 `~/.pi/agent`。运行产生 3 个真实 `branch_model` Candidate，Authority 先返回，三个 Candidate 均以 incompatible／late 取消并清理，没有伪造 Proposal 或 Promote；`task_completed` succeeded、materialization `published=true`、`materialization_started` 前 host workspace 未被写入，fixture 原始验收 3 项通过。

同日更早的 `/tmp/opencode/deepseek-flash-branch-AxlJOR` 是在补充 `ASCOUT_FILESYSTEM_TOOL_PLANS` 宿主环境与修正真实运行 Policy 之前提交的预验证，工具没有经过 holder，且 Policy 的 `opportunity_pass` 场景未命中；它被排除在最终验证之外，最终验证本身只提交一次 prompt。修复前的 `/tmp/opencode/real-branch-model-1` 是旧二进制、0 Candidate，已终止其残留进程，也不作为本轮证据。

## 2026-11-15：Pi 默认目录使用宿主 `~/.pi/agent`（取代 HOME overlay 方案）

用户确认：Pi 使用宿主默认的 `~/.pi/agent` 路径与运行方式；不设置 `PI_CODING_AGENT_DIR`，不复制 credentials。认证读取和锁创建发生在宿主默认目录。同日早些时候“由 FilesystemBackend 的 `isolated_paths`／overlay 覆盖 HOME，使宿主文件保持不变”的方案废弃；那个方案与“Pi 是宿主进程、HOME 不作为分支文件视图”的当前语义冲突。

本地脚本验收可以用临时 HOME（`tests/benchmark-task.mjs` 即如此）来隔离测试环境，但它是测试夹具，不是正式方案，也不代表需要复制 credentials。真实模型验证必须使用宿主默认配置。

## 2026-11-14：恢复使用当前宿主 bottom lower

用户明确取消普通宿主文件的历史版本承诺：恢复允许读到后来修改的普通宿主文件，
但受管 upper/层、候选、Agent/Policy 状态及保存资料必须独立保留。废弃“无 saved_base
一律拒绝保存”和“通过复制整个挂载保证普通 lower 历史内容”的实现措施；仍保留代码、
配置、依赖和保存资料的校验。恢复测试修改有契约变化注释，不是静默放宽断言。

常驻 Pi 不再持有 Authority overlay；内建文件工具按 Branch 计划执行。废弃把常驻
Pi 存活直接等同于活跃写者的判断；直接受管执行进程仍保留安全门槛。物化后新建
upper/work，不能清空 live upper。该条当时按“每次工具启动一个 bwrap”写下“后台
shell 必须等待所有后代结束”；2026-11-15 的单宿主 holder 语义取代它：工具命令在
直接 shell 退出时返回，后台后代留在 Branch namespace 直到环境结束，物化前随旧
holder 一起结束。

废弃组合 Runtime 将任一组件可共享等同于整体可共享的能力汇总；各状态域必须都可
共享才允许共享，否则在支持分支或共享引用的组件中逐个建立候选身份。晋级清理旧
Authority 环境不结束整棵树共用的 Pi 宿主；Runtime shutdown 统一结束宿主。
物化先持久登记旧 root，再切换新计划；切换完成后的回收失败只记录延后清理，不把
已成功发布变成失败重试。进展与验证边界见 current-state 和 recovery 文档。

## 2026-11-14：本机使用与统一构建入口

用户确定当前在本机运行控制面、用本机浏览器访问，批准根目录 Makefile 作为统一构建入口。默认 `make` 依次准备前端依赖、`npm run build`、`cargo build`；依赖关系保证 `make -j` 也先前端后 Rust，失败即停止。依赖安装按 `node_modules` 内安装时间戳缺失或 package 文件更新触发，每次重新构建前端，不承诺前端增量。产物仍本地生成、嵌入 Rust 且不入库；Gateway 与 UI 的部署和统一启停见[控制面 §2.1](../design/control-plane.md#21-gateway)。

废弃“是否自动准备前端仍未定”及必须手工分步构建的当前使用要求；不采用 `build.rs` 或额外脚本。直接 `cargo build` 的行为不变，完整构建入口与验证见 [WebUI](web-ui.md#生产构建)。

## 2026-11-14：树几何持续更新与复用目标

用户确认：树隐藏或进入 Branch 详情后仍继续更新几何，可见性不是暂停计算的条件；无关对话或纯展示变化不触发几何重算，未变化几何复用，返回树直接使用最新有效结果。缓存有界，及时释放不再需要的结果；不改变既有绘制语义。

当前实现已把观察状态与几何缓存分离，只保留当前 task 每枝最新结果并独立复用中心线采样；事件级增量仍不等于几何增量，单枝追加的采样前缀、逐叶茎连接及局部宽度轮廓复用尚未实现。缓存大小随当前树规模增长，不是固定字节上限；长运行性能收益尚未验证。具体复用粒度与已知限制见 [WebUI](web-ui.md#性能目标与实现差距2026-11-14)，剩余验证留在根 Discussion。

## 2026-11-14：恢复源侧投机起点与父枝验证截止

此前 2026-11-13 因分叉衔接回归撤回了 Chance 准入源时间、`ProposalValidated`、验证 work id 到 Accepted 的关联和父枝验证点截止。本轮重新实现这组目标：`chance_created` 携带接入侧 `admit` 后的 `speculation_started_at_ms`，无候选 Chance 也保留；控制面在 `match` 返回后、`dispose` 前测量 `validated_at_ms`，公共 `proposal_validated` 绑定 Chance、Proposal 与本次验证 id；只有 Agent 确认 Promote 后应用的那次验证 id 进入 Accepted。由此 Chance 定位改用源侧准入时间，父枝在子枝接受后按该验证时间截取原时间映射，终点显示短横线；缺少或不匹配时明确报错，不回退记录时刻、Promote 时刻或旧结果。

本轮只同步目标、代码与观察字段说明；真实模型、浏览器画面及全部极端参数尚未验收。协议字段见[观察](observation.md)，几何规则见 [WebUI](web-ui.md#源侧起点与父枝截止2026-11-14) 与[参数](tree-parameters.md)。

## 2026-11-14：移除 B3 固定场景脚本（废弃记录）

用户明确批准删除 `tools/replay/verify/b3.ts`、`b3-deep.ts`、`b3-repeat.ts` 及确认独占的资源，不扩大为整个 verify 清理，不新建替代验收框架。三个入口分别固定一次、三次深度、五次重复执行，后两种由 repeat 实现；固定 prompt、双候选、层数 1/3/4/10、父目录 `b3-policy.yaml`／`decisions.py`／`runtime.json` 和端口布局一并退出当前入口。仓库内未发现额外独占资源；策略夹具 `verify/b3-policy.yaml` 已在更早任务删除。当时因 `export-content.test.ts` 仍消费 `export-content.ts`，在该次 B3 删除范围内暂时保留通用每键队列比较与回归；后续测试清理确认该测试是唯一调用者，二者仅比较手造数据，不检验真实导出。现按用户已授权的测试清理成对删除这两个文件，无替代测试，也不改变产品功能。B1/native、其余回归、通用 replay 与 assistant null 支持保留。外部实验配置、原始录制和用户数据不删除。移除脚本不表示任何整链验收通过，也不取消通用机制验证缺口。

以下为从当前文档迁入的历史事实，保留各轮先后与失败限定，不作为当前命令或待修脚本任务：

- 2026-11-13 Pi session 审查的当时 B3 首跑／回放各 1 Chance、2 Candidate、1 Promote、2 Discard，4/4 命中，权威 role/text 与 NLE 相同；产物 `/tmp/ascout-replay/b3/acceptance.json`，日志 `/tmp/opencode/pi-review-b3.log`。此前两次分别因过宽宿主进程断言、旧预测 `moved` 正文断言失败；修正后才通过。清理仅覆盖本次子进程和端口。
- 原生导出阶段单层真实工具回放 3 hit／1 miss、四层也缺键失败；日志 `/tmp/opencode/native-b3.log`、`/tmp/opencode/native-deep4.log`。当时四层导出 13 项、十层 31 项；三层为 10 项，不能混写为四层 10 项。深度样例后续 Chance 都在祖先 Promote 后产生，未覆盖活跃祖先 Discard 后的嵌套失效。
- NLE 独立 lgen 初始化修复后，单层／四层各三次独立尝试分别 4／13 hits/consumed、零缺失／耗尽／未消费，但旧日期 before 消息及四层旧权威预测历史仍导致严格失败；一次深度入口的内部三轮未完成，三次独立调用不能称作三轮通过。证据 `/tmp/opencode/{nle-fixed-b3.log,nle-fixed-b3-three.log,nle-b3-final-3.log,nle-fixed-deep4.log,nle-deep4-final-2.log,nle-deep4-final-3.log}`。不兼容旧日期／旧历史，不改源或弱化匹配。
- 随后的当前语义基线为 `/tmp/opencode/ascout-replay-b3-repeat-{bIAF7f,K0uZLH}/run-1`，只读复制到 `/tmp/opencode/current-replay-inputs-0k1c9rtd/{single,deep4}/source`；`provenance.json` 记来源与 SHA-256，`policy-fix.json` 记父目录 single 策略顺序修复，source 不变。Policy 身份交换与跨键数组比较问题分别复现后修复，小回归保留。初错 `/tmp/opencode/current-single-5.log` 及 `ascout-replay-b3-repeat-9WDxkS`；当时 `node tools/replay/verify/b3-repeat.ts …/single/source` 五轮、`DEEP_ROUNDS=4 node tools/replay/verify/b3-deep.ts …/deep4/source` 三轮严格通过，每轮分别 4／13 hits/consumed、零缺失／耗尽／未消费，结构、权威正文、NLE 和每键回复队列一致，清理通过。产物 `/tmp/opencode/ascout-replay-b3-repeat-{tsCtg4,OhpcwY}`，日志 `/tmp/opencode/{current-single-5-final.log,current-deep4-3-final.log}`。这些是当时短轨迹结果，不覆盖长轨迹、自然重复键、取消调度或活跃祖先丢弃。
- 2026-11-14 Gateway 检查最初并行受固定 19091 端口冲突干扰，`gateway-replay-single.log`／`gateway-replay-deep.log` 不作为有效整链结论。串行 `/tmp/opencode/gateway-replay-{single,deep}-serial.log` 对应 `ascout-replay-b3-repeat-{AEVXM8,a4jA1u}`：单层 Promote=0、四层止于首层，report 各 1 hit/consumed，剩余 3／18，cleanup 通过。独立六请求 Pi 捕获定位为 replay 在匹配前拒绝合法 assistant `content:null`，不是 NLE 缺键；原探针和 Gateway 通信证据保留于当前开发文档，后续 null 修复不追认本轮通过。
- null 修复后串行单层／四层产物 `/tmp/opencode/ascout-replay-b3-repeat-{1rQJm5,j5MZJ6}`，日志 `/tmp/opencode/replay-null-fix/{b3-single,b3-deep4}.log`：分别 1／4 Chance、2／8 Proposal/Candidate、1／4 Promote、2／8 applied Discard、1／4 NLE action；4／13 requests/hits/consumed、3／9 cancelled，misses/exhausted/pending/cancel_timeouts=0。单层 unconsumed=0，四层剩余 6 个源 aborted 项未请求（源 13 成功＋6 aborted 共 19 项）。两组均在 repeat 第 97 行完整 audit 因最终权威正文为空失败，源正文分别为 `scripted complete`／`deep complete after 4 rounds`。旧脚本第 94 行以 Discard 达目标结束，第 96 行立即 shutdown，赢家保存 aborted／Request aborted／空 content；最后 Discard→取消分别为 1237→1257ms、3219→3242ms，而回复输出等待 500ms。证据支持结束过早取消回答，consumed 不代表交付。未到重导出或第 101 行固定 `depth*3+1=13` 计数断言，四层首轮失败后两轮未跑；不将未触发的计数问题称作本次失败原因。两组 cleanup clean=true、直接子进程退出 0、相关后代／端口无残留，源 bundle 55 个普通文件 SHA-256 不变；快照和独立 diff 在 `/tmp/opencode/replay-null-fix/`。结束条件和固定计数随脚本退出，不再作为待修入口；最终交付、取消时序及剩余项解释仍为通用验证要求。

## 2026-11-14：首版固定 Policy 与权限位物化范围

用户确定：“我们先按 Policy 不能修改来实现。我觉得保持一致就好了吧，就修改权限位吧”。首版同一 session 加载的 Policy 固定，多轮 task 使用同一份 Policy，不实现修改或切换；端侧求值、异步求值及保存／恢复保持一致。长期允许修改 Policy 的目标保留，切换时机和活动 Chance 的旧／新 Policy 适用规则留待后续支持时确定，不再列为首版待定或阻塞项。

文件属性变更的首版物化范围仅为权限位；其他属性不新增修改支持，其扩展范围不阻塞首版。此决定不授权破坏原文件未改属性或链接结构，也未批准上一轮助手提出的 ACL、UID 等具体处理建议；不能将这些建议作为用户决定或实现方案。物化仍未实现，权限位及保留要求均需后续落实、验证；能力不足须明确报告，不能静默丢失或重置未改属性。

本次范围收敛取代下文将 Policy 切换规则及文件属性支持范围整体列为当前未决问题的表述；不把首版固定 Policy 升格为长期禁止修改。session 一棵树、task 多轮、独立 `/pause`、候选环境承接及已有保存恢复能力均保留。本轮仅同步文档，不编码、不测试、不提交，保留并行工作。

## 2026-11-14：Gateway 审查收尾

用户批准精简 Gateway：废弃 HTTP 经 Presentation.request／Interface.request 再转发请求的路径，HTTP 与 TUI 直接共享 main 装配的 Gateway Connection；Presentation 只提供展示查询，Interface 不再保存请求字段。保留连接断开拒绝、HTTP 状态码及通信协议。Runtime 废弃无外部消费者的 send_with_context／call_with_context，统一由 send 构造既有请求上下文；保留 send、call_with_id、wait_event 测试用途及 30 秒等待，wait_result 不再二次克隆。Runtime trait、Agent 写侧半关闭与停止接收的分离保持。

废弃 `control_plane::session` 的兼容 re-export：本仓 `Cargo.toml` 为 `publish = false`，仓库及相邻源码引用核查仅发现 `tests/firecracker_e2e.rs` 尚在使用旧路径，未发现外部兼容承诺。迁移该调用至 `gateway::runtime`，移除冗余模块，不保留 stub。通信协议及 Runtime 会话实现不变。

完整 Gateway 审查同时覆盖更早中断任务；`gateway-20261114-before` 已含部分迁移，不能作为完整重构前基线。B3 首错经绕过 Gateway 的真实 Pi／本地 HTTP 探针定位为 replay 已提交校验器拒绝 assistant 的 `content:null`，不在本任务扩张修改 replay、Policy 或 NLE。证据与验收限制见 `gateway.md`。

## 2026-11-14：回放工具迁移 TypeScript

用户批准 `tools/replay` 核心与 verify 脚本整体迁移 TypeScript，Node 26 直接运行、局部 ESM package 与独立 strict 类型检查，不引入运行时库、构建发布或 binary。废弃该目录原 `.mjs` 入口，正式文档和跨仓 import 同步使用 `.ts`，不保留兼容 stub。保留已使用 CLI/export/serve 文件模式、报告选项、`messageBody`、成功与取消项、原生父链及同键顺序。

审查确认自动输入只消费字符串正文，去掉返回输入项中无人读取的 `id` 及仅包装正文的对象，源 ID 去重仍在读取阶段保留；导出项 `branch_id`、`session_entry_id` 有验收和溯源消费者，保留。上轮删除未使用 `verify/b3-policy.yaml` 和报告覆盖源修复继续有效；B3 仍读取外部已有实验的策略文件。验证结果与限制记在 `replay.md`。

收尾仅将 `run.ts` 的读取校验 input-status、提交 prompt、推进自动输入拆成内部函数，不以减少行数为目标；保留 await 后暂停检查、序号等待、失败／完成状态与超时行为，不扩展类型、统计或父链重构。

## 2026-11-14：候选环境承接与任务终止／暂停

用户选择“继续使用候选环境”：接受候选并由其承接后续执行时，保留候选环境，不默认将候选改动复制回父分支，不新增环境迁移模式或 Policy 选择接口。必要状态交接、commit/discard、single-winner 及祖先未决依赖继续有效；嵌套接受不能越过祖先约束。

Agent 常用的 Ctrl-C 或退出均终止任务，结束候选并等待相关写入完成后物化。Runtime 向控制面报告物化完成／失败，失败不能宣告任务成功。暂停任务由 AScout 提供独立于原 Agent 的 `/pause` 入口，保留树、候选和环境等待继续；它与原 Agent 的 Ctrl-C／退出终止语义分开，暂停不意味着退出程序。用户已确定 `/pause` 名称，具体 UI、继续入口命令名称及 Ctrl-C 是否同时关闭整个程序未定。

本次名称同步取代“暂停接口名称未定”的记录；不实现 `/pause`，不确定 resume 命令，不修改已有回放工具、Policy 或 Runtime 中同名 `pause` 的行为。Policy 修改规则与文件属性支持范围仍按原未决记录保留。

废弃“候选接受目标尚未决定”“终止／暂停映射尚未决定”及把原 Agent 退出直接定义为保留候选等待恢复的目标表述。此前主动退出保存整树的设计和验证属于当时入口行为；保存恢复代码及能力保留，当前 Shutdown／SIGINT／SIGTERM 入口仍走保存路径，与新目标的差距须后续落实。本轮仅同步文档，不编码、不测试、不提交，也不宣称候选环境承接或物化已实现。

## 2026-11-14：session 控制域、task 完成与物化报告

用户确认：task 是 Agent 的一轮工作，Agent 完成本轮工作并开始等待用户输入时完成。一个 session 对应一个 AScoutAgent 和一棵 Speculative Tree；允许修改 Policy。Runtime 向控制面报告物化完成／失败，物化失败等同操作失败，控制面必须等物化成功后才能宣告任务成功。无存活候选、任务相关写入完成、不为物化暂停、最终物化及无人引用后释放等既有约束保留。

废弃每个 task 创建独立 AScoutAgent／Speculative Tree、以 task 与 AScoutAgent 等同作为调度范围，以及 task 内嵌套 Chance 必须始终使用同一 Policy 的目标表述；不得由编译产物内容固定推导整个 session 的 Policy 不可修改。下文 2026-11-05 的按 task 建树记录及早期 task／AScoutAgent 并列表述仅为历史依据。

Policy 切换时机、活动 Chance 使用旧／新 Policy、同 session 各 task 是否绑定不可变 Policy，以及暂停任务与现有主动退出恢复的映射均未决定。此次仅同步文档，未修改代码、运行测试或提交；实现路径仍需核查对齐，详见 [参考负载](reference-workload.md)、[Policy 原型](policy-engine.md) 和 [物化提案](../fs-backend/materialize-proposal.md)。

## 2026-11-14：调参只用本地存储恢复

用户确认叶茎及参数保持现状；停止将树形参数写入 URL 或从 URL 恢复的方案。调参只通过当前浏览器 localStorage 保存和恢复，移除复制调参链接按钮；`?tune` 面板开关保留。页面对历史已知调参键作一次性清理，包含已移除的七个固定绘制字段，同时保留其它查询参数及 hash。废弃此前 URL 优先于本地存储、每次调整写回地址栏、复制链接复现形态的方案；原大采样回归记录中的 URL 恢复验证只属于当时实现，不构成现行规则。叶茎绘制与参数默认值不变。

## 2026-11-14：Policy 按独立目录组织，当前策略不入库

用户批准每份策略独立拥有声明与算法：`assets/policy/policy/`、`assets/policy/nle/`、`assets/policy/pi-tool/` 各含 `policy.yaml` 与 `decisions.py`，`extent-input.json` 归默认 `policy/`。沿用原声明 stem，目录可容纳示例及未来正式内置策略；当前三份均不成熟，暂不 Git 跟踪。单文件算法仅为简单组织方式，不限制 Engine 的多文件、包和相对导入。

废弃平铺的 `assets/policy/policy.yaml`、`assets/policy/nle.yaml`、`assets/policy/pi-tool.yaml`、`assets/policy/decisions.py` 与 `assets/policy/extent-input.json` 路径；取消原已跟踪 `decisions.py`、`pi-tool.yaml` 的索引条目，内容拆分／迁至本地策略目录，配置逐字保留。CLI、测试及文档使用新路径，克隆后需另行准备这些被忽略的本地文件。默认禁用选择和原有不完整预测／验证配置保持；Runtime 引用仍按启动 cwd 解析。实现与限制见 [Policy 实现](policy-engine.md)。

## 2026-11-14：物化完成条件、写回及恢复边界（文档确认、未实现）

以下为本日早先阶段记录；其中 task 单位、完成判定及物化报告职责的待议状态已由本页“session 控制域、task 完成与物化报告”取代，仅暂停／退出映射等未决项继续保留。

用户最新确认：正常完成仅在任务完成、无存活候选、任务相关写入完成后物化并重置
基准，不为物化暂停执行。取消／失败／主动退出立即进入物化处理，实际写回同样须
满足无候选及相关写入完成；只有暂停任务保留环境等待继续，此时仍有候选。
task 单位、完成判定、控制面是否需要接收最终交付完成／失败事实及如何承接，以及暂停与既有主动退出恢复入口
的映射仍待明确，不废弃整树与候选恢复能力，不新增接口。

写回工作目录及任务允许修改的其他挂载，仅落实 Authority 相对基准的改动，包含
文件属性（具体支持范围待细定）。冲突 Authority 覆盖，检测用于确定改动、异常
诊断与冲突报告；运行期间不编辑宿主仅是使用约定，只需确认任务相关写入完成。
暂允许中间态；部分失败保留已写部分、重试剩余，不承诺回滚；失败阻止任务成功
报告。写入完成后正常退出可见，不新增掉电保证。旧环境及基准无人引用时释放。

撤回上一轮“候选被接受只写入父分支”的固定规则：接受目标须按实际情况考虑，
本轮未重新确定目标。废弃上一轮“具体物化时机、取消／退出、整体原子性与后台
写者处理均未确定”的笼统状态，以及正式文档中“控制面不知道物化专用语义”
对最终交付通知问题的预先裁定；Runtime 内部整理责任保留，控制面是否需要接收最终交付完成／失败事实、如何承接仍待讨论。
现行要求与缺口见 `../fs-backend/materialize-proposal.md`，物化仍未实现。

## 2026-11-14：物化必须最终落地与冲突覆盖（阶段记录，父分支规则已撤回）

用户确认系统最终必须把 Authority 的结果物化到现实；候选被接受只写入父分支，
不意味着每次接受都立即物化。物化冲突由 Authority 覆盖，运行期间不将用户正常
编辑宿主目录作为预期使用方式。具体物化时机、结束时机、取消／退出、锁、整体
原子性与后台写者处理仍未确定。

废弃此前“物化是可选能力、不在关键路径上”及“冲突覆盖尚未最终确认”的表述；
旧“任务结束即结束 authority 使命”不再作为物化与结束时机的规则。早期“冲突默认
不覆盖”及 journal 逆序恢复提案仅作历史记录，不作为现行要求。
现行边界见 `../fs-backend/materialize-proposal.md`；本轮只同步文档，物化仍未实现。

## 2026-11-14：外部回放职责收敛

废弃控制面 `--replay` / `ASCOUT_REPLAY` 编排、Pi 专用 `ascout-replay` provider、运行侧共享 `agent-access/replay/` 和人工输入／模型切换自动停回放逻辑。回放读取和父链还原移入 `tools/replay/`；外部工具启动普通 AScout，以本地 OpenAI SSE 服务供给响应，通过正常接口顺序提交历史输入。用户先手动 `pause` 再交互。结束静默停自动输入，失败明确报错但保留系统；双方主动退出协调走正常 Shutdown/save。普通保存恢复能力保留，不引入确定性调度。新增通用追加模型配置、策略 artifact 加载和可输入状态接口，不覆盖 Pi 用户配置。

## 2026-11-14：回放取消项修复

废弃将明确 aborted assistant 从回放队列过滤的做法：该做法使源实际存在、被 AScout 取消的请求变成缺键。取消项现在按源顺序保留，命中后等待客户端取消，超时明确失败；成功输出统一等待一次 500ms。其他 error 不扩展为取消。验证中同时修正 HTTP Buffer 分块导致的 UTF-8 损坏，未改变匹配材料或放宽缺键断言。实施与证据见 `docs/development/replay.md`。

## 2026-11-14：Gateway 三侧通信与 Registry 归属

用户确认：名称采用 Gateway，统一 Agent 接入侧、UI 与 Runtime 三侧的通信接入、消息分发和送达；管理连接／会话、握手与断线，进行入口格式、身份、上下文和权限检查，转交请求／事实／回执，交付命令／已编译本地 Decision／结果／事件。消息对投机执行的变化由相应控制逻辑决定，Gateway 不拥有 Tree、Policy 或投机编排。Agent 与接入进程的实际启动由 Runtime 负责；Gateway 与 UI 的部署和统一启停见[控制面 §2.1](../design/control-plane.md#21-gateway)。

Registry 已属于 Runtime，保留登记、契约兼容性、能力声明校验、主动验证和独立证据职责；Adapter 不得自填验证状态或以自测结果代替证明。控制面使用带验证状态的能力视图匹配 Policy、协调生命周期。当时 Agent、UI、Runtime 三条通信入口尚未统一实现；同日审查收尾已将三侧入口归入 `src/control_plane/gateway/`，见上文“Gateway 审查收尾”。

废弃正式名称 Agent Control Gateway／ACGateway、将 Registry 列为控制面组件以及 Gateway 实际启动 Agent 的表述；此前“Gateway 是否覆盖 UI／Runtime 未定”仅为历史。本轮只同步文档，不修改代码、不提交、不运行模型或代码测试。

## 2026-11-14：取消物化冲突检测与全量基准副本

用户确认“直接写入”：废弃物化阶段宿主/共同祖先比较、冲突检测与报告及其全量
publication/execution base 副本方案。直接按 Authority 增量最终视图覆盖受影响路径，
保留未涉及路径，不做 Git diff/merge 或新增全量哈希，禁止复制整个挂载。
不能用复制宿主挂载模拟运行中执行基准重置；该协调尚未完成。既有 recovery 的
独立 lower 全量复制也不视为满足新约束，能力差距见 recovery/current-state 文档。

## 2026-11-13：前端构建产物不入库

废弃“构建产物提交到 `webui/dist`、`cargo build` 不依赖 Node”的交付方式：用户决定不提交构建产物，改为在本地生成后嵌入 Rust 二进制；代价是首次克隆及前端源码变化后必须先安装前端依赖并构建，再运行 `cargo build`，缺少嵌入文件会编译失败。当前不实施自动前端构建。

## 2026-11-13：直接构造连接原型

用户批准第二种方案：利用生成轮廓时的位置映射选择实际可见父侧端点，沿子侧选有限终点，按端点和切线直接构造 cubic。替换根区的映射必须指向新连接，不能用隐藏普通边界。保留锚点、中心线、宽度、方向和叶茎 placement；局部外形允许改变，视觉验收由用户完成。未增加参数，未实施 task/tuning 缓存分离。

废弃布局热路径的两侧首交点、全父环求交、交点后弧长距离及单位置父枝专用 roundedBase 规则：这些规则对短粗枝与相切退化易返回 null，且求交和弧长反演昂贵。以下旧验收数字不作为直接构造轮廓的验收证据。

2026-11-14 用户已检查并接受当前画面，批准小范围清理，以代码和文档整洁降低心智负担。核实生产仅调用 `connectSides` 后，删除 `smoothConnection.ts`、只验证旧算法的 `tests/tree-smooth-connection.mjs`，以及无生产消费的求交、弧长积分、反演和辅助函数。删除 `Join`（包括曾以 `exitArc=0`、`parentArc=位置` 兼容的诊断字段）、布局结果的叶茎 `connection`、子枝 `connection.path/joins`；删除未读取的 `attach.fork` 和重复 bounds 遍历。实际绘图 `d`、控制点和所有保留位置数据在七案例逐值一致，现有布局／placement 测试和构建通过。未重做轮廓生成、缓存或构造规则，未增加公共命名或参数；视觉接受不等于穷举极端参数。后续增量布局、输入分离和有界缓存仍未完成。

旧阶段证据归档：stage5 六树21枝78叶茎，56连接22回退，完成位置和189个不变量通过；其V7以每cubic 200段检查407771100对非相邻线段，横穿／接触／重合为0，非解析证明，材料 `/tmp/stage5/`。stage5b保留48段种子与公式，仅包围盒排除／缓存，23项测试5956.58ms，大采样4457.66ms，六树原生值一致；独立检查仍发现旧新共同漏检及非最早交点，材料 `/tmp/stage5b/`。第四阶段默认角nested/fourChildren、lead=0/.5共22/22连接；固定30°/90°补充44/44，不能替代整体验收，材料 `/tmp/stage4/final/`。这些旧外形数据不再作为当前直接构造验收依据。

## 2026-11-13：首交点连接、范围夹角与查询串默认值同步（连接规则已废弃）

当前连接以子枝两侧外沿各自的首交点为依据，父侧和子侧各取独立单侧距离，默认均为 0.15W、范围 0–2、步长 0.01；三次曲线控制柄分别为有效距离的 2/3。保留 React Flow + SVG，纯几何模块位于 `webui/src/geometry/`。本次只修改参数默认值与文档，没有修改算法，也没有修复第二阶段 V3、V7、V11 未通过项。

废弃记录：

- 父枝总 1.3W 弧长窗口被“各首交点沿父轮廓远离另一交点取单侧距离”取代，原因是父窗口端点与子侧出口不匹配会形成绕钩。
- `minChildAngle` 与将超限请求角钳至 90° 的规则被合法范围参数取代：`childAngleMin/Max` 各范围 5–90°，默认 30/90；`childAngle`／`childAngleRandom` 的固定值／范围随机双模式被范围参数取代。范围以 `${id}/childAngle` 稳定采样，选侧检查初始绝对方向是否超过 90°。
- 子枝起点位于父外沿的规则被父中心线起点取代，原因是外沿起点可能让一侧无交点。Chance 圆心仍在按实际首段方向选侧的父外沿，不能把它与子枝起点混同。
- 向父内重叠 `overlap`、`rootCap` 及 `parentCurveCount` 等窗口式连接结构被首交点闭合轮廓与单位置父枝的 `roundedBase` 取代；空枝只画芽。
- 默认 K=20、τ=60000ms、转角 3°、最大倾角 60°、根宽 45、宽度指数 0.5、Chance 半径比例 0.5 被用户完整查询串的真实解析对象取代，分别为 60、10000ms、18°、72°、9、0.1、0.05。其余字段逐一沿用解析值，键、范围与算法不变；`tau` 直接使用毫秒，面板 `scale=1000` 不表示 URL 换算。

第二阶段报告实际为 42 配置、231 次分叉，18 次成功连接和 213 次 null；V3 外部根点 28，V7 漏盖 8 点及额外材料 262 点，V11 的 null 图缺标记。成功连接数不能代替任意交点数量；短粗仅为待验证归因，不把文档同步或默认变化表述成算法验收通过。当前实现与缺口见 `tree-connection-repair.md`。

## 2026-11-13：本轮明确批准保留 dispose.context.authority_outcome

用户在本轮明确回复“同意”，批准保留 `dispose.context.authority_outcome`，用于向 Policy 提供对应 Chance／operation 的真实结果。该字段不自动给模型追加消息；是否追加真实结果通知仍由 Policy 通过可选 `notification` 决定，默认完全不追加。Match evidence 与 `context.match` 保持不变。

早先文档把该字段写成“用户同意”时，缺乏可独立核实的批准依据；本条记录的是本轮明确批准，不追认早先已确认。同步纠正接入实现文档的批准溯源，清除 Discussion 中对应待确认项。本轮仅文档收尾，不修改实现、不运行测试、不委派；只提交本次可隔离的文档差异，Discussion 被忽略则不强制纳入提交。

## 2026-11-13：Pi 原生分支持久化实施进展

按 6b99d9b 接通独立原生持久 SessionManager，复制 native entries 和 ID；不用消息投影重建 checkpoint，也不在活跃 manager 调用会修改 receiver 的 createBranchedSession。汇总使用临时文件及 rename，校验父引用和循环，只有停止写入且成功汇总的源文件才清理。运行 manager 保留预测上下文；汇总父链单独按真实结果整理。旧 message_end 向共享文件生成新 ID 的方式废弃。

废弃记录（2026-11-13 用户批准清理）：曾在调用前计算 requestKey，以 assistant 原生 ID 写入 authority custom 的 data.requests，并写入恢复快照；导出优先读取这些键。此方案使正常运行依赖系统外回放工具，并为测试额外记录专用数据，现已移除。临时拒绝含 authority 标记文件的限制也已移除，改为按原生 transcript ID、Candidate/Chance 和 applied Promote 在导出内存中还原父关系。测试套件先从已有运行导出，不先录制；不为测试新增运行期元数据、不兼容旧格式、不通过正文或时间猜测。没有新增 Policy 类别，默认不追加真实结果通知。

同仓库 rollback 验证保留 targetId、ref、repoRoot 与 Git object，可读 blob 且启动 GC 不删引用；不等于跨仓库迁移、执行 restore 或完整任务继续。摘要跨越预测输入、任意 custom 外部引用和机制状态交接仍需收敛，不自行改写摘要或选择新的 Backend。

## 2026-11-13：真实结果整理历史与正确状态交接

用户确认最终权威 session 的含义改变为经过验证、用真实结果整理后的执行历史，不再要求准确复原候选当时收到的预测输入。保留预测输入节点，将其后第一条后续记录的 `parentId` 改接对应真实结果 ID；原主／丢弃分支保留，single-winner 与单一最终 session 不变。不采纳额外强制另存完整投机后续路径的建议。

废弃“历史消息父关系永不改写”“assistant 父记录必然等于原请求输入”“整理后恢复上下文必然与原始运行上下文相同”的旧承诺。Promote 保留 winner 执行进展，但不能从中推导历史整理可直接替换运行中上下文。compaction/custom 必须保存；整理中的引用／摘要、在飞推理和 rollback 外部资源关联仍是实现差距，不擅定重启、强制取消或替换运行上下文。

运行分支采用 Pi 原生完整持久化、多 session 过渡、接入侧汇总。源分支停止写入且成功汇总后才能删临时文件，活跃 Promote winner 不能删。这是已定方向，不是已完成迁移；当前消息结束事件向共享树文件追加的实现尚未满足它。

正确状态交接是机制保证；FS 共同快照差异合入 winner 仅为讨论方向，不写成已实现 Backend，也不授权通用状态 merge。是否追加真实结果通知由 Policy 决定，默认完全不追加，不是空消息，不强制新增 Policy 名称或类别。历史重挂后的模型回放不能直接假设父链是原请求上下文；当前原生记录还原实现及验证边界见 replay.md。

本轮只更新设计、开发和 Discussion 文档；目标见 `../design/agent-integration.md` §3，实现差距见 `agent-access.md` §4 和 `replay.md`。不编码、不跑测试、不调用真实模型。

## 2026-11-13：讨论清单整理与旧方案归档

用户确认：Discussion 只保留未决问题、理解问题所需的已确认前提和简短文档入口。目标语义、实现状态与验证证据分别下沉到对应主题；已确定但未完成的能力列为实现或验证缺口，不重新当作未决设计。清单汇总不等于授权同时实施各项。

本次从 Discussion 清除的旧方案补记如下，均为历史，不是当前设计：

- 2026-11-10 第一版树形将长度解释为事件顺序，以圆形、六边形、方形等区分事件，同类信息可合并；第二版以当前权威路径居中、按子树重量分配左右，子枝放在父枝下方，连续同名工具或模型调用合并。上述顺序布局、权威驱动几何和调用合并后来被真实调用时间、稳定 Branch 几何及每次调用独立绘制取代。相关旧提交 `16ed127` 的 dist 曾包含当时未提交的并行类型迁移，不能视为独立源码与产物一致的验收依据。
- 2026-11-11 连续对话早期方案曾把历史权威 Branch 窗口改名并取消选择；随后用户澄清为新增独立权威窗口，保留各 Branch 的只读窗口。旧“模型调用是小树枝”措辞改为模型调用叶茎、工具调用叶片。当前行为见 web-ui.md。
- 2026-11-12 独立 SVG 原型 `/tmp/opencode/tree-prototype-v3/` 被放弃；后续以仓库 WebUI 为准。早期默认 K=150、τ=24 秒以及屏幕像素补偿已被后续调参和统一树内坐标替代；当前参数及共享几何见 tree-parameters.md。旧样例的不同轮次重叠数、尺寸与缩放结果只证明当轮状态，不构成当前验收标准。
- Pi 原生 PTY／终端模拟器嵌入方案已被 2026-11-03 UI 责任边界修正取代：通过公开 SDK 取得 Pi 信息，由 AScout UI 展示，不修改 Pi 仓库或嵌入产品终端画面。旧队列中的“接通完整 InteractiveMode／原生终端传输”不再作为既定实现任务，实际交互需求另行确认。
- 原 Discussion 的“src 尚未归组”不再符合当前 `src/lib.rs`：它已只公开 control_plane、protocol、runtime。仍需审查的是公开接口和依赖方向，不据文件迁移推断架构全部完成。
- 原 Discussion 关于物化 journal／备份恢复的安全提案不作为现行要求；后续物化决定已明确暂不保存被覆盖宿主版本，2026-11-14 已确认冲突由 Authority 覆盖，以 materialize-proposal.md 为准。控制面判断压平的早期提议亦未获定案，须先核对 Runtime 自主整理边界。

已下沉的 NLE 闭环、退出、Chance 状态与 session 保存记录不再复制到 Discussion。其历史证据与能力限制继续分别由 reference-workload.md、policy-engine.md、replay.md 等保存。本次没有重新运行这些验收，也不将历史的“不提交”说明提升为永久协作规则。

## 2026-11-13：整个投机面板拖动调整宽度

用户确认在主面板与整个投机面板之间提供可拖动分隔线，调整整个投机面板，不是临时调参面板。沿用现有 React/CSS，不引入布局依赖，不修改树绘制公式或控制语义。

初始宽度沿用 `clamp(410px, 34vw, 620px)`。桌面分隔线宽 8px，投机面板至少 410px，主面板至少保留 480px；1020px 及以下沿用 `min(94vw, 560px)` 抽屉并隐藏分隔线。用户请求宽度只保存在页面内存中，关闭再打开保留，刷新恢复默认；窗口缩小时限制实际宽度，恢复大窗口后恢复请求宽度。

分隔线支持聚焦、左右键、Shift 加速和 Home/End，提供 separator 无障碍属性。指针释放、取消、捕获丢失、关闭面板、容器尺寸变化及卸载均结束拖动。实现与既有无模型浏览器验收见 `web-ui.md`。本轮保留全部混合未提交改动，App、CSS、开发文档与 dist 含既有工作，不能安全隔离提交，不整体暂存或提交。

## 2026-11-13：连接过渡起点、出根侧与亮缝修正

用户批准子侧过渡从“子枝两侧外沿各自露出父枝可见轮廓的位置”起算 0.15W，并明确授权修正错误的根锚点位置；父侧参数定义、初角、宽度、中心线时间长度与叶茎叶片规则不变。据此废弃以下旧规则：

- 旧“从子枝原始根点沿初始方向投影 0.15W”：端点会落在父枝填充内部，可见出口出现折角。现改为露出父枝可见填充的位置沿该侧外沿再前进 0.15W。
- 旧“父侧边缘取 `trunkSide` 按中心线与宽度计算的边”：空枝、单位置父枝和父枝自身连接之后的实际外沿与它不一致。现改为父枝实际可见闭合轮廓上的弧长窗口。
- 旧“连接控制柄取沿切向投影或 0.15W/3”：短到接近零，切向正确也会在根部横穿子枝外沿，填充不覆盖子枝中心线，视觉上像从另一侧出根。现改为弦长 1/3。
- 旧“父枝与子枝填充共享同一条边”：抗锯齿在共享边留下亮缝。现在连接填充向父枝内部多伸 0.08×根宽，由上层父枝遮住。
- 旧“空枝芽也外加父侧连接”：芽根宽为零、两侧在根部与父外沿相切，外加连接反而制造肩部。现在空枝只画芽，不再生成连接。
- 旧“单点父枝照常截父外沿”：没有纵向外沿可截，现改用绕分叉点的圆根；父枝只有一个位置时其子枝初角可超过最大倾角，现按子枝实际第一段方向纠正根锚点所在侧，分叉距离和宽度不变。

实现、浏览器填充并集证据与仍存缺口见 `tree-connection-repair.md`。本轮仍不提交。

## 2026-11-13：按用户批准撤回分叉回中与 Proposal 验证事件

用户报告最新改动破坏分叉衔接，确认仅撤回最近一轮。废弃本轮世界中轴分叉概率、世界左右与范围初角向内调整，撤回 `ControlResult::Validation`、`ProposalValidated`、验证 work id 经 Promote Applied 到 Chance Accepted 的关联、Chance 准入源时间字段、父枝验证点截止及端点/数据缺口标记和对应测试。恢复上一轮的本枝局部概率、范围稳定采样、Chance 创建记录时间定位及完整父枝绘制；真实验证时间仍是未实现能力，不猜测替代时间。

保留此前共用轮廓法线的叶茎锚点、同时间零长采样及单位置截面、逐点累计 bounds 的栈溢出修复、全部调参、共享几何和 NLE 等混合工作。本轮先备份文件及 diff 后做精确反向补丁，不整体恢复文件、不删除 tree 目录、不一揽子提交。

## 2026-11-13：NLE 初始化回归修正

e89533d 保存／恢复逻辑曾为每次新世界增加独立随机 lgen 种子，废弃该初始化选择；它改变既有 core seed 的地图生成行为。新世界保持 lgen=None（地图生成沿用 core RNG），保存记录仍按自身完整 initial 重建。不撤销保存／恢复及指纹验证，不改变候选共享世界语义。旧日期消息及旧权威历史差异尚未解决，不通过重新录制、伪造工具结果或放宽匹配／验收绕过。证据见 replay.md 的 2026-11-13 核实。

## 2026-11-13：叶茎完成位置与填充连接

废弃按调用开始位置放置叶茎，因为叶茎表示已完成的模型调用。
废弃 `stemAnchor` 及外沿偏移锚点，因为连接必须从完成位置中心线与最终可见轮廓求交。
废弃叶茎描边与线帽，因为显式等宽填充轮廓才能复用两侧平滑连接并限制局部宽度。
废弃只凭稳定随机偏好放侧，因为附近同侧子枝和叶茎需要参与角度间隔比较；稳定偏好仅用于同分。

## 2026-11-13：通用回放持久化范围纠正

废弃“回放与后续输入的 session/events 必须只在内存，因而改造 Ledger、SessionTree 和运行目录”的要求。用户明确允许正常新 run 的原生记录、日志和恢复资料照常落盘；不需要的是额外导出缓存、recording 副本。实现移除该过渡改造，直接复用正常运行路径；源 run 只读、每次重新读取，在内存准备回复队列。通用回放入口不包装 B3，也不推断其父目录配置布局。

## 2026-11-12：轮廓锚点修复与父枝验证截止的观测阻碍

废弃叶茎锚点使用调用点前一段航向、而枝干轮廓使用相邻采样中心差法线的两套截面计算；两者必须复用同一法线与宽度。同时间调用不添加零长采样；单位置枝保留真实宽度横截面，不添加纵向尾段。

用户取消父枝“短突出”与“忽略时间”的方案。目标改为子枝被采纳后，父枝按原时间映射最多绘制到对应 Proposal 的真实验证点，终点用 SVG 短横线及悬停说明，不混淆 Chance 圆。实现暂受阻：公共事件没有 Proposal 验证时刻，不能以 Promote、Accepted 或 Chance 创建时刻代替。

分叉应表示投机开始，但当前几何取 `chance_created.timestamp_ms`，该值由 Ledger 追加时生成，不是 `admit` 实际发生时刻；候选 `branch_created.speculation_started_at_ms` 才是准入请求的接入侧投机起点。Chance 无候选时公共字段缺少这一源侧时间；本轮不擅自改变已确认语义或扩大事件协议。详见 Discussion.md 与 web-ui.md 的待补观察说明。

## 2026-11-12：第十一轮叶茎、封顶、障碍与距离制弯曲

本轮在第十轮基础上修正五处：

- 叶茎长度删去 15 单位的固定底座，改为 `9 × log1p(秒)`，下限 4、上限 34 CSS px。耗时越短叶茎越短，不存在“所有叶茎都从 15px 起跳”的常数。
- 叶茎颜色只跟随所属 Branch，删除“权威祖先在分叉点之后的叶茎改按候选变暗”的第二套规则。旧规则会让白干上长出粉茎，但树上并没有对应的粉色 Branch。全树现在每个 Branch 的叶茎只有一种颜色。
- 子枝整条轮廓按父枝在分叉点的宽度等比缩小，使起点不超过父宽的 0.9 倍。这条上限只在单子枝或子枝占据几乎全部剩余工作时生效；第十轮的 90 → 9.6 落差不再出现。
- 主干作为子枝的障碍。子枝起点若在某个祖先树干轮廓内，视为从该树干长出，不判为穿越；起点在轮廓外、路径却进入祖先树干时，换成镜像角度重排；两侧都撞时保留原来的确定方向。取代“离开种子轴就向外”的单一启发式。
- 弯曲从“占整枝长度的比例 + 两遍弧长重采样”改为“已走距离的函数”：180–340 单位的指数回正，叠加约 420 和 210 单位波长的两根低频正弦。追加更晚的事件只延长枝尾，已画出的前缀不变；实测删掉 `child-b` 最后两次调用后，共享前缀的最大横向偏差约 0.015 布局单位。

- 主干宽度上限不再固定 90，改为整棵树逻辑跨度的 2.4%，夹在 4–90。旧的固定上限会让一次只有几次调用、十几秒的短运行得到一根宽度约等于树高三分之一的主干；长运行的宽度几乎不变。分叉处由“子枝整棵子树在分叉点一次性计入父宽”造成的矩形台阶，用两遍 3 点平滑改成接近分叉时逐渐张开、离开后收窄的过渡。
- 候选 Branch 上的成功叶改用该枝的玫红（`--leaf-candidate`：深色 #F0A2D2、浅色 #A83A7E），失败、取消、超时仍保留红、空心灰、空心琥珀。否则粉紫枝上全是绿叶，两个通道读起来互相矛盾。未采纳枝的枯叶覆盖仍然优先。
- Pi 适配器在 `start_branch` 收到 `role=authority` 时把该 Branch 采纳为权威。复用同一个 run 目录时，`session.jsonl` 仍带着上一次运行写入的 authority marker；旧代码只在这个 marker 存在时设权威，控制面新创建的 `authority` 却不在里面，于是第一个 prompt 被 “Only the authority accepts user input” 拒绝。控制面显式启动的权威 Branch 应当覆盖旧 marker。

本轮实测：9 个 Branch、66 根叶茎、66 片叶；叶茎 4.0–9.7 CSS px，30 分钟那次 34.4；叶片 14.7；叶茎锚点 66/66 落在轮廓半宽处；祖先主干穿越 0 处。fixture 里没有子枝触及 0.9 上限，该项只做了代码与类型检查。

## 2026-11-12：第十轮树形态以时间与后续调用为准

本轮废弃第九轮及以前按常数近似树形的做法：

- 子枝长度不再取 `max(min(自身自然长度, 主干高度 × 0.48), 模型调用数 × 125 + 120)`，不再给主干加 100 单位尾巴，不再为了塞下装饰做 `contentScale` 压缩。长度改为该 Branch 从创建到最后一个事件（丢弃/清理事件优先）的逻辑距离加 12 单位短尾，与调用和 Chance 共用 `150 * log1p(age/24000)`。
- 取消调用之间 82 单位的强制间距。调用和 Chance 的位置只由逻辑时间决定；短间隔挤在一起是事实，不再用视觉间隔改写时间。
- 废弃 `trunkSegments[].width` 这套“算了但没画”的死代码，以及根枝 90、子枝父宽 × 0.55 的直线收缩。宽度改为全局函数：该点之后（含子枝子树）的调用数经指数 0.72 映射到 2.5–90 单位；分叉处父宽一定不小于子枝，单子枝时连续。
- 废弃子枝中心线外推 `(父半宽 + 子半宽 − 6)` 和根端圆点。子枝中心线从父枝中心线的 Chance 点开始，父枝绘制在上层覆盖子枝平口；不再有外推的弯根，枝尖不再出现圆点。根枝起点圆头保留。
- 废弃固定六角度集合和全部落在 48–55° 的映射。初始角改为稳定 hash 在 26–56° 连续取值，靠近种子轴的兄弟枝左右随机游走并在单侧多两个时换边，同侧近邻至少相差 15°。分叉点离开种子轴超过 10 单位的子枝固定向外生长，避免孙枝横穿主干。
- 废弃叶片的 `+ 6*unit` 悬空锚点、把叶片转到主干切线、`clearBlade` 上下挪位避让、固定 18×7.5 叶片和固定 24 单位叶茎。模型调用是一根叶茎：从真实轮廓半宽处长出，长度取模型耗时；工具叶片沿叶茎方向排开，大小取工具耗时；叶片只按角度散开，不再移动事实锚点。
- 废弃 `finished`、`discarded`、`cleaned` 合并成同一个灰，以及从未被节点输出的 `tree-trunk-abandoned` / `tree-twig-abandoned` 样式。枝的显示状态改为 `authority`、`authority-finished`、`candidate`、`aborted` 四种；当前权威白色，历史权威（含已结束的旧主干）为按 72% 与背景混合的较淡白色（不用透明度，避免与下层子枝混色），未采纳灰色 40% 且叶片统一变枯。
- 废弃 `.tone-success` 等叶片状态色只做明暗滤镜、并覆盖所属枝颜色的做法。调用状态改为独立通道：成功实心绿、失败实心红、取消空心灰、超时空心琥珀；枝的透明度包住叶片，未采纳枝上的叶子不会再保持鲜绿。
- 删除多余的 Chance 菱形分支：所有 Chance 都是统一圆点，空 Branch 由自己的节点画芽。

## 2026-11-12：第九轮树几何修正（工作区实现）

本轮保留现有 NLE/map 与 fixture 事实，不调用真实模型、不新增仓库测试、不提交。Chance 的 SVG 元素改为统一直径的实心圆；只有已有的空 Branch 才显示实心菱形芽，缺少对应 Branch 的 Chance 仍显示为 Chance 圆。根接口改为按父干局部轮廓向内插入，主干端点补圆头并由父节点层级保护父干轮廓。`c9` 没有现有子 Branch，`child-e` 没有现有 Chance/孙枝事实，不能凭空补事件，最终 PNG 验证须明确写 `unproved`。

本轮废弃的固定外移 Chance、Chance 菱形、无端点圆头和仅靠 DOM 声明尺寸的做法只保留在历史记录中，不作为第九轮证据。

## 2026-11-12：第八轮投机树独立角度与同组墨迹

废弃第七轮全部子枝强制55°、子节点覆盖父节点、法线分离Chance连线与屏幕overlay、空心环芽、半圆枝尖以及叶片反缩放。稳定hash独立确定初始30–55°、s0 0.25–0.5、末端0.4–0.7初始角且至少12°；中心线累计真实弧长反馈，约三分之一后半段轻微回摆，远外枝回正更强。父干前置，子根宽硬取父处0.55，连接轮廓在父缘外偏并位于父干下；树尖连续锥形，不画根盘。

Chance、芽、叶茎、叶片进入同一随React Flow变换的SVG组，尺寸以初始fit作为CSS参考，此后随真实zoom一起变换（不在每次zoom时反缩放）。近邻Chance沿父干±7px视觉错位、无连线，事实时间不变。空芽28×4.5px实心菱形；叶片18×7.5px，叶轴跟随叶茎末端，1px本色暗15%中脉不越端。图例同步三类枝色；暗色authority #F4F8FC、finished #7F8B99。fit按墨迹测量居中，上下目标各50px。

仅临时fixture调整已有调用时钟：root七个Chance目标8/20/31/32/50/68/85%，c3普通child-c、c4 empty，保留孙枝，不增加Branch/叶。第八轮脚本与PNG写/tmp，forks与nearby使用不同真实zoom/聚焦；删除旧轮次验收字段。SVG几何与PNG证据分开，未证明的指定项说明原因，accepted保留false。不commit/push/stash，不调用真实模型，不新增仓库测试；保护既有NLE/map工作。

最终临时验收：root Chance约7.93/19.94/30.95/31.95/49.96/67.97/84.99%；圆盘零相交，同枝叶茎锚点最小28.018px。PNG上下墨迹边距约49.55/54px；forks真实zoom2.609、nearby3.757，forks加高至2200 CSS px以完整容纳c2/c3及各自前300物理px。暗色权威/已结束对比约3.25，两者对暗底16.10/4.95。第八轮末项修复后，empty bud沿主干边缘绘制28×4.5 CSS px实心菱形，capture从实际屏幕端点量得28.0 CSS px；叶簇避让改为按实际叶茎切线检查并保持同枝锚点间距，暗/亮两套SVG三点检测均为0片压干，最小叶茎间距约28.12 CSS px。PNG辅助检查也未检出叶片颜色与主干的重叠样本；芽的PNG量测受抗锯齿和端点覆盖影响约25.5 CSS px，故仍保留为辅助证据，不宣称全部完成。

## 2026-11-12：第七轮投机树实现与未通过验收

当前实现采用父侧缘出枝、按中心线累计弧长反馈的指数回正、父宽0.55递乘与连续锥形轮廓，Chance改为固定10 CSS px青色实心点；近邻保留事实锚点并沿局部法线偏开视觉点、加连接线。空Branch绘制28 CSS px芽与空心环实心核，模型调用各保留叶茎，每茎至多三片16 CSS px叶片。只用mock事件，不调用模型，不新增仓库测试，不commit/push/stash。

本轮增量把子枝根部中心外移到父宽与子根宽之和的一半，并继续增加半子宽与法向偏移，使两个闭合轮廓在父缘外相切；React Flow 节点按树深递增 z-index，使子枝首段在共享 Chance 处可见。稳定 hash 仍保留枝间差异，但产品角度映射到48–55°以抵消轮廓与栅格化损失。最终 PNG 复测（暗/亮相同几何）child-c 为 `-39.91°/-39.91°`，child-e 为 `37.68°/37.68°`，grandchild 为 `-42.61°/-42.61°`，late-child 为 `36.94°/36.94°`；暗色 authority/finished 对比 `4.41`，浅色保持 `2.53`，叶片压干为 `0`，末端可测宽度为 `0.195 CSS px`。Chance 改为实心菱形，empty 改为贴干实心菱形；截图保留 c3/child-c、c4/empty、grandchild。

废弃第六轮固定36布局单位衰减、以子枝宽度限制Chance直径、空Branch DOM高度350、以矩形或CSS声明替代PNG结果以及capture直接写viewport transform的做法。旧第六轮“自动检查全部通过”的表述不构成第七轮验收证据。

验收材料放在/tmp/opencode，正式PNG为/tmp/ascout-tree7-{rich,forks,chances-nearby,authority}-{dark,light}.png。当前accepted=false：六个同父方向在±30–55°和两两间隔15°限制下无法同时容纳；屏幕空间不足以使36次独立调用均相隔40 CSS px。嵌套宽度下限与比例、空芽晚到Chance连接、叶片/叶茎完整碰撞避让仍有缺口。合成PNG无法独立分离证明的角度、曲率、尺寸和碰撞字段必须写null并列入remainingGaps，不以DOM替代、不声称全部完成。不得再以“全图有重叠”为由无条件拒测所有枝：2026-11-12脚本已按法向PNG连续色截面实际取边界中点，child-a/b/c/e/late-child有效截面104/165/64/30/62，角度、宽度和可测分段长度已记录于web-ui.md及tree7-validation.json。整树包围盒明确包括叶片；Chance填充与描边外径分开报告；forks前300px记录实际PNG存在及canvas边距，但c3空芽不足300px仍失败。rich最小叶茎锚点距离21.528637 CSS px明确标为DOM辅助证据，未混作PNG结果。

第七轮其他未完成项：真实弧长参数化、父杆外80物理px可见角验证、统一连接圆角、父末端切线规则、无模型工具入口统一、追加曲线前缀稳定性。当前真实forks缩放约2.24倍，DOM八个Chance圆盘无相交且PNG均检出青色内填充；这不等于PNG已证明全部枝线几何或Chance对父杆对比度。

## 2026-11-12：第六轮投机树可见性与形态修正

第五轮的实现和验收口径废弃：不再把透明 HTML 叶片点击层当作叶片像素证据，也不再以 DOM 节点高度代替空 Branch 的视觉证明。当前实现使用闭合实心树干、Chance 实心点加反色描边、子枝根部不重复绘制 head；SVG 叶片为细长柳叶形并使用叶片本色中脉；空 Branch 具有专用 `aria-label`/`title`，其晚到子树仍保留逻辑时间锚点。

第六轮的临时 headless 断言、截图和 PNG 像素统计只放在 `/tmp/opencode`，不新增仓库测试、不调用真实模型、不提交临时材料。forks 截图必须通过 React Flow 的真实放大控制并检查 Chance 与 Branch 端点包围盒；若空 Branch 的小芽盒和子树锚点无法同时满足，validation 的 `remainingGaps` 必须明确记录，不能用 CSS/DOM 声明替代 PNG 证明。

## 2026-11-12：第五轮验收纠偏

废弃用最小事件槽间距移动 Chance 的做法；调用与 Chance 直接共享事件 timestamp 到逻辑距离映射，叶簇避让不改变时间锚点。废弃 capture 脚本强制 DOM disabled 的做法，fixture 保持 SSE 连接并发布 ready，按钮截图通过真实输入/清空触发组件状态。废弃把宽度误报为 treeAspect、把任意高对比像素当作三类树干证据的验证；现在合并全部 Branch 屏幕矩形，并在对应树干的已知位置读取最终 PNG 像素。旧第四轮验收数值不代表第五轮通过。

第五轮验收进展写在 web-ui.md 与临时 validation 的 remainingGaps；不提交 Git、不调用真实模型、不新增仓库测试。旧的角度硬截横偏会使起始切线偏离分配角，现以保留起始切线并单调回正到 20° 的曲线替换；工具叶片使用随 viewport zoom 补偿的 SVG 尺寸和共点叶柄。

## 2026-11-12：第五轮投机树像素验收（工作区实现）

第五轮把分叉方向限制为初始同号的 5°–65° 候选，并让路径沿父干向下回正；主干继续使用一条闭合实心轮廓，空 Branch 最短 28px。Chance 的可见半径取子枝根宽与父点宽的较小值并保留 3px 直径下限，命中层仍透明。模型叶茎与工具叶片保持独立，叶片为闭合柳叶 path 并增加 1px 中脉；禁用发送按钮使用灰色约 0.45 opacity 和 not-allowed 光标。

第五轮 rich fixture 的 root Chance 分布改为 15%、30%、31%、52%、70%、86% 的逻辑时间目标，并保留 30 分钟调用、空 Branch 子树和 60+ 根调用。所有 PNG、截图和断言脚本继续只放在 `/tmp/opencode`，不调用真实模型、不新增仓库测试、不在本轮提交。

## 2026-11-12：第四轮投机树最终渲染规则

本轮废弃第三轮中以多个 stroked segment 叠加绘制树干、以 CSS 变量而非最终合成像素判断对比度、以及用非闭合叶片视觉表示调用的旧实现。当前目标语义是一条单一闭合实心变宽轮廓 path，宽度按剩余调用比例全程渐细，模型叶茎与工具叶片各自独立绘制；工具叶片必须是闭合杏仁/柳叶形，Chance 为实心点加描边。旧实现仅保留在下方历史记录，不得作为本轮验收证据。

本轮临时验证最新实际生成 6 条闭合树干 path、5 个可见 Chance、59 个 SVG 叶片；同一 model twig 内的叶片 overlap 为 false，HTML LeafButton 叶片视觉透明。Playwright 临时隐藏非 root Branch 后生成 root-only PNG，root 中心线法线扫描输出 `10/9/7/5px`，root/tail 达到要求；布局样本也保持单调。Chance coverage 自动记录 root c1/c2/c3 为 front、c4 为 middle、c5 为 back。旧的 CSS-only 和跨 Branch 宽父矩形采样结果不得作为验收证据。

## 2026-11-12：第三轮投机树最终几何规则（历史）

本轮取代下文 11 月 11 日关于“稳定代价交换左右通道”和旧压缩实现的临时规则：兄弟方向不再通过 cost 交换，创建顺序固定；确定性候选角度扫描保证同一父枝在 40px 内的兄弟分叉至少相差约 15°。主干使用 1 秒逻辑时间格与 Branch 自有年龄的 `log1p` 距离，调用与 Chance 共用追加式最小 28px 槽位；模型/工具尺寸也使用对数映射。树干后缀调用数非线性归一化到 1.5~9px，保留至少 1.5px 的空尾巴。Chance 圆的直径等于子枝根宽度，透明命中区显示既有 Chance/Proposal 信息，但子枝根头本身不重复绘制。旧实现仍保留在本文件仅作为历史记录，不再是当前语义。

验证结果：`npm run typecheck`、`npm run build`、`cargo build` 和 `git diff --check` 均通过；`/tmp/opencode/assert-tree3.mjs` 的 fixture 数值断言全部通过。针对 rich fixture 的 129 个事件实测 `foldEvents → layoutTree` 约 9.1ms，生成 473 个采样点、468 个树干段；方向 hash 使用 `task/branch/tick`，14px 仅是渲染细分。root 的 72 calls 首段宽 9px，8-call child 首段约 2.944px，28-call child 首段约 4.641px，均向 1.5px 尾部收窄。57 个工具叶片的 DOM rectangle 无重叠；Chance 圆心误差约 0.0052px；light/dark 有效前景-背景对比度实测分别约 16.30:1 / 14.51:1，composer textarea 宽度 1016px。未添加仓库测试，未调用真实模型。

## 2026-11-12：Transcript 引用与持久化 session 树迁移

以下两项是已确定目标方案下的旧实现废弃记录；实现仍在迁移中，不表示目标能力已完整实现。

- 废弃 Ledger 的 Transcript 原样保存 Pi 完整消息对象或正文的旧实现。目标方案由 Pi 格式的 `session.jsonl` 保存正文，Ledger 仅保存 Branch id、树文件记录 id、role 等引用事实。
- 废弃 Candidate 使用仅存在内存的 Pi session，以及 Promote 只设置 `authority=branch` 的旧实现。目标方案中，Candidate 和 Promote 依赖持久化 session 树文件与权威路径标记；相关实现仍在迁移中。

## 2026-11-12：投机树长度、方向、宽度与配对规则重写

废弃以下旧实现与文档表述：把整枝长度写成“从 Branch 创建到当前的累计时间”的一次对数映射；把没有开始时间的调用跳过或用事件序号补时间；由“创建到最后事件（丢弃或清理更晚则取它）”决定枝长并追加固定短尾、最短长度和空枝固定长度；用指数回正加两根低频正弦决定航向；用 0.72 次幂、跨度 2.4%、4–90 的上限、26–56° 初始角、同侧 40 单位至少差 15° 等经验常数决定宽度和分叉角度。

现在的规则：布局前先把工具调用配对到它之前最近的模型调用；所有调用必须带开始时间，缺失显式报错。纵向长度逐截计算，每截等于相邻两次调用开始时间间隔的对数，位置逐截累加；Chance 不切段，只在相邻两截之间插值；枝长止于最后一次调用。方向改为逐截随机行走：横向位置决定回中概率，每截相对上一截转固定小角度，方向限制在 ±60°。宽度按该点之后剩余调用数占本枝总数的平方根缩放；根宽是独立的树内参数（默认 45 单位），不随段长映射变化；子枝根宽按剩余份额从父枝宽度推导；删除全局上限与 0.9 封顶。Chance 半径取子枝根宽的一半，叶片与空枝芽改为无尖端的椭圆。前端仍以事件折叠结果作布局输入，观察事件格式不变。

## 2026-11-11：投机树按逻辑时间生成自身路径

废弃 lane、固定横向轨道、子树重量排版、`FORK_DROP`、`FORK_ROOT_HEIGHT`、`STUB_FORK`、人为根部弯曲和父子独立连接线。WebUI 只根据 Branch 自身事件生成路径：Branch 起止使用已有 `timestamp_ms`（未结束到任务逻辑时间），模型/工具使用各自的开始、结束和持续时间字段；缺少逻辑时间时仅隔离地按事件顺序降级并记录，不以浏览器时间或 sequence 冒充时间。模型叶茎左右交替，工具均在该模型叶茎末端成簇。

父子接点必须来自 chance 的真实父事件。当前观察 `ChanceState` 没有保留 `chance_created.timestamp_ms`，而 `branch_created` 有 `timestamp_ms`，所以本轮临时使用子 Branch 创建事件作为明确接点，并在 WebUI 文档记录数据缺口；不修改后端协议、不使用父 Branch 最后一个 sequence 猜测。树干宽度由当前 Branch 与全部后代在该逻辑时刻之后的调用后缀数连续 clamp 映射，Promote 不能改变几何。方向由横向偏移连续改变的概率函数和稳定 hash 决定。

本文记录已接受的实现架构与验证决策。目标语义归 `docs/design/` 所有。

## 2026-11-11：投机树接枝与复叶语义修正

废弃虚线分叉、候选中空描边，以及“模型小枝条上沿茎排列工具叶子”的表示。Branch 以初始权威根轴为中心分配绝对左右通道，嵌套链复用已空通道，不累加横向偏移；分叉由子 Branch 的 SVG 直接绘制连续实线并接入枝干。模型调用是一根叶茎，工具调用只在末端形成叶片簇；每次模型调用独立，仅同一模型调用内的连续同名同状态工具可合并。权威变化仍然只改变样式，不参与几何计算。枝头缩小为 6px 状态标记，保留透明点击范围。

## 2026-11-11：第二轮树形几何与 headless 验证

- 方向概率使用当前树点相对初始权威中轴的全局 x 坐标；稳定随机值同时决定转向方向和相对上一段方向的转角幅度。Branch 局部偏移和递归 seed 不再作为方向依据。
- 子 Branch 的首个树干点直接位于父 Branch 的 Chance 分叉点；不再绘制侧向连接曲线、额外根部偏移或虚线。子 Branch 从该点以自己的稳定初始角度继续向下生长。
- 同一父 Branch 的子枝按 Chance 创建时已知的时间/sequence 排序；左右方向只在布局创建时按稳定代价交换，Promote、Discard 和后续事件不会重排已存在的前缀。
- 空 Branch 保留最短树干并可继续承载子 Branch。父干粗细在 Chance 位置之后计入所有后代的调用总数，而不是用父 Branch 的 sequence 近似判断。
- 长逻辑时间使用单调有上限的压缩映射；本轮丰富 fixture 包含 3 层 Branch、3 个同父 Chance、已接受 Branch、空 Branch 子树、30 分钟调用和几十次权威调用。headless Chromium 测得 5 个 Branch 的节点包围盒约 `193px × 920px`；深色和浅色主题截图均已生成。

废弃本轮之前的“子 Branch 侧向连接曲线”实现；它虽然恢复了视觉上的侧向生长，但根部不再是 Chance 点的直接分叉。

## 2026-11-11：投机树以 Branch 树为唯一几何依据

废弃“以当前权威路径为居中主干、按子树宽度在权威两侧平衡、同一父节点的子枝头部统一排到父枝底部”的投机树布局。该布局让 Promote 改变几何：权威路径一变，主干、左右分配和分叉位置都会重排，已经长出的树形随之移动，分叉也被搬到新权威下方。

现在的目标语义是：根 Branch 的枝顶固定在原点，子枝的横纵位置只由 `parentBranchId` 与 `createdAtSequence` 决定；子枝从父枝上“创建它时的调用槽位”（最后一个 `firstSequence <= createdAtSequence` 的调用）分叉，之后新增调用只向下延长枝干，不改已有槽位与分叉。只有新增兄弟分支时才允许重新分配左右并整体重排，且兄弟子树的横向范围不得相交。权威路径只进入样式：权威枝干实色更粗，候选细描边；权威祖先在通往当前权威的分叉之后的调用变暗。调用语义固定为“一次模型调用一根小枝条，其后的工具调用是这根枝条上的叶子，第一次模型调用之前的工具调用长在枝干上”；连续同签名调用可在同一根小枝条内合并。

实现上每个 Branch 是一个 React Flow 节点，节点内部用一张 SVG 连续绘制枝干、小枝条、叶子、枝头标记和分叉起点；分叉本身用平滑曲线连接到子枝枝头。旧的独立枝干/小枝条/叶子节点类型与相关常量已删除。图例、悬停文案和详情点击行为随之改为新语义。

## 2026-11-11：WebUI 树形与权威窗口

### 已确定

- 用户视图保留独立的权威窗口；它沿 `authorityHistory` 连续展示已成为权威路径的对话，候选内容只有 Promote 后才进入该时间线。按 Branch 查看仍是独立的只读窗口。
- 投机树使用观察事件的逻辑时间和稳定哈希：Branch 是连续树干，模型调用是短叶茎，工具调用是末端叶片；长逻辑时间用单调平方根压缩并设单 Branch 上限，Chance 根部从父树干的对应时间点向侧面连接。
- 权威切换只改变样式，不移动既有几何；候选未采纳使用中性文字。
- 权威窗口的消息组件使用稳定身份和 memo；事件更新只刷新末尾内容，滚动只在用户贴近底部时跟随。

### 废弃

- 废弃把 Branch 作为垂直绝对槽位、把模型调用画成独立分支、以及按真实毫秒线性展开整棵树的做法；这些方案会破坏树干语义或使长任务不可读。

## 2026-11-16：嵌套 full/part 的接受范围

内层 Chance 的 full/part 可以向直接父级移交。父级仍在投机时，移交只作用于父级的隔离环境和会话路径，保留外层未决 Proposal。随后只有外层接受才能向外层提交。暂停顺序和文件级结果协议见 [Runtime 生命周期](runtime-lifecycle.md#full-与-part-接受)。

## 2026-11-10：嵌套 Chance 接通与提交资格

废弃“Chance 的权威等待点要求父 Branch 是当前权威”的实现限制。Chance 的权威值来自父 Branch 自己的真实执行，只要父 Branch 仍存活（权威或候选），等待点就成立；候选 Branch 上再次 `admit` 的嵌套 Chance 与根 Branch 上的 Chance 使用同一套 `ChanceStage`、验证和失效规则。嵌套候选 Branch 的 `AssumptionSet` 含祖先未确认的 Proposal，祖先 Chance 未 Accepted 前没有提交资格；match 成功但尚不可提交时保留在 Resolving，待祖先确认后再按同一 Promote 流程处置。父 Branch 进入 Discarding/Discarded 时，其上未收敛的 Chance 以 `Discarded(AncestorInvalidated)` 收敛，这是嵌套失效的唯一传播路径。候选 Branch 的外部效果仍受 effect 闸门约束。

## 2026-11-10：Agent 接入侧通用层与 Pi 适配分离

Agent 接入侧的通用逻辑迁到 `agent-access/core/`：本地 Policy 求值调用、`extent` 计数与求值、`effect` 闸门、`branch_suspended` / `branch_resumed` 上报和等待控制面处置的 hold。`agent-access/pi/main.ts` 只做 Pi 执行点接入并调用通用层，行为不变；产品专有的消息、checkpoint、session 和 SDK 细节不进入通用层。`extent` 与 `effect` 由通用接入层求值，Product Integration 不求值；控制面代码不新增求值。接口、覆盖范围与限制见 `docs/development/agent-access.md`。

## 2026-11-10：Chance 状态与 BranchStatus 合并，废弃 BranchPhase 等旧实现

Chance 现在是 Tree 中 `admit` 通过时创建、拥有不透明 id 并持有 Proposal、关联 Branch、权威值状态与验证依据的记录；Branch 只保存 `chance_id`。`select` 的输入是全部 Pending Chance 的 `frontier_view`（字段 `chance_id`），不再只处理单个机会；`predict` 不再接收或猜测 NLE 专有的 `action` 字段。`AssumptionSet` 改为 `(chance_id, proposal_id)` 引用。

由此废弃以下旧实现与表述，正式文档不再保留：

- `BranchPhase` 枚举及 Branch 执行状态上的 `phase` 字段、phase 相关观察/展示。控制面实际推进到哪一步不再由 Branch 上的阶段表示；Decision 历史由 Ledger 和观察事件记录。
- `BranchStatus::AwaitingValidation`。已跑完、等待裁决的状态由 `BranchStatus::Finished` 表示。
- “`launch=false` 等于不创建候选”。现在 `launch` 只决定是否暂不取得权威值，Proposal 与 candidate Branch 仍按 `predict` 创建。
- “`select` 只收单机会”。现在 `select` 接收全部 Pending Chance 的有序子集；未被选中的 Chance 保持待选，并在权威值返回后按 `NotSelected` 收敛。
- Proposal 内嵌在 `BranchRecord` 中。Proposal 现在属于 `ChanceRecord`，Branch 只保存不可变候选执行与 `chance_id`。
- 验证依据借存于 `StartCandidate` 的 pending operation。权威结果与 Runtime 证据现在挂在 Chance 上，`PendingOperation` 只保存操作身份和状态。

## 2026-11-10：模型用量不得静默丢弃，`action_id` 可为 null

真实 NLE 运行中模型已被调用，但控制面没有收到任何 `model_usage`。根因是 Pi 在 `message_end` 之前先用 `stream.result()` 结算模型流并执行 `beforeModelInference` 的完成回调；该回调把 action 从配对队列中移除，随后 `message_end` 找不到配对项，旧实现直接丢弃 `Usage`。修复后完成回调只结束 action 生命周期，`message_start` 标记对应的 action，`message_end` 才取走并发送 `Usage`；只有没有产生消息的请求才在完成回调中移除。

此前要求 `Usage.action_id` 必填字符串。现在接入侧确实无法把一次模型调用与已上报的 action 配对时（例如调用没有经过 Hook），仍然发送 `Usage`，`action_id` 为 null；公共事件 `model_usage.action_id` 也允许 null，不伪造身份，也不静默丢弃。控制面按 Branch 保存每次模型用量，不再用 action id 作为唯一键，因此同一 Branch 的多次未配对调用都能保留并计入结算成本。

## 2026-11-10：对话观察移除 unverified 字段

废弃 `GET /api/branches/{branch_id}/conversation` 响应与 `conversation_updated` 事件中的 `unverified` 字段，以及“候选内容必须明确标记为未验证”的旧表述。候选是否属于投机由 Branch 的角色（Authority/Candidate）与状态完整表达；对话接口改为返回该 Branch 当前的 `role` 与 `status`，前端据此显示投机身份。旧字段名仅保留在本记录。Agent Adapter 能力验证语境中的“未验证”是另一件事，含义不变。

## 2026-11-10：状态目录重叠默认拒绝启动

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

## 2026-11-10：fork 的逐挂载 Chain / Flatten 布局与链深度上限

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

## 2026-11-10：第三轮特权失败修复与证据

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

## 2026-11-10：第四轮特权验证通过与范围

用户 2026-11-10 第四轮验证中 Chain、Flatten、mixed、direct-root 四测试逐项 ok，
4 passed、0 failed、0 ignored；三项 btrfs 场景均选中真实 btrfs，
`incremental_diff=true fast_delete=true`。状态目录位于根 btrfs 挂载，workspace 在
独立 `/tmp` tmpfs，默认非重叠布局成立。7 项进程测试全通过、0 failed、0 ignored，
自动机制为 cgroup，强制 process_group fallback 也通过。原始子卷创建、只读快照、
增量 send dump、严格删除及初始 userns 直接 lower=/ 均通过；脚本退出 0，
全部 PASS，最终 `ALL PRIVILEGED CHECKS PASSED`。

亲读 `tests/privileged.rs` 与 `tests/support.rs` 确认：Chain/Flatten 检查实际布局、
真实进程修改/删除/新建、含父目录的精确 diff、accept 发布及后续真实视图与清理；
mixed 检查两代 Chain→Flatten、accept 回 child1 且 root 不变及清理。accept 传入
`target_mount_pid=None`，不是运行中 live-target 重放验收；不外推全部内核生成的
overlay 元数据、特殊文件/ACL/ownership、偶发超时根因或物化完成。

废弃的 `priv_btrfs_fork_diff_accept_discard` 已由 Chain/Flatten/mixed 三项替代，
不再作为当前测试名单；先前失败与修复过程留在本文件，正式状态以第四轮证据为准。

## 2026-11-10：删除用户日志与 FS 状态文档复查

用户明确授权删除根目录 `log.txt`、`log-2.txt`、`log-3.txt`、`log-4.txt`。
删除前已读取第四轮结果并与当前特权测试及 `652f60c`、`8c54403` 核对；当前文档
保留准确测试名、通过数、能力值和边界，不再依赖已删除日志的行号。上文旧日志名与
行号只作为失败/修复历史标识，原文件已按用户要求删除。

复查修正：状态目录重叠默认拒绝，只有内部测试/诊断开关允许 Flatten；小 upper
Flatten 只避免追加当前层，不会压平全部继承层，搬移状态目录也不会缩短既有链。
ProcessBackend 已承担启动、冻结、终止及 namespace 记录，不再将这些现成功能
列在未实现清单；跨 Backend 编排进一步统一与逃出进程组的后代仍保留为真实边界。
信号驱动 trash、元数据隔离失败关闭、diff 比较目标移除的既有决定保持不变。

## 2026-11-10：观察事件与增量读取

按已确认设计，弃用 `GET /api/run-view` 整体轮询及 `ui.rs` 每次 `state.tree = tree.clone()` 的整体替换。内部 SpeculativeTree 不再作为外部序列化模型；原始 Agent/Runtime/Ledger 消息不进入公共观察流。AScoutRunView 仅为从公共事件折叠的本地 TUI 视图；对话通过独立读取接口取得。现有简易页面改为首次事件快照 + SSE，后续 React + React Flow 前端尚未实现。具体接口与差距见 observation.md。

## 2026-11-10：状态事件集、Ledger 观察日志与 Tree 转换入口

废弃观察接口的 snapshot / “快照”命名、`GET /api/observation/snapshot` 和 `ObservationSnapshot`：它们读取的是完整有序事件，不是被保存的状态。当前统一称状态事件集，英文直译为 `StateEventSet`，路径为 `/api/observation/state-event-set`。

废弃 ui.rs 独立保存观察日志及通过 Tree 前后差异推断事件的做法。Ledger 在 events.jsonl 中以 `message: observation` 区分观察记录，直接提供只读句柄；Tree 在创建、生命周期、Authority 和环境转换入口产生待发事实，控制循环交给 Ledger 追加。保留同一次处理内的中间状态与入口明确给出的原因。观察日志不承担崩溃恢复整棵树的要求，不逐次保存完整树，也不实现折叠缓存。

## 2026-11-10：投机起点与工具开始分离

废弃“用 `SpeculativeAdmissionRequest.timestamp_ms` 合成 `ActionStarted` 作为工具开始时间”的做法：准入请求只登记 action 身份并表达投机起点，工具开始时间只来自接入侧真实的 `ActionStarted`。某 action 只有准入请求而尚无 `ActionStarted` 时，`tool_finished.started_at_ms` 为 null，耗时在起点和终点都可得时才计算。被准入投机的起点由 `branch_created.speculation_started_at_ms` 表达，取自对应准入请求的接入侧时刻；事件公共 `timestamp_ms` 仍是控制面记录时刻。`model_usage` 与 `tool_finished` 增加源侧 `started_at_ms` / `finished_at_ms`；`model_usage.action_id` 由可空改为必填字符串。

## 2026-11-10：结算的实测/估计分离与未命中时间语义

废弃 `settlement` 每个维度的 `{value, measured}` 形状（一个带符号数字加一个布尔标记），以及“未命中时间 = −(候选从投机起点到被丢弃的存活时长)”的语义。当前 `time_ms`、`money` 都是 `{measured, estimated}`：实测与估计各自独立，永远不相加，也不用其中一部分填充另一部分；未命中只记投机给整场运行（权威路径）造成的额外延迟的负值，不记候选存活时长或快速模型自身的“白耗”，没有可靠测量时对应部分为空。目标语义见 `docs/design/control-plane.md` §5，实现与数据缺口见 `docs/development/observation.md`。

## 2026-11-10：控制面 WebUI 的构建与嵌入

废弃 `src/bin/ascout-control-plane/web/index.html` 手写单页。浏览器界面改由仓库根目录 `webui/` 的 React + React Flow 应用实现，Vite 构建产物提交在 `webui/dist/`，由 `web.rs` 用 `include_bytes!` 嵌入二进制；`cargo build` 不依赖 Node，前端源码变化时重新构建并提交 `dist/`。控制面静态资源路由固定为 `/`、`/assets/app.js`、`/assets/app.css` 和 `/favicon.svg`；CSP 的脚本来源改为 `'self'`，不再需要内联脚本。界面只展示权威窗口对话，投机面板只读，结算的实测与估计分别显示；连续 `conversation_updated` 流式块在投机树中合并为一个就地更新的节点。实现说明见 `docs/development/web-ui.md`。

## 2026-11-10：Edge 改为抽象模型，由 Chance 取代“有 id 的机会对象”

废弃“Edge 是有 id、可枚举、被 `select` 选择、出现在 AssumptionSet（`Assumption=(edge_id, proposal_id)`）里的对象”以及“Edge 即投机机会”“嵌套 Edge 有自己的 Proposal、Branch 和过程状态”等表述。Edge 现在只是描述投机如何进行的抽象模型，不是运行时对象；其具体实例称为 Chance（被 `admit` 接纳的投机机会），从 `admit` 通过起拥有 id，直到被验证后接受或抛弃为止。待选集合、AssumptionSet 引用和运行时 Decision 作用对象均为 Chance。

## 2026-11-09：物化责任边界与 Firecracker 清理

控制面只报告操作间隙，Runtime 自行决定内部整理；控制面不持有物化专用语义。
撤回 materialize 提案中控制面触发 `FilesystemMaterialize`、新增物化状态/确认
字段、由控制面选择物化冲突策略的建议。本轮只同步文档，不实现新接口；宿主锁、
冲突处理及环境保留仍待后续明确。

Firecracker 停止未确认不标记 Terminated、不丢 Child、不拆资源；清理失败保留环境
和网络所有权供重试，全部清理成功才释放。启动失败回滚适用同一规则。生命周期
操作暂串行化，环境锁覆盖同环境操作，避免清理、重建与旧状态更新竞态。

## 2026-11-09：物化可选、宿主写回要求与后台回收（物化可选及冲突未定已废弃）

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

历史范围说明：上段为当时实现，启动承载已由 2026-11-15 的宿主 Pi／环境 holder 分离取代；2026-11-17 起 `invoke_tool` 对 `read/bash/edit/write` 改走 Runtime 内部文件工具适配器，其余工具仍委托 inner。generic operate 与 tool_context 仍委托 inner，不重复执行同一业务命令；当前路径见 [Runtime Adapter](runtime-adapter.md)。

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

## 2026-11-05：控制状态归树与消息触发

用户确认：task 创建时建立树，具体投机状态归树；Controller 不持有 task 状态，可协调多棵树。Policy 为决策函数，树的方法负责合法转换和后续操作要求。消息／定时器触发同一 task 顺序处理，耗时操作异步返回。Gateway 的统一范围暂未决定。

废弃实现：AgentController 持有 ControlState 和编号，及主循环每 100 毫秒扫描、同步调用 Policy／Runtime 的组织方式。此前 start_candidates、advance_candidates 与同步回调包装由树的事件处理和无状态协调替代。未经用户同意不提交。

## 2026-11-04：Runtime 整体边界与首阶段范围

已收敛：AScout 自身支持一套完整 Runtime，承载 Agent 一步动作，使其正常、正确运行，并把对真实世界或 Authority 的影响限制在容忍范围内。Runtime Adapter 与 Effect Coordinator 是这个整体中的接入和效果管理职责，不是平级的独立运行系统。Agent Session 内部管理仍归接入侧，Runtime 负责动作承载和外部效果边界；状态可以复制、共享、写时复制、重建或恢复，优先低成本方式。

能力可插拔、组合或缺失，但必要保证不能缺失。第一阶段先实现覆盖 Agent 及其派生进程文件操作的 Overlay 文件系统，不要求 VM；Overlay 本身不是 Runtime 的固定必需项。后续才展开 Runtime 与 Policy 的协调、协商和运行时检测。该决定最初只更新文档；文件系统 Backend 其后已实现 overlay/btrfs 数据面、逐挂载 overlay、live mount accept 与进程 Backend，当前覆盖范围和偏差见 [文件系统 Backend 当前实现状态](../fs-backend/current-state.md)。

**废弃记录**：不再采用“Runtime 仅是 AScout 不拥有的外部后端”“Runtime Adapter 是独立一级侧面”“外部效果执行只属于控制面而不属于 Runtime”的架构定义。此前四侧路径中的 Adapter 现在作为 Runtime 的接入边界；效果协调归入 Runtime 整体责任。这里改变的是职责定义，不预定 crate、进程、transport 或部署方式。既有 Branch、状态可见性、commit/discard 和 single-winner 规则保持不变。

目标说明见 `docs/design/runtime.md`；首阶段与未实现能力见 `docs/development/runtime-adapter.md`。本节取代本文其他历史记录中关于上述架构边界的表述。

## 2026-11-03：Ledger 接管后的归档失败所有权

废弃此前“不新增 Ledger 失败语义、错误直接返回调用方”的表述：它只说明了错误如何传出，没有说明已经移出活动 Tree 的 Branch 由谁保存。按值移交后若追加失败，Branch 会随调用返回被释放，历史在任何组件都不负责的情况下消失。

现行规则：Branch 只有在 `Cleaned`、非当前 Authority、无未完成控制命令且无子 Branch 引用时才从活动 Tree 移交给 Ledger。首次归档写入失败后，在安全恢复完整行追加边界后重试一次（最多两次实际写入）；无法恢复时明确报告恢复失败及重试无法完成。最终失败先写入包含 session id、Branch id、失败原因的系统运行日志，经既有 UI Projection 通道报警，再释放 BranchRecord。不保留 unarchived 集合、不放回 Tree，不恢复已结束的控制状态。系统日志写入失败沿用 stderr 回退并保留身份和原因。

实现：Ledger 保存失败前的追加边界，仅在确认文件及完整行边界后截除失败尾部并解除 failed 标志，再实际重试。普通观察写入仍不自动重试或发布失败记录；成功不包含强制 `flush`。回归通过注入首次零字节／部分写入失败、连续两次失败及 `/dev/full` 无法恢复，检查成功行、实际写入次数和 Projection 报警。已先确认旧实现首次失败直接返回错误。

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

## 2026-11-02：独立 Agent 接入核心的第一步

已确认并开始落实四层边界：控制面继续拥有 Speculative Tree、全局 Policy 和 Branch 生命周期；TypeScript 接入包承载 Agent 无关的协议类型、命令传输、命令去重、能力声明和局部执行 Policy；具体 Agent 通过 Adapter 提供会话与工具切点能力；Runtime 继续负责任务环境和工具效果约束。

第一步已加入 `pi/packages/ascout-agent-access`。公共协议只使用 `branch_id`、`operation_id`、`tool_call_id` 和不透明的 `CheckpointHandle`，不暴露 Pi transcript 或 session 类型。`CheckpointHandle` 只能由产生它的 Adapter 解释，公共层只保存和转发。

## 2026-11-02：Pi Adapter 与 Runtime 工具声明

已完成第二步：

- `AgentAccessCore` 真正驱动 Agent 连接和命令分发，并缓存重复命令的结果；同一 `command_id` 携带不同命令时返回 `rejected` 命令结果，而不是把协议冲突误报成 Branch 失败。
- `agent-access.ts` 只保留入口连接与生命周期，Pi 的 session、transcript、checkpoint、工具拦截和候选续跑全部移入 `PiAdapter`。
- Runtime Adapter 在 `Hello` 中声明 `RuntimeCapabilities`：可执行的 `RuntimeTool` 列表、是否共享底层环境、是否支持独立 `branch_environment`。
- 当时控制面不再识别 `nle_action`，按 Runtime 声明的工具名调用 `tool_context` 和 `invoke_tool`，用 Runtime 返回的 `result` 更新 Agent 工具通道，用 `evidence` 交给 Policy 匹配。此为历史实现：2026-11-17 已改为 Instrumentation 经独立连接执行工具并上报结果，控制面只查询上下文、授权和裁决，不再代执行或注入真实结果。
- NLE Runtime 声明 `nle_action` 工具；Candidate 仍使用控制面分配的独占 Environment，具体 workload 内部世界行为不暴露给控制面。
- Pi Adapter 从 `ASCOUT_RUNTIME_TOOLS` 读取 Runtime 工具描述并动态注册；它不知道 NetHack、动作编号或 NLE 语义。
- 控制面在 ToolPending 后立即发送一次 `resolve_pending`；候选处置阶段只决定 Candidate 生命周期，不再重复发送同一 `command_id` 的 resolve 命令。

验证：`npm run check`、`cargo check`、`cargo test --lib --bins` 通过。本地脚本模型加真实 Pi 工具完成一次 Promote 和旧 Parent 清理，并在 Promote 后继续下一轮。真实模型 DeepSeek V4.1 Flash 加真实 NLE 完成 10 次动作：9 次 Promote、1 次 Discard、10 次 Discard／Cleanup 命令 applied、0 次 `branch_failed`，总花费约 $0.0060；候选 Promote 后继续执行，旧 Parent 被清理。

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
- **共享 Backend 实例不等于共享 Environment**。NLE bridge 可在一个进程内维护多个独立 World，并按父 World 的初始记录和公开动作历史重建子 Environment；作用前阻止仍是 Candidate 工具暂存策略，而不是用共享 Environment 伪装 `branch_environment`。
- **NLE 只是测试用例，不属于 AScout 实现**。NLE 的 bridge、策略和运行环境作为外部 workload／适配保留，不进入核心 crate 的能力边界；核心契约不得出现 NLE 专用语义。
- **AScout 不与 Pi 耦合**。Pi 是首个接入实现，不是架构组成；设计文档不得把 Pi 写成 AScout 的前提，控制面也不应把 Pi 的路径、工具或会话结构固化进核心协议。

当时实现差距：`src/runtime/backends/nle.rs` 仍在核心 crate 中。该阶段控制面不对 NLE 调用 `branch_environment`，按声明的共享环境及 `tool_context`／`invoke_tool` 工作；这些路径已由后来的 Environment 独占绑定及 2026-11-17 Instrumentation 独立执行连接取代，不作为当前规则。Pi 接入的启动已按 2026-11-02 决策移入 Runtime Adapter。

## 2026-10-31：NLE 投机改为动作边界暂存，废弃进程级 fork 分支

已确认环境不可修改：不为适配 AScout 在 NLE 或任何外部 workload 侧增加克隆、分支或回放能力。先前在 `examples/nle-runtime-bridge.py` 中用 `os.fork()` 复制环境进程、用 socketpair 建私有通道、失败后反向移动回退的方案废弃。废弃原因：

- fork 让 workload 脚本承担了分支语义，而隔离策略本应属于 Runtime；
- 它只能从根环境分支，从已 Promote 的候选再分支会得到没有游戏对象的空壳；
- 子进程异常退出时会掉回父进程主循环，产生重复响应；
- 单世界下反向移动仍消耗回合，怪物会行动，“可逆”只是位置可逆。

CRIU 一类进程快照同样不采用：需要特权、镜像 I/O 和 FD 重绑，每步一条分支不划算。

现行实现：NLE bridge 可在同一 Backend 进程中维护多个独立 World；`branch` 按父 World 的初始记录和公开动作历史重建独立 Environment，Backend 进程和依赖仍可共享。Candidate 只在预测结果下推演一步，工具调用暂存到 Promote；动作前固定延迟（`action_delay_seconds`，当前配置 1 秒）；预测由固定程序从公开观测枚举，模型不负责预测。

## 2026-10-25：随机双 Candidate 与 Parent 不等待裁决

用户确认 Workload 与 Policy 分离：Parent 真值随机取 example/true/false，Policy 用非固定随机抽取两个不同预测；候选执行 Predicted 命令，Parent 默认继续 spec fail。Parent 结果交付不等待比较，只有 Promote 才取消旧 Parent。等待点按 Parent 和 operation 归属，不因一条消息有多个工具而整组拒绝；其他工具结果需真实配对，不允许重复其效果。Promote 后取消旧 Parent 下其余候选，保持 single-winner。工具准入不写入 Pi；由 Policy、AScout 和 Runtime 的能力边界共同决定。

废弃先前 afterToolCall 等待 ResolvePending、单候选容量、固定 example 预测及以单工具数量作为准入条件的实现。等待点准入和结果匹配在示例 Policy；Pi 不按工具名或命令格式筛选 Candidate 工具。当前候选结果交付和非标准上下文的范围见 [Agent 接入](agent-access.md#4-session-持久化与状态交接已定方向及实现差距)。

随机场景位于独立脚本模型接口，不是控制面业务逻辑，也不是大模型成功证据。已有脚本命中实测确认两个候选先执行、Parent 真值返回、单一 Promote 和 loser/Parent 清理；既有在飞输出验证与本次随机场景验证应分开描述。

## 2026-10-25：TUI 与实际投机过程观察优先

先实现稳定的 AScoutAgent TUI，由 AScout 接收 Agent 输出和运行事件并呈现，输入与中断经对应 Agent 接入实现传递；产品差异留在接入实现，不依赖 Pi 原有主界面。增加当前 AScoutAgent 整棵 Tree 的只读观察，可选择具体 Branch 深入查看。先使用简单环境，不把接通完整 Runtime Adapter 环境管理作为前置条件；不据此声称已实现完整环境隔离。实际启动、执行、预测比较、Policy 处置与清理必须有事实依据，不能用两个独立模型请求或发送命令代替完成证据。当前已完成自有 TUI 与受限单工具真实等待点续跑，具体可用范围以 reference-workload.md 为准。

用户已批准 `/ascout` 为 AScout TUI 内的整棵 Tree 观察操作，选择 Branch 只改变观察对象，不切换主对话或触发生命周期操作；命令由 AScout 处理，不作为提示词发送给 Agent。该名称与含义记录于术语表。

废弃助手提出的“复用 Pi 主界面”的实现方向；目标是整合原 Agent 的体验，而非把 AScout 用户端绑定到 Pi TUI。首版仍接入 Pi，不同时实现其他 Agent 的适配。

首步实现：现有 `ascout-control-plane` 开发入口改为 AScout 自有终端界面，保持 socket 接入、控制面 Tree 和连续 Pi AgentSession，加入流式输出与只读 Tree/Branch 观察。废弃该入口原先在 Branch 启动时直接发起无共同等待点的候选、把模型最终文本作为 ParentOutcome 的演示路径；接入侧不再产生该占位结果。退出保留运行目录和诊断文件，不声称已确认所有工具后代及环境清理。

本轮连接真实投机：ToolPending 报告后让真实工具继续一次，复用既定 ParentOutcome 名称表示真实工具结果，不另引入 ToolOutcome。Candidate 使用独立目录与独立内存 SessionManager，保存共同上下文与预测 toolResult 后 continue，不以新用户提示词伪造续跑。控制面求值 admit/select/predict/launch/match/dispose，在合法 Promote 后发布缓冲输出并保持活跃 Candidate，不命中回退 Parent。未验证 Candidate 的工具调用由 AScout 的 Branch 处置控制：Pi 只等待控制命令，不按工具名或命令格式判断安全性；Runtime 能力与 Policy 决定仍需由控制面接入。默认示例不自动开启投机，新增可显式选择的 pi-tool.yaml。实测使用本地脚本模型接口加真实 Pi/bash，不能作为真实大模型效果证据。

废弃 D-031-I 中“用户不可直接观察候选”的限制；保留主对话只属于权威路径、候选不建立用户对话、用户不直接裁决 Branch 的边界。只读观察明确标记候选内容未验证，不向其他 Branch 反馈这些内容。

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

## 历史：早期实现决策（D-0xx-I）

以下条目保留早期实现阶段的决策与废弃记录，编号沿用当时的 `D-0xx-I`。当前语义以正式设计文档为准。

### D-029-I：废弃单 Edge 即时启动与固定 frontier 策略

2026-09-04 决定：废弃由 `admit` 精判直接决定投机是否立即启动的七槽决策面，现由 `admit` 决定机会是否进入待选集合，独立的 `select` 在同一 task / AScoutAgent 的全部待选 Edge 间选择当前启动项。也废弃将 best-first、固定的 `P(存活) × 期望延迟节省 / 资源成本` 排序、浅层分叉/深层单链或强制 `survival` 字段写成 AScout 机制规则的表述；这些机制作为 Policy 可选策略继续受支持。当前语义见 `docs/design/policy-decisions.md`、`docs/design/control-plane.md` 与 `docs/design/decisions.md` D-043。

### D-030-I：废弃 Scheduler 固定暂停与抢占策略

2026-09-04 决定：废弃“所有投机工作固定低优先级”以及 Scheduler 根据内置价值排序自行暂停或抢占投机工作的表述。Scheduler 继续拥有运行队列、资源和硬约束，但暂停、恢复和价值驱动的抢占由独立 `rebalance` Policy Decision 选择；抢占后的处置由 `dispose` 决定。当前语义见 `docs/design/policy-decisions.md` §2.9 与 `docs/design/decisions.md` D-044。

### D-033-I：Speculation Coordinator 吸收 Scheduler 的控制面职责

2026-09-04 决定：在控制面组件模型中，不再将 Scheduler 作为独立组件。其待选机会、运行调整、预算和硬约束执行职责并入 Speculation Coordinator；`select` 与 `rebalance` 仍由 Policy Decision 决定，Speculation Coordinator 在统一的全局投机视图上负责激活、复核和执行这些意图。D-030-I 中关于 Scheduler 的描述保留为历史决策记录，当前组件结构以本条和 `docs/design/control-plane.md` 为准。

### D-031-I：废弃端用户直接参与投机 Branch 裁决

2026-09-04 决定：废弃通过专门投机 UI 让端用户直接观察、选择或裁决内部 Branch 的表述。端用户只通过 AScoutAgent 视图操作当前有效的权威执行主线，并只能看到该路径的内容、状态与交互；人作为 Authority 提供输入或中断时，也通过该主线进入系统。Branch 接受、丢弃、重新挂载与清理在内部完成，不要求用户选择或重绑定 Branch。当前语义见 `docs/design/agent-integration.md`、`docs/design/branch-semantics.md` 与 `docs/design/decisions.md` D-045。

### D-032-I：前提变更暂不兼容或修复 Branch

2026-09-04 决定：废弃在用户操作改变 Proposal 前提后继续保留、挂起、兼容判断、修复或重新对齐受影响 Branch 的候选方向。现阶段所有依赖已变更前提的 Branch 及其 subtree 一律剪枝；未依赖该前提的 Branch 不受无关输入影响。当前语义见 `docs/design/branch-semantics.md` §6、`docs/design/policy-decisions.md` §4 与 `docs/design/decisions.md` D-046。

### D-034-I：抽象札记收敛为论文证据

2026-09-14 决定：`spec-exec-abstraction.md` 只保留论文覆盖、实验结果、负结果、效果处理现状、shadow/replay 方法学、接入与隔离事实、嵌套投机事实及待验证问题。已经在 `docs/design/` 正式呈现的生命周期、Policy Decision、Effect、架构、组件职责、契约和指标方案从该文删除；该文不再作为目标语义或实现结构的来源。

### D-015-I：首个真实 Workload 使用 Delayed-Choice Coding

首个真实端到端 workload 通过 Product Integration 的观察和拦截接缝接入 Agent，使用真实仓库。不要求 Agent 主动调用 AScout，也不开放显式生命周期协作。具体产品和接入方式尚未选择。Sequential 与 speculative run 使用相同 Agent、任务、outcome 和预算。

**废弃记录（2026-09-17）**：原先使用 `ascout codex`、Command + Skill，由 Agent 主动调用命令并先验证显式路径的路线错误，已废弃。确定性测试 Agent 专用于 conformance tests 的初期测试安排也不再采用，初期测试范围见 D-035-I。

Workload contract 与指标见 [reference-workload.md](reference-workload.md)。

### D-035-I：控制面最小闭环与初期实现约束

2026-09-17 确定：先实现控制面最小闭环。保持代码简洁、干净，避免为尚未需要的扩展增加抽象和占位实现；能以简短清晰代码解决的问题不引入冗长框架。

2026-10-23 更新测试约束：原型阶段默认不编写测试。只有实际遇到可复现、且今后很可能再次出现的问题时，才针对该问题编写测试；先让测试稳定复现问题，再修复代码直到测试通过。已有测试不因此自动删除，真实执行验证也不等同于必须新增自动化测试。

**废弃记录（2026-10-23）**：不再采用“设计初期先定少量端到端测试、须经用户同意并在业务逻辑实现前定稿”的普遍门槛。早先等待 test 通过／失败的两个 Proposal、确定性 FollowUp 及两个控制面场景的首批测试安排不再作为实现前提。

早期 `tests/test_outcome.rs` 及其确定性替代实现已经删除，不再作为当前正确性证据。当前 Runtime Adapter 端到端测试直接启动真实 Firecracker，验证 microVM 建立、guest 内操作、物理暂停与恢复、终止与清理、请求重试去重、在飞操作打断、意外停止事件，以及 snapshot/restore 分支和 sibling 隔离；SSH 诊断同样使用真实 virtio-net 与 guest sshd。当前已实现 task 身份校验、Proposal 独立存储和待选集合；仍未实现全局唯一 Proposal identity、九类 Decision 的完整接通、fallback、嵌套树、效果处理和主动环境验证。当前 Rust 接口仍是实验接口，不是稳定对外契约。

Adapter Registry 的主动环境验证延后实现，初期先检查接入接口与契约的对应关系。这是原型限制，不改变目标契约：接口检查不构成隔离保证的实际验证，相关能力不得标记为已验证。尚未实现的差距是 Registry 自主建立环境、执行验证场景并形成验证证据。

### D-047-I：重做由控制面拥有 Agent 状态的 Pi 最小接入

2026-10-24 确定：此前位于 Pi `packages/coding-agent/src/core/ascout.ts` 的 `SpeculativeTree`、`BranchRecord`、`ControlState` 和根目录一次性 `ascout.ts` 驱动不符合责任边界，统一作为废弃实验材料，不在其上继续扩展。它们把 AScout 的 Tree、Branch 生命周期、Policy 落实和用户入口编排放进 Agent 接入侧，不能作为正式系统或端到端完成度证据。

后续实现由 AScout 控制面拥有每个 Task 的 Speculative Tree 和 Branch 状态，控制面创建、登记、推进、接受、丢弃和清理 Agent 执行；Pi 只通过最小 Product Integration／Interposer 接入，报告 Agent 行为事实和 outcome，执行控制面返回的命令，不维护 AScout Tree、Policy 状态或生命周期裁决。控制面与 Pi 之间的通信必须传递受契约约束的事实、命令、结果和事件，不能以 Pi 的本地对象作为控制面状态。

第一版不等待完整 Runtime Adapter：候选 Branch 使用 AScout 运行管理建立的独立目录作为最低程度的文件系统隔离，目录创建、可见范围、清理和 Branch 归属由控制面／运行管理落实。仅修改 Pi 的 cwd 或在 Pi 内创建目录不构成已实现的隔离。

根目录一次性启动程序不得作为系统入口。测试可以是 AScout 内部测试，或从外部模拟用户行为的端到端脚本；测试脚本不能承载正式 Tree、Policy 或 Agent 管理职责，也不能把固定 `choice` 场景包装成通用系统能力。当前旧原型的真实模型运行只保留为历史实验记录。

首个实现步骤采用已确认的最小名称 `AgentMessage`、`AgentCommand`、`SpeculativeTree`、`BranchRecord`、`ControlState`、`BranchRole`、`BranchStatus` 和 `AgentController`。它们位于 AScout 控制面代码中：控制面创建 Branch 工作目录、处理 Pi 报告的启动和 outcome，并生成 Promote、Discard、Cleanup 命令。内部 Rust 测试验证控制面状态转移和目录隔离；随后已通过 `ascout-control-plane` 与 Pi `agent-access.ts` 的 Unix socket 连接完成真实跨进程验证。

已完成的下一步（2026-10-24）：旧 Pi-local 实现和一次性驱动已直接删除。Rust 新增 `ascout-control-plane` 二进制，Pi 新增最小 `agent-access.ts` 接入；二者通过长度前缀 JSON Unix socket 通信。控制面负责调用 Policy 的 `admit`、`select`、`launch`、`predict`、`match` 和 `dispose`，并且只有 `dispose` 的 Promote 意图才会触发 Tree Promote。真实运行已验证两个跨进程 Pi AgentSession、真实 `deepseek/deepseek-flash` 模型请求、Candidate 独立目录、控制面 Tree 更新和旧 Authority 清理。

该实现随后被发现仍有三项边界错误，不能作为正确闭环继续扩展：Pi 接入由用户手工启动而非由控制面启动；控制面把目录直接当作独立执行环境；控制面用 Candidate output 而不是 Parent 的真实 outcome 作为 `match` evidence。后续实现必须由 AScout 用户入口向控制面提交请求，由控制面启动 Pi 和 Agent 接入；由 Runtime Adapter 创建并报告抽象的独立 Candidate 环境，当前目录只作为该 Adapter 的实现细节；并由 Parent outcome 与 Candidate hypothesis 完成 Policy 匹配。

### D-036-I：Policy DSL 首版编译为 Python 脚本

2026-09-17 确定：PolicyDefinition 只通过 Policy DSL 定义；九类 Decision 都需要根据各自的输入作出规定输出，每个位置必须具有明确行为。固定取值和常用规则只是决策逻辑的快捷表达，例如 Eager 编译为恒定返回 `true` 的 `launch` 决策。

首版 Policy Engine 将 DSL 编译为可交付的 Python 脚本，并通过运行脚本对已编译的 Policy Decision 求值、作出本次决定。控制面可以直接调用 Policy Engine，也可以按需查询编译结果并将脚本交付给 Agent 接入侧，由接入侧以相同语义求值；Policy Engine 不负责落实决定、选择运行位置或传输。脚本是内部编译产物，不是另一种用户入口，也不能包含控制面内存对象或仅在编译进程内有效的引用。

2026-10-23 确定编写形式：YAML 描述 Policy 的结构、各 Decision 的行为和配置，并通过文件路径引用独立的算法源码。首版算法语言仅支持 Python，九类 Decision 暂时分别显式指定；不能通过省略某类 Decision 静默补齐行为。YAML 和被引用源码共同组成一个 PolicyDefinition，不增加独立算法加载入口，也不将自定义策略限制为内置算法菜单。源码语言与编译产物是独立选择，具体库范围和调用方式仍待确认。

2026-10-23 确定参数作用域：用户可在 YAML 中为各 Python 函数分别指定算法参数，传给对应函数；首版不支持全局共享参数，不设全局／函数配置的继承、合并或覆盖规则。参数与系统提供的运行时数据、授权能力分开。具体字段布局和调用约定仍待确认。

系统按每类 Decision 的约定提供数据，算法只在规定的数据及授权能力范围内计算，返回值经检查后由状态拥有者落实。源码路径在编译时解析并固定所用代码、明确依赖；运行期使用编译结果，跨侧交付不能只传作者机器上的原始路径。这里确定的是职责和交付边界，尚未确定具体 YAML 字段、函数入口或依赖打包方式。YAML、参数限制和静态检查都不能替代真实执行权限边界。

这一选择用于优先验证 Policy DSL、九类 Decision 的输入输出契约和跨侧运行路径，不把 Python 固化为长期执行载体。后续更换执行形式仍须保持已选语言语义和 Decision 接口，不能假定源码自动具备跨后端可移植性。

2026-10-23 用户授权开始实际实现，并确认 `policy`、`PolicyEngine`、`CompiledPolicy` 和 `Decision` 的代码命名。现有实现已打通 YAML 检查、独立源码固定、各函数参数传递、可序列化产物与按 Decision 导出 Python 脚本、隔离加载检查、实际求值、基本输入输出检查和失败返回。部署使用 bubblewrap namespace、用户 import 前 seccomp、rlimit、限量临时空间及受信调用侧超时监督，不使用普通 Python 子进程冒充隔离。实际遇到 bubblewrap 不支持 `--preserve-fds` 后，先新增真实环境的回归测试复现，再修复管道启动方式。

尚未实现的内容包括完整领域 schema 和 Contract／Fidelity 检查、外部模型等能力调用、第三方库环境固定、标准库内容身份、增量预测、score 规则、多数效果与处置计划检查、异步取消、cgroup 总量统计、生产级交付认证以及状态拥有组件和端侧真实接入。未知 YAML 字段和不支持的结果会被拒绝，不静默降级或伪造成功。具体 API、运行方式、数据清单、验证与缺口见 [Policy Engine 原型](policy-engine.md)。

开始实现不表示其全部候选能力已完成。代码中实际采用的原型接口以开发文档为准，不把尚未实现的草案能力写成可用能力。

### D-016-I：默认 Runtime 优先高效隔离

默认 Runtime 面向可信 Child Agent 与工具，优先低开销环境隔离，不提供 adversarial containment。它必须强制维护声明的环境、资源和效果边界，以及停止和 cleanup。Agent Session、执行实体的本地上下文、operation 归属和 capability 路由由 Agent 接入侧管理。

**废弃记录（2026-11-04）**：此前曾以“Runtime 只提供环境能力”为由，把外部效果实际处理移到控制面。此边界已被本文顶部的 Runtime 整体边界取代；Session 仍由接入侧管理，环境与效果执行归 Runtime，Tree 与全局裁决归控制面。

需要 adversarial containment 时，用户选择另行适配的 sandbox/hardened Runtime。AScout 不协商 security guarantee；当前首个实现为文件系统 Backend（overlay/btrfs）与进程 Backend，见 [文件系统 Backend 当前实现状态](../fs-backend/current-state.md)。

### D-017-I：新 Rust 实现不继承旧代码

新实现继续使用 Rust。旧 `src/**`、Cargo manifests、tests、examples、C header、构建产物和实验文件已经删除，未读取、迁移或复用。新实现只能依据已接受的设计与开发决策建立。

### D-024-I：实现组织服从 AScout 可控边界

当前实现组织遵循 Agent 接入侧、控制面与 Runtime 的责任边界。Runtime Adapter 是 Runtime 的接入边界，Effect Coordinator 属于 Runtime 的效果协调职责；这些职责不直接推出 crate、进程或部署数量。

Execution、Workspace、Input、Effect、Outcome 与 Interposer 的职责语义不绑定具体 crate、模块、进程或服务边界。

后续 crate、模块、进程和服务边界必须依据责任、依赖方向、权限、替换频率与部署需求决定，不能由“一侧一模块”或“一组件一 crate”直接推出。具体 Adapter transport、discovery、连接生命周期和 Runtime backend 属于 Adapter/部署实现选择。

**废弃记录**：此前先要求“实现组织服从四侧一级架构”，随后又把 Runtime 定义为 AScout 不拥有的外部后端、把 Effect Coordinator 放在控制面。两种解释均已由 2026-11-04 的 Runtime 整体边界取代；不再以“四侧”推导源码或文档拓扑。

### D-025-I：项目名称统一为 AScout

2026-08 决定：项目现行名称统一为 **AScout**（Agent Scout），相关现行复合名称使用 `AScoutAgent`。正式设计、架构、使用和开发文档不再使用 `ASCout` 作为现行名称；本条仅保留旧拼写用于历史追溯。该名称变更不改变既有生命周期、Policy、Runtime 或接口语义。
# 2026-11-16 交互实现纠正

废弃助手此前写入的“TUI 提示回答暂缓、纯 TUI 只能持续等候”及“输入 / 时重新请求 Pi 清单”的表述。当前决定是适配 AScout 终端文字提示；命令清单在 Pi 初始权威 session 完成扩展绑定与资源发现后读取一次，重连和换权威复用缓存。没有新的自动过期、轮询或 Runtime 工具取消设计。


### 运行身份命名统一（本轮确认）

AScout 管理的运行身份命名为 run id，保存位置称“运行记录目录”；CLI 使用 `runs`、`--resume <run-id>`，启动和结束输出 `Run id`，新目录使用 `run-` 前缀，系统日志归属字段使用 `run=<id>`，配置仍为 `runs_dir`。旧 `session-*` 是不透明的既有 run id，按原 id 恢复，不迁移目录或改写 Agent 引用。Agent 原生 session/session id、AScoutAgent 的会话概念与 workspace 保留；run id 不等于 task id，新用户 prompt 仍创建新 task。本次不改变 task 机制，也不增加常驻多运行服务、EEC 或已结束会话打开命令。此前决策中的 session 命名保留为历史表述，以本条和当前正式文档为准。

本轮补充确认了从已结束的 Agent session 继续工作的规则，以及常驻 control plane 管理多个 run 时关闭界面、结束指定 run 和停止整个进程的收尾范围。会话沿用原权威引用、run id 和运行记录目录，下一条 prompt 创建新 task，历史终态保留。具体规则见 [control plane 设计](../design/control-plane.md#1-总体设计)，实现进度见 [恢复开发记录](recovery.md)。

后续确认并实现：同一个 `ascout-control-plane <pi-root> --resume <run-id>` 根据 Ledger 打开已保存暂停的 task 或已结束的权威会话。已结束会话以原历史启动新的控制上下文，等待下一条 prompt。运行目录内的持有锁保护同一 run 的会话和记录，原运行身份沿用。验证和兼容路径见 [恢复开发记录](recovery.md)。

### Runtime 宿主与环境资源归属

`AgentBackend` 管理 run 唯一的 Agent 与 Instrumentation 宿主，统一所有配置的启动和退出。`FilesystemBackend` 管理环境文件视图、holder、冻结与恢复、挂载轮换和清理。Runtime 通过 Backend 接口调用这些环境操作，保留现有配置和调用顺序。声明工具经授权后使用独立执行端点，控制协议保留 `tool_context`。更大的组件重组、通用顺序和晋级交接待讨论。

### Runtime 组件命名与关闭顺序

已确认 runtime proxy、backend registry、base runtime、agent supervisor 和 External Effect Coordinator。runtime proxy 对接 control plane。启动入口读取并规范化节点配置。backend registry 登记实现、入口、接口和能力声明。base runtime 使用自己的路由配置创建和回收实例，管理 Environment、参与者、操作和结果。agent supervisor 接替上条的 `AgentBackend`，只在检查、启动或回收可信宿主时持锁。base runtime 编排宿主启动和环境执行资源启动。runtime proxy 停止接入并取消在飞工作。base runtime 关闭操作受理、停止宿主、排空操作，然后回收环境。尝试保存或恢复后进入保留模式，退出只停止执行资源。原型支持内置 Rust Backend 和按配置接入独立可执行程序。跨节点登记及通信待设计。

本次实现将两层协调收归 `BaseRuntime`，保留既有启动命令与内置恢复布局。独立程序使用版本化 JSON 行接口，配置与程序握手分别检查。无模型核查以构建完成后新写入的 Python 程序验证配置登记、父子环境生命周期、命令结果、业务拒绝及退出回收。缺少接口声明时没有启动程序，缺少 `cleanup` 方法时启动失败且子进程已回收。另用收到 `shutdown` 后不退出的程序验证了超时诊断、强制终止与进程回收。接口、错误处理与覆盖边界见 [独立可执行程序](executable-backend.md)。

## Runtime 执行和 backend 参与路径统一

Branch 只在 control plane 与 Instrumentation 中用于查找 Environment。Runtime 创建或恢复启动时交付访问地址。执行按 Environment 和 operation 关联，Environment 内保留请求去重、generation 与取消终态。内置与可执行 backend 按同一登记、参与、路由和清理规则运行。文件系统 holder、冻结及物化由文件系统 backend 管理。

独立程序的工具调用使用既有执行内容帧。工具声明到 Agent 的交付留在约定范围之外。普通工具取消按 Environment 和 operation identity 通知程序，Runtime 等待工具终态，保留共享实例。超过时限或通信失败时，Runtime 回收共享进程并将其所有 Environment 标为部分可用。无法确认的资源回收会报告失败。

验证通过 `cargo check --all-targets`、218 项 Rust 库测试、两项 FS 集成测试、12 项 Pi 执行与恢复测试，以及真实 Firecracker 的无模型工具测试。默认并发的首次全量库测试有两项 FS holder 挂载超时，限制为四线程后全量通过。旧组件索引的加载用既存的 NLE 与 FS 保存代验证。上一轮独立程序验收覆盖两个 Environment 的归属、失败建立的回滚、文件和命令的二进制帧与结构化工具结果。同一 Runtime 二进制分别载入原配置和增加程序后的配置，SHA-256 保持一致。本轮取消回归经 Instrumentation 的 ASC-Proxy 验证了相同实例 PID、取消与完成竞争、上传与输出分片、等待发送的取消、故障报告及关闭回收。

## 2026-11-18 用户启动与安装入口收敛

用户入口采用 `make install`、`make uninstall` 和 `make purge`。安装生成默认配置并同步内置 Policy、Agent 接入脚本和三个可执行文件。卸载保留配置与运行记录，清理命令才删除全部用户数据。两个清理入口通过 `run.lock` 拒绝影响活跃运行。

普通 filesystem 运行以用户启动目录作为 Agent workspace。控制面在 run 目录中生成带随机控制端口的 Runtime 配置和独立 `fs-state`，并将 workspace 写入 Tree 持久化数据。恢复和旧存档继续使用旧的 run-local workspace 兼容路径。单 run 内动态切换 Policy 或 Backend，以及上游超时重试控制仍待讨论，见根目录 `Discussion.md`。

## Coding Policy Unit 2A

启用 `fast_model_speculation` 后，`active_post_edit_test` 默认允许在修改文件后直接重复本任务最近的验证命令。候选在模型 checkpoint 上记录预测的工具调用，并在隔离环境中执行。开始验证无需模型推理，验证完成后可以使用 Authority 的模型继续。匹配且覆盖 Authority 工具调用的候选可晋级。已完成的测试输出（含失败输出）和去除已覆盖行的只读结果可以通过 `Salvage` 或 `Yield` 交付，不复制文件。

快模型探索默认关闭 thinking。候选范围由连续失败、重复操作、只读探索没有新增信息，以及 `max_steps: 12` 约束。源代码修改后的宽范围验证若已有覆盖当前修改的等效或更广验证，无论通过还是失败，都不再预测失败分支。通用 Python Policy 模块自动加入编译产物及其摘要。

### Sphinx `spx-s4-2` 回归

改后测试只针对尚未验证的修改。有效的失败结果与通过结果均能覆盖当前修改；已交付的推测验证在同一段 Authority 历史中也参与判定，未改文件时不重跑或重复交付。失败候选先查代码、调用位置和测试，再修改并验证。`/dev/null` 和可识别的临时路径不算源代码修改。安全结果默认按完整输出交付，不设字符上限；Coding 策略保留每批四项的限制。

# 待讨论事项

本文件是设计讨论队列，不代表其中内容已经成为设计决策。讨论完成后的目标语义应写入 `docs/design/`，实现细节应写入 `docs/development/`。已经下沉到权威文档的历史讨论不在此重复保留。

## 当前讨论

### 真实大模型多步任务的端到端验证方向（2026-10-28）

- 用户确认首个真正可用的端到端验证不能以单步任务为目标；目标应是一个真实大模型参与的多步 Agent 任务。实现时可以先截取其中一小段作为原型，但这段原型必须属于多步任务的真实流程，并能逐步扩展，而不是继续使用与实际 Agent 流程无关的静态 workload。
- 用户确认 Parent 保持 Agent 原有的能力和权限，不因接入 AScout 被额外收紧。只有 Candidate 需要增加额外控制；Candidate 的实际动作必须同时满足 Policy、Runtime 能力和 Agent 本身已有的权限，不能通过投机执行扩大 Agent 原有权限。用户未授权访问的文件等资源，Candidate 同样不能访问。
- 因此后续端到端用例必须同时验证：真实模型参与 Parent 与 Candidate 的多步推理；真实工具结果驱动比较；Candidate 的不支持动作被阻止且不伪造结果；Parent 不因 Candidate 受阻而失去原有执行能力；整个过程可观察。具体业务任务和第一段原型范围尚未收敛。
- 用户提出首个多步端到端任务采用二维地图解谜小游戏：开局隐藏大量地图信息，Agent 每次探索后才能获得该格及邻近区域的视野；怪物、陷阱、道具和地形事前不可见，不能在开局规划完整路线，必须根据新情报逐步调整。Candidate 的命中或不命中必须由游戏自身的隐藏状态和揭示机制自然产生，不能由测试额外故意制造。
- 当前适合的机制是：隐藏地图保持只读；Parent 与 Candidate 各自拥有当前位置、路径和已揭示区域等局部状态。一次探索产生真实的地图观察结果，Candidate 只能根据当时可见信息预测该结果并提前推理；没有新信息时下一步可能可由现有信息推导，有新信息时路线可能自然改变。Candidate 的探索若超出 Agent 权限、Policy 准入或 Runtime 能力，必须被阻止，不能访问隐藏地图或伪造观察结果。具体地图规则、工具接口和第一段原型仍待收敛。
- 用户同意优先尝试真实 NetHack + NLE；如果其接口或状态分支能力不适合当前验证，再评估 DCSS + dcss-ai-wrapper；两者都不适用时才制作一个借鉴真实游戏迷雾、二维地图、逐步揭示和回合机制的简化小游戏。NLE 实际基于 NetHack，不是 DCSS 的接口。用户指出：Runtime 的核心形态不能被概括为保存 Candidate 动作后接受才执行；这只是地图游戏和特定投机策略下的一种可选实现。Runtime 的一般责任仍是提供可声明、可观察、可停止、可清理的执行能力，具体策略是否暂存动作、建立分支状态或接受后提交，应由对应的控制面、Policy、Runtime 能力和 workload 共同决定。用户使用的“暂存器”目前只作为说明性比喻，尚未批准为正式术语。

### 优先实现可使用的 TUI 与候选观察（2026-10-25）

- 用户确认项目目标不变，当前优先级是先做出正确的用户体验，使用户能实际尝试一次投机执行，并能观察运行过程、确认候选是否真实执行；不是先完成完整 Runtime。
- Agent 接入侧管理具体 Agent/session/Branch 的本地执行上下文；控制面继续拥有全局 Tree、Policy 和 Branch 生命周期，不把控制权放回 Pi。
- 用户要求稳定的 AScoutAgent 入口，整合对应 Agent 的使用体验，直接接入当前主路径的输出；首版使用 AScout 自己的 TUI，不复用 Pi 主界面。AScout 接收 Agent 输出和运行事件并呈现，可调整展示；输入与中断经相应 Agent 接入实现传递。更换 Agent 产品时调整接入实现，不重写主界面；具体产品可保留的交互能力须如实说明。
- 用户要求增加观察候选状态的方式。这修改了原先用户只能看到权威路径的限制：主对话仍只展示当前权威路径；候选可通过单独的只读观察查看，并明确标记为未验证。观察不等于接受候选，不建立候选用户对话，不赋予手工转正或绕过 Policy 的权限；候选内容不反馈给其他 Branch 的推理上下文。
- 观察应区分预测、实际启动、模型或工具活动、真实操作结果、比较结论、Policy 处置和清理确认；未接入、未执行、等待、失败不能显示为成功。收到命令不等于执行完成，候选执行完成不等于可以转正。
- 环境管理暂不接通是有意的开发顺序，不作为本轮启动前置。先使用简单环境并明确保证范围；不同目录不等于任意 shell 或外部副作用已隔离。
- 用具体例子解释结果比较：Parent 运行真实测试，候选假设“通过”提前继续；测试结束后用真实测试结果判断预测，不用候选自己的回答证明预测正确。首轮真实体验仍须实现这条路径，不能用两个无共同等待点的模型请求冒充。
- 用词约定：不新增未经用户同意的名称；必要概念先解释、取得一致并记录。助手此前的“替代选择”不是项目术语，不采用；直接说“同一次投机的候选至多一个转正”。
- 用户已批准 `/ascout`：AScout TUI 内打开当前 AScoutAgent 整棵 Speculative Tree 的只读观察入口，展示 Branch 关系、当前主路径与投机状态，再允许选择具体 Branch 深入查看执行活动、输出及处置依据。选择只改变观察对象，不改变主对话路由，不接受或丢弃 Branch，不向所选候选发送用户对话；不是调用 Agent 自带的同名命令，也不是跨 Tree 查询。定义同步到术语表。
- 用户已批准持续交互通信名称：`Prompt` 给指定 Branch 发送用户输入；`Abort` 中断指定 Branch 当前执行；`Output` 报告该 Branch 的实际输出或工具活动；`TurnCompleted` 报告本轮执行结束，不表示可以转正；`CleanupCompleted` 表示接入侧确认已停止并释放会话，不代表环境目录已清理。它们扩展现有 AgentCommand/AgentMessage，不新增组件。首步接通真实持续对话与观察，尚未接通的投机路径明确说明，不制造候选或结果。
- 首步已实现：AScout 自有终端界面自动启动 Pi，按协议展示实时文本、工具活动和模型错误，保留 session 用于持续对话；`/ascout` 展示控制面 Tree，可按编号查看 Branch，观察不改变主对话。已有占位双会话路径已移除，不再把模型回答当作 ParentOutcome。Rust 编译、已有两个控制状态测试、Pi 完整 check 通过。真实 tmux 操作确认启动、输入、Tree 和 Branch 观察，以及中断后的结束事实；默认模型服务返回 503 `model_not_found`，未获得成功模型回答。真实投机、完整编辑排版、宽字符与退出确认式清理仍未完成，详见开发文档。
- 用户已批准真实等待点通信名称：`ToolPending`＝接入侧报告 Parent 已产生真实工具调用但尚未执行，带操作标识、工具名、参数和可继续上下文；`StartCandidate`＝控制面在 Policy 允许后要求接入侧从该上下文注入预测工具结果并继续 Candidate；`ResolvePending`＝控制面报告等待点的裁决，不再阻塞 Parent 结果交付；Promote 仍使用对应命令。它们是既有 AgentCommand/AgentMessage 的协议消息，不是新组件。
- 当前路径：`ToolPending` 后真实工具执行一次；原有产品钩子处理后报告 `ParentOutcome`，Parent 立即用真实结果继续，与验证并行。Policy 决定准入、预测、比较与处置；控制面保存 Parent/operation/Candidate 的明确归属，至多一个 Candidate Promote，其余及旧 Parent 被取消清理，保留胜出 session。`pi-tool.yaml` 无固定种子地从 example/true/false 中抽取两个不同字符串枚举预测；默认 Policy 仍禁用候选。布尔预测作为独立 JSON 类型支持，不能与字符串 `"true"` 混同；Pi 仅把二者转换为 Agent 工具所需的文本表示。标准上下文限制暂缓解决。并行工具逐项处理，候选补齐其他工具真实结果后续跑，不重跑效果；专项验证未完成。等待点准入和匹配已移至 Policy；复杂预测转换仍未完成。
- 用户确认的 Workload：Parent 随机产生 example/true/false，两个 Candidate 执行 `sleep 1; printf 'Predicted{n}: {预测值}'`；Parent 默认继续执行 `sleep 1; printf 'spec fail'`，仅 Promote 时停止旧 Parent。Workload 不等于 Policy，非固定随机不等于结果可确定复现。新增独立脚本模型接口承载该场景，不将业务命令写入控制面。实测两个候选在真值前完成、命中后单一 Promote 与 sibling/Parent 清理；脚本不是大模型成功证据。
- 验证：本地可控模型接口（脚本，不是真实大模型）通过真实 Pi、socket、TUI、Policy 隔离求值和真实 bash 工具运行，已验证工具返回前候选输出、返回后在飞 Promote、继续流式输出、保留上下文处理 follow-up，以及下一轮不命中 Discard、Parent 使用真实结果恢复、候选清理。模型接口与 Pi 配置位于 `/tmp/opencode`，不修改用户原配置。真实大模型成功验证仍未完成；当前服务此前返回 `503 model_not_found`。既有两个控制状态测试、Rust 编译与 Pi 完整 check 通过。`events.jsonl` 记录 Branch 消息、真实 outcome 与最终处置；退出仍不声称任意工具后代/外部资源已清理。
- 待完成：未验证工具的 Policy/能力协商、复杂结果转换与并行工具专项验证，详见开发文档；不在本轮展开更深层协议。
- 仅记录、暂不展开：非标准消息上下文的复制与恢复支持。
- 仅记录、暂不展开：用户希望外部副作用隔离以容易插入系统的轻量插件提供，使用感类似 Runtime。后续确认其接入与责任边界；本轮不据此把外部效果实际执行责任移到 Runtime 或新增一级组件。

### 用户交互与执行实体分离（2026-10-23）

- 用户提出：在用户与 Agent 执行之间增加稳定的用户端，用户不感知后端 session 变化；优先切换用户端对应的 Agent 执行实体，而不是切换或重建运行中的 session。不能等会话结束才接替，也不希望因接替中断候选的在飞执行。
- 用户已确认并要求实现：用户持续连接现有 AScoutAgent 视图，控制面完成合法 Promote 后更新该视图对应的权威执行和输入／输出路由；候选保留自己的 session 与在飞进展，不以结束候选会话作为 Promote 前提。不新增一级架构侧面或组件命名。根模型、架构及 Agent 接入／Branch 语义已同步这一边界。
- 必须保留的边界：仅 compatible 不等于可以接替，提交条件、未决依赖和效果约束仍须满足；已展示的旧路径内容不可撤回，候选未验证输出不提前展示；切换前后输入、中断和输出事件须保持归属与顺序，旧主路径失去发布权后仍需受管停止／清理，不能依赖前端断开代替生命周期操作。无用户会话中断不表示可以承诺任意在飞工具都无需暂停或没有取消成本。当前先收敛此交互方向，不展开传输、事件协议或在飞工具交接细节。
- 已实现原型并真实验证：Pi ascout.ts 不再等待候选模型生成结束，受限无业务工具候选在其唯一 Proposal 获确认后具备接替条件；通过 onPromote 将存活 session 和完成 Promise 交给稳定用户入口。入口保持 prompt／subscribe／abort，候选输出接受前缓冲、接受后依序发布并继续实时转发，输入在入口排队并按实际送达时的权威执行路由。真实运行证明切换时 candidateEnded=false、oldParentStillRunning=true，候选首轮请求一次且无 abort，旧主路径请求一次、权威工具一次；切换附近排队输入送到 winner，之后再问答仍使用同一入口；用户主动中断可取消在飞 winner。失配取消候选且不发布其内容；默认策略不创建候选。完整 Pi check 通过。当前是 /tmp/opencode/ascout.ts 中的可调用用户端原型，非完整 TUI／Web 或通用跨进程控制面；未完成任意在飞工具、后代管理、持久化或断线恢复，不据此声称零成本切换。
- 复杂场景已收敛并修复：候选主动失败会被 Discard、不会发布缓冲文本，parent 继续；权威回调失败会清理候选并由 parent 处理工具错误；不合作且永不返回的权威回调不会再让 AScout 永久等待，取消只结束 AScout 等待，不宣称外部操作已取消。稳定入口取消当前轮时会轮换 AbortController、清除已完成的 winner completion；`abort-recover` 随后的新请求已真实返回 `RECOVERED`。`queue-many` 的 ALPHA／BETA／GAMMA 按提交顺序到达 winner。驱动中的 `candidate-error`、`authority-error`、`authority-hang`、`abort-recover` 均作为针对性回归场景，Pi check、Agent／Agent-loop 80 项测试和 Rust Policy 测试通过。
- 用户已确认继续实现三项修正：清理“候选必须完成后才能 Promote”的历史表述；所有 AScout 对 Agent 的候选建立、运行、Resolving、Promote、Discard、失败和清理都必须落到真实 Tree 状态；增加最小控制状态记录。已批准并采用 `SpeculativeTree`、`BranchRecord`、`ControlState` 名称。当前 Pi 原型已实现接入侧内存状态：Policy 求值记录、Branch 父子关系与生命周期、Promote 前置 Tree 更新、Discard／失败／清理记录；这仍不是完整持久化控制面 Tree 或多候选协调器。
- 用户已重新校正实现方向（2026-10-24）：当前 Pi 原型中的 `SpeculativeTree`、`BranchRecord`、`ControlState` 放错责任边界，不能作为正式实现继续扩展。Speculative Tree、Branch 生命周期、Policy 落实、Agent 管理和运行协调必须由 AScout 控制面拥有；Pi 只保留完成最小 Agent 接入所需的观察、拦截、会话控制和结果／事件传递，不在 Pi 内维护 AScout Tree 或控制状态。当前 Pi 修改、临时 Policy artifact 和一次性 `ascout.ts` 驱动统一降级为实验材料，不能作为系统入口或完成度证据。
- 已执行删除而非保留（2026-10-24）：Pi `packages/coding-agent/src/core/ascout.ts`、根目录一次性 `ascout.ts`、旧 Policy artifact 和旧 Rust `examples/ascout.rs` 已删除。Pi 现在只保留 `agent-access.ts`：连接控制面、运行 AgentSession、收集模型事实并执行控制命令；Tree、Policy 和 Branch 生命周期不在 Pi 内存在。
- 已完成首个真实控制面闭环（2026-10-24）：`ascout-control-plane` 通过 Unix socket 向 Pi 接入发送 Authority/Candidate 命令；两个跨进程 Pi AgentSession 使用真实 `deepseek/deepseek-flash` 请求；Candidate 使用控制面创建的独立目录。Rust 控制面执行 Policy 的 `match` 和 `dispose`，在 `compatible + Promote` 时更新 Tree authority，向 Pi 发送 Promote/Cleanup，并删除旧 Authority 目录。运行结果确认 Candidate 为 promoted authority、旧 Authority 为 cleaned，winner 目录保留。该闭环仍不等同于完整 Runtime、通用 pending-result 拦截、多 Candidate 或 coding workload。
- 用户进一步校正运行模型（2026-10-24）：Pi 接入连接控制面是受控通信／接入边界，用于让控制面启动和管理 Agent、接收 Agent 行为事实与 outcome、下发控制命令；它不是用户入口，也不拥有 Tree、Policy 或 Branch 生命周期。正式运行时用户应与 AScout 交互，由控制面启动 Pi Agent 和对应接入组件，不能要求用户手工启动两个进程。
- 用户进一步校正 Runtime 责任（2026-10-24）：控制面创建的是抽象的独立 Candidate 执行环境；“独立目录”只是当前接入 Runtime 能提供的最低实现，不是控制面的完整语义，也不能把目录创建直接等同于环境创建。目录的创建、隔离和清理应由 Runtime／Runtime Adapter 按控制面请求落实并回报。
- 用户进一步校正验证语义（2026-10-24）：Promote/Discard 不能根据 Candidate 自己的生成结果裁决。Candidate 只产生并保存 hypothesis／预测执行事实；真实 Parent operation 完成后，控制面取得 Parent 的真实 outcome，把 Parent outcome 与 Candidate hypothesis 交给 Policy `match`，再依据 `dispose` 结果裁决 Candidate。当前 `ascout-control-plane` 用 Candidate output 作为 evidence 的实现违反该语义，必须重做，不能继续称为正确闭环。
- 用户已确认新的首要实现范围：由控制面真正管理 Pi Agent；不要求先接入完整 Runtime，第一版使用另一个目录作为最低程度的文件系统隔离，但隔离边界必须由 AScout／运行管理落实，不由 Pi 自行假设。控制面创建和登记 Branch，向 Pi 接入侧发送控制命令，接收 Agent 事实与结果，再根据 Policy 更新唯一的 Tree 状态。
- 当前不再使用“垂直切片”作为完成描述。这里原本想表达“部分功能完整运行的局部系统”，但当前原型没有按设计分配责任，因此不能称为已完成的局部系统。下一阶段目标改为按设计实现一个可运行的最小闭环：用户入口启动受控 Pi Agent，控制面拥有 Tree 并管理 Parent/Candidate，Candidate 使用独立目录，Policy 决定并由控制面落实 Promote 或 Discard，Pi 只执行控制面命令；每个边界都能观察和验证。若任一责任仍由一次性驱动或 Pi 私有状态冒充，视为未完成。

### Runtime 改用进程级方案（2026-10-23）

- 用户对当前 Firecracker 原型不满意，希望换为进程级方案；主要担忧是 VM 环境建立、分支及资源开销可能抵消投机执行收益，甚至造成整体性能下降。此处为用户的性能担忧，不是已经测量确认的结论。
- 本轮先讨论替换方向与所需环境能力，不开始替换代码，也不据此删除现有实现或改变正式契约。具体进程级技术方案及隔离、状态派生能力尚未确定，不引入新命名。
- 已收敛：优先采用新进程与显式续跑状态，不以透明复制任意运行中 Agent 为首版目标。用户提出复制对应 session 文件或使用 Agent 自带的 fork 能力作为接入候选；这里不将 Agent 会话分支等同于 OS `fork`，具体产品支持情况待确认。
- 已收敛：当前优先级是在已有设计约束下跑通真实投机执行，不继续把完整后端设计与 Registry 主动验证作为启动前置条件。进程与 Firecracker 是同一 Runtime Adapter 契约下的实现选择，不为更换后端修改 Branch、commit/discard、single-winner 或效果处理语义；实际提供的保证仍须如实说明，不能因接口相同而宣称隔离能力相同。
- 后续待确认：首个真实接入的 Agent 产品及其 session 续跑／会话分支入口。复制会话记录不自动复制 workspace，也不证明已覆盖工具状态；只补齐首个真实场景必需的状态与效果边界，不要求先解决任意 Agent 的透明复制。
- 候选已明确：`https://github.com/earendil-works/pi` 与 `https://github.com/deepseek-ai/deepseek-harness`。助手查阅两仓库当前官方文档及 Pi 扩展类型后，初步推荐 Pi 作为首个接入对象，理由是 session 管理、SDK、工具扩展和子进程接入路径直接；不是用户已确认的产品选择，未安装或实跑二者，也无性能对比数据。
- 对比依据：Pi 的 `packages/coding-agent/docs/sdk.md`、`docs/extensions.md`（均相对其 coding-agent 包）提供持久 session、显式 cwd、工具前置拦截和结果变换；扩展类型中的会话 fork 属于命令上下文，不能在工具等待钩子里直接切换父会话。dsh 的 `docs/architecture.md`、`docs/tool-execution-pipeline.md`、`packages/core/agent-loop/README.md` 提供可替换插件、工具执行 middleware、seed 创建与 resume，但文档中的 fork 入口定位为 turn boundary，不能直接视为已支持工具等待中的分支续跑。两者都还需实际验证未完成工具调用的会话截取、预测结果配对和独立进程续跑。
- 已收敛：用户同意选择 `earendil-works/pi` 作为首个真实 Agent 接入对象，本轮不同时适配 dsh。保留 Pi 原有推理循环，通过工具接入、显式会话状态和独立进程续跑连接 AScout；不把 Pi 原生命令级 fork 当作工具等待点分支的现成实现。该选择不表示已经完成安装、接入或真实投机执行验证。
- 用户同意先核实工具等待点的会话截取与续跑路径。源码核实固定到 Pi commit `8a2ebd38549c3521a02db0e5d758a24320e97f94`：`agent-session.ts` 在 `message_end` 扩展事件和公开通知之后才持久化消息，所以不能在该通知里假定最新 assistant 已落入 SessionManager；`session-manager.ts` 的 `getBranch()` 可取当前路径，`appendMessage()` 可追加工具结果，`createBranchedSession()` 会改变被调用 manager 的会话身份，不能直接用于仍在运行的父 manager；`sdk.ts` 从 manager 恢复消息，`agent.ts` 的 `continue()` 接受工具结果结尾并沿原 loop 继续，无需伪造新用户提示词。当前仅为源码可行性核实，未调用模型或实跑。
- 首轮接入建议（未批准为固定机制限制）：从工具包装执行入口截取已有 assistant 调用的会话路径，深拷贝为新 session，在创建候选 AgentSession 前补齐匹配 toolCallId 的预测工具结果，再通过原 Agent loop 继续。必须处理同一 assistant 中所有工具调用的配对，不能只填一个结果便带着其他未决调用请求模型；首个实际场景可选择单工具调用，不将它固化为通用机制限制。父 session 不切换、不混入预测结果，workspace 和外部效果仍需真实边界。
- 用户授权直接 clone Pi 并修改源码。首轮允许把实现范围缩小到 filesystem，或暂不经过独立 Runtime Adapter，仅允许无本地副作用的操作进行投机执行；不要求先实现通用进程后端。助手优先选择后者开展接入：拒绝候选的任意 shell、写文件和未经授权的外部业务操作，不将仅更换 cwd 当作隔离。该范围不删除 Runtime 目标架构，不声称已实现写入隔离或完整 coding workload；session、日志及模型调用的系统管理写入／费用与候选业务效果分开记录。
- 获取进展：GitHub Git clone 多次因 TLS／连接错误失败，改为下载固定 commit 的官方源码归档，已解压到 `/tmp/opencode/pi-8a2ebd38549c3521a02db0e5d758a24320e97f94`。这是源码归档，不是已成功 clone 的 Git checkout。已启动 `npm ci --ignore-scripts` 准备依赖，尚未修改 Pi 代码或执行模型。
- 用户批准接入文件名 `ascout.ts`，位置为 Pi 的 `packages/coding-agent/src/core/`。已在上述源码目录新增该文件，默认导出续跑函数，不引入额外命名的项目实体。当前实现深拷贝父 session 当前路径、校验单个待返回工具调用与预测结果的 ID／名称对应、创建独立内存 session 并注入预测结果，再沿原 Agent loop 继续；无业务工具、无资源自动发现、无扩展、关闭 compaction／自动重试／cache warming，返回候选消息而不发布为权威结果，支持 AbortSignal 取消与 dispose。
- 验证状态：依赖 `npm ci --ignore-scripts` 安装成功；首次 `npm run check` 因源码归档不含生成的模型数据失败，执行官方 `npm run hydrate:model-data` 后重新运行完整 `npm run check` 通过。未新增测试、未调用模型、未执行完整 build。当前只是同进程候选续跑入口，尚未连接工具拦截、Rust 控制面、Policy、独立候选进程、权威验证、采用／丢弃或顺序对照，因此不声称已跑通真实投机执行。Pi 改动目前仅在外部临时源码目录，未纳入 AScout Git。
- 真实运行（2026-10-23）：接入入口随后连接真实 Policy Engine，通过固定可执行文件 `target/debug/examples/policy` 的 `evaluate` 子命令求值 `CompiledPolicy`，没有在 Pi 内另写策略。默认 `assets/policy/policy.yaml` 改为 `max_edges: 0`，`select` 返回空集合，真实工具只执行一次并走普通路径；临时把 `max_edges` 设为 `1` 后，候选在 20 秒权威延迟内完成，真实工具仍只执行一次，`match=compatible`，`dispose` 按当前唯一支持的 `Discard` 清理候选。两次均使用真实 `deepseek/deepseek-flash` 模型调用，`calls=1`。`rebalance` 与 `effect` 未调用；候选与主会话仍同进程；Policy Engine 尚不支持采用候选结果，成功匹配也不会产生投机收益。驱动脚本在 `/tmp/opencode/ascout.ts`，未提交 Git。
- 用户已确认本轮采用路径（2026-10-23）：命中并满足提交条件后，优先采用 **Promote**，即保留候选 Branch identity，将候选替换为主路径的唯一 winner；旧 parent、其他 sibling 按既有 single-winner 规则失效，候选已有 descendants 保留。当前不先实现把候选 outcome／受管状态交给原主延续的 **Reuse**；Reuse 仍是后续深度嵌套投机所需能力，不能从本轮范围删除，也不能与 Promote 混写。Pi 接入的下一步是让命中的候选后续会话成为主路径，而不是把候选文本复制回旧 parent。
- 已实现并实跑最小 Promote（2026-10-23）：Rust 接受 accepted 下的单个 Promote；Pi 保留候选 AgentSession，在旧会话工具结果持久化后的 turn 边界结束旧运行，驱动随后释放旧会话并切换主引用到候选。真实命中运行 calls=1、parentRequests=1、主 session ID 等于候选 ID，且在同一候选 ID 下继续一轮问答；失配运行 discarded、calls=1、parentRequests=2、从真值 other 继续；默认 ordinary、calls=1、parentRequests=2。实际发现 finishTurn 回调被运行快照固定，先新增测试复现多余请求，再修复并通过 80 项 agent／agent-loop 测试、Pi 完整 check 和 Rust policy 测试。尚无完整 Tree、sibling／descendant 及嵌套管理，候选仍无业务工具，只覆盖完成候选的同进程接替，不宣称通用后端或收益验证完成；源码／驱动仍在 /tmp/opencode。详见 docs/development/reference-workload.md。

### Policy DSL 的语言实现（2026-10-23）

- 用户要求：参考 `docs/dls/references/1409.2378v1.txt` 讨论系统中的 DSL 实现，并允许继续查阅该文引用和引用该文的论文。沿用已有 Policy DSL 名称。
- 已收敛（2026-10-23）：采用 YAML 描述 Policy 的结构、各 Decision 的行为和配置，算法使用成熟语言在独立文件中编写，由 YAML 引用文件路径。YAML 与其引用的算法源码共同组成一个 PolicyDefinition，不增加绕过 Policy DSL 独立加载算法的入口，也不把策略能力缩减为内置算法配置菜单。
- 已收敛：系统规定每类 Decision 何时被调用、能获得哪些数据和必须返回什么结果；算法只在系统规定的数据和授权能力范围内计算，不任意访问或修改系统状态。“以九类 Decision 为中心”不是九个内置算法、固定流水线或必须九个文件。算法可共享计算逻辑，不能由此获得额外权限。
- 已收敛：提供给算法的是允许读取的数据值或快照，不是控制面内部对象、可变系统引用或任意查询入口；模型等外部能力需要明确授权。Decision 返回选择，系统检查结果并由状态拥有者落实；Branch 生命周期、效果、commit/discard 和 single-winner 等既有机制边界不变。执行权限必须实际落实，不能把参数限制、YAML 或静态检查当成隔离保证。
- 已收敛：文件引用用于编译时找到源码；编译需固定所用代码、明确依赖，并形成可跨侧交付的结果，运行期不重新依赖作者机器上的原始路径。此处只确定原则，不固定依赖打包方式或函数入口语法。
- 已收敛（2026-10-23）：首版算法语言仅支持 Python；YAML 暂时分别显式指定九类 Decision，不通过省略 Decision 隐式补齐行为。源码引用及函数调用方式由助手按通用做法起草，再由用户调整。首版编译产物仍为 Python 脚本，但源码语言和执行载体是两个独立选择，不据此固定长期语言范围。
- 已同步：PolicyDefinition 的组成、输入数据和结果边界见 `docs/design/control-plane/policy-engine.md`，Policy 与 Mechanism 的边界见 `docs/design/policy-decisions.md`；YAML 与独立算法文件的实现选择及未实现差距见 `docs/development/decisions.md` D-036-I。不改变全局模型，不新增实体、Decision 或接口名称。
- 研究依据：Karsai 等《Design Guidelines for Domain Specific Languages》（DSM 2009，2014 年上传 arXiv）§2.1 的用途与使用者分析、Guideline 5–6 的语言复用、8–12 的必要概念与简洁性、14 的熟悉记法；§3 要求按场景权衡，不把准则当成必须全部实现的清单。算法文件复用 Python 时仍须接受并说明其缩进语义，不能声称完全遵循 Guideline 24。
- 已查阅原文所引 [18]：Spinellis，《Notable design patterns for domain-specific languages》（2001），[作者公开稿](https://www.spinellis.gr/pubs/jrnl/2000-JSS-DSLPatterns/html/dslpat.html) §4.1、§4.5、§4.6 讨论借用已有语言能力、裁剪已有语言和源码到源码转换；这些方法不是安全性保证。
- 已查阅直接引用原文的 McNutt，《No Grammar to Rule Them All: A Survey of JSON-style DSLs for Visualization》（2022），[全文](https://arxiv.org/html/2207.07998v1)，参考文献 [46] 为 Karsai 等。§1.1、§3.3 说明序列化格式不免除领域语义设计，内嵌代码可能引入工具支持和隐藏依赖问题；独立算法文件与代码字符串不同，不能直接沿用对字符串的批评。该描述性调查不证明 AScout 的某种语法更好用。
- 已补充查阅直接引用原文的 López-Fernández 等《Example-driven meta-model development》（2013 年在线发表，2015 年卷期），[正文](https://doi.org/10.1007/s10270-013-0392-y)，参考文献 [37] 为 Karsai 等。可借鉴从具体示例逐步形成语言模型的思路，不引入自动元模型推导框架，也不将其作为已完成工业实证的依据。
- 当前实现准备度：Rust 源码主要承载 Runtime Adapter、Firecracker 与通信路径，尚无 Policy Engine 或 YAML／算法编译求值实现。需要把作者编写方式、九类数据约定、编译交付和执行权限整理为可实施的约定；不把预先编写测试作为原型实现前置条件。
- 用户曾授权助手先完整起草，再共同调整，不再先逐项批准或只从 `extent` 开始讨论；随后明确要求开始实现并批准代码命名。以下整体草案仍不全部等同于已接受目标语义，实际采用的原型接口和范围见本节实现记录及开发文档。
- 首批可独立验证真实 YAML、算法源码、编译和求值路径，不必等待整个 Branch 执行模型的全部议题收敛；但这不等于已完成投机执行系统接入，也不能证明 Reuse、接受或清理等机制语义。完整集成仍须落实相应数据来源、调用时机和结果应用。多语言、广泛第三方依赖与其他后续能力可排队，首版缺失范围必须明示，执行权限边界不能省略。
- 本轮设计范围：完整编写示例、九类输入输出与检查、依赖固定与跨侧交付、Python 执行隔离、外部能力授权、超时取消和失败处理。更深的后端实现及扩展仍排队，不要求用户同时回答全部细节。
- 已收敛（2026-10-23）：用户可按需在 YAML 中为各 Python 函数分别指定算法参数，由系统传给对应函数；首版不支持全局共享参数，也不设全局／函数级合并或覆盖规则。参数与系统提供的运行时数据、授权能力分开。`policy_config` 沿用已有草案中的写法，具体 DSL 字段位置和调用签名仍属待审接口细节。
- 用户要求开始实现并确认命名（2026-10-23）：将 YAML 编译及运行时 Policy 求值真正接入现有 Rust 代码，批准 `policy` 模块及 Rust 接口 `PolicyEngine`、`CompiledPolicy`、`Decision`。已打通 YAML、独立 Python 源码、固定编译产物和真实求值路径，不以预设返回值代替算法；不据此声称已完成 Branch 生命周期或九类决定的系统应用。采用草案中的 `file`、`function`、各函数 `policy_config`、统一 Python 签名及明确的可选 `inputs` 作为原型接口；省略函数参数映射时传空映射。真实隔离、输入输出检查及不支持能力的显式拒绝属于实现范围，不能静默退回无隔离 Python。
- 实现记录（2026-10-23）：新增 `src/policy.rs`、内嵌 Python runner、`assets/policy/` 完整九类示例和 `examples/policy.rs` 编译／求值入口；当前范围与未实现差距下沉到 `docs/development/policy-engine.md`。已真实执行十个函数入口、固定源码交付和参数调整路径，并检查非法声明、文件／网络／进程访问和无限循环。实际发现 bubblewrap 不支持 `--preserve-fds` 后，按既定规则先以 `tests/policy.rs` 稳定复现，再修复为标准输出接管并分离结果／诊断。未提交工作区改动。
- 已收敛（2026-10-23）：原型阶段默认不编写测试；只有实际遇到可复现、且今后很可能再次出现的问题时，才针对该问题编写测试，先稳定复现，再修复到通过。已有测试不因此自动删除。后续实现按这条规则处理真实遇到的问题；沿用未经用户明确允许不得 Git commit 的约束，不提交已有工作区改动。

#### Policy DSL 整体草案：编写、数据与执行（待确认）

本草案按用户授权一次写全，供后续调整，不表示下述细节已经全部通过。继续沿用 PolicyDefinition、Policy Engine 和九类 Decision，不增加项目实体或语言名称。用户要求开始实现后实际采用的字段与调用方式见 `docs/development/policy-engine.md`；**其他尚未采用的字段、参数和函数拼写仍是待审示意，不能据此声称已实现或已确定正式接口。** 三部分分别回答“作者怎么写”“系统给什么、收什么”“如何编译和安全执行”。

##### 一、作者编写与函数绑定

###### 1. 最小编写形式

YAML 是唯一加载入口。九类 Decision 分别显式绑定 Python 实现；首轮实现可以先不提供固定值、规则菜单、继承或多种等价简写，恒定行为直接写成 Python 函数。以后添加简写也必须编译为相同调用形式，不能形成另一种运行语义。

下面示例完整展示九类绑定；数值和源码文件名只是示例，**不是系统默认策略，也不表示配套算法已经实现**：

```yaml
version: 1
language: python
files:
  - decisions.py
fidelity: Best-effort
decisions:
  admit:
    opportunity_pass:
      file: decisions.py
      function: opportunity_pass
    feasibility_pass:
      file: decisions.py
      function: feasibility_pass
  select:
    file: decisions.py
    function: select
  rebalance:
    file: decisions.py
    function: rebalance
  extent:
    file: decisions.py
    function: extent
    policy_config:
      max_steps: 5
  predict:
    file: decisions.py
    function: predict
  launch:
    file: decisions.py
    function: launch
  effect:
    file: decisions.py
    function: effect
  match:
    file: decisions.py
    function: match_proposal
  dispose:
    file: decisions.py
    function: dispose
```

- `version` 标识 DSL 的结构和数据接口版本；未知版本拒绝，不尝试猜测兼容。
- `language` 首版只接受 `python`。Python 解释器版本由部署方选定并固定到编译结果要求中，不让作者通过 YAML 选择任意可执行程序。
- `files` 是进入 PolicyDefinition 的完整本地文件清单，包括入口源码、共享源码和声明的数据文件；不递归扫描整个仓库，也不靠静态分析猜测所有 import。
- `policy_config` 是可选地写在该函数绑定下的只读逻辑配置；不同函数的配置相互独立，不从全局配置继承或合并，每次调用获得独立副本。不包含宿主凭证，不将配置中的字符串自动解释成路径、表达式、环境变量或查询。
- `decisions` 恰好包含九类，缺失、重复或未知项目均报错。`admit` 的两个入口仍是一类 Decision 的两个判断，不新增第十类 Decision。
- 不要求九个文件；多个入口可共用模块和辅助函数。YAML 的 `match` 可以绑定 `match_proposal` 等正常 Python 函数，不依赖拼写猜测入口。
- YAML 只接受与 JSON 对应的标量、列表和字符串键映射。拒绝重复键、自定义 tag、锚点／别名、merge key 和多文档，限制文件大小与嵌套深度；不借这些功能再造配置继承。布尔值只接受明确的 `true`／`false`，不把 `yes`／`on` 或日期字符串暗中转换。

###### 2. 文件与入口解析

- 所有路径相对 YAML 所在目录解析；拒绝绝对路径、`..` 越界、URL、环境变量展开、符号链接和非普通文件。编译时打开并读取实际文件，再以读到的字节形成固定副本，后续检查与打包使用同一副本，不在检查后重新读取原路径。
- 入口文件必须在清单中。首版入口为模块顶层的普通同步函数；不采用类实例、任意属性链、装饰器注册、自动扫描或 `eval` 字符串绑定。
- 本地模块沿用正常 Python 包和 import 规则，保留清单中的相对目录结构；需要的 `__init__.py` 也必须列入清单。不允许本地文件覆盖所选执行环境的标准库／系统调用辅助模块。未打包的动态 import 在隔离环境中找不到时显式失败，不回到作者机器寻找。
- 函数必须接受统一调用约定；签名、可调用性和 import 可用性在隔离的加载检查中验证。加载模块会执行顶层代码，因此该检查不能在控制面进程内进行。

###### 3. Python 函数调用约定

建议全部入口采用同一种外层签名，Decision 差别只在 `data` 的内容与返回结构：

```python
def extent(data, *, policy_config, capabilities):
    process_state = data["process_state"]
    if process_state["steps_completed"] >= policy_config["max_steps"]:
        return "stop"
    return "continue"
```

- `data`：本类 Decision 的允许数据快照；不是系统对象。`policy_config` 是 YAML 中专属于当前函数绑定的配置副本，不与其他函数共享。`capabilities` 仅能代理本次已经授权的计算能力，不提供状态查询、Runtime 操作或生命周期方法。
- 首版不依赖模块全局变量在调用间保存状态；一次求值使用全新进程和临时目录，模块导入产生的缓存只在本次调用内有效。需要历史时显式传入规定的历史数据，不能把共享 Python 进程当作跨 Branch 通信渠道。
- 作者可以正常使用条件、循环、函数、局部类和集合计算，不另造通用算法语法。接口要求返回可传递的数据值，不代表算法内部只能写表达式。
- 输入输出使用 JSON 对应的数据类型，映射键必须为字符串；整数限定到有符号 64 位，浮点数必须有限，计数必须非负。没有 pickle、Python 对象引用、自动对象构造、函数或文件句柄传递。`True` 不是整数 `1` 的替代输出，布尔结果检查实际布尔类型。
- 配置、输入和输出分别校验；类型标注可帮助编辑器，但不代替运行期检查。修改输入或配置副本不会修改系统真实状态。

下面展示可选声明如何附着在既有绑定上，不是要求普通作者填写第二套配置。字段和约束布局仍待审：

```yaml
predict:
  file: decisions.py
  function: predict
  inputs: [history]
  capabilities: [model]
  outputs:
    generation: [batch]
    max_proposals: 3
```

这里 `capabilities` 是向部署方提出的授权请求，不是自行授予权限；`max_proposals` 是作者声明的返回上限，不替代系统的资源硬上限。各入口默认只得到其必需字段；额外数据与外部计算需要分别声明，不能互相隐含。`requirements` 是 Policy 层的后端／效果要求，按当前 Contract 的内容与保证维度展开，不与计算能力别名混用。

##### 二、九类数据、输出与检查

###### 1. 共同的数据规则

- 系统按 Decision 的版本化清单准备数据，而不是提供 `get_state`、SQL、目录、任意 URL 或“完整上下文”对象。任务与领域数据必须经过对应接入的明确 schema；增加一个任意字典不能成为绕开数据清单的后门。
- 每类有必需输入。需要可选字段时，在该入口的 YAML 中用待审字段 `inputs` 明确列出，例如 `inputs: [process_state.active_time]`。这里只能选择该类允许的数据路径，不能写表达式、查询条件或通配系统路径。声明需要的字段在当前接入中无法产生时拒绝绑定；运行期本应存在却缺失时返回数据错误，不伪造为 `0`／空列表。
- `process_context` 只包含当前执行原本允许读取的规范化输入、阶段和已有结果；`history` 只限当前 Task 内授权的轨迹，不默认包含其他 Task、sibling 私有数据、完整提示词或进程内存。其内容和裁剪规则随领域 schema 明确，截断必须标记，不能把缺失历史假装成“历史为空”。
- ID 都是受管范围内的逻辑标识，不是后端句柄或授权凭证。调用方另保存 Policy 内容摘要、Task／Edge／Branch 归属、输入视图版本和调用标识；算法不能通过返回另一个 ID 改变本次调用的身份。
- 每份输入记录其未决 Proposal 依赖。结果默认继承实际输入的依赖，算法不能自行清除；`match` 形成的兼容意见也不等于已经确认所有依赖。传入调度函数的跨 Branch 内容仅是允许汇总的控制事实，不夹带 sibling 私有 payload 或 Agent context。诊断日志同样受依赖和可见范围约束。
- Policy Engine 检查输出结构、输入成员资格和声明限制；状态拥有者在应用前再次检查最新有效性、能力、预算、依赖和权限。两次检查回答不同问题，不能用编译时或快照时通过代替实际执行条件。

###### 2. 各 Decision 的调用数据表

以下路径和对象布局是本草案的候选接口；目标语义继续以 `docs/design/policy-decisions.md` 为准。

| Decision | 系统提供的数据 | 结果 | 关键检查与责任 |
|---|---|---|---|
| `admit.opportunity_pass` | `op`：当前操作类别、是否产出被等待的结果、是否已有针对该结果的预测手段等可观察事实 | 布尔值 | 不传预测结果、资源竞争排名或模型评分；`false` 时不做后续精判，普通执行继续 |
| `admit.feasibility_pass` | `op`、`state`：当前实例未决依赖及其成本、机会保留条件与管理成本；`contract_context`：已校验的能力和限制 | 布尔值 | 只在粗筛通过后调用；不把瞬时资源压力和与其他机会比较塞回精判。通过表示登记待选，不表示启动 |
| `select` | `frontier_view`：同一 Task 全部有效待选 Edge 的快照；`scheduling_context`：预算、资源压力、等待时长、依赖和已观测成本 | Edge ID 的有序列表，可为空 | 必须为输入集合无重复子集。保留 Policy 顺序，不用机制内置价值排序改写；Coordinator 启动前复核，暂不启动的有效项仍保留 |
| `rebalance` | `active_view`、`paused_view`：同一 Task 已运行／暂停的投机 Branch 摘要；`scheduling_context` | 有序动作列表，每项为 Branch ID 与 `pause`／`resume`／`preempt` | 不接收待选 Edge；首版同一结果中每个 Branch 至多一个动作，拒绝矛盾动作。`preempt` 进入 `dispose(preempted)`，不是直接清理 |
| `extent` | `process_state`：必需的 `steps_completed`、`phase`，以及声明读取的可选进展事实 | `stop` 或 `continue` | stop 表示达到本过程前沿并等待，不自动 discard／commit。子过程独立计数；已被效果或 Runtime 阻塞时不再检查 |
| `predict` | `edge_context`、`process_context`、`assumption_set`、`capability_context`；可选 `history`；该函数绑定自己的 `policy_config` | `ProposalSet`，包含 `proposals` 和 `generation` | payload 符合当前边界结果 schema；代码不能分配可信 Proposal ID、创建 Branch、标记 Authority 或改主 Agent 配置；ID、依赖与创建均由系统落实 |
| `launch` | 当前 `edge_context`、权威来源是否可发起／已发起／已有结果、当前候选与验证状态摘要，以及声明读取的预算事实 | 布尔值 | `true` 表示当前请求取得权威值；来源固定，启动去重与 once-only 归状态拥有者。外部天然输入不能因返回 true 被伪造或强行产生 |
| `effect` | `effect_descriptor`：规范化请求、StateDomain／EffectClass 与保证范围；`context`：当前 Branch 依赖、提交条件、obligation 及服务插件可用方案 | `EffectPlan` | 不将请求凭证、连接或插件可变对象交给算法。分类由机制／服务契约给出，Policy 不能改成 PURE；计划由 Effect Coordinator 检查并实际落实 |
| `match` | `proposal`、`evidence`、`match_context`：同一 Edge 的匹配对象、证据来源／状态／依赖与同一规则上下文；需要时提供声明的候选 outcome | `MatchResult` | 证据由系统关联，不接受算法自称的 Authority；compatible 仍须通过依赖、Fidelity 和提交检查。不能因 score 高就越过屏障 |
| `dispose` | `branch`：本次对象的受控摘要与已登记产物／效果引用；`resolution`；`context`：已有匹配意见、提交证据、权威路径状态、未完成责任和当前合法处置范围 | `DispositionPlan` | 不重新做 match，不由 Python 直接调用生命周期；引用限本次已授权对象，所有权交接和清理按实际结果确认 |

`admit` 对外仍是粗筛与精判的合取。两次调用的数据严格分开；只有同一操作和仍然有效的前提才能串联两个结果。机会通过后等待期间由机制维护有效性，不再重复进行策略精判。

为避免表中的“上下文”变成任意塞数据的口袋，建议首版固定以下基础记录。**此表仍是字段草案，不是新实体**；未列出的业务内容必须使用显式引用的领域 schema，不任意透传内部对象。

| 记录 | 基础字段与类型 | 扩展／缺失规则 |
|---|---|---|
| `op` | `kind: string`、`produces_result: bool`、`result_pending: bool`、`has_predictor: bool` | `kind` 来自当前接入的稳定操作类别。可选规范化参数按该类操作 schema 声明，不传 shell／Python 执行入口 |
| `edge_context` | `edge_id: string`、`branch_id: string`、`op`、`result_schema: string`、`assumption_set: list[string]` | 只描述当前 pending-result boundary。嵌套位置／依赖点摘要是可选事实，schema 标识必须能够解析到具体输入结构 |
| `process_state` | `steps_completed: nonnegative integer`、`phase: string` | 可选 `active_time` 统一为非负秒数；资源计数含计量单位；`result_facts`／`semantic_facts` 各有任务领域 schema。没有主观效用分数的隐式注入 |
| `frontier_view` 的每项 | 当前 `edge_context` 的受控摘要、`wait_time: nonnegative seconds` | 必须能按 edge_id 引用；不传 sibling 私有参数。可选成本／已观测时延带单位与来源，估计不得伪装成测量值 |
| `active_view`／`paused_view` 的每项 | `branch_id: string`、`edge_id: string`、`assumption_set`、允许汇总的 `process_state` 进度摘要 | 只含本 Task 的投机工作；不传权威主线作为可抢占对象。暂停原因与可恢复性是事实，不仅凭所属列表就推导恢复一定合法 |
| `scheduling_context` | 已知资源项目的 `used`／`limit`／`unit`、采样时点和可用槽位等控制事实 | 资源键来自部署的固定目录；Policy 必需的资源项目在接入阶段确认，未测量项保持缺失，不能等同于零压力或无限资源 |
| `evidence` | `kind: authority 或 validator`、`source: string`、适用 Edge／Proposal 引用、`payload`、`assumption_set` | source 与引用由受信调用方封装；payload 符合该来源 schema。无证据用明确空值，未决／错误状态不能包装成成功证据 |
| `effect_descriptor` | 受管效果引用、规范化请求的 schema 标识、StateDomain、EffectClass、保证范围 | 只提供当前已识别效果；服务认证由机制处理，Policy 不能通过重写请求数据自行制造更多服务权限 |
| `branch`／处置 `context` | 本次 Branch／Edge／Proposal 引用、已有 resolution、匹配结果、提交条件和已登记产物／obligation 摘要 | 产物和效果采用受管引用加 schema，不是可访问的磁盘路径；缺少交接条件就是不具备能力，不能补一个成功布尔值 |

`process_context`、`history`、规范化请求与 `contract_context`／`capability_context` 进一步引用各自明确的领域或 Contract 视图：前两者列出允许的输入／结果类型，后两者沿用真实保证范围、验证状态与限制。引用 schema 是压缩文档重复，不是承诺任意 JSON 都合法。上述状态与证据由相应拥有组件形成；Python 返回同名字段不会使它成为可信事实。

###### 3. 结构化输出的候选格式

- `rebalance`：`[{"branch_id": "…", "action": "pause"}]`；空列表是明确的“不调整”，不是求值失败。
- `predict`：`{"proposals": [{"payload": value, "provenance": optional_data}], "generation": "batch"}`。空候选集合是正常的“本次没有候选”，由调用方继续权威／普通路径；异常不能冒充空集合。每个已发出 Proposal 固定不变，来源描述只是作者声明，系统另记录真实生成版本和授权调用来源。
- `effect`：`{"mode": "block"}` 等；模式沿用 `default`、`simulate`、`transform`、`stage`、`mediate`、`block`、`allow`、`deny`。需要选择具体方案时，只能引用当前服务插件已登记、带参数 schema 的方案，不能返回命令、回调或随意指定 URL。`default` 必须解析到真实声明的方案；没有声明就报错。`simulate` 仍生成未验证、标记来源的值，不冒充 Authority。
- `match`：`{"verdict": "compatible"}`，或带 `score`；verdict 沿用四种值。score 必须是有限数值，其定义、方向、范围和阈值属于 Policy 匹配规则的声明。AScout 不把 score 解释成概率，不跨 Edge 的不同规则／版本／尺度比较。声明阈值时也检查 verdict 与该规则一致，缺少可比较的证据不能默认通过。
- `dispose`：`{"actions": [{"intent": "Reuse"}]}` 等，动作按声明顺序表达依赖。意图沿用七种既有取值；需要产物、效果或目标引用时，必须符合对应受管交接方案的 schema。不接受把任意新 payload 当成“已验证产物”返回。首版同一计划不同时选择 `Reuse` 与 `Promote`，也不允许引用不属于本次受管范围的对象。
- `dispose` 的空动作列表只在尚可合法等待的 `indeterminate`、`error` 或 `extent_reached` 情况表达“保持阻塞、暂不处置”；不为已否定依赖、最终取消／抢占或硬清理要求保留继续执行的例外。它不是一个新 Decision 或新的 Branch 状态。

复杂结果不能只验证 JSON 格式：`Salvage + Discard` 要先完成合法交接再清理；`Compensate + Discard` 要保留并收敛补偿责任。未实现相应交接／补偿的组件必须拒绝该计划，不能只改 metadata。

###### 4. 输出声明、能力要求与 Fidelity

为使动态算法可检查，建议每个绑定可声明待审的 `outputs` 约束：布尔／verdict／处置意图的允许范围、effect 模式集合、候选数量上限、score 规则等。运行期返回超出声明的值即错误；省略时表示完整合法值域，不表示自动选择一个安全结果。声明范围不是对任意 Python 行为的静态证明。

Policy 可在待审的 `requirements` 中列出对后端环境保证、服务效果方案和可选状态版本／读依赖能力的要求。环境保证复用当前 Contract 的内容、属性和范围，不另造第二套能力强弱排序；外部算法计算授权与 Runtime 环境保证分开表达。必要能力缺失、未验证且无部署确认，或已经验证失败时，不得按可用能力加载。

`fidelity` 使用既有等级，并须指明需要的观察与保证范围；示例的 Best-effort 不申请额外等价保证。任意 matcher 自称 exact、或在 YAML 写 Trajectory-equivalent，都不能证明该等级成立。加载检查只接受有足够依据的上界：已知匹配规则与边界 schema、允许输出、效果纪律、处置方案和后端保证必须能支持声明；无法判断时拒绝更强声明，不凭元数据放行。运行期审计仍有独立责任。

首版建议拒绝能力不满足的绑定，不自动重写算法、不自动降级 Fidelity。若以后支持明确降级，必须由作者声明另一条完整合法路径，任务接入时显式记录实际采用路径，不能由 Engine 临时换一个“差不多”的算法。

###### 5. 与已有执行语义逐项核对

- `Reuse`：只交付已验证 outcome 和声明需要的受管状态，主延续保留身份、推理和上下文；交接成功后才清理 candidate。不把任意 Agent context 合并或复制当成 Reuse。
- `Promote`：接受候选并替换主延续，保留既有 sibling 失效和 descendant 规则；DSL 不引入 independent offer 例外。
- `accepted`、compatible、提交条件满足、实际 accept 完成是不同事实。Policy 输出不自动清空 AssumptionSet，不自行确认 obligation，也不证明真实环境操作已经完成。
- 嵌套 Edge 继续使用 Task 绑定的同一 Policy 及各函数分别绑定的配置，不根据深度静默换策略；使用 depth 作为算法输入只能影响选择，不能推导身份或权限。
- 批量验证可以产生供逐 Edge 使用的结构化证据，但本接口不新增批量接受操作。证据与结果必须保留前缀依赖；前一步失效后，后续证据不得继续用于接受。修正动作、复杂嵌套产物和在飞工作交接仍由各自受管方案约束，不用 DSL 输出绕过尚未实现的能力。

##### 三、编译、交付与受控执行

###### 1. 编译路径与可交付产物

```text
读取并检查 YAML
    -> 按 files 读取固定源码／数据副本
    -> 检查九类绑定、输入声明、输出限制和能力要求
    -> 在隔离环境中检查 Python 语法、导入与函数签名
    -> 生成 Python 脚本和不可变编译记录
    -> Task 接入时对照当前能力与授权再次检查
```

- “编译”不是把 Python 算法翻译成另一套算法：YAML 变为固定绑定、每个函数各自的配置与检查依据，独立 Python 源码和声明数据被收入可交付脚本。源码只打包一次；不把每个 Decision 的代码粘贴改写成另一种语言。
- 建议首版生成一个 Python 脚本，内含经转义编码的声明、绑定、源码和数据文件内容。运行准备阶段只从该固定内容建立只读文件视图，保留原相对目录、逻辑文件名和行号。具体嵌入编码不构成 DSL 语法，也不执行配置字符串。
- 编译记录包括：DSL／数据接口版本、全部输入文件的相对路径与内容摘要、配置、九类绑定、读取范围、允许输出、能力要求、所需 Python 执行环境标识和生成器版本。Policy 内容摘要覆盖这些影响语义的内容；修改任一项都产生新版本，不在运行中的 Task 内热替换。
- Python 执行环境标识包括解释器版本与构建、标准库和已批准库的内容版本、平台与隔离规则版本。首版由部署方提供一个固定的 CPython 环境；端侧不能找到一个 `python3` 就假定兼容。不交付 pickle、marshal 对象或把 `.pyc` 当成跨版本协议。
- 首版建议支持标准库和清单内的本地 Python 模块／数据文件，不进行隐式 pip 安装、联网解析依赖、执行 setup 脚本或沿用作者虚拟环境。模型调用走下面的授权计算能力。需要第三方库时，由部署方离线构建、审核并固定到执行环境中；不把“依赖已写名字”当成环境已具备。
- Python 语法解析、加载检查和打包都有资源上限。语法检查不执行算法；模块加载会执行顶层语句，但不触发真实 Decision，不提供 Task 状态和真实外部调用权限。只在调用时才 import 的分支仍可能运行失败，不能宣称加载检查证明了全部路径。
- Policy Engine 的按 Decision 查询返回固定脚本、所选入口及该入口的约定和要求，全部可以序列化。首版可保守携带整个已声明文件清单，不提前实现精确 Python 依赖裁剪；“按 Decision 交付”指只开放对应求值入口，不承诺代码包只含该函数的文本。
- 接收方校验来源、内容摘要、版本、所需环境与本侧权限后才执行。摘要用于确认内容一致，不代替来源认证。Agent 接入侧不重新读取作者 YAML、不重新编译 DSL、不修改绑定或补装依赖；CPython 对固定源码的正常加载不等同于重新解释 PolicyDefinition。

###### 2. Python 实际执行边界

建议原型使用 **Linux 上的独立受限进程**：由受信的 Rust 调用侧监督，复用 bubblewrap 构造最小文件与 namespace 边界，配合 seccomp 和资源限制。它是 Policy 求值的内部执行手段，不新增控制面组件，不把已有 Firecracker Branch 承载强制变成 Policy 的执行位置。

| 访问类别 | 约束 |
|---|---|
| 文件 | 只读的固定 Python 执行环境、Policy 编译内容和声明数据；本次独享、限量的临时目录。没有宿主 home、仓库工作区、Agent workspace、凭证目录、Docker／控制面／Adapter socket |
| 网络 | 独立网络 namespace，不提供外网、宿主或其他 Branch 连接；清除继承的网络描述符。不允许算法自己请求任意 URL，授权计算只走固定管道 |
| 进程与 IPC | 独立 PID／IPC namespace，不暴露宿主进程或共享内存；不继承会话、TTY、调试入口和不必要的文件描述符。用户模块导入前生效的 seccomp 规则限制进程创建、调试、namespace／mount 等扩大权限的系统调用 |
| 权限 | 无可提权凭证、无宿主管理权限，禁止获得新权限；只暴露执行确实必需的设备和系统视图，不挂宿主 `/proc`／`sys` 或整棵宿主根目录 |
| 资源 | 本次调用有限的 wall time、CPU、内存、临时存储、描述符、输入／输出／日志大小和计算能力额度；使用已授权的 cgroup v2 资源域与 rlimit 等真实机制约束，不只让 Python 自觉检查 |
| 状态 | 每次全新进程、全新临时空间，不跨 Task／Branch／Decision 保留用户 Python 进程。共享只读代码允许复用，不共享可变模块状态 |

- 隔离必须覆盖 **编译时加载、import 顶层代码和实际函数体**。不能只在函数开始后包一层检查，也不能在加载阶段给代码更多宿主权限。
- Python `-I`／禁用用户 site、清理环境与固定 import 路径是避免环境污染的辅助措施，不是文件或网络隔离。`-I` 会忽略 `PYTHON*` 环境变量，不能一面启用它一面声称靠 `PYTHONHASHSEED` 等变量取得可重复性。
- bubblewrap 本身只是构造隔离的工具，保护范围取决于真实挂载、namespace、描述符和过滤规则。必须在运行预检中确认所需机制可用；缺少 user namespace、资源授权或过滤能力时，拒绝该侧求值，不能退回普通 Python 子进程冒充受控执行。
- 启动 CPython 所需的权限与用户代码执行阶段的权限分开处理，后一阶段限制必须在导入用户源码前完成。精确 syscall 列表、cgroup 委派和可运行的最小 Python 文件清单属于下一层实现细节，不能凭本草案称其已经安全验证。
- 本方案落实常规 OS 权限内的数据与访问边界，不声称能证明任意 Python 正确、不存在内核漏洞、没有所有侧信道，或已经满足恶意多租户的强化隔离要求。这不改变已有 Branch Runtime 的威胁模型。

###### 3. 授权的模型等计算能力

算法不因采用 Python 就能联网。候选调用方式如下，拼写待确认：

```python
reply = capabilities.call(
    "model",
    {"messages": [{"role": "user", "content": "…"}], "max_output_tokens": 128},
)
```

- `model` 是部署方预先注册的计算能力别名，而不是模型厂商 URL。该入口在 YAML 中显式请求该别名，部署方对当前 Policy／Task／Decision 授权后，调用侧才提供本次权限。没有声明或授权时接口不开放；默认无外部计算权限。
- 首版建议先支持结构化的文本模型推理请求与结果：消息列表和输出额度等字段有固定 schema，结果包含文本和真实 usage。模型／版本和 provider 参数由已登记的别名配置固定；未覆盖的工具执行、任意 HTTP 参数、任意查询或文件读取不穿过该接口。
- 受信调用侧通过本次独享管道接收请求，按当前绑定校验能力别名、数据出域范围、请求结构、预算和截止时间，再调用真实服务。凭证只留在受信侧，不注入 Policy 环境；请求中的 Task ID 或别名字符串不能为自己提权。
- 能力响应是计算输入，不自动变成 Authority、已验证 evidence 或可提交结果；实际来源、版本、请求／响应和费用由系统记录。依赖投机数据的请求和响应继续携带对应依赖，出域授权不能解除信息流约束。
- 计算能力只开放已定义的计算职责，不开放 Branch 操作、Runtime 操作或共享业务写入。若某个服务调用本身产生保证范围内的业务效果，必须先进入既有效果治理，不能称作“计算”绕过 Effect Coordinator；无法兑现时拒绝注册为这种能力。
- `admit` 粗筛不开放远程昂贵计算；其他 Decision 的可用计算也必须按本类职责授权，不能因为拿到模型别名就拿到额外系统数据。授权和限额由部署与机制决定，Engine 不自行发现服务或替作者换模型。

###### 4. 求值协议与增量预测

受信调用侧按已选编译结果建立本次上下文，通过带长度前缀的 JSON 消息与受限进程交换数据；约定与现有 transport 的 framing 可以一致，但必须在分配缓冲区前校验长度，并区分完整响应、截断和 EOF。这里不直接假定当前 `src/transport.rs` 已满足这些要求。

- 请求携带调用标识、Policy 内容摘要、Decision／子入口、归属和输入视图版本；算法只获得规定的数据副本，不获得可修改这些归属的句柄。
- 协议通道与 stdout／stderr 分开。`print` 只是限量诊断，不当成返回值；用户代码即使伪造管道消息也必须通过受信侧的上下文、长度、schema、授权和结果校验，管道持有本身不给系统操作权限。
- 一个普通调用返回一次正常结果或一次明确失败；协程、生成器、不可序列化对象和异常退出不自动转成成功。只有真实完整结果通过检查后才交回状态拥有者。
- `predict` 的增量生成不依赖一个长期存活的 Python generator。候选方案是在本类输入增加 `generation_context`，包含本 Edge 的轮次、已经发出的 Proposal 摘要和上轮显式返回的 JSON 续算数据；输出继续是 ProposalSet，`generation = incremental` 时另带可选 `continuation`，为空表示结束，非空供下一次调用。
- 每轮都用全新进程，旧 Proposal 不能被覆盖。系统将新 Proposal 分配 ID、绑定依赖并不可变地追加；续算数据属于同一 Policy／Task／Edge，大小受限、继承依赖，不是可自行确认的状态。Edge 结束、前提失效或 Policy 不一致时不能继续使用。
- 下一轮何时调用由既有运行编排与 Policy 配置决定，不规定必须紧密循环，也不因为返回 continuation 就无限占用执行。batch 不接受非空 continuation。上一轮已经发出的合法候选不因下一轮超时自动改写，后续去向按其实际有效性和当前处置决定。
- 同一调用的重复投递必须关联为同一逻辑结果，不能追加第二批同名候选或重复应用计划；无法确定是否成功的外部推理请求不能假定自动重试没有费用。相关去重状态由受信调用方保存，不放在用户模块全局变量中。

###### 5. 超时、取消、错误与结果应用

- 超时和取消由受信监督侧控制，不依赖用户函数检查 flag。超时预算包含启动、import、算法执行及计算能力等待；单次能力请求也受更小的剩余额度约束。具体额度是部署配置，不在机制中固定一个适用于所有 Decision 的数值。
- 到期或取消后，停止接受本次输出和能力请求，终止并回收本次整个执行资源域，等待进程退出并清理临时空间。不能只杀父进程便声称全部结束；无法完成回收时报告未完成责任。外部模型已经消耗的费用如实记录，不宣称取消回滚了外部计算。
- 迟到消息、重复响应、旧 Policy 或旧绑定的结果不落地。输入视图可能已变化时，由状态拥有者检查本次决定所依赖的具体前提；无关变化不必一律导致重算，依赖失效则不能继续使用旧结果。
- 一份计划在应用前先整体做结构和引用检查；执行中每步仍需确认当前条件和真实结果。多动作 `rebalance`／`dispose` 不凭 JSON 列表获得跨后端事务原子性，部分完成必须逐项记录，不能靠撤回内存中的列表假装外部动作已撤销。

| 失败位置 | 向调用方报告 | 不能做什么 |
|---|---|---|
| YAML、引用、版本、能力或加载检查 | 编译／绑定失败，定位到 YAML 路径或源码位置 | 不能省略出错 Decision、补默认函数或加载半份 Policy |
| 输入缺失、超范围或不符合 schema | 本次输入错误 | 不能补造事实后继续求值 |
| Python 异常、import 错误、进程崩溃 | 本次求值失败，附受限诊断 | 不能改成 false、空候选或其他正常策略选择 |
| 超时、取消、额度或权限被拒绝 | 对应明确失败及真实回收／调用状态 | 不能接受随后返回的“成功”或悄悄更换执行环境 |
| 返回类型、值域、引用或声明不合法 | 结果不合法，不应用该结果 | 不能裁掉非法部分后把其余部分冒充作者的完整选择 |
| 应用前已经过期或能力失效 | 状态拥有者记录未应用／延后／需重算 | 不能仅因函数执行成功就启动、提交或接受 |

求值失败本身不是一个正常 Decision。已有 candidate 的错误进入现有错误处置上下文；`match` 的异常可以由调用侧显式形成带原因的匹配错误事实，再进入 `dispose(error)`，不能变成 compatible。`effect` 未得到合法计划前不释放效果；`dispose` 自身失败时由机制维持阻塞或进入必须执行的安全取消／清理，不递归调用失败函数到无限循环，也不丢掉仍未完成的 obligation。是否恢复普通／权威路径由当前执行语义与已有有效选择决定，不是 Engine 偷换策略。

诊断至少保留 Policy 摘要、调用归属、Decision、阶段、YAML 字段位置／Python 文件与行号、错误类别和限量堆栈；路径使用逻辑源码路径。日志不默认输出完整输入、提示词、凭证或 sibling 产物。Ledger 保存实际运行事实；日志写入不等于决定已经应用。

###### 6. 本地求值、性能与首批边界

- 求值位置由控制面编排选择，而不是 YAML 中任意指定机器。只有所需数据、能力授权和隔离条件都能在本地满足的 Decision 才可下放；初期优先粗筛与 `extent` 等本地事实充分的入口。`select`／`rebalance` 保持控制面的同一 Task 全局视图，外部效果仍由 Effect Coordinator 落实。
- 本地和控制面使用相同固定代码、配置、schema、能力绑定与错误规则；不因位置不同静默换数据来源或算法。接入侧已完成的合法本地求值不在控制面再次执行，控制面仍检查归属和当前应用前提。
- “相同语义”不承诺任意 Python／模型每次逐位产生同一结果。算法若需要可复现，应使用显式提供的观测时点、配置种子或记录的能力响应，不依赖宿主时钟、随机 hash、集合迭代顺序或全局缓存；审计区分重放记录和重新调用随机模型。
- 每次新进程和隔离启动有延迟与 import 开销，尤其影响高频粗筛／`extent`。首版接受其作为可观察原型成本，不承诺加速；只缓存不可变产物／只读文件。后续进程池、批量计算或预加载优化须证明不串 Task／Branch 状态、不改变本次授权与错误语义，再另行讨论。
- 完整设计涵盖九类接口，不等于当前控制面已能应用全部意图。复杂 Reuse／Salvage 交接、补偿、状态版本／读依赖、前缀验证和在飞工作归属须由对应组件真实支持后才开放。多个 compatible 候选也不能靠 Engine 调用先后偷偷固定 winner；仍使用已有授权 resolver／Policy 的合法处置路径，缺少所需选择上下文时不能完成 Accept。
- 建议后续实现按三段推进：先九类数据与绑定检查，再真实隔离的编译／求值／序列化交付，最后逐个接入状态拥有组件。每段明确已实现范围，不用 stub、模拟结果或 metadata 变更声称生命周期已落实。不主动新增测试；仅在实际遇到符合既定条件的问题时先写稳定复现测试，再修复。

###### 7. 依据与留待更下一级的内容

本草案对应 Karsai 等的 Guideline 5–6（复用成熟语言）、8–12（必要领域结构、简单且避免重复）、14（熟悉记法）、19（组织多个文件）、21（保持一致调用形式）。接受 Python 缩进语义是对 Guideline 24 的取舍，不宣称某种 YAML 结构由论文唯一推导得出。实现事实另参考 [Python 命令行文档](https://docs.python.org/3/using/cmdline.html) 的 `-I`／`-S` 和 [bubblewrap 官方说明](https://github.com/containers/bubblewrap#sandbox-security)：二者都不能单凭名字证明本系统的隔离保证。

排队而不在本轮要求用户逐项回答：每个业务 schema 的字段注册与升级、精确 Linux 过滤／挂载配置及部署授权、第三方库构建与模型 provider 适配、严格 Fidelity 的可信依据、跨 Task 学习、进程复用优化和复杂交接方案。这些不改变已批准的 YAML／Python 分工、九类显式配置或机制边界。

### 投机执行模型的算法适用性审查（2026-10-22）

- 用户要求：检查当前模型是否适用于 `docs/survey` 中的算法，定位不符合的位置并提出通用模型调整建议；本轮在已有七算法首轮审查上继续覆盖全部算法，不直接改写既有目标语义。
- 已完成设计级审查与原文续查：读取 survey 算法部分全部 20 篇论文（ISP、DSP 分别计）及 TomasuLLM 的 arXiv HTML 正文，按预测对象、提前执行工作、验证依据、采用内容、失败去向和所需能力核对。覆盖矩阵、反例和建议在 `docs/development/spec-exec-abstraction.md` §8；原文位置索引在 §8.8。NVIDIA / LangChain 保留基于 survey 官方来源的映射判断，本轮未重审其代码；基础设施与相邻工作不混入算法覆盖率。
- 结论：Edge／Proposal、九类 Decision、树形依赖和效果屏障可保留，但尚不能声称完整覆盖。须区分正式条款冲突、已有意图的语义缺口、明确范围之外的能力与原型实现差距；未运行论文实现、未编写或运行测试，未复现性能或正确性结果。
- 当前优先收敛：已有 Reuse 如何只采用调用结果／受管状态，并保留主 Agent 的执行延续。续查确认 `glossary.md` 将 Reuse／Promote 一概映射为 accept，而 Branch Accept 按整 Branch 替换主执行；这与接入侧沿原调用返回结果及多篇算法保留主 Agent 的要求冲突。ObsSpec 还说明，采用候选生成进展时必须保留真实工具完成后的环境。
- 首项调整草案已具体化（尚未经用户确认），见札记 §8.7：区分结果 Reuse 与整 Branch Accept；明确采用范围、依赖与状态有效性；衔接原调用跳过和在飞工作；一致采用返回值及状态／效果；以实际结果完成责任移交与清理；single-winner 仍约束互斥选择，不把每次结果采用都解释为主 Agent 替换。只使用已有术语，不新增 Decision、实体或接口。
- 本轮按用户要求详细解释首项，未形成新决策：下一工具调用相同，只能支持在相应调用／状态契约下采用其执行结果，不能自动证明预测路径与主 Agent 分叉后的全部上下文等价。完整候选替换可以适用于已验证完整延续的情形；结果 Reuse 则需要保留原调用方，两者必须有各自明确的落实规则。D-031 已记录 command-level reuse 的输入／状态等价条件及 result、delta、artifact 的范围，接入侧也已有跳过原调用并返回授权结果的路径，因此缺口是这些意图如何与通用 Branch 生命周期一致衔接，不是整个系统从未考虑结果复用。工具返回值、必要状态／效果、未决依赖、在飞工作归属和清理必须一并处理；只返回 stdout、直接清除全部假设或不加条件拼接两个 Agent context 均不足以解决。建议先明确“复用结果不必一概等同于整 Branch Accept”，具体转换及能力要求留在对应下层；用户尚未批准该调整。
- 用户进一步指出：首项也涉及主 Agent 向候选传递信息与候选向主 Agent 传递结果的方向选择；若主 Agent 继续，必须明确树上如何表达已实施且成功采用的投机，避免与失败混淆。助手补充建议（未批准）：两方向都可能成立，但传递必须有明确范围和一致性依据，不能默认合并任意 Agent context。树边继续表示候选派生关系，受控结果交付不自动变成新的结构父子边；成功采用与失败回退可能得到相同的最终活跃树，仍须在控制状态中分别确认结果已采用、原调用已跳过、状态／效果责任已移交或失败后原调用继续，之后再清理，并由 Ledger 记录历史。不能仅因候选环境被清理就把它记作 failed／从未创建；现有“未被接受的 Branch 失败收敛”需与结果成功采用显式衔接。是否将完整投机历史保留在 Tree 内尚未决定，不能仅凭日志存在宣称当前模型已解决。
- 用户补充必须考虑嵌套过程中两个 Agent 都产生了产物。当前仍讨论首项，未形成决策。助手分析：须区分产物的内容、产生时所依赖的状态及未决 Proposal、采用后影响的对象；不能仅按产物来自 parent／candidate 或存在时间判断能否合并。主 Agent 分叉后修改了候选实际读取的文件，候选即使调用相同也不能自动复用；已识别为 Proposal 前提变化时沿用现有否定与依赖清理规则。有状态产物可能相互冲突，不能通过结果回传默认开放 merge。嵌套候选的后代还可能依赖候选未移交的状态：只采用父候选的一个结果不代表采用其完整执行基础，也不能自动移除后代全部假设或把后代挂到主 Agent。须证明目标执行提供后代所需的基础和依赖才能保留；不能证明时不沿该路径采用或按既有规则取消。候选原环境可继续保留与产物进入主执行是两回事。两方向信息传递都不能免除上述一致性检查；结果 Reuse 草案尚不能据此视为完整模型，通用 merge 仍未获授权。
- 第二轮分类（2026-10-23）：把“对不上”分成三类。A. 正式条款矛盾：`glossary.md` 把 Reuse／Promote 一并映射为 `accept`，而 `branch-semantics.md` §4.5 只描述整 Branch 替换；同时 §4.5 在接受候选时让所有 sibling 失效，会误杀 AOSpec／DynBranch 不依赖被拒 Proposal 的独立提前结果。B. 能力契约缺口：状态版本／读集合、批量验证证据如何拆成逐条 Edge 决定、候选执行配置。C. 范围边界：跨 task 缓存、DAG fan-in、engine plane、用户预览、TomasuLLM 的完全乱序提交。A 的第一项只需统一文字和交接规则；A 的第二项是唯一真正动模型的点，需在“保守不覆盖 independent offer”和“扩展其寿命”之间单独决定；B 作为 Contract／接入侧能力补，不放进九类 Decision。分类表见 `docs/development/spec-exec-abstraction.md` §8.9。
- 用户已确认（2026-10-23）：第 1 项按 §8.9 方向调整——`accept` 是 resolution = accepted 的落实；`Promote` 替换主延续；`Reuse` 只交已验证结果和按声明移交的受管状态，主执行保留身份、推理和上下文；带状态变更的 Reuse 受 D-031／Effect Management 约束；树、single-winner、效果屏障不变。已完成最小正式同步：`branch-semantics.md` §4.5、`glossary.md` 的 accept 映射，以及 `decisions.md` D-007／D-008／D-009 的 Promote 范围说明。嵌套产物、成功采用在树上的表示和在飞调用归属仍未定，后续再补。
- 用户已确认（2026-10-23）：第 3 项把状态版本／读集合做成可选能力，不要求所有后端；Policy 需要时由 Contract 校验，缺失则拒绝或明确降级；不改变 Branch 语义。具体能力字段和 Contract 表述留待下层设计，不新增命名。
- 用户已确认（2026-10-23）：第 5 项 `predict` 只生成 Proposal，不直接改主 Agent 的进程配置；候选执行配置属于 candidate／Instance 声明与 Contract／Fidelity 校验。
- 第 2 项 AOSpec §4.3 已解释：fork 记录 speculative action 和 origin environment version；JASV 仅在 action 相等且 pre-execution version 相等时复用；中间调用不改变受管 CoW 文件系统则 version 仍匹配，`PruneInvalid` 只清 stale fork；非文件系统输入／效果不在该验证范围。第 4 项“批量审核结果”是描述性说法：一次验证覆盖多步并返回结构化结果（SpecAgent `(j, r, ã)`；LEAP 在 `k+1` 个前缀上下文上并发取得 target 结果并按前缀消费；SMC 为宏级接受）。
- 用户选择第 2 项暂按 A（2026-10-23）：保留 §4.5 的 sibling 失效规则，不引入 independent offer 例外；AOSpec／DynBranch 的独立提前结果当前不覆盖，优先把系统跑起来。B 留作后续扩展。
- 用户提出第 4 项可由逐步审核原语组合成一整段（2026-10-23）。助手确认方向：不新增批量审核原语，`match`／`dispose` 保持逐 Edge；Policy 可以在一次审核后把结果分发到各 Edge。需补三点：按前缀顺序应用决定；修正动作要有进入执行的入口；后续 Edge 的证据要携带依赖，前一步被拒时失效。若暂不做修正动作，现有逐 Edge 接受／拒绝已足够。
- 后续优先问题（尚未展开设计）：批量验证证据如何拆成逐条 Edge 决定、前缀接受与错误后缀清理；DynBranch 的 independent offer 是否保留、跨 task 结果复用是否纳入底座范围及其授权、失效与所有权边界。局部验证可以提前，不能将它重新串行化；验证结果进入权威路径时仍须满足其前缀依赖。
- 其余排队，不在当前同时展开：预测器跨 task 记忆与在线更新；状态版本／读依赖及远期调用寿命（AOSpec 不应因每次 sibling 清理丢掉仍有效候选）；候选模型与执行配置；模型服务能力与 Fidelity；ISP 等投机预览的用户交互边界；未决数据流必须覆盖实际 Proposal 依赖，不能只要求接收方 AssumptionSet 非空。
- 第 1、3、5 项方向已确认，第 2 项暂按 A，第 4 项按逐 Edge 组合；嵌套产物／成功采用表示／在飞调用归属仍未定。本轮已同步 `docs/design/` 的最小文字，不修改 mental-model，不编写业务代码或测试，不提交已有工作区改动。

### 投机执行 survey 更新（2026-10-22）

- 已收敛：修正 `docs/survey/Speculative-sur.md` 的日期、缺失链接、方法与适用范围描述；首次提交日与修订日分列，公司参与依据正文机构或官方发布，不依据作者经历、资助或硬件名称推断。
- 已收敛：按投机执行算法、投机执行基础设施／沙箱区分主要贡献，交叉工作注明两类职责；普通并行与异步执行作为相邻路线单列。
- 已收敛：补入 TomasuLLM、SMC、SpecHop、Chronos，B-PASTE 作为初步工作记录；区分论文报告、公开实现、仅项目页和未独立复现的证据状态，不改变项目设计。
- 已收敛并补入 survey：本轮检索的 Speculate with Memory、Speculate While You Reason、AOSpec、DynBranch、LEAP、ObsSpec、SpecAgent 均值得记录；区分预测质量、整任务延迟和接受规则，并注明公开实现状态。
- 已收敛：SpecBox、Memory Compression for High-Fanout Agent Sandboxes（AgentZip）、腾讯云 CubeSandbox 不作为投机执行工作收录；预测资源需求、降低沙箱成本或提供快照/克隆能力，不等于投机执行。基础设施部分不能仅以“可被投机执行采用”为收录理由，其他候选也需据此重新筛选。
- 已收敛并补入 survey：Kairos 按安全用途的投机效果控制基础设施记录；Execution Edits 安全检查、Tracking State Footprints 按相邻安全语义研究记录，不称为投机执行系统。CUA-Sandbox、DSec 不因提供分支、隔离或规模化运行而收录。
- 已缓存本轮审查涉及的 21 篇论文 PDF／TXT 到 `docs/survey/`，按 arXiv 编号命名为 `<id>.pdf`、`<id>.txt`；`.gitignore` 已排除，文本头部记录来源、下载时间、PDF SHA-256 和提取命令。后续直接读本地文本，不重复下载。

### Rust 首批实现（2026-09-17）

- 已收敛：当前不要求 Agent 主动调用 AScout，也不开放显式生命周期协作；修正 reference-workload 的错误接入路线，具体产品尚未选择。
- 已收敛：先实现控制面最小闭环；保持代码干净简洁，避免过度抽象和冗长实现。
- 已收敛（2026-10-23）：原型阶段默认不编写测试；实际遇到可复现且很可能再次出现的问题时，先编写能稳定复现该问题的针对性测试，再修复代码到测试通过。不要求先定测试才能开始业务实现。
- 已收敛：开发阶段允许测试保持失败；测试必须直接验证真实功能和真实机制，不能为了获得通过结果而引入替代后端或模拟实现。此前为协议流程编写的本地测试后端及其端到端测试已删除；Firecracker 路线的测试必须直接覆盖真实 Firecracker 行为。
- 已收敛：Adapter 主动验证延后，初期先检查接口对应；不将接口检查冒充能力验证，目标语义不变。实现差距已记录在开发决策 D-035-I。
- 已收敛：初期使用一个 ascout 库，契约先作为内部 contract 模块，不独立拆分 ascout-contract crate；实际出现独立消费者的依赖需求时再讨论拆分。
- 已收敛：未经用户明确允许不得 Git commit；此约束覆盖仓库原先收敛后立即提交的约定。
- 待说明：select 返回 Edge 有序子集的含义；该项是既有策略语义解释，不新增设计。

### Policy Engine 首版职责（2026-09-17）

- 已收敛：九类 Policy Decision 都需要根据各自的规定输入作出规定输出；固定取值和常用规则只是决策逻辑的快捷表达。`launch` 的输出为布尔值，`true` 表示启动权威路径；Eager 只是恒定返回 `true` 的快捷定义。
- 已收敛：PolicyDefinition 当前只采用 Policy DSL 这一种入口。采用 DSL 的首要目的是降低 Policy 作者的心智负担；Policy DSL 自身规定 PolicyDefinition 的结构及九类 Decision 的输入输出契约，不为该规范另设 Template 实体或名称。
- 已收敛：每个 Decision 位置都必须有明确行为；快捷定义在编译后与直接编写的运行时算法使用同一种执行形式。
- 已收敛：Policy Engine 只有两项顶层职责：编译 Policy，以及根据规定输入对已编译的 Policy Decision 求值、作出本次 Policy Decision。解析、展开、检查、保存和提供编译结果均属于编译职责的内部过程或接口，不与两项顶层职责并列。作出 Policy Decision 不等于落实 Policy Decision：把 Policy Decision 应用到 Branch、Speculative Tree、外部效果或 Runtime 的行为不属于 Policy Engine。该职责不由运行位置限定；控制面内由 Policy Engine 求值，下放后由 Agent 接入侧以相同语义求值。Policy Engine 不负责选择运行位置或把编译结果交付给 Agent 接入侧。
- 已收敛：控制面根据自身编排需要查询编译结果，并负责把可在 Agent 接入侧求值的部分交付给接入侧。编译结果必须可序列化和反序列化，以支持跨进程或其他部署边界交付；接入侧只对已编译结果求值，不自行加载、编译或修改 Policy。
- 已收敛：首版不采用自定义结构化执行形式。Policy Engine 将 Policy DSL 编译为 Python 脚本；控制面可以调用 Policy Engine 对脚本求值，也可按需查询脚本并交付给 Agent 接入侧求值。以后可以在不改变 Policy DSL 和 Decision 接口的前提下改用其他执行形式。
- 已收敛：Python 脚本是初版编译产物和执行载体，不是 Policy 的第二种入口；用户仍只提交 PolicyDefinition DSL。脚本必须可序列化和反序列化，不包含控制面内存对象或进程内引用。

### Runtime Adapter 首版实现（2026-09-17）

- 已收敛：首版 Runtime Adapter 作为独立进程实现，与控制面通过本机 Unix socket 通信；这是原型实现选择，不改变目标语义，AScout 仍不规定统一 transport。
- 已收敛：控制面监听 socket，Adapter 启动后主动连入；Adapter 通过自身配置获得控制面 socket 路径。该路径属于控制面配置，不是 Adapter 私有细节，不违反 D-035 关于控制面不依赖 Adapter 私有 socket path 的约束。
- 已收敛：Adapter 连入后先提交能力声明；登记完成后，同一条连接才用于环境操作。初期登记只做结构与兼容性检查，主动环境验证仍延后，接口检查不得标记为已验证。
- 已收敛：首版实现按一个 Adapter 连接设计；多 Adapter 的连接与能力管理留到 Registry 实现时再定。
- 已收敛：控制面与 Adapter 由测试或启动脚本各自独立启动；控制面不负责拉起、重启或监督 Adapter 进程。
- 已收敛：首版隔离度仍按文档默认档——面向可信 Agent 的语义互不干扰（siblings 的文件、进程和内部状态互不可见），不承诺对抗性隔离；Firecracker 的 VM 级边界高于该档，属于机制侧加强，不改变威胁模型。
- 已收敛：第一目标后端选定 Firecracker（microVM / KVM）；先前的容器（Docker）选择废弃。选择理由是候选可以直接从父的快照恢复，做内存级延续，不再依赖重建或重放；本机 KVM 已确认可用。
- 已收敛：候选创建的目标路径是快照/恢复，而不是“共同基线 + 会话恢复”；引入时机见待展开项。已知代价：需要自备内核镜像、rootfs、guest 内执行代理（vsock）和网络方案；快照/恢复有纪律要求——宿主内核与 CPU 特征一致、磁盘按恢复克隆、时钟与熵重播、网络连接重建。
- 已收敛：候选的延续方式（重建 vs 快照）属于机制与后端能力，不进入 Policy 表达；Policy 不能要求快照分叉这类具体机制。延续能力由 Adapter 能力声明与 Contract 校验承载，并与 Fidelity 上界推导关联；机制侧在硬约束内选择可行方式，不可行时拒绝或降级。
- 已收敛：首版事件类型只保留一类——环境状态变化（意外停止或失败）；异步操作完成事件在结构中预留，首版按同步完成实现，不依赖它。
- 已收敛（首版范围）：环境与 Adapter 会话绑定，连接断开即按会话丢失处理，重连视为新会话并重新登记，不接管旧环境；Adapter 应保证不留下无人管理的孤儿环境。第一版不实现、不测试断线处理。
- 已收敛：分支是固定操作，所有合格 Adapter 必须支持（固定操作从六类变为七类）；接口按“分支”表达，不把快照或 fork 作为接口概念，快照/恢复只是分支的内部实现手段。
- 已收敛：分支的固定语义——从父环境当前状态派生一个独立的子环境；父环境保持运行；子环境与父、与其他 sibling 互不影响。不要求全内容派生；派生范围由后端能力决定，在声明阶段表达。
- 已收敛：分支在父环境停在 pending-result 边界时发起，不需要单独的分支点对象；按需建立仍然成立。
- 已收敛：第一版不提交能力声明、不做验证，只实现固定接口。
- 已收敛：Firecracker 资产（二进制、内核、rootfs）由用户准备并放入约定目录；Adapter 从配置读取路径。
- 已收敛：Firecracker 首批测试直接覆盖真实 microVM，不使用 mock 或替代后端；测试范围为 Adapter 接入、完整生命周期、请求重试、在飞操作打断、Firecracker 进程意外停止和真实分支六个场景，开发期间允许保持失败。
- 已收敛：Firecracker 实现同时提供 SSH 与 vsock 两条真实通信通道；原始 rootfs 不因测试被修改，测试只操作独立副本。
- 已收敛：`operate_environment` 只使用 vsock Agent 接入侧；SSH 仅用于真实连通性与故障诊断，不进入操作执行或自动回退路径。
- 已收敛：主机侧可执行文件命名为 `ascout-runtime-adapter`，guest 内 Agent 接入侧命名为 `ascout-agent-access`。
- 已收敛：Runtime Adapter 通过 `--config` 接收 JSON 配置；首版字段为 `control_socket`、`adapter_instance`、`firecracker_path`、`kernel_path`、`rootfs_path`、`guest_access_path` 和 `state_directory`。SSH 诊断配置不进入 Runtime Adapter 配置。
- 已实现并由真实 Firecracker 验证：Adapter 接入、microVM 建立、vsock guest 命令执行、暂停/恢复、终止/清理、请求重试去重、在飞操作打断、意外停止事件，以及通过全量 snapshot/restore 建立两个相互隔离的 sibling。另已通过隔离网络 namespace 内的真实 virtio-net + SSH 诊断测试。
- 已收敛：早期实验代码直接删除，不留废弃记录。
- 已收敛：分支操作命名为 `branch_environment`。
- 已收敛：suspend 对在飞操作采用立即中断语义——先取消并结束在飞操作，再冻结环境；第一版不引入优雅停止；取消时终止整个进程组，等待 guest 侧清理完成后再冻结。
- 已收敛：网络是环境的基本能力，由 Firecracker 后端在建立环境时创建（独立 netns、网桥、TAP、veth、双层 NAT），cleanup 时回收；Adapter 通过免密 sudo 调用 `ip` 与 `iptables`，Firecracker 在 netns 内以降权用户运行，终止按进程组回收。
- 已收敛：codex 启动验证的测试命名为 `codex_starts_and_reaches_the_model_service`，经 Adapter 建立环境并在 vsock 通道内执行。
- 待展开：环境网络参数（上游接口、地址与 MAC 分配）是否进入 Adapter 配置；当前上游自动探测，guest 使用固定地址。
- 待展开：codex 验证的断言强度——模型服务端业务拒绝（如额度不足）是否算作通过。
- 待展开：派生范围的最低要求与在能力声明里的表达（声明阶段）。
- 待展开：延续能力如何进入 Adapter 能力声明、如何与 Agent 接入侧的声明配合，与 continuation branching backend 议题合并；Policy 要求侧如何在 Policy DSL 中表达隔离与能力要求，与 Policy DSL 讨论合并。
- 待讨论：Registry 初期接口对应检查的具体内容。

### Runtime Adapter 组合模型（2026-09-18）

- 已收敛：组合模型放在开发层（方案 b）——契约不引入“后端”概念，环境仍是唯一单位、声明仍按环境内容给最终保证；AScout 自研 Adapter 时提供参考组合框架，按内容装配机制并由装配推导组合声明。第三方 Adapter 不受这套框架约束。
- 已收敛：覆盖模型采用乙——每个机制声明自己能影响哪些环境内容，允许重叠；组合后的保证逐内容、逐属性取最弱，并计入接缝（共享目录、代理、转发、控制入口）。典型情况每个内容有一个主要边界归属，其他机制是参与者。
- 待展开：参考组合框架的生命周期编排、声明推导、身份与所有权、后端接口边界。

### SOW 1.2.2 投机决策模型

已收敛：SOW 1.2.2 采用“投机决策模型”作为技术方法名称。本节按“投机语义对象的统一建模、策略决策面的组合化表达、策略意图的受管闭环落实”组织，形成“对象—决策—落实”的连续结构。

已收敛：对象建模以待决结果边界、Edge、Proposal、Branch、Authority/validator、Speculative Tree 和未决 Proposal 依赖统一表达单候选、多候选、连续及嵌套投机；典型等待真实结果流程不再表述为所有投机都必须遵循的固定线性骨架。

已收敛：SOW 1.2.2 中的技术名词必须在第一次出现时完成必要定义，不能先使用、再用后文补充例外或修正含义。对象本身作为句子主语，避免以“项目如何做”为重复句式；只保留理解对象关系所必需的解释，不再额外复述已经由定义表达清楚的区别。

已收敛：策略表达覆盖九类 Policy Decision 及其阶段映射，明确阶段是职责视图而非固定调用顺序；breadth、嵌套 depth、树形展开方式和 Fidelity 均由决策组合产生，不另设为孤立的固定算法开关。

已收敛：九类 Policy Decision 在 SOW 中按“机会与运行时机、投机展开方式、验证与处置”三组解释，每组单独成段；先说明阶段视图和非固定顺序，再说明各组职责，最后说明 breadth、depth、拓扑和 Fidelity 如何由决策组合产生。

已收敛：受管落实说明 Agent 接入、本地与全局 Decision 分工、事件驱动控制闭环及 Policy/Mechanism 边界。1.2.2 只说明固定约束由底座强制，不展开具体隔离、效果或后端机制；候选内部状态和外部效果的治理留在 1.2.3，策略要求与 Runtime 能力的声明、验证和降级留在 1.2.4。

已收敛：受管落实按“Policy 不拥有运行状态、Agent 执行接入、本地与全局 Decision 分工、事件驱动闭环、固定约束与章节边界”分段，避免在同一段混写责任边界、接入方式和控制流程。

已收敛：SOW 1.2.2 的最终正文采用 `1.2.2.1` 投机语义对象的统一建模、`1.2.2.2` 策略决策面的组合化表达、`1.2.2.3` 策略意图的受管闭环落实三级标题；正文以用户确认的版本为准。

### SOW 1.2.3 行为状态治理

已收敛：SOW 1.2.3 以“确定治理边界、形成处理方案、完成状态收敛”为分类依据，不再把本地状态、外部效果、资源和失败处理作为四个并列类别。对应三级标题为 `1.2.3.1` 治理边界的确定、`1.2.3.2` 状态变化的差异化处置和 `1.2.3.3` 状态生效与失败收敛。

已收敛：治理边界依据变化的归属和可见范围确定，区分 Branch 本地状态与能够影响 Branch 外部可观察行为的共享或外部影响。Outcome 和运行消耗不作为与二者并列的主要类别，只在需要受控交付、被 Policy 明确约束或改变保证范围内业务行为时进入相应治理路径。

已收敛：状态处置保留 `StateDomain × EffectClass` 二维模型，只说明 StateDomain 表示变化发生在哪里、EffectClass 表示变化能够如何收敛，不在 SOW 中枚举完整状态域、资源项目或实现清单。差异化处置直接承接本地状态隔离、外部影响控制和伪装执行。

已收敛：收敛部分只说明受管变化的 Branch 归属、可见范围、未决责任以及接受、淘汰和失败后的去向；控制状态必须依据实际完成结果更新，不能以删除候选记录代替真实清理。

已收敛：SOW 1.2.3 必须按“Branch 行为产生变化—识别纳管范围—选择差异化处置方式—记录未决责任—随接受、淘汰或失败完成收敛”的因果链展开，使治理边界、差异化处置和失败收敛三个部分构成连续过程，而不是三个并列的分类说明。

已收敛：SOW 属于任务书，只说明行为状态治理的方法、作用、主要处理方式及其前后关系，不展开服务插件接口、内部组件调用或状态域实现清单。伪装执行需要在这一尺度上说明适用问题、基本过程、适用限制和验证约束：它拦截受支持的真实服务请求并生成有来源标记的模拟响应，使 Branch 在不产生真实外部影响时继续推进；模拟响应是未验证假设，不是真实 Authority 或自动成立的 Proposal，不能未经验证被 winner 或共享业务状态依赖。

已收敛：SOW 1.2.3 使用“处置方式”串联状态识别与最终收敛，不把“处理计划”扩展为覆盖 Branch 本地状态和外部效果的统一正式对象。`EffectPlan` 仅保留其在正式设计中作为 `effect` 决策返回对象的既有含义。

### SOW 1.2.4 投机执行契约

已收敛：SOW 1.2.4 按“后端能力验证、策略要求对齐、运行履行”组织，不再把运行确认和能力偏差拆成两个相互重叠的环节。对应三级标题为 `1.2.4.1` 后端能力的声明与接入验证、`1.2.4.2` 策略要求与已验证能力的统一表达和 `1.2.4.3` 契约履行与失配收敛。

已收敛：契约直接约束 Policy 的决策表达和 Runtime Adapter 在控制面边界上的可观察行为，不规定 Runtime 内部实现，也不承接 1.2.3 的外部影响治理。SOW 只说明策略要求、后端保证及其共同语义，不枚举完整环境内容、隔离属性和结果状态。

已收敛：Adapter Registry 通过实际环境操作验证声明并形成 Capability View，Policy Engine 据此检查策略是否存在合法执行路径；无法满足时拒绝或明确降级。运行期由 Adapter 的实际结果和事件确认契约履行，能力偏差、配置变化和部分失败均在同一环节显式收敛。

已收敛：异步运行记录、影子运行和轨迹回放只作为结果保真度、资源成本和契约履行的验证证据，不参与当次实时决策，也不单列为与契约表达、校验或履行平级的技术环节。

已收敛：SOW 1.2.4 按“Runtime Adapter 声明后端保证—接入校验形成可依赖的能力视图—Policy 产生显式能力要求并与能力视图统一对齐—运行结果持续确认履行—能力失配时收缩并收敛”的因果链展开，使后端能力验证、策略要求对齐和运行履行构成连续过程。

已收敛：SOW 1.2.4 属于任务书，重点说明投机执行契约解决的跨层错位问题、三阶段方法及其作用，不展开具体接口字段、校验用例或 Runtime 原生调用。能力声明只表示后端声称能够提供的保证，接入校验只确认存在合法执行路径，实际履行仍以运行期返回的结果和事件为准。

### SOW 1.2.5 技术路线

已收敛：SOW 1.2.5 按“系统设计完善—原型系统实现—实际负载接入与系统完善—效果评估、优化与学术成果形成”四个阶段展开，从技术方案、可运行系统逐步推进到实证结果和学术成果。各阶段必须说明主要工作、形成的阶段成果及其对下一阶段的支撑关系。

已收敛：实际负载阶段与效果评估阶段承担不同任务。实际负载阶段用于确认系统在真实 Agent、代码仓库和代表性任务中能够正确、完整且稳定地运行，并据此完善系统；效果评估阶段在系统版本稳定后，通过可比实验判断投机执行的收益、适用条件和资源代价，进行针对性优化和重新评估，最终形成可复现的实验结论与学术论文。

已收敛：技术路线不能只用整段文字概括阶段目标。四个阶段分别拆成多个按依赖顺序推进的小点，每一点说明具体工作及其为下一点形成的基础，使路线呈现“逐步实现、逐步接入、逐步验证和逐步形成研究结论”的实施过程，并与1.2.2至1.2.4侧重技术原理和方法的写法区分开。

### Speculation Contract 的定位与实现

已收敛：`Contract` 直接约束 Policy 的表达，以及 Runtime Adapter 在控制面接入边界上声明和返回的可观察行为，目的是保证策略与后端接入语义正确且对齐，而不是约束控制面或定义 Runtime 内部实现。控制面是 AScout 自身实现的 Mechanism 主体，是 Contract 的实现方，不属于被约束对象。Contract 不是部署配置或独立 Manager，不能由单个任务或 Policy 修改。

已收敛：`Mechanism` 只作为抽象责任词，是 Contract 的固定实现，不是插件或独立组件；它由 AScout 自身能实现、能强制的固定机制共同构成，由控制面组件协同完成检查与编排，不归属任何单一组件。

已收敛：Runtime 是通用执行后端（例如 docker、虚拟机或进程管理能力），不要求提供 AScout 专用接口。Branch 隔离、效果、cleanup 等 AScout 语义由 Runtime Adapter 用后端原生能力组合表达；控制面只能通过 Adapter 访问后端，因此接入校验的对象是 Adapter，Adapter 通过即代表其背后的 Runtime 能承载相应语义。Adapter 不能静默放宽语义，后端确实表达不出某语义时应拒绝接入或明确降级。

已收敛：Contract 的执行路径统一描述为 `Policy 决策 → Policy Engine 校验决策取值 → 控制面机制编排为受管操作 → Runtime Adapter 映射 → Runtime 执行并返回结果/事件 → 控制面更新状态`。Policy Decision 描述高层语义，取值合法性随后端能力变化，需结合 Adapter 报告能力校验；高层决策展开为受管操作序列，Adapter 按控制面接入契约映射为后端行为。通过 Adapter 的能力声明、接入时基本语义测试与运行期校验来判断其是否兑现后端能力；不直接校验或约束 Runtime 内部实现。

已收敛：失败语义。对尚未被接受的投机 Branch，失败收敛到"该 Branch 在 Speculative Tree 中从未被创建"，不重放该操作；被接受的 Branch 已完成使命，不适用。Contract 文档写共同承诺一句，撤销与清理机制归 control-plane。

已收敛：文档拆分。`docs/design/speculation-contract.md` 收窄为"定位 / 约束什么（单节）/ 如何约束 / 完整控制闭环"；控制面内部机制（组件分工、状态 owner、操作编排、失败清理）下沉到 `docs/design/control-plane.md`。同步更新 glossary、design-model、mental-model、index、effect-management、decisions 与 architecture 的相关引用。

已收敛：`spec-exec-abstraction.md` 不再保留独立的“控制面组件职责”章节。Agent Control Gateway 的目标职责补入 `control-plane.md`；Policy Engine、Speculation Coordinator、Effect Coordinator 与 Ledger 的现行职责以该文档为准。论文相关表格和数据继续作为事实依据，分别留在札记的 Effect、资源与准入、Shadow/Replay 等研究主题中；旧六插件、`site`、旧状态模型和重复贯穿示例不迁移。

### ASCout Agent 分支执行底座

当前目标范围是树形、单一 winner、无通用 merge、同级分支默认不协作的 Agent 分支执行。后续若支持 merge、多个 winner、DAG 或同级协作，应作为新的执行模型扩展，不能反向破坏当前基础语义。

已收敛：端用户只通过 AScoutAgent 视图直接操作当前有效的权威执行主线，只能看到该路径产生的内容、状态与交互。投机 Branch 不建立另一条用户对话，也不能携带未决 Proposal 直接向用户输出或提问。用户输入或中断由当前有效主线接收后，仍可按输入传播规则交付给运行中的 descendants。

已收敛：Branch 接受、丢弃、重新挂载与清理不需要端用户参与。Policy 与授权 resolver 形成决策或处置意图，控制面编排，Runtime 经 Adapter 落实状态转换。权威路径变化后，用户继续操作更新后的有效主线，不选择或重绑定 Branch。

已收敛：用户阻止当前权威路径上的操作时，依赖该操作继续发生或依赖其原结果的 Proposal 前提已经变化。现阶段不做兼容判断、保留、挂起、修复或重新对齐；凡识别出 Proposal 前提变更，所有依赖它的 Branch 及其 subtree 一律剪枝、取消并清理。未依赖该前提的 Branch 不因无关输入而全局失效。

下一项只讨论以下第 1 项，其他项目暂不展开：

1. **Branch 执行状态与控制上下文**：明确 Branch 除树关系外需要保存的执行状态、假设、能力、提交证据、未决约束、等待/暂停/失败状态和 Runtime 对齐信息。当前 `committed: bool` 与 disposition 只能视为临时 skeleton，不能直接视为长期公共状态模型。
2. **信息与约束传播**：继续明确用户或 parent 向 descendants 的传播、child 向 parent 的 proposal/outcome summary，以及同级 Branch 不直接协作的边界；Proposal 前提变更导致受影响 subtree 一律剪枝的当前语义已经确定。
3. **Resolution 与 canonical continuation**：明确 `accept` 选择的是后续 canonical continuation，而不是简单宣称 candidate 是客观 ground truth；继续明确 winner、旧 parent 和必要前置条件的关系。
4. **控制面操作与后端结果对齐**：明确语义动作、Adapter 可观察结果、规范化事件和控制状态更新之间的闭环，特别是原子性、失败和状态对齐；不定义 Runtime 内部机制。
5. **Policy 与用户自定义操作的组合边界**：明确 policy 可以选择什么、控制面固定什么、Adapter 必须证明后端落实什么，以及上层可使用的 Branch context、事件、outcome 和约束输入。

### Mental model 的后续维护

- `docs/mental-model.md` 是项目最高层的全局认知框架，默认稳定但不是不可修改的真理。
- 局部设计和实现不能静默反向改变 mental model；若目标、证据或错误假设要求改变全局模型，必须显式回到这里讨论，并检查直接子文档影响。
- 当前已收敛的 Agent 边界是：Agent 默认无感知，但可通过受控入口协作；AScout 面向规范化 operation 和 `pending-result boundary`，具体 Hook、CLI、命令拦截和进程观察方式不属于目标语义。
- 已确认：`Policy / Mechanism / Runtime` 是抽象责任模型，描述策略、机制与运行能力之间的抽象关系；`Agent 接入侧 / 控制面 / Runtime Adapter / Runtime` 是系统结构模型，描述责任、所有权和跨侧协作方式。两者是不同视角，不互相替代；Mechanism 横跨多个架构侧面，Runtime Adapter 是四侧结构中的连接边界。
- 已确认：不保留独立的 `design-principles.md`。原有四项内容不作为独立设计原则文档保留；跨领域设计取舍准则随 `design-model.md` 迁移，见下节「文档职责与 overview.md 解散」。
- 待同步（design-model 已重组，本节未改）：design-model §2 工作模型现只保留"投机执行过程"（§2.1）；原 §2.2 投机边 Edge（含决策面）与原 §2.3 组合与嵌套已并入支撑模型 §3.1，成为新的"Policy Decisions 与投机边"节。`mental-model.md` §2 中"工作模型包含投机边 Edge、组合嵌套形式化"及 §3 中"Policy Decisions"的描述需相应更新。

### 文档职责与 overview.md 解散

- 已确认：`docs/development/ascout-agent-positioning.md` 中的全局定位、四侧架构、对象 ownership、控制面职责、Policy 边界、第一版接口和长期方向已有正式文档承接；连续 Agent 体验和用户操作导致的 Branch 前提变更也已经收敛并写入正式设计。该开发文档已无独有内容，因此删除，不保留空壳或索引引用。
- 已确认：`docs/development/spec-exec-abstraction.md` 中“正交轴与取值域”整体迁入 `docs/design/policy-decisions.md`。本次先原样迁移，不同时修正其中的轴分类、论文证据、Fidelity 定位和 Mechanism/Policy 划线；这些内容后续在 Policy Decisions 文档中逐项整理。
- 已确认：`docs/design/overview.md` 是职责混杂的"抓包"文档，不保留为合并决策的总文档；其内容按主题归位，详见下方清单。
- 已确认：`docs/design/decisions.md` 与 `docs/design/speculation-decisions.md` 只是**速查手册与原始讨论记录**，记录决策当时的选择与理由，不替代正式主题文档，也不充当语义权威。每条决策的语义内容应分散到各自合适的主题文档。
- 已确认：新建 `docs/design-model.md`（设计模型），与 `docs/architecture.md` 平级、均为 mental-model 的直接下一级。`architecture.md` 指导**实际系统结构**（mental-model §4），`design-model.md` 指导**抽象概念**（mental-model §2 工作模型 + §3 支撑模型）。其下一级主题文档包括 `policy-decisions.md`、`effect-management.md`、`speculation-contract.md` 及待定的 Branch 语义文档。
- 已确认：5 条跨领域设计约束随 `design-model.md` 一起迁移，不再作为独立 `design-principles.md`。
- 已确认：`overview.md` 中"概述"段（"AScout 是什么"）删除，mental-model 已承担该职责。已执行的内容归位：Branch/Tree/可见环境/输入传播/隔离通信/Agent 感知/Workspace 归入新建的 `docs/design/branch-semantics.md`；隔离与 Runtime 安全边界归入 `architecture.md`；世界状态分类归入 `effect-management.md`；跨领域约束随 `design-model.md`。

### Edge 的定位与粒度（design-model §3.1）

- 已确认：**Edge 是一次投机机会**（`u` 的输出 pending、`v` 依赖该输出的可投机依赖点）；一个 Edge 上可有多个 Proposal / SpecTask / candidate Branch。它是**抽象模型**，不是注册实体。可投机性由效果纪律与决策点共同承载，不再使用独立的 admissibility 元数据。
- 已确认：**投机注册动作 = 在 pending-result boundary 上创建携带 Proposal 的 candidate Branch**（即 branch-semantics 的 branch 调用），不引入 site / "Edge 注册"作为额外注册实体；Speculation Coordinator 的 `(edge_id, proposal_id)` 依赖状态以 Edge 和 Proposal 标识关联，site 并入 edge 术语。
- 已确认：Edge 的职责定位 = ① 统一不同投机对象（LLM 调用、工具、命令、远程请求、人类回复）的抽象；② 作为设计 policy decision 与 contract 的作用对象；③ 局部视角（单次投机机会如何投机的决策点）。嵌套与组合是补充的全局视角（多次投机机会如何串联、共享前提与信息流）。
- 已确认：policy decision 多数挂机会粒度：权威启动（N 个候选共享同一 `A_u` 与真值 `i`）、BestOfN(breadth)、single-winner 处置都在 round 层面；逐 Proposal 的是 Predictor 与 Equivalence。机制侧证据：同一 Edge 下的 Proposal 共享权威依据并统一进入裁决，`edge_id` 与全局唯一的 `proposal_id` 各自承担机会归组与候选引用，§2.1 允许“一个或多个 Proposal”。
- 已确认：**设计模型决策面 = 组合视图，不与 §2.1 阶段流程逐段对应**。§2.1 阶段是流程视图（机会发现→…→处置），§3.1 决策面是组合视图（policy 真正被问的作用点）；决策面是 Policy 的落地接口。
- 已确认：**决策面为九槽元组** `S = (admit, select, rebalance, extent, predict, launch, effect, match, dispose)`。`admit` 以粗判和精判决定机会是否进入待选集合，不负责立即启动；`select` 面向同一 task / AScoutAgent 内全部已准入但尚未启动的 Edge，直接选择当前启动的有序子集；`rebalance` 面向同一 task 内已运行或暂停的投机 Branch，决定暂停、恢复或抢占。其余槽分别负责单次过程推进、Proposal、权威值取得、效果、匹配与处置。具体 depth、breadth 与排序方式由 Policy 表达，不是机制固定参数。
- 已确认：**authority 不再独立成槽**（D-041）。权威值来自谁是 Edge 的固有事实，不是可替换决策；`launch`（Eager / Lazy / Absent）决定是否取得、何时取得；Absent 允许放弃天然存在的权威值（含用户输入），是否允许由 Fidelity 契约校验；validator 归属 `match`。原"轴 1 / 轴 5（authority）拆开"的旧表述废弃。
- 已确认：**admissibility 并入 effect，不再作为独立概念**。原"Edge 注册与 admissibility"机制项、Edge 携带 admissibility 元数据、六实体表均已从 design-model 移除或改写；可投机性由效果纪律与 effect/准入决策承载。阻塞语义：投机执行遇到不可安全暂存/释放的外部效果时停在门口保留前置，accept 才释放、discard 即清理，而非事前整条拒绝。
- 已确认：**固定约束不进槽**（Mechanism 强制）：效果纪律与阻塞、隔离与信息流（非空 AssumptionSet 不进权威路径）、single-winner 与共享验证依据、注册必经 candidate Branch。**Fidelity 是导出契约不是决策槽**：由槽位取值推导上界、加载期对撞、运行期 shadow 审计。
- 已确认：原 §2.2 投机边与原 §2.3 组合/嵌套已并入支撑模型 §3.1；工作模型只保留 §2.1 投机执行过程。
- 连带待改（本轮已处理 design-model、policy-decisions、effect-management、glossary 与 spec-exec-abstraction 的相关正文）：speculation-decisions D-008 的"Edge 注册与 admissibility"机制项措辞；其他历史决策记录中的旧术语暂不改，待后续统一清理。
- 已知但本轮不展开的三条缝隙：流式/交互依赖（`u` 未产完 `i` 时 `v` 部分消费）、L3/Bypass 无真实 `i` 的措辞、sibling command reuse 与投机边的关系。
- 已确认文档归属：`design-model.md` 只保留九类 decision 的上层定义与组合/嵌套抽象；各 decision 的完整语义下沉到 `docs/design/policy-decisions.md`；效果到达处理边界时的阻塞、接受后继续和淘汰后清理由 `docs/design/effect-management.md` 定义；Contract 对 Branch 执行要求与 Runtime 能力的匹配由 `docs/design/speculation-contract.md` 承载。
- 已确认：Effect 不是 Edge 的一次静态可投机标记。一次投机执行可包含多个 step 与多种效果；Policy 选择处理纪律，Effect Management/Runtime 校验并落实。遇到不能提前释放或暂存的效果时，可以阻塞当前投机执行并保留前置进展；只有无法安全阻塞、接受后继续或淘汰后清理时，才拒绝相关环节的投机或明确降级。
- 已确认：`extent` 适用于所有投机执行，限制单次过程内部的推进边界；到达边界后暂停等待裁决，不自动 discard。它不同于嵌套深度：嵌套是在新的 Edge 上递归调用 `admit`，`extent` 不创建新的投机层级。
- 已确认：`extent` 是 Policy 根据过程事实动态计算的停止条件集合，不要求 AScout 通用估算未来接受概率、收益或恢复成本。`step` 是可被控制面观察、具有明确前后状态的 Agent 继续单元；Runtime 内部 token、系统调用和线程调度默认不是 step，等待 pending-result 不算持续推进，嵌套 Edge 拥有独立 step 计数。`extent` 指标收敛为三类谓词：`progress_stop(process_state)`（已观察进度，如 step 数）、`utility_stop(process_state)`（已实现且可观察的目标/效用或明确预算，不是未来收益预测）和 `semantic_stop(process_state)`（任务语义完成）；任一成立即表示 Policy 期望边界到达。裁决边界、特殊效果边界、Runtime 硬上限、取消和失败不属于 extent，分别由验证/生命周期、effect、Contract/Runtime 和运行控制处理；extent 不能绕过这些边界。
- 已确认：当前层只规定 `process_state` 的输入事实类别，不枚举无限增长的具体指标字段。必须可见的类别是过程进度事实（至少 step 数和阶段/位置）、已实现结果事实（输出、完成项、明确预算或资源计数）和过程语义事实（任务/Policy 定义的完成状态）。pending-result、effect 阻塞、Runtime/Contract 限制、取消和失败等控制事实可用于说明停止原因，但不是 extent 指标；具体字段、事件格式和领域扩展指标下沉到过程状态与接口文档。
- 已确认：去掉通用 `position` 字段。`process_state` 当前层确定的规范字段为 `steps_completed`、`phase`、`active_time`、`resource_counters`、`result_facts` 和 `semantic_facts`；后四项可按领域扩展，但必须保持为可观察事实，不能变成预测分数或机制推断值。
- 已确认：去掉通用 `position` 字段。`process_state` 当前层确定的规范字段为 `steps_completed`、`phase`、`active_time`、`resource_counters`、`result_facts` 和 `semantic_facts`；其中除前两项外可按领域扩展，但必须保持为可观察事实，不能变成预测分数或机制推断值。字段语义已在 `policy-decisions.md` 以表格列出。
- 已确认：`predict` 的输入契约包括 `edge_context`、`process_context`、`assumption_set`、可选 `history`、`policy_config` 和 `capability_context`。输出统一为 `ProposalSet`，支持单候选、有限候选集、带 Policy 元数据的候选集和增量生成；每个 Proposal 至少有 `payload`，可带 `provenance`。Proposal 是 Edge 边界的候选输入，可为原子值或结构化值，不是 Branch 或已执行轨迹；`rank`/`score` 只供 Policy 排序调度，不能被 Mechanism 解释为正确性或 `match` 结果。Proposal 数量是 breadth 的取值结果，无法生成有效候选时不创建空候选 Branch。
- 已确认：`match` 使用 `match(proposal, evidence, match_context) -> MatchResult`；`evidence` 可以来自 Authority 的真实结果，也可以来自 validator 的可接受性判断。`MatchResult` 至少区分 `compatible`、`incompatible`、`indeterminate`、`error`，并可带 `score`。分数由用户/Policy 规定函数计算并定义含义、范围、方向和阈值，AScout 只提供承载、记录和在同一 Edge 的相同匹配规则上下文内比较的机制；不把分数解释为正确概率，也不跨 matcher、版本或 scale 比较。确定性 predicate 可无分数，模糊或带容差关系可用分数；不确定或出错不得静默接受。
- 已确认：`dispose` 根据 `resolution` 和上下文返回可组合的 `DispositionPlan`，而不是单一终态。`resolution` 至少区分 `accepted`、`rejected`、`indeterminate`、`error`、`cancelled`、`preempted`、`extent_reached` 和 `failed`；处置意图包括 `Reuse`、`Promote`、`Salvage`、`Re-execute`、`Discard`、`Compensate` 和 `Overwrite`，可组合为例如 `Salvage + Discard` 或 `Compensate + Discard`。Policy 只表达处置意图，不重新判断 `match`、不决定继续推进；Branch 转换、清理、资源释放和 commit/discard 由 Mechanism 编排，并由 Adapter 返回足以确认后端已落实的结果。
- 已确认：`effect` 对每个 step 的已识别效果返回 `EffectPlan`，模式包括 `default`、`simulate`、`transform`、`stage`、`mediate`、`block`、`allow` 和 `deny`。`simulate` 是伪装执行：服务 Adapter 拦截请求，不访问真实服务，在 Branch 内返回明确标记为模拟结果的响应；该响应不是真实 Authority，必须记录生成来源和未验证假设，并由后续 Authority、validator 或明确规则验证。`default` 使用 Adapter/Contract 声明的默认方案，未声明时不得静默放行。`StateDomain` 与 `EffectClass` 保持正交，处理模式需经 Effect Management、Contract 和 Runtime capability 校验。
- 已确认：组合与嵌套当前只定义用户输入这一类跨 Edge 信息传播。用户中断也是用户输入的一种，和普通用户输入一样传播给当时仍在运行的全部 descendants；普通输入作为输入事实进入 Branch，中断触发各 Branch 独立停止/取消和清理。中断不通过 `AssumptionSet` 传播，也不引入集中式子树回收机制。
- 已确认：组合与嵌套的完整边界已写入 `policy-decisions.md` §4：同一 Edge 上的 Proposal/Branch 只是并行候选，由 `predict` 的 breadth、隔离和 single-winner 约束决定，不另设组合语义；嵌套 Edge 继续使用当前 task 已绑定的同一 Policy 及其 task-level 配置。`AssumptionSet` 只表达未验证依赖，跨 Edge 还通过树结构归属、用户输入传播和生命周期控制连接。祖先假设被否定时，控制面向受影响 Branch 直接发送取消命令，各 Branch 独立停止和清理，不设置集中式整棵子树回收机制。
- 已确认：AScout 暂存通过 `admit` 但尚未启动的 Edge；粗判识别可能的投机入口，精判只决定是否登记和保留，不决定立即生效。Policy 的独立 `select` 在同一 task / AScoutAgent 内的全部有效待选机会间直接选择当前启动的有序子集，可选一个、多个或暂时不选；未选机会继续等待，直到后来被选中或客观前提失效，不要求额外的主动放弃接口。Speculation Coordinator 维护待选集合及其有效性，并在资源、Contract 与正确性硬约束内执行选择，启动前不重复 Policy 精判，也不使用固定价值公式替代 Policy。独立的 `rebalance` 面向已运行或暂停的投机 Branch，决定暂停、恢复或抢占；`preempt` 以 `resolution = preempted` 进入 `dispose`。best-first、跨 Edge 排序公式、浅层分叉/深层单链与 breadth/depth 分配均为可选 Policy。
- 已确认：`spec-exec-abstraction.md` §9 压缩为“论文覆盖”“成本证据与候选 Policy”“正式边界、能力依据与研究问题”三节。Edge/Branch 生命周期、待选集合、`select`、`rebalance`、依赖传播和重新挂载不再重复定义；论文覆盖、命中率与能力表格继续作为事实依据，survival、深层误接受风险和不同决策组合的成本/保证继续作为待验证假设。
- 已确认：删除单独的 `Assumption` 概念，保留 `AssumptionSet` 作为 Branch 的未决 Proposal 依赖集合；Proposal ID 全局唯一，`AssumptionSet = frozenset[ProposalId]`。集合元素只是对尚未确认 Proposal 的引用，不是另一种实体；Proposal 确认时移除其 ID，否定时向受影响 Branch 直接发送取消命令。
- 已确认：`design-model.md` 的组合与嵌套只保留宽泛叙述，细节统一下沉到 `policy-decisions.md` §4。原有未决依赖形式化、反射 gating、`P(实例存活)`、frontier budget 和浅层/深层成本分析不保留在上层设计模型，也不未经重新讨论直接迁入 Policy Decisions；其中“候选 Branch 像普通执行一样继续运行并自然遇到新 Edge”补入 `policy-decisions.md` §4.2。
- 暂不验证“不同场景是否共享同一机制”；该项保留到 Edge、Proposal、Branch 关系收敛之后。
- 下一项只讨论 **Edge、Proposal 与 Branch 的关系**；`mental-model.md` 最后统一修改。

### 架构四侧与对称框架

- 已确认：四侧是完整调用路径中的逻辑位置，不表示四侧都由 AScout 实现或拥有。Agent 接入侧与控制面是 AScout 可以定义和实现的系统部分；Runtime 是 AScout 依赖但不能定义其内部架构的外部能力来源。执行、隔离、资源和效果最终由后端原生机制承载，但 AScout 只通过 Adapter 的可观察行为判断这些能力是否兑现。
- 已确认：AScout 能定义的是控制面接受的 Runtime Adapter operation/event 边界、能力声明和接入条件；不能控制 Adapter 内部如何调用 Runtime、组织模块或选择 transport。Adapter 的强制内容来自控制面接入契约，内部实现只适合给出开发建议与接入指南。
- 已确认：`architecture.md` 的直接子文档不按四侧机械地一侧一份。直接子文档包括 Agent 接入侧与控制面设计；Adapter 的具体实现与接入方式进入开发/接入指南；Runtime 不建立通用设计子文档。
- 已执行：新建 `docs/design/agent-integration.md` 作为架构的直接子文档，新建 `docs/development/runtime-adapter.md` 作为开发/接入指南；`spec-exec-abstraction.md` 中相应的现行职责已迁移，论文证据与 backend 候选分析继续留在开发札记。
- 已确认：Interposer / ASC-Daemon 的语义由控制面定义（决定它们做什么）；它们部署在 Runtime 创建的 Branch 隔离环境内，运行生命周期由 Runtime 管理（决定它们在哪里跑、何时启停）。该决定不推翻"ASC-Daemon 能在 Branch 隔离环境内独立运行、不与控制面长期 metadata owner 混同部署"的既有约束。
- 已确认：保留 `Agent 接入侧 -> 控制面 -> Runtime Adapter -> Runtime` 四侧调用路径，但不再将其解释为对称的四个 AScout 子系统或四份实现拓扑。控制面居中，Agent 接入侧是 AScout 自有接入边界，另一端经 Runtime Adapter 使用外部 Runtime。
- 已收敛：Runtime Adapter 的具体接入契约不并入 `speculation-contract.md`，单独建立 Adapter 契约设计文档；`speculation-contract.md` 只保留 Contract 的总体定位、Policy 与 Adapter 之间的关系，以及到具体契约文档的引用。`runtime-adapter.md` 继续作为面向 Adapter 作者的开发与接入指南。
- 已收敛：控制面新增 `Adapter Registry`，负责 Runtime Adapter 的接入登记、接入校验和已通过校验的能力信息发布。它是控制面内的固定职责，不是 Runtime Adapter 本身；Policy Engine 使用其发布的能力信息进行 Policy 匹配，Speculation Coordinator 不承担 Adapter 接入登记职责。
- 已收敛：通用 Runtime 只负责隔离执行环境及其环境级执行、资源和边界状态；完整 Agent 执行实体、Branch context、operation 归属和 capability 路由由 Agent 接入侧管理，Speculative Tree 与 Branch 投机语义由控制面管理。Runtime Adapter 接入检查不能把这些上层职责转嫁给 Runtime。
- 已收敛：Runtime Adapter 的基础门槛包括建立隔离环境、在环境内建立 Agent 接入侧并执行接入侧无权完成的环境级操作、停止和清理环境，以及继续使用仍有效的环境。隔离是贯穿所有环境操作的固定语义，不是可关闭的 capability；校验按进程、workspace、I/O 与通信、资源、权限、停止残留、cleanup 和继续状态分别进行。
- 已收敛：Adapter Registry 以接入时主动验证为主要证明手段，运行期持续验证只负责发现主动验证遗漏和实际偏差。能力状态分为已验证、未验证和验证失败；未验证能力默认不可用，但部署方可以主动确认并限制在指定部署范围内使用；验证失败不能通过确认绕过。
- 已收敛：`runtime-adapter-contract.md` 定义正式的能力声明、Registry 解析与验证、固定环境操作、结果/事件语义和能力降级；`runtime-adapter.md` 只负责依据该契约指导 Adapter 的实现、声明编写、主动验证、注册、排查和实现选择。两篇文档不再拆出第三篇 operation/event 文档。
- 已收敛：Adapter 声明描述当前实例、当前 Runtime 配置下的实际隔离保证，不描述理论最大能力。隔离内容按 `process`、`workspace`、`memory`、`I/O`、`network`、`resource` 和 `environment-local control surface` 分别声明，并使用 `visibility`、`modification`、`interference`、`lifetime`、`cleanup`、`scope`、`limitations` 等维度；`input`、`outcome`、Branch context 和外部业务效果分别属于受控信息流或 Effect/Service Adapter。
- 已收敛：Adapter 配置注册后不得运行期变化。若实际能力与声明不符，全部任务使用降级后的已验证范围；依赖失效能力的投机活动停止并清理，不依赖的活动可以继续；重新验证通过后才恢复新的能力范围。
- 已收敛：Adapter 契约使用形式化的 `AdapterDeclaration -> CapabilityView` 模型。声明描述当前 Adapter 实例和当前 Runtime 配置下的实际隔离保证；`ContentIsolation` 按 `content`、`visibility`、`modification`、`interference`、`lifetime`、`cleanup`、`scope`、`limitations` 描述；Registry 解析、规范化、主动验证并附加验证状态，Policy Engine 再将 Policy 要求与能力视图匹配。最终 wire schema 和属性枚举留待实现阶段，但必须保持该对象关系与验证语义。
- 已确认：`runtime-adapter.md` 中的工作表用于把 Adapter 的声明落实为 Runtime 机制、可观察证据和验证方案，为 Registry 的主动验证准备依据；它不是新的对外契约。没有设计依据或实际证据的“常见错误”清单不进入接入指南；实现部分统一说明 Adapter 作者可自行选择的实现方式。
- 已收敛：接入验证由 Adapter Registry 自主完成。Registry 建立验证环境、执行验证场景并观察结果；Adapter 只提交声明并提供建立验证环境、执行环境操作和观察结果所需的接入信息，不提交自测结果作为接入证明。`runtime-adapter.md` 不包含 Adapter 内部工程实现方式或 Adapter 作者自建验证方案。

### 控制面 §2 组件与状态的引入与整体职责

- 已收敛：本节（`control-plane.md` §2）的目标是阐明 §1 所述控制面职责在控制面内部如何落实，读者应能从本节回答：每一类责任与固定语义由哪个组件承担，以及每个组件自身的职责（即使不属于控制面的主要责任也要讲清）；各组件拥有/不拥有哪些控制状态（状态归属）；组件之间通过什么入口、按什么顺序协作，哪些修改必须走固定状态入口。
- 已收敛：本节是组件职责与状态归属的权威描述，但只定义目标职责、状态与协作边界；实现细节与论文依据留在 `docs/development/spec-exec-abstraction.md` 对应主题。
- 已收敛：不保留抽象的 owner 总则（如“有状态组件只能修改自己拥有的状态，无状态组件不能代替状态 owner 作决定”）。该约定是软件设计的基本纪律，不承载设计信息；信息应落在每个组件“负责什么 / 维护哪些状态、不持有哪些状态 / 与谁如何协作”的具体描述上。删除对象为 `control-plane.md` §2 引入中的该类总则句。
- 已确认：项目内其余 “owner/拥有” 表述若为具体边界（控制面与 Runtime 的状态归属、Policy 不拥有队列或运行状态、Effect Coordinator 不持有真实效果状态等），是有效信息，不属于要删除的抽象总则。
- 已收敛：Policy 由控制面统一加载、解析、校验和版本管理。所有不依赖或不改变全局状态的 Decision，可以通过 Policy Engine 生成的执行表示注入 Agent 接入侧执行，以降低高频路径上的通信延迟；接入侧必须记录本地 decision log，并把结果与日志交给控制面。控制面不重复执行已经满足本地范围条件的 Decision，但仍确认执行表示、Branch 上下文和结果格式有效。依赖或改变全局状态的 Decision，以及所有 Speculative Tree、Branch 生命周期、资源、效果和 Runtime 状态转换，仍由控制面负责。
- 待展开：Agent Control Gateway 的接收与传输方式属开发未定事项；可考虑进程内调用、本地 IPC 或网络，但同进程部署不是强隔离 Runtime 下的架构要求。既有原则是 typed operation/event 语义不随 transport 改变。
- 已收敛：以 Agent Control Gateway 替代 Entry API，作为 Agent 接入侧进入控制面的统一受管通信边界。Runtime 创建 Branch 执行环境后，Agent Control Gateway 在该环境内创建并启动 Agent 接入侧，二者共同形成完整的 Branch Node。Gateway 负责 Task 启动、Agent 接入侧会话，以及 Policy dialect、本地 Decision 结果、decision log、运行事件和受管控制请求的双向传递，并进行入口级上下文、权限和格式检查；除通信会话所需的暂时状态外，不持有控制面或 Runtime 的权威状态，不直接操控 Runtime，也不独立决定 Policy Decision 或 Branch 生命周期转换。
- 已收敛：Agent Control Gateway 不绑定网络接口。进程内调用、本地 IPC 或网络属于部署与开发选择；同进程部署不是强隔离 Runtime 下的架构要求。无论采用何种传输，Gateway 接受的 typed request/event 及其控制语义保持一致。
- 已收敛：以 Speculation Coordinator 替代 Assumption Manager，并吸收原 Scheduler 的控制面职责。Speculation Coordinator 作为全局投机视图与控制循环的协调核心，根据 Agent 接入侧和 Runtime 经 Runtime Adapter 规范化后的事件，以及当前依赖和 Branch 状态，激活受影响 Edge 的 Policy Decision；各专业组件形成相应的操作意图，所有 semantic operation 经统一的控制面操作出口交给 Runtime Adapter 执行，并由返回结果或事件更新全局视图。它不直接触发固定 Runtime 操作，也不重复执行已经满足本地范围条件的 Decision。
- 已确认：投机 Branch 的逻辑创建与独立 Runtime 环境的创建可以分离。采用 lazy 创建时，控制面先在 Speculative Tree 中登记和推进逻辑 Branch；只有在即将发生需要独立 Branch-local 状态的操作前，才请求 Runtime 建立独立环境。实际环境完成相应操作后，控制面再把逻辑视图标记为已与实际状态对应，并区分逻辑视图已推进但实际环境尚未对齐的阶段。
- 已确认：Branch-local 状态不因属于内存、进程上下文或其他完全可逆状态就一律要求强隔离。默认只保证 Policy 或 Contract 要求保护的状态域；未被要求保护且不会影响保证范围的可逆变化可以共享或忽略。若后续操作需要该状态按 Branch 独立延续，仍必须在操作前建立独立环境，不能把已经共享的状态事后假定为可分支状态。
- 已确认：Agent 接入侧负责在高频路径上识别即将发生的 Branch-local 变化，并在变化发生前请求 lazy 创建；未知或无法确认的操作按需要建立独立环境处理。Effect Coordinator 不负责普通 Branch-local 操作，而专门协调共享、外部及 Policy 要求纳管的效果，尤其通过可扩展的服务效果插件支持 `simulate`。插件只在控制面启动或 Policy 加载阶段注册；Task 使用的 Policy 生效后不热替换插件目录，具体注册接口和插件实现留在开发文档。
- 已确认：效果插件实际运行在控制面内，由 Effect Coordinator 统一注册、选择和调用。Agent 接入侧只在效果真正发生前识别命令或 URL，并将受管请求转交控制面；插件可以基于全局视图执行去重、统一审计及跨 Branch 的效果协调。`simulate` 等可在控制面完成的结果由插件生成后经 Agent Control Gateway 返回；只有需要 Runtime 提供暂存、代理、转换或其他执行机制时，才经 Runtime Adapter 请求 Runtime。
- 已确认：Ledger 在当前目标语义中只负责异步旁路记录控制面决策、运行事实及逻辑视图与实际执行状态的对应变化，用于状态对齐、问题定位、审计校验、shadow/replay 和分析；它不参与实时决策，不从记录中重新决定生命周期，也不承担控制面重启后的状态恢复。WAL、检查点、事件重放顺序和从 Ledger 重建 Speculative Tree 等恢复协议留在开发文档。

## 暂停讨论

以下事项已经记录，但要等当前分支执行模型或源码基础进一步稳定后再展开：

### 控制面执行模型与 Policy

- 已确认：Policy 是由 Policy Decision 及相关决策逻辑组成的静态定义，不是用户编写并直接参与运行控制的程序。控制面通过 Policy Engine 使用 Policy 作出运行时高层决策，再将决策编排为 semantic operation；机制没有固化 Policy 决策，一切策略选择仍依照 Policy。
- 已确认：控制面维护每个 Task 的 Speculative Tree 控制面视图。AScoutAgent 是这棵 Tree 向用户呈现的统一抽象，Branch 是 Tree 中的子树，三者不是并列且彼此独立的状态对象。
- 已确认：semantic operation 经 Runtime Adapter 请求 Runtime 执行；控制面收到结果或事件后校验操作是否按语义完成并更新 Tree 的控制面视图，同时记录运行事实用于状态对齐、问题定位和审计校验。
- 控制面内部包括 Agent Control Gateway、Policy Engine、Speculation Coordinator、Effect Coordinator 与 Ledger，不设置额外的总控 Core。
- Policy 只能在 AScout 固定生命周期、single-winner、capability、obligation、隔离和 cleanup 语义内作选择，不能拥有或修改 Runtime mutable state。

### Agent-facing control surface

- 已确认：当前不向 Agent 暴露主动使用 AScout 控制能力的控制面接口；Agent-facing control surface 不作为当前 Agent 接入路径。
- 不提供独立 `publish-outcome`；它仍是由 Interposer 自动触发的 Runtime lifecycle semantic operation。

### Interposer 与 Operation Integration 合并

- 已收敛：`Interposer` 与 `Operation Integration` 合并为一个统一责任实体，不再将二者描述为两个独立的 Agent 接入组件。
- 已收敛：Interposer 统一负责 Agent 请求拦截、规范化 operation 识别、`pending-result boundary`、本地 Decision 执行、真实 operation 的 once-only 纪律以及 outcome 自动发布。它可以通过拦截器、Hook、wrapper、effect broker 或其他产品适配机制实现。
- 已确认但未展开：端侧管理节点是否需要另一个统一的本地控制循环，以及 Interposer、ASC-Daemon 和产品 Integration 在实现层的具体组织方式，继续作为后续议题讨论。

### Interposer 的事件驱动处理

- 已收敛：Interposer 按拦截事件与控制面命令逐次处理 Agent 执行，并在需要时通过 ASC-Daemon 与控制面进行异步协作。
- 已收敛：不引入“本地控制循环”作为 Interposer 或端侧管理节点的设计概念。单次处理等待控制面结果属于该次 Agent 执行的处理过程，不表示存在持续运行的本地调度循环。
- 不提供日常 `delegate-resolver`；需要控制面代为裁决时，delegation 作为 managed launch 或 Branch 创建配置表达。

### ASC-Daemon 职责

- 已收敛：ASC-Daemon 是端侧管理节点与控制面之间的本地通信入口。它建立并维护双方的通信会话，同时证明当前通信来自对应的端侧管理节点，避免信息伪造。
- 已收敛：ASC-Daemon 负责传递端侧提交的事实、请求、Decision 结果和 outcome，以及控制面返回的命令、结果和事件。
- 已收敛：ASC-Daemon 不负责解析或路由 Branch 上下文，不执行 Agent 步骤或本地 Decision；这些职责分别由 Interposer 和端侧管理节点承担。
- 已收敛：ASC-Daemon 启动后与控制面建立通信会话，在会话建立过程中确认通信双方的身份，并在后续通信中通过通信凭证证明消息来自对应的端侧管理节点。一个 ASC-Daemon 对应一个端侧管理节点及其 Interposer；具体凭证、会话建立方式、消息格式和传输协议留给实现决定。


### 端侧管理节点与 Branch、Instance 的关系

- 已收敛：端侧管理节点围绕一组 Agent 进程及其执行活动提供本地管理。Agent 主进程及其 sub-agent 进程共同构成一个 Agent 执行实体，由同一个端侧管理范围统一观察和处理。
- 已收敛：Branch 为执行活动提供逻辑归属，端侧管理节点为每个 Branch 承载独立的本地管理上下文，并依据 Branch identity、operation 归属和 capability 路由事件与控制命令。
- 已收敛：多个 Branch 在尚未需要独立 Branch-local 状态时，可以由同一个端侧管理节点在共享执行环境中承载，各 Branch 仍保持独立的管理上下文。Branch 需要独立环境时，其管理上下文随执行活动进入相应环境；具体是复用、迁移还是重新建立实际承载方式，留给实现决定。
- 已收敛：sub-agent 的推理、工具和命令活动，只要处于 integration profile 可观察范围内，就由同一个 Interposer 按所属 Branch 上下文处理，并归属于该 Branch。sub-agent 与主 Agent 的普通协作属于同一 Agent 执行实体；只有显式创建新的 AScout Branch 时，才形成新的 Branch 上下文。
- 已收敛：一个执行环境建立一个端侧管理节点，环境内的每个 Agent 在启动时通过 Integration 接入该节点，并持有独立的 Agent 执行上下文；同一节点可以承载环境内多个 Agent 及其 Branch 上下文。

### Agent product 接入能力

- 已收敛：Agent product 需要提供关键执行活动的观察点，以及在真实 operation 开始前允许 Interposer 暂停或阻止执行的拦截点。
- 已收敛：接入 profile 需要支持 operation 开始、`pending-result boundary`、完成和 outcome 的识别；需要管理的 Agent 主进程及其 sub-agent 执行活动也应处于可观察范围内。
- 已收敛：暂停或阻止真实执行后，Interposer 可以等待控制面结果或执行本地 Decision，再让真实 operation 继续执行，或让当前调用沿 Agent 原有的完成路径使用已经获得语义授权的结果完成。Agent product 提供的是观察和拦截接缝；Branch 上下文、Decision、控制面通信、日志和结果授权属于端侧管理节点与控制面职责。

### Interposer 职责补充

- 已收敛：Interposer 负责处理 Agent 的拦截步骤，从拦截点提取步骤事实，识别 operation、`pending-result boundary`、Branch-local 状态和 Effect，并根据本地 Decision 或控制面处置结果控制步骤继续、暂停、阻止或跳过。
- 已收敛：Interposer 可以暂停一次 Agent 步骤，等待本地 Decision 或控制面结果返回后继续处理。继续时沿产品原有路径执行；跳过时不执行真实 operation，并在需要时使用已经获得语义授权的结果沿原有完成路径结束。
- 已收敛：Interposer 在 Branch-local 状态或受纳管 Effect 发生前提交相应请求，保证真实 operation once-only，从原调用返回路径取得 outcome，将其归属对应 Branch 并通过 ASC-Daemon 提交控制面，同时记录本地 Decision 的输入事实、执行内容、Policy 版本、结果和执行上下文。

### Integration 的步骤接入约定

- 已收敛：Integration 是面向具体 Agent product 的产品适配组件，依据接入约定以最小改造将产品执行接缝连接到端侧管理节点。
- 已收敛：Agent 即将执行可管理步骤时，Integration 将步骤交给端侧管理节点并等待返回处置结果。返回继续时按产品原有路径执行；返回跳过时不执行原步骤，并在需要时让调用沿产品原有完成路径使用端侧管理节点提供的结果结束。暂停通过等待处置结果体现。
- 已收敛：Integration 从被拦截步骤的原调用返回路径自动取得成功、失败或取消结果，并将其交给端侧管理节点形成 outcome；产品不需要提供额外的完成回调。
- 已收敛：不同 integration profile 可以使用不同的产品接缝和步骤表示，但遵循相同的步骤接入约定。Integration 不解释步骤的投机语义，不判断 Policy Decision、Branch 或生命周期，也不决定跳过步骤时使用的结果。

### Agent 接入侧职责与当前范围

- 已收敛：Agent 接入侧不是单纯的请求适配层，而是对应 Agent 的**端侧管理节点**。它负责管理 Agent 的本地执行、观察 Agent 行为，并承担授权范围内本地 Decision 的独立判断和执行责任。
- 已收敛：Agent 接入侧可以执行由 Policy Engine 注入的、不依赖或不改变全局状态的本地 Decision，并向控制面报告 Decision 内容、结果、输入事实、Policy 版本和执行上下文。涉及全局控制状态或生命周期权威的决策必须交还控制面。
- 已收敛：当前不承诺“完全无修改、无需了解 Agent 内部实现”的通用接入，只支持具备足够拦截点或 Hook 的特定 Agent product。接入适配可以依赖产品内部执行结构，以观察和控制所需的推理、工具及 operation 生命周期；后续再评估更通用的接入方式。
- 已确认但未展开：是否允许 Agent 发起显式协作请求，以及 Agent-facing 请求的上下文绑定与转发边界，继续作为后续议题讨论。

### Agent 接入侧文档结构

- 已收敛：删除 `agent-integration.md` 的“固定约束与未定实现”整节。固定约束已在前文和相关设计文档中说明，具体实现问题由开发或使用文档承载，不在本文重复总结。

### 通用控制面框架

- 长期目标是让上层 Agent 使用不同 speculative 算法或分支执行方案，而不要求修改其算法或业务逻辑；但接入实现可以依赖具体 Agent product 的内部结构和拦截点。
- 该目标仍不扩展为任意分布式计算平台，后续必须继续遵守控制面、Runtime Adapter 与 Runtime 的 ownership 分离。

### Adapter 可观察的原子转换语义

- 需要明确控制面要求 Adapter 对 Execution、Workspace、Input、Effect、Outcome 转换呈现怎样的原子性、可见性、失败收敛和 cleanup result。
- Adapter 与目标 Runtime 内部如何协调这些转换不由 AScout 设计；实现建议留在开发/接入文档。

### Control session 与 Workspace identity

- 需要定义 typed control session 的 request identity、幂等、event ordering、取消/超时与断线语义。
- 需要定义 Workspace revision/object identity 及并发 ancestor/descendant write 的线性化规则。

### Runtime 与状态细节

- Continuation branching backend：比较逻辑状态重建、完整进程树 checkpoint/restore、microVM 和定制内核机制。
- Effect adapter 的实际建立：定义 deferred、virtualized、deduplicated、compensatable、irreversible effect 的记录、验证、建立与清理。
- 身份与控制协议：定义 logical Branch identity、PID 生命周期、generation/lease、pidfd、错误、取消、超时和 orphan cleanup。
- 输入边界：定义流/消息边界、partial read、EOF、阻塞行为和真实 FD 所有权。
- 持久化与故障恢复：定义 control/runtime 状态持久化、WAL、ASC-Daemon 恢复、连接排空和隔离环境内后台进程归属。

## 已下沉的讨论

下列主题已经在权威文档或开发决策中记录，不再在本文件保留逐条历史过程：

- **外部效果与资源影响**：额度、限流、并发槽等资源使用默认不纳入 Branch 世界状态，只归属和计量；Policy 可以将其中指定的资源或额度声明为准入约束、预算约束或取消条件。只有它改变保证范围内的可观察业务行为时，才进入 Effect 管理。正式语义见 `docs/design/effect-management.md`，决策记录见 `docs/design/decisions.md` D-029。
- **Effect Management 的范围**：已确认 Effect Management 管理投机分支对分支外部、共享或最终执行状态产生的影响，以及这些影响在投机期间的暂存、限制、提交、取消和清理。Branch-local state 也属于管理范围，但主要由 Runtime 自动完成隔离、清理和 winner 延续；描述中心放在共享、可观察的业务变化。Outcome 和 operational footprint 的管理方式与共享业务变化相似，但是否纳入额外的 Effect 管理由 Policy 决定。单个动作或工具调用只是投机执行过程中的一个环节，不称为一次投机执行。正式语义见 `docs/design/effect-management.md`。
- **Effect Management 文档组织**：已确认按“目的与描述中心 -> 管理范围与保证边界 -> 效果管理过程 -> 效果处理纪律与状态域 -> 责任边界 -> 固定保证与明确边界”的逻辑组织。管理范围与保证边界合并为第 1 节，并保留 `1.1` 到 `1.5`；不设置独立的 Branch 生命周期章节，Branch 的 lifecycle 由 Branch 语义文档负责。`EffectClass` 与 `StateDomain` 在同一节中以完整连续内容说明，不拆成多个概念小节。正式文本见 `docs/design/effect-management.md`。
- **伪装执行命名**：已确认“伪装执行”为正式中文名，对应 `effect` 决策中的 `simulate` 模式标识符，二者并存并建立术语映射。伪装执行是对已接入专门 Adapter 的服务拦截请求、不转发给真实服务、在 Branch 内生成明确标记并记录来源的模拟响应，以避免不可逆外部影响的处理方式；模拟响应作为未验证假设参与 AssumptionSet 确认/否定，仍需 Authority、validator 或其他明确规则验证后才能被 winner 依赖。正式语义见 `docs/design/effect-management.md`，决策模式见 `docs/design/policy-decisions.md` §2.5。
- **Agent 接入表述**：Agent 可以在不修改自身执行逻辑的情况下接入 AScout，但具体产品可能需要 Product Integration、Hook 或其他拦截适配，接入实现应尽可能保持修改面小。该表述不承诺完全不修改产品，也不要求 Agent 主动参与 Branch 生命周期。
- **Mechanism 与四侧架构的关系**：Mechanism 不是第五侧，而是横跨 Agent 接入侧与控制面的抽象实现责任。Agent 接入侧负责拦截和本地落实，控制面负责全局编排、校验和状态协调。
- **Effect Coordinator 的责任边界**：不引入独立的 `Effect backend`。Effect Coordinator 是外部效果的统一实际执行者，负责受管效果的实际处理及其执行状态；服务级 `simulate` 通过可注册的效果插件扩展。Runtime 负责隔离执行环境及环境级执行状态，不承担外部效果的实际执行责任。
- **抽象设计札记边界**：`docs/development/spec-exec-abstraction.md` 只保留论文事实和论文调研观察，不定义 AScout 设计；所有当前设计以正式设计文档为准。
- **过期 Agent 生命周期决策**：原 `docs/design/decisions.md` D-001 已过期并删除。当前默认 Agent 不感知 Branch，显式 Agent 感知只保留为未来设计位置。

- 项目定位、名称、四侧架构和控制面/Runtime ownership：见 `docs/mental-model.md`、`docs/architecture.md`、`docs/design-model.md`。
- Policy Decisions、Proposal、Authority、Equivalence 和 Disposition：见 `docs/design/policy-decisions.md`。
- 状态、效果、obligation 和独立执行环境的建立：见 `docs/design/effect-management.md`。
- Speculation Contract：见 `docs/design/speculation-contract.md`。
- Branch、Accept、Discard、Commit、single-winner、输入传播和生命周期：见 `docs/design/branch-semantics.md`、`docs/design/decisions.md` 与 `docs/design/speculation-decisions.md`。
- Branch barrier、Resolution barrier、外部效果 observation surface 和 Agent execution activity：见 `docs/design/branch-semantics.md` 及对应设计决策。
- 首要验证 workload、默认 Runtime 和新 Rust 实现边界：见 `docs/development/reference-workload.md` 与 `docs/development/decisions.md`。

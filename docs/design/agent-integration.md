# Instrumentation 设计

本文展开 [AScout 架构](../architecture.md) 中的 Instrumentation，定义 Agent 或 Agent product 如何进入 AScout 的受控路径，以及接入组件之间的责任边界。

## 1. 总体设计

Instrumentation 是围绕 Agent 执行环境提供本地管理的责任边界，不另设“端侧管理节点”这一并列组件。它负责管理由 Agent 主进程及其 sub-agent 组成的执行实体，以及这些进程产生的执行活动。Agent 与随附 Instrumentation 在 Branch 隔离环境外的可信宿主运行，由 Runtime 管理生命周期；每个 Agent 在启动时接入该侧，并持有独立的 Agent 执行上下文；每个上下文再通过 Branch 上下文区分不同的逻辑执行延续，并依据 Agent、Branch、operation 归属和 capability 处理事件与控制命令。

Instrumentation 与 control plane 共同完成投机执行：它观察和拦截 Agent 行为，在本侧处理属于本地范围的 Policy Decision，并将依赖全局控制状态或生命周期的决策交由 control plane。Branch 在尚未建立独立 Environment 时只存在于 control plane 视图；一旦建立环境，Branch 的工作区访问、shell、受管子程序和效果在该独占环境执行；Agent 与 Instrumentation 管理上下文留在环境外的可信宿主，由 Runtime 管理生命周期。

### 1.1 职责

Instrumentation 在 Agent 执行过程中落实 Policy Decision 对投机执行流程的决策。对于由 Policy Engine 编译并交付的 Decision，Instrumentation 结合当前 Agent 执行上下文和可观察事实，对属于本地范围的 Decision 独立作出判断，并向 control plane 同步 Decision 内容、输入事实、结果和 log 信息。

对于依赖全局控制状态、资源或 Branch 生命周期的 Decision，Instrumentation 将相关事实和请求提交 control plane，由 control plane 完成全局判断、生命周期编排和后续处理。Instrumentation 接收 control plane 返回的结果和命令，并据此继续处理对应的 Agent 执行；相关事实、Decision、结果和事件通过 Instrumentation 与 control plane 之间的双向通道传递。

Instrumentation 负责观察和拦截 Agent 的执行行为，识别 operation 的开始、`pending-result boundary`、完成和 outcome，并处理 Agent 主进程及其 sub-agent 产生的可观察执行活动。它根据本地 Decision 或 control plane 返回结果，允许真实 operation 继续、在执行前挂起或取消，并管理候选 session 的独立执行与接替。

Instrumentation 负责在 Branch-local 状态或受纳管 Effect 发生前识别相应边界，并保持 Branch 管理上下文、事件和控制命令的正确归属。Branch 在尚未建立独立 Environment 时只存在于 control plane 视图，各 Branch 仍保持独立的管理上下文。当某个 Branch 需要独立执行环境时，Instrumentation 向 control plane 请求建立该 Branch 独占的环境，该 Branch 的文件访问和受管进程使用该环境，Agent 与 Instrumentation 不随环境迁入或因挂载切换重启。识别到共享、外部或 Policy 要求纳管的 Effect 时，Instrumentation 在 Effect 发生前将相关请求交给 control plane 处理。真实 operation 保持 once-only，完成后的 outcome 自动归属对应 Branch 并发布到控制路径。

## 2. 接入组件

### 2.1 Product Integration

Product Integration 是面向具体 Agent product 的产品适配组件，负责依据接入约定，将 Agent 提供的执行点连接到 Instrumentation。Agent 在启动时通过 Product Integration 接入随附的、位于 Branch 隔离环境外的 Instrumentation；Product Integration 以尽可能小的产品改造接入 Agent 的内部执行结构，使 Instrumentation 能够观察目标执行活动，并在真实 operation 开始前暂停或阻止其执行。

当 Agent 即将执行可管理步骤时，Product Integration 将该步骤交给 Interposer，并落实继续、执行前挂起或取消。放行时，产品按原有路径执行步骤一次；挂起期间原调用尚未执行。候选在独立 session 中使用预测输入或替代模型推进。Full 由候选 session 继续执行，并取消已过时的父调用。Part 保留父 session，将实际接受的结果作为后续模型输入，原调用仍按自身等待点处理。全部候选不可接受时，放行原调用一次。

Product Integration 从被拦截步骤的原调用返回路径自动取得成功、失败或取消结果，并将其交给 Instrumentation 形成 outcome。它不修改 Agent 的业务逻辑，也不要求 Agent 主动调用 AScout API；具体产品适配可以了解其内部执行结构，但应将改造限制在观察和控制所需的接入范围内。只有产品确实提供真实的执行前接缝时才能声明支持该类 action；SDK 的输出观察事件（例如 `message_start`）本身不是推理前 Hook，不能冒称为此能力。

`admit` 的 `opportunity_pass`、`feasibility_pass`、`extent` 与 `effect` 属于 Instrumentation 求值的本地 Decision，由与具体产品无关的 Interposer 统一完成；会改写全局控制状态或必须在多个 Chance、Branch、Proposal 之间协调的其余 Decision（包括 `predict`）由 control plane 求值并落实，所缺的本地事实由 Instrumentation 计算后提交；Product Integration 连接产品执行点，落实继续、阻止、挂起、取消及 session 接替，不自行求值 `extent` 或 `effect`。产品专有的消息、checkpoint、session 和 SDK 细节留在 Product Integration，不进入通用求值逻辑。

Product Integration 负责完整持久化运行分支的 session 记录，并在 Instrumentation 汇总为单一最终权威 session，再报告可定位的记录引用；通用层只传递文件位置、Branch 与记录引用等契约数据，不解释产品格式。完整记录包括 compaction 和 custom 记录，不限于消息正文。文件写入或读取位置提示不等于 Agent session 释放、Branch 执行释放或 Runtime 清理。

模型输出回放在系统外读取这些记录，通过正常模型服务和用户输入接口驱动普通 AScout。Instrumentation 不承担回放队列、自动历史输入或测试服务编排；普通模型配置、认证、扩展、持久化及恢复仍归正常运行。自动输入依据通用权威轮次完成／可输入状态推进，不要求所有投机分支结束；输入排队成功不是轮次完成。

### 2.2 ASC-Proxy

产品工具参数、schema 与结果格式化属于 Product Integration，由它独立适配，一次完整工具调用对应一次完整 Runtime 操作。Runtime 提供 `read_file`（互斥 byte_range／line_range）、`write_file`（完整内容、create_parents）、`replace_text`（有序替换，默认 exactly_one，all 显式启用）、`list_directory` 和 `exec`（cwd、env、timeout、流输出及退出状态）。replace_text 在目标环境内存中依次检查全部匹配，全部成功后写入；失败不修改原文件。Runtime 和 backend 不加载产品 SDK，不为每个调用建立环境。同一 Environment 内，受管理的 write_file／replace_text 按目标环境解析的 canonical path 串行，路径别名共享队列；独立 Environment 队列独立，任意 exec 写文件不受此队列保证。执行连接以 `environment_id` 和 `operation_id` 关联操作，以各 Environment 的请求序列去重，并以 generation 拒绝取消前的延迟请求。backend 声明的 NLE 等工具共用接收、输出、取消与终态生命周期，业务语义由 backend 保留。单操作取消的隔离范围取决于 backend，必须返回相应终态。取消整个 Environment 时关闭该环境的准入，推进 generation，等待活跃操作终态并确认写者已停止。

Pi 将绝对和相对文件路径原样交给已绑定的 Environment，由目标环境解释 `..` 与符号链接。相对路径和未指定 cwd 的 `exec` 使用该 Environment 的工作目录。通用 `exec` 调用可显式覆盖 cwd。Runtime 成功交付的命令结果携带退出码。Pi 的 `bash` 保留输出与退出码，并在非零退出时报告工具失败。传输错误、取消和超时仍沿各自的 Runtime 路径处理。

ASC-Proxy 是 Instrumentation 进程内的受管 Unix socket 客户端，不是独立代理进程，也不要求用户手工启动或直接管理。它经 Gateway 与 control plane 建立控制会话，同时管理不依赖具体 backend 的独立 Runtime 执行连接。控制会话关联 Agent、Branch 与 action。执行连接使用 Instrumentation 保存的 Environment 映射和 operation 身份。通信认证实现及能力差距见 [接入实现](../development/agent-access.md)。

通信会话建立后，Instrumentation 通过 ASC-Proxy 向 control plane 提交 Agent 执行事实、控制请求、本地 Decision 结果和 outcome；control plane 通过同一会话向 Instrumentation 返回命令、结果和事件。ASC-Proxy 负责消息的发送、接收和传递，不参与 Agent 步骤的执行、Decision 的判断或 Branch 生命周期的处理。Runtime 负责实际启动、监视和回收 Agent 进程，Gateway 管理通信连接，用户不直接管理 ASC-Proxy。

ASC-Proxy 同时拥有控制通信连接与独立的 Runtime 执行连接。创建和恢复启动返回目标 Environment 的执行地址，由 control plane 转交 Instrumentation。执行端点根据 Environment 状态接收操作，并通过同一接口返回增量输出、终态和取消确认。取消必须等待 backend 终态，不能用本地等待结束代替；结果确认后释放正文，保留有界身份窗口以拒绝重放。

Branch 取消与执行准入必须具有共同边界：取消覆盖边界前已接受的工作，清理完成前不得接受新工作；已取消的延迟请求不能在清理后复活。取消等待相关操作终态与遗留写者清理，清理失败保持执行关闭但允许重试清理。成功取消不永久禁止保留的 Authority 开始下一轮 task。恢复使用当前 Runtime 提供的执行访问方式，保存时的通信端点不能作为恢复后的访问依据。

### 2.3 Interposer

#### 执行事实与来源

Agent 接入协议围绕 action，而非工具或 Branch 角色组织。action 可以是模型推理、工具执行、启动 sub-agent、发送消息、等待或结束；每个 action 关联 `task_id`、`branch_id`、`action_id`、`action_kind`，可用 `parent_action_id` 说明因果归属。Authority/Candidate 由 Tree 查询，不由事件名称声明。

`ActionStarted` 报告实际开始；执行前的 `SpeculativeAdmissionRequest` 只登记等待点及 action 身份，不表示真实 action 已经执行。普通 action 事实与投机准入请求分开；普通执行无需等待 control plane 投机决策。`ActionFinished` 报告成功、失败、取消或超时，并携带 result 和可选 evidence；取消请求不等于已取消，control plane 等待期限届满也不等于 Agent 已停止。相同 action 的结果必须与已登记身份配对。

`ActionStarted`、`ActionFinished` 与 `SpeculativeAdmissionRequest` 各自携带 Instrumentation 测量的 Unix 毫秒 `timestamp_ms`，该时刻由 Instrumentation 在事件发生时测量，不使用 control plane 收到消息的时刻。工具的 `ActionStarted` 记录工具真正开始的时刻；`SpeculativeAdmissionRequest` 只表示投机起点，不能作为 action 的开始时间，control plane 也不据此合成开始时间。投机开始以 `SpeculativeAdmissionRequest` 的 `timestamp_ms` 为准；权威路径真实结果到达的时刻以对应 `ActionFinished` 的 `timestamp_ms` 为准。`Usage` 应关联 `action_id`，并报告 Instrumentation 测得的模型请求开始与结束时间；Instrumentation 确实无法把一次模型调用与已上报的 action 配对时（例如该调用没有经过 Instrumentation Hook），仍然上报这次用量，`action_id` 为 null，不伪造 action 身份，也不静默丢弃。control plane 由此得到源侧耗时，而不是用自身记录时刻冒充执行耗时。

当前通用协议提供显式的 `SpeculativeAdmissionRequest` 路径，只有 Instrumentation 本地 Decision 产生该请求时，control plane 才会进入投机准入和预测。Pi 的工具执行前 Hook 已接入下发的 Policy artifact，并按其 `admit` 结果区分普通 action 与投机准入请求；模型推理前已有真实 Hook，位于 Pi 分支的 `sdk.ts`，在请求发出前执行，可延迟请求或抛错阻止。sub-agent 启动前目前仍无统一 Hook，Pi 接入声明 `subagentPreExecutionInterception: false`，暂不支持该边界。Pi 的缓存预热等后台模型调用不经过该 Hook，不得宣称所有模型请求都被拦截。

Agent 连接、就绪、断开、失败、释放，以及 Branch 创建、开始、就绪、暂停、恢复、失败、释放，属于各自主体的元事件。Agent session 释放、Branch 执行释放和 Runtime 环境清理分别表达。Agent 不能报告 Tree 已 Promote/Discard，也不能代替 Runtime 确认环境清理。

`CommandResult` 是 Agent Adapter 对控制命令的接收、开始及终态回执；Applied 只确认 Instrumentation 已落实，不代表 Runtime 或整棵 Tree 的转换全部完成。control plane 分别处理 Runtime 的操作结果、环境事件和 Agent 回执，再推进 Tree。输出、执行记录和使用量按 Branch 归属；未接受的 Candidate 输出只可作为诊断内容，不进入权威用户输出。

Interposer 是 Instrumentation 中介入 Agent 执行接缝并落实本地 AScout 语义的责任实体。Agent 即将执行可管理步骤时，Product Integration 将步骤交给 Interposer；Interposer 从拦截点提取步骤事实，识别 operation 的开始、`pending-result boundary` 以及可能产生的 Branch-local 状态和 Effect，并暂停当前步骤，等待本地 Decision 或 control plane 返回处置结果。普通步骤根据处置结果沿原有路径继续执行，不需要进入投机处理。

属于本地范围的 Decision 由 Interposer 结合当前执行事实直接判断和执行；依赖全局控制状态、资源或 Branch lifecycle 的 Decision，则由 Interposer 通过 ASC-Proxy 将相关事实和请求提交 control plane，并等待 control plane 返回结果或命令。

工具与模型推理都支持 eager 和 speculative-first 两种顺序。eager 在候选运行的同时放行原权威调用；speculative-first 将原权威调用保持在执行前，由独立候选 session 使用预测输入或替代模型先行推进。Policy 根据真实结果或明确的可接受性依据裁决。Full 接受候选 session 作为该 Chance 的后续执行，Part 则保留父 session。全部候选不可接受、预测失败或没有候选时，放行原调用一次。取消与重复命令不能导致重复执行。

Interposer 还负责在 Branch-local 状态或受纳管 Effect 发生前识别相应边界。需要独立 Branch-local 状态时，它通过 ASC-Proxy 向 control plane 请求建立相应的独立执行环境；识别到共享、外部或 Policy 要求纳管的 Effect 时，它在 Effect 发生前将相关请求提交 control plane。真实 operation 保持 once-only；继续执行时，Interposer 从原调用返回路径取得成功、失败或取消结果，将 outcome 归属对应 Branch 并通过 ASC-Proxy 提交 control plane。Interposer 同时记录本地 Decision 的输入事实、求值内容、Policy 版本、结果和执行上下文。

## 3. Agent 执行接入路径

用户交互入口与后端 Agent 执行实体分离：用户始终使用同一 AScoutAgent 视图，接入实现依据 control plane 已确认的权威执行处理输入、输出和中断路由。Promote 不要求用户重新连接、加载另一个 session 或等待候选会话结束，也不为路由变化取消或重启候选正在进行的执行。已展示内容不可撤回，未获接受的候选内容不得作为正式回复提前发布；切换时必须保持输入归属及输出顺序，旧主路径失去发布权后仍按生命周期规则停止和清理。运行能力不满足时应拒绝该接替路径，不能以路由切换伪造接受或效果处理完成。

用户可通过独立的只读观察查看候选状态及内容，候选内容与主对话分开展示，并随 Branch 的角色与状态显示其投机身份。Instrumentation 按 Branch 报告实际模型、工具和执行活动；control plane 提供预测、比较、处置与生命周期事实。界面应区分请求已发送、实际开始、执行结束和清理确认，不能把未知状态显示为已完成。只读观察不使候选获得用户对话或裁决权限，也不把其内容注入其他 Branch 的上下文。

AScout 管理显示区域、Tree、窗口选择和控制事实，并将用户输入和中断交给相应接入实现。Instrumentation 通过产品公开 SDK 报告消息、思考、工具调用、结果和会话事件；AScout 的 UI 负责排版这些事实，不嵌入或修改产品的终端画面，也不调用产品内部展示组件。浏览器 UI 与 TUI 通过共同的事实和操作接口工作，彼此不依赖绘制实现。窗口切换不等于会话切换或重启；候选观察保持只读。对应产品的交互能力按实际接入范围整合，未支持的能力明确说明，不承诺不同产品的全部交互完全一致；交互接口同样只整合 Instrumentation 实际声明的命令与提示，未声明即不展示。

观察先展示当前 AScoutAgent 整棵 Speculative Tree 的 Branch 关系、当前权威路径与投机状态，再允许选择具体 Branch 深入只读查看。选择 Branch 不改变权威执行、输入路由或接受条件，也不向该候选建立用户对话。用户界面可拦截自己的观察操作，不把它作为普通提示词交给 Agent；本地 TUI 的具体操作见开发文档与术语表。

Agent 按照自身原有流程执行步骤。当 Agent 即将执行处于接入范围内的步骤时，Product Integration 将该步骤交给 Interposer。Interposer 根据本地 Decision 或 control plane 返回的处置结果处理该步骤；需要 control plane 参与时，通过 ASC-Proxy 传递相关事实和请求，并接收结果或命令。Product Integration 落实原调用放行、执行前等待或取消，以及候选 session 的创建与 Promote 接替。预测 toolResult 只注入候选 session。Pi 保留预测中的原生文本和图像块及其顺序，以及显式成功／失败状态和适用的原生消息详情。普通字符串作为文本，其他没有原生内容形式的 JSON 值以文本呈现。工具调用身份由实际 checkpoint 决定。替代模型只在对应候选 session 执行，不替旧父调用完成。

真实步骤继续执行时，Product Integration 从原调用返回路径取得结果并交给 Interposer；Interposer 将其作为 outcome 归属对应 Branch，并通过 ASC-Proxy 提交 control plane。Agent 主进程及其 sub-agent 产生的可观察步骤都经过同一处理路径，因此 AScout 介入的是步骤执行边界，而不改变 Agent 原有的执行流程。

Parent 已被放行并获得真实工具结果后照常接收该结果。该 operation 有存活候选待裁决时，Parent 在开始下一轮模型推理前等待裁决收敛。没有存活候选、全部候选被丢弃或 Part 保留父 session 时，Parent 立即继续。Full 接替父 session 时，旧 Parent 受管停止。每个工具 operation 独立提交事实和判断准入，同一消息包含多个调用本身不是拒绝条件；候选续跑前必须保证全部调用与结果正确配对，不得为了恢复上下文而重复执行其他调用的效果。工具与结果兼容性由 Policy 结合接入能力判断，能力不足须明确报告或回退，不能伪造结果。

最终权威 session 表示经过验证的执行历史，不要求准确复原候选当时收到的预测输入。有对应真实结果时，预测输入节点保留；整理将候选在预测输入之后的第一条后续记录的 `parentId` 改接到真实结果的记录 ID。由独立可接受性判断接受且未执行原权威调用时，保留候选原生历史，不伪造真实结果记录。原主分支和被丢弃分支的历史继续保留，不授予其执行权或发布权，也不改变 single-winner。此规则不额外要求另存一条完整的投机后续路径。

最终单一 session 可以由多个运行期 session 形成。Full 保留胜者 session，后续调用关联直接父级合并后的环境。Part 保留父 session，以 Runtime 回执确认的结果作为下一次模型调用的输入。父级仍在投机时，获准内容只进入父级的延续，外层未决依赖继续限制权威路由和效果交付。历史整理不决定权威，也不直接替换运行中的上下文。原始运行上下文与从整理后历史恢复的上下文可能不同，这是上述历史含义的组成部分，不承诺二者必然相同。

是否向获胜候选追加真实结果通知由 Policy 决定；默认完全不追加，而不是追加空消息。不因此强制引入新的 Policy 名称或类别。真实结果的验证与必要状态交接仍是机制责任，不依赖这条通知是否追加。

既有 `dispose.actions` 的 `Promote` action 可携带可选 `notification`，必须是非空白字符串，拒绝 null、错误类型及其他 intent 携带该字段。Policy 从 `context.authority_outcome` 取得对应 Chance／operation 的真实返回值，`context.match` 保持比较结果，Match 的 evidence 输入不变。全部正文由 Policy 构造，机制不自动补写业务文本。缺省时命令不序列化该字段、不产生占位记录。

Runtime 成功后才向 winner 发送带通知的 Promote。Pi 使用原生 `customType: notification` 和 `sendCustomMessage(..., {triggerTurn:false})`：空闲时保存，运行中等待当前 assistant 及完整工具批次结束后追加，后续自然请求可见。不使用 prompt／steer，不启动新 turn，不取消或重启推理。Promote Applied 确认接替而非通知落盘；队列接受与持久化交付必须区分。同一 `command_id` 重传只追加一次，原生 details 保留关联；不承诺跨崩溃 exactly-once。清理或 EOF 检查已接受通知，未确认持久化时报告失败并保留源文件，不能声称清理成功。

compaction 摘要保留原文，即使包含预测，也不阻止汇总或满足其他条件后的源清理，不自动重新生成摘要。通知原生 entry ID、正文及关联须在持久化、汇总、重开后保留。正常运行不为回放计算或保存专用匹配键；系统外导出只使用原生 session/events，不能把整理后的父链直接当作原请求上下文。

状态交接必须正确完成，不能把历史父链整理或权威路由更新当作交接已经完成。在飞推理、rollback 与外部资源的关联仍有实现差距，见 [Agent 接入实现](../development/agent-access.md)；这里不据此规定重启、强制取消或替换运行中上下文的方案。

### 3.1 交互接口（2026-11-16）

输入侧的可用命令与提示属于 Agent 的能力，不由 AScout 定义。Instrumentation 声明并提供交互接口，AScout 只发现、排版和转发：命令清单、编辑器能力与特殊输入模式由 Agent 侧给出，确认、选择、输入、编辑器等提示也由 Agent 侧发起；AScout 不硬编码 Agent 的命令表，不嵌入或复制 Agent 的原生界面与终端画面，也不解释提示的语义。Pi 启动并完成资源加载后获取一次命令清单；浏览器 UI 与 TUI 从已有清单展示命令，输入 `/` 不重新查询 Pi。权威切换及前端重连复用已有清单，不因 Branch 变化假设清单变化，不设自动过期或轮询。选中命令原样交回 Agent 执行。浏览器 UI 与 AScout 终端均提供确认、选择、单行输入和多行编辑提示的回答及取消入口，答案经 control plane 校验后回传，不作为普通聊天发送。

工具执行前是否批准同样由 Agent 侧判断（例如 Pi 的权限扩展发起确认提示）。Instrumentation 提供提示通道，AScout 把提示变成用户可见请求并原样回传答案；拒绝之后是工具失败交回模型还是终止本轮，由 Agent 自行处理，AScout 不新增 Policy Decision，也不定义拒绝后的结果形态。

候选 Branch 上的提示不转给用户，一直挂起；该 Branch 晋升为权威后才转给用户。Branch 被丢弃、清理或退出时，Instrumentation 必须取消挂起的提示，不能让等待无限残留；task 完成要求不存在存活候选，与既有生命周期规则一致。control plane 停止时同时关闭 Agent；提示发出后无人回答时，等待行为完全按 Agent 自身（一直等待或自行超时），AScout 不另设超时，也不补默认答案。

取消正在执行的命令，由 Instrumentation 通知该命令的执行端点停掉它。对进程来说就是杀掉这次命令。control plane 不中转这次取消。整轮中断已有基础：中止权威执行、丢弃候选，并把仍在请求阶段的工具操作标记为失败。交互实现先做到完整展示 AScout 已经接收到的 Agent 内容；在这个基础上，再提供接近 Agent 产品本身的完整交互体验。首版交互能力只承诺工具执行前的批准／拒绝与整轮中断。

## 4. 部署与生命周期

Instrumentation 在 Branch 隔离环境外的可信宿主建立本地承载，由 Runtime 负责启动、监视和回收，并由 ASC-Proxy 与 control plane 建立会话。每个 Agent 持有独立执行上下文，各 Branch 的 session 与管理上下文相互独立；共用宿主进程不合并候选上下文。control plane 按需通过 Runtime 建立 Branch 独占 Environment。执行端点的访问方式由 Runtime 交给 control plane，再交给 Instrumentation；Instrumentation 不直接读取 Runtime 留下的地址文件。拿到访问方式后，文件、shell 和 Runtime backend 声明的工具都由 Instrumentation 发给对应端点执行，control plane 不代为执行。control plane 需要的结果由 Instrumentation 执行后上报。工作区读写、shell、受管子程序和效果使用对应 Environment；Agent 模型、内存和控制通信直接在宿主运行。宿主不随 Branch 挂载切换或物化重启，环境内子程序仍按环境归属取消与清理。Agent Session 的内部管理、复制、恢复和继续运行由 Instrumentation 负责。产品扩展及自定义工具不能把宿主文件系统当作 Branch 视图；只有已适配到 Runtime 的工作区与进程访问才受隔离保证，无法拦截的直接宿主访问必须明确限制。

目标协议中，建立 session 的受管上下文并接入 Policy 时，control plane 从 Policy Engine 查询已经编译的本地 Decision，并经 Gateway 交付给对应的 Instrumentation。Pi integration 不重新编译或修改 Policy，而是执行下发 artifact 中带有既有 runner、配置和依赖的脚本；工具 Hook 的本地 `admit` 结果决定是否提交 `speculative_admission_request`。普通工具事实不因 control plane 只处理准入而停止。Runtime backend 声明的工具也不再由 control plane 代为执行。需要全局事实或生命周期裁决时仍提交 control plane。

## 相关文档

- [AScout 架构](../architecture.md)：调用路径与责任边界
- [Branch 语义设计](branch-semantics.md)：Agent-facing operation 所受的 lifecycle 约束
- [control plane 设计](control-plane.md)：Gateway 后的控制路径
- [Agent 接入状态](../usage/agent-guide.md)：当前实现与可用性
## 5. 任务终止、暂停与恢复

Agent 常用的 Ctrl-C 或退出均终止任务；Instrumentation 落实中断并报告真实完成事实，control plane 协调结束候选及相关写入，再由 Runtime 物化并报告结果。Ctrl-C 保留程序和 session，留在对话中，待上述处理完成后可接下一轮。物化失败须向用户显示失败，不能宣告任务成功。

暂停任务由 AScout 提供独立于原 Agent 的 `/pause` 入口，明确继续命令为 `/continue`，恢复原任务执行，保留原树、候选和环境关系。暂停期间的普通消息先留着，不解除暂停，也不在暂停期间交给模型；必须先 `/continue`，之后再按原顺序交给模型。它与原 Agent 的 Ctrl-C／退出终止语义分开，暂停不意味着退出程序。需要保存恢复点时，等待原生中断确认、已发出操作与命令的确定回执及所有 backend 状态持久化后再发布。保存失败保留暂停的原运行和上一恢复点，允许用户继续或重试。恢复必须使用各 Branch 的原运行上下文及其等待关系，并与 control plane、环境状态、配置及外部资源身份校验对应关系；不能从整理后的最终历史推定原上下文，也不能因缺少回执重试可能已经执行的工具。保存源与恢复后的可写源必须独立。恢复完成后等待用户输入，与 Pi 原生重新打开 session 的体验一致。实现状态与入口调整缺口见 [恢复](../development/recovery.md)。

已发布恢复点的原生运行源与继续执行的可写源必须相互独立；保存失败后继续或从恢复点继续，都不得改写此前保存的原生源。

环境中的可变数据同样必须保存独立内容；不能仅以宿主路径或 inode 相同推定内容未变。未复制的系统只读依赖和原生扩展资源须绑定可验证的内容或版本，不满足保存或恢复前提时明确拒绝。恢复中途失败须回收已经启动的运行资源，不得启动 Agent 推理；独立保留的 rollback 对象不依赖原可变 ref 仍存在。

原生工具帧的取消终态与 control plane Chance 的结果等待分别管理。原生取消不能自行表示 Chance 被 Discard；已经发出的真实动作及控制命令必须取得确定回执或保留可恢复关系，已发生的副作用不撤回、不重复。恢复的等待以 Branch、调用、checkpoint 和配对结果身份重建；只有用户继续后才解除恢复暂停并允许有效分支推进。

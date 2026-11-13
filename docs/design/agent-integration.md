# Agent 接入侧设计

本文展开 [AScout 架构](../architecture.md) 中的 Agent 接入侧，定义 Agent 或 Agent product 如何进入 AScout 的受控路径，以及接入组件之间的责任边界。

## 1. 总体设计

Agent 接入侧是围绕 Agent 执行环境提供本地管理的责任边界，不另设“端侧管理节点”这一并列组件。它负责管理由 Agent 主进程及其 sub-agent 组成的执行实体，以及这些进程产生的执行活动。一个执行环境可以建立相应的接入侧承载，环境内的每个 Agent 在启动时接入该侧，并持有独立的 Agent 执行上下文；每个上下文再通过 Branch 上下文区分不同的逻辑执行延续，并依据 Agent、Branch、operation 归属和 capability 处理事件与控制命令。

Agent 接入侧与控制面共同完成投机执行：它观察和拦截 Agent 行为，在本侧处理属于本地范围的 Policy Decision，并将依赖全局控制状态或生命周期的决策交由控制面。多个 Branch 可以暂时共享同一个执行环境和接入侧承载；当 Branch 需要独立的 Branch-local 状态时，其管理上下文随执行活动进入相应的独立环境。

### 1.1 职责

Agent 接入侧在 Agent 执行过程中落实 Policy Decision 对投机执行流程的决策。对于由 Policy Engine 编译并交付的 Decision，接入侧结合当前 Agent 执行上下文和可观察事实，对属于本地范围的 Decision 独立作出判断，并向控制面同步 Decision 内容、输入事实、结果和 log 信息。

对于依赖全局控制状态、资源或 Branch 生命周期的 Decision，Agent 接入侧将相关事实和请求提交控制面，由控制面完成全局判断、生命周期编排和后续处理。Agent 接入侧接收控制面返回的结果和命令，并据此继续处理对应的 Agent 执行；相关事实、Decision、结果和事件通过 Agent 接入侧与控制面之间的双向通道传递。

Agent 接入侧负责观察和拦截 Agent 的执行行为，识别 operation 的开始、`pending-result boundary`、完成和 outcome，并处理 Agent 主进程及其 sub-agent 产生的可观察执行活动。它根据本地 Decision 或控制面返回结果，允许真实 operation 继续、暂停或阻止真实执行、等待后续结果，或者让当前调用沿 Agent 原有的完成路径使用已经获得语义授权的结果完成。

Agent 接入侧负责在 Branch-local 状态或受纳管 Effect 发生前识别相应边界，并保持 Branch 管理上下文、事件和控制命令的正确归属。多个 Branch 在尚未需要独立 Branch-local 状态时，可以在共享执行环境中承载，各 Branch 仍保持独立的管理上下文。当某个 Branch 需要独立的 Branch-local 状态时，Agent 接入侧向控制面请求建立相应的独立执行环境，该 Branch 的管理上下文随其执行活动进入能够提供相应隔离保证的环境；具体承载方式可以复用、迁移或重新建立。识别到共享、外部或 Policy 要求纳管的 Effect 时，Agent 接入侧在 Effect 发生前将相关请求交给控制面处理。真实 operation 保持 once-only，完成后的 outcome 自动归属对应 Branch 并发布到控制路径。

## 2. 接入组件

### 2.1 Product Integration

Product Integration 是面向具体 Agent product 的产品适配组件，负责依据接入约定，将 Agent 提供的执行点连接到 Agent 接入侧。Agent 在启动时通过 Product Integration 接入当前执行环境中的 Agent 接入侧；Product Integration 以尽可能小的产品改造接入 Agent 的内部执行结构，使 Agent 接入侧能够观察目标执行活动，并在真实 operation 开始前暂停或阻止其执行。

当 Agent 即将执行可管理步骤时，Product Integration 将该步骤交给 Agent 接入侧，并等待 Agent 接入侧返回处置结果。返回继续时，Product Integration 允许产品按原有路径执行该步骤；返回跳过时，Product Integration 不执行原步骤，并在需要时让当前调用沿产品原有的完成路径使用 Agent 接入侧提供的结果结束。暂停通过等待处置结果体现，不需要额外的暂停机制。

Product Integration 从被拦截步骤的原调用返回路径自动取得成功、失败或取消结果，并将其交给 Agent 接入侧形成 outcome。它不修改 Agent 的业务逻辑，也不要求 Agent 主动调用 AScout API；具体产品适配可以了解其内部执行结构，但应将改造限制在观察和控制所需的接入范围内。只有产品确实提供真实的执行前接缝时才能声明支持该类 action；SDK 的输出观察事件（例如 `message_start`）本身不是推理前 Hook，不能冒称为此能力。

`extent` 与 `effect` 属于 Agent 接入侧在本地范围内求值的 Decision，由与具体产品无关的通用接入层统一完成；Product Integration 只把产品的执行点交给通用接入侧，并执行其返回的继续、阻止、暂停或使用提供结果的处置，不自行求值 `extent` 或 `effect`。产品专有的消息、checkpoint、session 和 SDK 细节留在 Product Integration，不进入通用求值逻辑。

Product Integration 负责完整持久化运行分支的 session 记录，并在接入侧汇总为单一最终权威 session，再报告可定位的记录引用；通用层只传递文件位置、Branch 与记录引用等契约数据，不解释产品格式。完整记录包括 compaction 和 custom 记录，不限于消息正文。文件写入或读取位置提示不等于 Agent session 释放、Branch 执行释放或 Runtime 清理。

### 2.2 ASC-Proxy

ASC-Proxy 是 Agent 接入侧与控制面之间的受管通信代理。它可以由控制面随 Agent 一起启动的进程内组件、本地 IPC 客户端、网络客户端或独立进程实现，不要求用户手工启动或直接管理。它与控制面建立通信会话，并在会话建立过程中确认通信双方的身份；在后续通信中，它通过通信凭证证明消息来自对应的 Agent 接入侧，避免信息伪造。

通信会话建立后，Agent 接入侧通过 ASC-Proxy 向控制面提交 Agent 执行事实、控制请求、本地 Decision 结果和 outcome；控制面通过同一会话向 Agent 接入侧返回命令、结果和事件。ASC-Proxy 负责消息的发送、接收和传递，不参与 Agent 步骤的执行、Decision 的判断或 Branch 生命周期的处理。正式用户流程中，控制面负责启动和回收该 Agent 及其接入会话，用户不直接管理 ASC-Proxy。

### 2.3 Interposer

#### 执行事实与来源

Agent 接入协议围绕 action，而非工具或 Branch 角色组织。action 可以是模型推理、工具执行、启动 sub-agent、发送消息、等待或结束；每个 action 关联 `task_id`、`branch_id`、`action_id`、`action_kind`，可用 `parent_action_id` 说明因果归属。Authority/Candidate 由 Tree 查询，不由事件名称声明。

`ActionStarted` 登记 action 生命周期；`result_pending=true` 明确表示接入侧停在真实执行之前的等待边界，不能据此断言真实 action 已经执行。普通 action 事实与投机准入请求分开；普通执行无需等待控制面投机决策。`ActionFinished` 报告成功、失败、取消或超时，并携带 result 和可选 evidence；取消请求不等于已取消，控制面等待期限届满也不等于 Agent 已停止。相同 action 的结果必须与开始事实配对。

`ActionStarted`、`ActionFinished` 与 `SpeculativeAdmissionRequest` 各自携带接入侧测量的 Unix 毫秒 `timestamp_ms`，该时刻由接入侧在事件发生时测量，不使用控制面收到消息的时刻。工具的 `ActionStarted` 记录工具真正开始的时刻；`SpeculativeAdmissionRequest` 只表示投机起点，不能作为 action 的开始时间，控制面也不据此合成开始时间。投机开始以 `SpeculativeAdmissionRequest` 的 `timestamp_ms` 为准；权威路径真实结果到达的时刻以对应 `ActionFinished` 的 `timestamp_ms` 为准。`Usage` 应关联 `action_id`，并报告接入侧测得的模型请求开始与结束时间；接入侧确实无法把一次模型调用与已上报的 action 配对时（例如该调用没有经过接入侧 Hook），仍然上报这次用量，`action_id` 为 null，不伪造 action 身份，也不静默丢弃。控制面由此得到源侧耗时，而不是用自身记录时刻冒充执行耗时。

当前通用协议提供显式的 `SpeculativeAdmissionRequest` 路径，只有接入侧本地 Decision 产生该请求时，控制面才会进入投机准入和预测。Pi 的工具执行前 Hook 已接入下发的 Policy artifact，并按其 `admit` 结果区分普通 action 与投机准入请求；模型推理前已有真实 Hook，位于 Pi 分支的 `sdk.ts`，在请求发出前执行，可延迟请求或抛错阻止。sub-agent 启动前目前仍无统一 Hook，Pi 接入声明 `subagentPreExecutionInterception: false`，暂不支持该边界。Pi 的缓存预热等后台模型调用不经过该 Hook，不得宣称所有模型请求都被拦截。

Agent 连接、就绪、断开、失败、释放，以及 Branch 创建、开始、就绪、暂停、恢复、失败、释放，属于各自主体的元事件。Agent session 释放、Branch 执行释放和 Runtime 环境清理分别表达。Agent 不能报告 Tree 已 Promote/Discard，也不能代替 Runtime 确认环境清理。

`CommandResult` 是 Agent Adapter 对控制命令的接收、开始及终态回执；Applied 只确认接入侧已落实，不代表 Runtime 或整棵 Tree 的转换全部完成。控制面分别处理 Runtime 的操作结果、环境事件和 Agent 回执，再推进 Tree。输出、执行记录和使用量按 Branch 归属；未接受的 Candidate 输出只可作为诊断内容，不进入权威用户输出。

Interposer 是 Agent 接入侧中介入 Agent 执行接缝并落实本地 AScout 语义的责任实体。Agent 即将执行可管理步骤时，Product Integration 将步骤交给 Interposer；Interposer 从拦截点提取步骤事实，识别 operation 的开始、`pending-result boundary` 以及可能产生的 Branch-local 状态和 Effect，并暂停当前步骤，等待本地 Decision 或控制面返回处置结果。普通步骤根据处置结果沿原有路径继续执行，不需要进入投机处理。

属于本地范围的 Decision 由 Interposer 结合当前执行事实直接判断和执行；依赖全局控制状态、资源或 Branch lifecycle 的 Decision，则由 Interposer 通过 ASC-Proxy 将相关事实和请求提交控制面，并等待控制面返回结果或命令。

Interposer 根据处置结果允许真实 operation 继续，或跳过真实 operation。继续时，Product Integration 按产品原有路径执行步骤；跳过时，真实 operation 不执行，并在需要时让当前调用沿产品原有完成路径使用已经获得语义授权的结果结束。Interposer 可以暂停一次 Agent 步骤，并在本地 Decision 或控制面结果返回后继续处理。

Interposer 还负责在 Branch-local 状态或受纳管 Effect 发生前识别相应边界。需要独立 Branch-local 状态时，它通过 ASC-Proxy 向控制面请求建立相应的独立执行环境；识别到共享、外部或 Policy 要求纳管的 Effect 时，它在 Effect 发生前将相关请求提交控制面。真实 operation 保持 once-only；继续执行时，Interposer 从原调用返回路径取得成功、失败或取消结果，将 outcome 归属对应 Branch 并通过 ASC-Proxy 提交控制面。Interposer 同时记录本地 Decision 的输入事实、求值内容、Policy 版本、结果和执行上下文。

## 3. Agent 执行接入路径

用户交互入口与后端 Agent 执行实体分离：用户始终使用同一 AScoutAgent 视图，接入实现依据控制面已确认的权威执行处理输入、输出和中断路由。Promote 不要求用户重新连接、加载另一个 session 或等待候选会话结束，也不为路由变化取消或重启候选正在进行的执行。已展示内容不可撤回，未获接受的候选内容不得作为正式回复提前发布；切换时必须保持输入归属及输出顺序，旧主路径失去发布权后仍按生命周期规则停止和清理。运行能力不满足时应拒绝该接替路径，不能以路由切换伪造接受或效果处理完成。

用户可通过独立的只读观察查看候选状态及内容，候选内容与主对话分开展示，并随 Branch 的角色与状态显示其投机身份。接入侧按 Branch 报告实际模型、工具和执行活动；控制面提供预测、比较、处置与生命周期事实。界面应区分请求已发送、实际开始、执行结束和清理确认，不能把未知状态显示为已完成。只读观察不使候选获得用户对话或裁决权限，也不把其内容注入其他 Branch 的上下文。

AScout 管理显示区域、Tree、窗口选择和控制事实，并将用户输入和中断交给相应接入实现。接入侧通过产品公开 SDK 报告消息、思考、工具调用、结果和会话事件；AScout 的 UI 负责排版这些事实，不嵌入或修改产品的终端画面，也不调用产品内部展示组件。浏览器 UI 与 TUI 通过共同的事实和操作接口工作，彼此不依赖绘制实现。窗口切换不等于会话切换或重启；候选观察保持只读。对应产品的交互能力按实际接入范围整合，未支持的能力明确说明，不承诺不同产品的全部交互完全一致。

观察先展示当前 AScoutAgent 整棵 Speculative Tree 的 Branch 关系、当前权威路径与投机状态，再允许选择具体 Branch 深入只读查看。选择 Branch 不改变权威执行、输入路由或接受条件，也不向该候选建立用户对话。用户界面可拦截自己的观察操作，不把它作为普通提示词交给 Agent；本地 TUI 的具体操作见开发文档与术语表。

Agent 按照自身原有流程执行步骤。当 Agent 即将执行处于接入范围内的步骤时，Product Integration 将该步骤交给 Interposer。Interposer 根据本地 Decision 或控制面返回的处置结果处理该步骤；需要控制面参与时，Interposer 通过 ASC-Proxy 传递相关事实和请求，并接收控制面返回的结果或命令。Product Integration 根据 Interposer 的处置结果让原步骤继续执行，或跳过真实执行并沿 Agent 原有的完成路径结束当前调用。

真实步骤继续执行时，Product Integration 从原调用返回路径取得结果并交给 Interposer；Interposer 将其作为 outcome 归属对应 Branch，并通过 ASC-Proxy 提交控制面。Agent 主进程及其 sub-agent 产生的可观察步骤都经过同一处理路径，因此 AScout 介入的是步骤执行边界，而不改变 Agent 原有的执行流程。

Parent 获得真实结果后沿原路径继续，不以候选比较完成为结果交付前提；只有控制面确认 Promote 时旧 Parent 才失去权威并受管停止。每个工具 operation 独立提交事实和判断准入，同一消息包含多个调用本身不是拒绝条件；候选续跑前必须保证全部调用与结果正确配对，不得为了恢复上下文而重复执行其他调用的效果。工具与结果兼容性由 Policy 结合接入能力判断，能力不足须明确报告或回退，不能伪造结果。

最终权威 session 表示经过验证、用真实结果整理后的执行历史，不要求准确复原候选当时收到的预测输入。预测输入节点保留；整理时，将候选在预测输入之后的第一条后续记录的 `parentId` 改接到对应真实结果的记录 ID。原主分支和被丢弃分支的历史继续保留，不授予其执行权或发布权，也不改变 single-winner。此规则不额外要求另存一条完整的投机后续路径。

最终单一 session 不等于运行期间只能有一个 session。Promote 后 winner 沿自己的 session 保留执行进展继续；历史整理不决定权威，也不直接替换运行中的上下文。原始运行上下文与从整理后历史恢复的上下文可能不同，这是上述历史含义的组成部分，不承诺二者必然相同。

是否向获胜候选追加真实结果通知由 Policy 决定；默认完全不追加，而不是追加空消息。不因此强制引入新的 Policy 名称或类别。真实结果的验证与必要状态交接仍是机制责任，不依赖这条通知是否追加。

既有 `dispose.actions` 的 `Promote` action 可携带可选 `notification`，必须是非空白字符串，拒绝 null、错误类型及其他 intent 携带该字段。Policy 从 `context.authority_outcome` 取得对应 Chance／operation 的真实返回值，`context.match` 保持比较结果，Match 的 evidence 输入不变。全部正文由 Policy 构造，机制不自动补写业务文本。缺省时命令不序列化该字段、不产生占位记录。

Runtime 成功后才向 winner 发送带通知的 Promote。Pi 使用原生 `customType: notification` 和 `sendCustomMessage(..., {triggerTurn:false})`：空闲时保存，运行中等待当前 assistant 及完整工具批次结束后追加，后续自然请求可见。不使用 prompt／steer，不启动新 turn，不取消或重启推理。Promote Applied 确认接替而非通知落盘；队列接受与持久化交付必须区分。同一 `command_id` 重传只追加一次，原生 details 保留关联；不承诺跨崩溃 exactly-once。清理或 EOF 检查已接受通知，未确认持久化时报告失败并保留源文件，不能声称清理成功。

compaction 摘要保留原文，即使包含预测，也不阻止汇总或满足其他条件后的源清理，不自动重新生成摘要。通知原生 entry ID、正文及关联须在持久化、汇总、重开后保留。正常运行不为回放计算或保存专用匹配键；系统外导出只使用原生 session/events，不能把整理后的父链直接当作原请求上下文。

状态交接必须正确完成，不能把历史父链整理或权威路由更新当作交接已经完成。在飞推理、rollback 与外部资源的关联仍有实现差距，见 [Agent 接入实现](../development/agent-access.md)；这里不据此规定重启、强制取消或替换运行中上下文的方案。

## 4. 部署与生命周期

Agent 接入侧可以在执行环境中按需要建立其本地承载，并由 ASC-Proxy 与控制面建立会话。环境内的每个 Agent 在启动时通过 Product Integration 接入 Agent 接入侧，并持有独立的 Agent 执行上下文。多个 Agent 及其 Branch 可以暂时共享同一个执行环境和接入侧承载；当某个 Branch 需要独立的 Branch-local 状态时，控制面通过 Runtime 接入边界请求相应的承载能力，并使该 Branch 的执行活动进入满足要求的环境。具体承载方式由实现决定，但必须保持各 Agent 和 Branch 上下文的独立性。状态可以复制、共享、写时复制、重建或恢复，优先采用满足要求的低成本方式。Agent Session 的内部管理、复制、共享、恢复和继续运行由 Agent 接入侧负责；Runtime 负责提供其所需的动作承载和环境约束。

目标协议中，Task 绑定 Policy 并建立受管上下文时，控制面从 Policy Engine 查询已经编译的本地 Decision，并经 Agent Control Gateway 交付给对应的 Agent 接入侧。Pi 接入侧不重新编译或修改 Policy，而是执行下发 artifact 中带有既有 runner、配置和依赖的脚本；工具 Hook 的本地 `admit` 结果决定是否提交 `speculative_admission_request`。普通工具事实不因控制面只处理准入而停止，Runtime 工具仍沿原有结果等待路径继续。需要全局事实或生命周期裁决时仍提交控制面。

## 相关文档

- [AScout 架构](../architecture.md)：调用路径与责任边界
- [Branch 语义设计](branch-semantics.md)：Agent-facing operation 所受的 lifecycle 约束
- [控制面设计](control-plane.md)：Agent Control Gateway 后的控制路径
- [Agent 接入状态](../usage/agent-guide.md)：当前实现与可用性
## 5. 主动退出与恢复

主动退出立即请求 Agent 原生中断，等待取消完成及所有 backend 状态持久化后发布恢复点，保留有效候选。保存失败保留暂停的原运行和上一恢复点，允许用户继续或重试。恢复必须使用各 Branch 的原运行上下文及其等待关系，并与控制面、环境状态、配置及外部资源身份校验对应关系；不能从整理后的最终历史推定原上下文，也不能因缺少回执重试可能已经执行的工具。保存源与恢复后的可写源必须独立。恢复完成后等待用户输入，与 Pi 原生重新打开 session 的体验一致。实现状态与边界见 [恢复](../development/recovery.md)。

已发布恢复点的原生运行源与继续执行的可写源必须相互独立；保存失败后继续或从恢复点继续，都不得改写此前保存的原生源。

环境中的可变数据同样必须保存独立内容；不能仅以宿主路径或 inode 相同推定内容未变。未复制的系统只读依赖和原生扩展资源须绑定可验证的内容或版本，不满足保存或恢复前提时明确拒绝。恢复中途失败须回收已经启动的运行资源，不得启动 Agent 推理；独立保留的 rollback 对象不依赖原可变 ref 仍存在。

原生工具帧的取消终态与控制面 Chance 的结果等待分别管理。原生取消不能自行表示 Chance 被 Discard；已经发出的真实动作及控制命令必须取得确定回执或保留可恢复关系，已发生的副作用不撤回、不重复。恢复的等待以 Branch、调用、checkpoint 和配对结果身份重建；只有用户继续后才解除恢复暂停并允许有效分支推进。

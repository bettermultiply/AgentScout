# Runtime 设计

## 目标

Runtime 是 AScout 用来承载 Agent 一步动作的受约束执行系统。一步动作可以是一次模型推理或一次工具调用。它不是替 Agent 决定如何推理，也不只是把命令运行起来，而是提供动作正常、正确运行所需的条件，并把动作对真实世界或 Authority 的影响限制在允许范围内。投机动作通常不应直接影响真实世界；具体允许哪些影响，必须遵守任务要求和既定的效果约束。

Runtime 面向一般投机执行，不专门为 NLE 或某个 Agent 产品设计，也不等同于虚拟机。AScout 自身支持一套 Runtime；Runtime Adapter、Adapter Registry、Effect Coordinator、环境、进程与资源管理都属于这个整体中的接入或实现职责，而不是几套平级系统。适配外部执行机制是为了便于接入，不意味着 AScout 只有适配器而没有自己的 Runtime。Runtime 经 Gateway 与控制面通信，语义边界仍由 Runtime Adapter 契约规定；实际启动、监视和回收 Agent 与接入进程由 Runtime 负责。

默认 Runtime 面向可信 workload；对抗性 containment 由部署方选择 hardened Runtime 与 Adapter 提供，不能改变 AScout 的生命周期语义。

## Adapter Registry

Adapter Registry（简称 Registry）属于 Runtime，负责 Adapter 登记、契约兼容性和能力声明校验，维护接入状态、契约版本和声明快照。首期只检查控制面需要的 API 是否存在并可解析，不主动建立验证环境或执行验证场景；因此首期不产生环境保证已验证的结论。Adapter 提供接入信息，不得自行填写验证状态、验证证据，或以自测结果代替未来的 Registry 验证。

Registry 向控制面提供带验证状态的能力视图，Policy Engine 据此检查 Policy 要求；Registry 不拥有 Tree、Branch 生命周期或 Runtime 执行状态，也不编排投机活动。能力降级、未验证范围与验证失败的处理遵守 [Runtime Adapter 契约](runtime-adapter-contract.md)，不因 Registry 属于 Runtime 而放宽证明要求。

## 状态与接入侧

Environment 的身份由 Runtime 创建并随建立或派生结果返回；控制面持久化 Branch 与该身份的独占绑定。初始 Authority 同样先请求 Runtime 建立环境；Candidate 派生只指定父 Environment。创建前 Branch 可无绑定，后续操作与恢复一律使用已返回的身份，不用 Branch 名称、authority 或工作区路径代替。内部 workload 可共享，但各 Environment 的生命周期独立；Backend 组合及其身份不交给控制面决定。

控制面与 Adapter 的 `ControlSession` 只承担网络连接、握手和请求/结果关联；单个 Runtime 对控制面表现为统一身份和统一入口，内部节点、进程、Backend instance 及路由不暴露。控制面为每个 Branch 显式维护独占的 `environment_id`，Runtime 普通请求只接收 `environment_id` 与 operation，不能从环境推导 Branch。Runtime 内部维护 Environment 与 Backend instance 的多对多组合，并以 `backend_id` 区分实例；控制面不传 `backend_id`。Adapter Registry 首期的 API 存在性/可解析检查也不等于环境保证已验证，Agent 接入验证与 Runtime 能力验证保持独立边界。

系统最终必须把 Authority 的结果物化到现实。Agent 完成本轮工作、开始等待用户输入时，task 完成；正常完成时，在没有存活候选且任务相关写入完成后物化并重置基准，不为物化暂停执行。取消、失败或终止任务立即进入物化处理，先结束候选并等待任务相关写入完成，再实际写回。Agent 常用的 Ctrl-C 或退出均表示终止任务；Ctrl-C 不关闭程序，保留 session 并留在对话中，待候选结束、相关写入及物化处理完成后可接下一轮。暂停任务由 AScout 提供独立于原 Agent 的 `/pause` 入口，明确继续命令为 `/continue`，恢复原任务执行，保留原树、候选和环境关系；普通消息是否也解除暂停未定。它与原 Agent 的 Ctrl-C／退出终止语义分开，暂停不意味着退出程序。Runtime 经接入边界向控制面报告物化完成或失败，物化失败等同操作失败，须向用户显示失败；控制面必须等物化成功后才能宣告任务成功。旧环境及基准在无人引用时释放。

接受候选并由其承接后续执行时，继续使用候选环境，不默认把候选改动复制回父分支。Runtime 保持必要状态交接、环境引用和效果边界；嵌套接受仍受祖先未决依赖约束，不能据此成为整棵树的 Authority 或提前物化。此选择不新增环境迁移模式或 Policy 选择接口。

文件系统物化覆盖工作目录及任务允许修改的其他挂载，只落实 Authority 增量最终视图的改动，含文件属性；具体属性变更支持范围见 [物化提案](../fs-backend/materialize-proposal.md) 的首版范围，不因此允许破坏原文件未改属性或链接结构。同路径 Authority 直接覆盖，不检测或报告物化冲突，不比较宿主与共同祖先，不新增全量哈希或复制整个挂载；运行期间不编辑宿主目录是使用约定。暂允许读者见到中间态，部分写回失败保留已写部分并重试剩余部分，不承诺回滚；保证写入完成后正常退出可见，不新增掉电保证。实现缺口与待讨论事项见物化提案。

Session、Workspace、进程和其他状态可以按需采用不同的承载方式，包括复制、共享、写时复制、重建或恢复。目标是在满足正确性和效果边界的前提下尽可能降低成本，而不是每次投机都完整复制 Agent 或创建独立进程、容器、虚拟机。一步动作是最小讨论粒度，不意味着环境只能存在一步，也不要求每步重新创建环境。

Agent Session 的内部管理、fork、恢复和继续运行由接入侧负责，因为它理解具体 Agent 的会话结构和执行切点。Runtime 为相应执行活动提供环境、状态和效果约束；接入侧与 Runtime 必须关联到同一 Branch 和动作上下文，不能只恢复 Session 而忽略它所依赖的文件或进程状态。控制面仍管理全局 Branch 关系与裁决；Session 派生、环境派生和 Authority 切换不能相互冒充完成。

## 可插拔能力

Runtime 的具体能力都可以插拔和组合，包括文件系统隔离、进程隔离、网络隔离、资源限制、状态保存与恢复、效果处理等。Overlay、Container 和 Firecracker 是不同的能力实现，可以替换或组合。Overlay 是文件系统隔离的一种实现，不是所有 Runtime 的必需结构；如果 Firecracker 等机制已提供符合要求的文件系统隔离，就没有额外使用 Overlay 的要求。

也可以不使用上述隔离机制而运行，只是可提供的保证和可承载的任务范围会缩小。能力退化不意味着可以放宽必要的正确性和效果边界：仍满足要求的动作可以继续；不满足要求的动作应等待、采用其他合法路径、明确降级或拒绝，不能把未隔离状态声称为已隔离。效果处理需要服从既定的 commit、accept、discard 和 single-winner 语义，接受 Candidate 不等于无条件复制文件回 Parent，也不增加通用状态合并能力。

任务需要哪些能力、当前能力是否足够，不能由 Runtime 单方面判断；它需要与 Policy 协调、协商，并结合运行时检测确认实际保证。具体协商与检测机制、当前开发范围见 [Runtime 开发说明](../development/runtime-adapter.md)。本设计不预先固定它们的接口、部署或实现。

## 相关文档

- [架构](../architecture.md)：接入侧、控制面和 Runtime 的职责。
- [Runtime Adapter 契约](runtime-adapter-contract.md)：Runtime 的可观察接入边界。
- [Agent 接入侧](agent-integration.md)：Session 和执行切点。
- [效果管理](effect-management.md)：状态变化与效果处理纪律。
- [Branch 语义](branch-semantics.md)：接受、丢弃和状态可见性。

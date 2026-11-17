# 设计决策记录

本文件以简短结论定位设计选择，重点记录『为什么』，不重复展开规则。目标语义以对应正式主题文档为准；本文不作为语义权威，也不表示相关能力已经实现。编号只在本文内唯一，跨文件引用必须写文件名加编号；编号不连续，缺号表示该条已落实进正式文档或已废弃。新增条目接续现有最大编号，不重排历史编号。

## D-004：Branch 覆盖完整 Agent 进程树

结论：Branch 覆盖完整进程树。
为什么：只复制主进程会丢失进程关系、协作状态以及仍在进行的子任务，无法表示完整候选世界。

## D-006：覆盖型 branch-local state 采用 latest-visible-write

结论：覆盖型状态以最新可见修改为准。
为什么：Ancestor state 是 child 执行环境的一部分，因此 ancestor 后产生的修改应自然进入 descendants，同时维持 sibling 隔离。

## D-007：Single winner，不做 merge

结论：单次裁决只选一个直接子树，不合并。
为什么：AScout 选择完整的计算延续，不定义跨文件、计算状态和全局服务的通用 merge 协议。

## D-008：允许提交包含活跃 descendants 的 subtree

结论：子树可带活跃后代提交。
为什么：是否等待 descendants 完成属于 Agent 的运行策略，不应成为底层机制的强制限制。接受后 winner subtree 整体保留，不要求先执行 bottom-up 压平。

## D-009：Accept 将树位置重定向到 winner

结论：Promote 保留胜者身份并替换树位置。
为什么：接受的是完整 candidate subtree，而不是把 candidate 压平回 parent identity。保留 winner identity 和 descendants 可以让 Accept 成为树指向变换，并避免强制 bottom-up。

## D-015：Branch 与 Branch Root 如何区分？

结论：Branch 表示子树，根节点另行区分。
为什么：Branch 操作以整个子树为范围，而控制面和 Runtime 仍需要指称其根节点及其对应执行单元。显式区分 Branch 与 Branch Root 可以避免把子树范围和单节点状态混为一谈。

## D-016：Ancestor 输入直接 fan-out 到 descendants

结论：祖先输入向后代传播，读取位置独立。
为什么：Ancestor 在 descendants 运行期间收到的新输入应自然进入这些 descendants，而不要求额外 acknowledge 或 commit barrier。

## D-017：Branch-local state 隔离，跨 Branch 显式通信

结论：分支本地状态隔离，跨分支显式通信。
为什么：Ancestor 可见性和全局环境共享不应破坏 branch-local state 的隔离，也不应隐式赋予 parent 或 sibling 对其他 Branch 内部状态的读取能力。

## D-019：Runtime 节点状态与外部 Branch 执行

结论：控制面定权威，接入侧管执行，Runtime 管环境。
为什么：Agent 调用的 CLI 通常只是短命子进程。让它在自身内部执行 fork 只会复制调用进程，不能满足 D-004 的完整 Agent 进程树语义。

## D-021：Agent-facing context 与 opaque candidate handle

结论：上下文隐藏裸身份，句柄限定直接子分支。
为什么：Agent 需要理解当前推演层级并恢复控制上下文，但不需要依赖 runtime 的内部标识格式。Direct-child view 支持 review，同时维持 D-017 的隔离和受控通信边界。

## D-023：Live-child discard 与受控结果披露

结论：可丢弃未提交子树，披露知识不可撤销。
为什么：跨步骤投机中，parent 常在 child 完成前获得真实 ancestor 结果，并应立即取消不匹配或不再需要的 candidate。结果披露必须维持 D-017 的隔离边界。

## D-024：Outcome-guided continuation branching

结论：从共同执行基础按预测结果推进候选。
为什么：该模型保留完整 Agent execution continuation，同时把真正需要同步的步骤结果建模为可验证事件，避免要求任意多进程内存 later-write propagation 或 parent/candidate merge。

## D-027：控制面、Runtime Adapter 与 Runtime 解耦

结论：控制面经适配边界使用 Runtime。
为什么：Adapter 作为接入边界使稳定语义与具体实现分离；粗粒度原子接口避免半完成的 barrier、Branch、Accept 或 cleanup，连接协议的变化不能泄漏为控制面 Policy 或改变 lifecycle 语义。

## D-028：Speculative Tree 关系与 Agent execution activity 正交

结论：树关系与执行活动状态正交。
为什么：Speculative Tree 是计算世界的所有权与派生结构，不应泄漏或固化 backend 的进程调度状态。分离两类状态允许等待时节省资源，并维持 single-winner 与 resolver authorization。

## D-029：外部效果使用声明式 Observation surface

结论：效果保证限定于声明的观察面与期限。
为什么：这覆盖返回值、trigger、webhook、消息和 shared quota 等真实的间接业务效果，同时避免把任意物理资源消耗和假设性的长期组织决策纳入不可验证的无限因果模型。

## D-030：Branch barrier 与 Resolution barrier

结论：分支与裁决使用短暂原子边界。
为什么：短暂原子边界提供一致 execution base 和 single-winner 转换，同时允许 parent 与 candidates 在 speculation window 内并行承担不同角色，避免把已删除旧原型的立即阻塞固化成目标语义。

## D-033：禁止跨 Speculative Tree 操作

结论：分支控制只限于同一棵树。
为什么：跨 tree 控制不属于 speculative subtree selection，会引入额外的 identity、授权、故障恢复与状态组合语义。需要在多个 execution 之间协调的上层系统应使用 AScout 之外的接口，而不是扩展 Branch lifecycle。

## D-034：OS privilege 留在 Runtime 执行实现一侧

结论：系统特权留在 Runtime 实现侧。
为什么：把特权交给控制面或 Agent 会混合 policy 与执行机制，并允许调用方绕过后端实际状态转换。AScout 只需规定 Adapter 必须返回足以确认操作结果的事件，不需要规定 Runtime 内部如何持有特权。

## D-035：Control transport 由 Runtime Adapter 决定

结论：控制传输由适配器实现选择。
为什么：高效默认 Runtime、独立 sandbox daemon、VM Runtime 与 remote Runtime 的通信需求不同。统一 transport 不增加 Branch 语义，却会限制 Runtime 的部署和优化空间。

## D-037：最小 Agent-facing control surface

结论：面向 Agent 的控制操作保持最小。
为什么：Agent 只需要表达自身意图，不应承担 Runtime 内部 outcome routing、capability transfer 或状态机复合操作。缩小 routine surface 可以避免裸 identity、隐式权限和 profile-specific lifecycle。

## D-038：ASC-Proxy 作为 Agent-facing context boundary

结论：ASC-Proxy 隐藏本地调用身份。
为什么：把上下文隐藏和本地调用入口收敛到 ASC-Proxy，可以让 CLI/library 使用稳定的无身份请求，同时保持 Gateway 的统一通信边界，并将控制面与 Runtime 的状态责任分开。

## D-039：接入侧、控制面与 Runtime

结论：接入、控制与执行承载分责。
为什么：调用路径、部署对象和内部实现是不同层级。接入边界使 Runtime 的具体实现保持开放，同时明确其作为完整执行系统的责任。

## D-040：Policy 控制的 best-effort 投机

结论：Policy 在固定正确性边界内尽力优化。
为什么：把策略选择集中在控制面，可以让不同 Agent 复用同一套机制并替换策略；把 best-effort 与固定正确性约束分开，既保留优化空间，也避免把"启用"误解为无条件分支或性能保证。

## D-041：authority 决策槽并入 launch

结论：权威来源固定，launch 决定取得时机。
为什么：launch 的取值已经完整决定权威值的取得方式（是否取、何时取）；来源本身由 Edge 模型决定，不构成独立策略选择，拆槽造成职责重叠与表述混淆。

## D-042：父 Proposal 确认后的后代 Branch 重新挂载

结论：父前提确认后，后代保留进展重新挂载。
为什么：嵌套 Branch 运行在自身隔离环境中，父 Branch 确认不需要搬迁或重放子 Branch 状态；只需更新控制面的树结构与未决依赖。该规则保留嵌套执行进展，同时维持 `AssumptionSet` 和权威信息流约束。

## D-043：待选机会由 AScout 暂存，Policy 直接选择启动项

结论：AScout 暂存机会，Policy 选择启动项。
为什么：单个 Chance 的独立“现在是否启动”无法表达多个机会竞争同一资源时的整体取舍，也会永久丢弃暂时不能启动但稍后仍有价值的机会。把登记、全局选择与机制复核分开，可以保留机会并让策略控制调度，同时不把队列状态或资源所有权交给 Policy。

## D-044：已启动投机工作由 rebalance 决定暂停、恢复和抢占

结论：rebalance 调整已启动工作的运行。
为什么：若暂停或抢占由 Speculation Coordinator 的固定排序规则决定，就会把运行策略固化进 Mechanism；将其作为独立 Policy Decision，既保留策略控制，也避免扩大 `select` 的职责。

## D-045：端用户只操作当前权威执行主线

结论：用户操作权威主线，候选只读观察。
为什么：端用户面对的是一个连续 Agent，而不是多条候选对话；只读观察让用户能够确认投机是否真实执行及如何处置，而不改变权威路径、Branch 生命周期和输入传播规则。

## D-046：Proposal 前提变更时剪枝

结论：前提变化时剪掉依赖分支，不修复。
为什么：前提变化后，既有投机进展不再对应当前 Proposal。现阶段缺少足以证明安全复用的兼容与修复语义，保守剪枝可以保持信息流和提交正确性。

## D-047：Branch 独占 Environment，Runtime 内部组合 Backend instance

结论：分支独占环境，后端资源内部组合。
为什么：控制面必须能明确审计 Branch 的资源归属并避免多个 Branch 互相污染；同时 Backend instance 的共享/独占策略属于 Runtime 内部，不应泄漏成控制面资源选择。

## 相关文档

- [设计模型](../design-model.md)：抽象概念层
- [Branch 语义设计](branch-semantics.md)：Branch 与生命周期语义
- [效果管理设计](effect-management.md)：效果与状态
- [Policy Decisions 设计](policy-decisions.md)：Policy 决策面
- [Speculation Contract 设计](speculation-contract.md)：契约语义
- [Agent 接入侧设计](agent-integration.md)：Agent 接入组件与受控路径
- [控制面设计](control-plane.md)：控制面组件与实现

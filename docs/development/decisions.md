# 开发决策

本文记录已接受的实现架构与验证决策。目标语义归 `docs/design/` 所有。

## 2026-10-25：TUI 与实际投机过程观察优先

先实现稳定的 AScoutAgent TUI，优先复用 Pi 原有交互体验，并增加独立的只读候选观察。先使用简单环境，不把接通完整 Runtime Adapter 环境管理作为前置条件；不据此声称已实现完整环境隔离。实际启动、执行、预测比较、Policy 处置与清理必须有事实依据，不能用两个独立模型请求或发送命令代替完成证据。当前尚未实现该 TUI 与真实等待点续跑路径；现有开发入口不满足本轮验收要求。

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

初期使用一个 `ascout` 库，契约类型放在内部 `contract` 模块，不单独拆分 `ascout-contract` crate。出现独立 Adapter 或产品接入的实际依赖需求后，再讨论拆分；契约类型不反向依赖控制面实现。

早期 `tests/test_outcome.rs` 及其确定性替代实现已经删除，不再作为当前正确性证据。当前 Runtime Adapter 端到端测试直接启动真实 Firecracker，验证 microVM 建立、guest 内操作、物理暂停与恢复、终止与清理、请求重试去重、在飞操作打断、意外停止事件，以及 snapshot/restore 分支和 sibling 隔离；SSH 诊断同样使用真实 virtio-net 与 guest sshd。尚未实现 task 身份校验、全局唯一 Proposal identity、九类完整决策、待选集合、fallback、嵌套树、效果处理和主动环境验证。当前 Rust 接口仍是实验接口，不是稳定对外契约。

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

默认 Runtime 面向可信 Child Agent 与工具，优先低开销环境隔离，不提供 adversarial containment。它必须强制维护隔离环境、环境级资源边界、环境内受控操作、停止和 cleanup。Agent 执行实体、Branch context、operation 归属和 capability 路由由 Agent 接入侧管理；受管 external effect 由控制面内的 Effect Coordinator 负责实际处理。

旧的“Runtime 直接维护 Branch identity/capability、完整 Agent 执行实体和所有 effect obligation”划分已废弃；它把投机执行职责固化进通用 Runtime，违背 Runtime 只提供环境能力的边界。

需要 adversarial containment 时，用户选择另行适配的 sandbox/hardened Runtime。AScout 不协商 security guarantee，默认 backend 技术尚未选择。

## D-017-I：新 Rust 实现不继承旧代码

新实现继续使用 Rust。旧 `src/**`、Cargo manifests、tests、examples、C header、构建产物和实验文件已经删除，未读取、迁移或复用。新实现只能依据已接受的设计与开发决策建立。

## D-024-I：实现组织服从 AScout 可控边界

完整调用路径仍为：

1. Agent 接入侧；
2. 控制面；
3. Runtime Adapter；
4. Runtime。

这四侧不再直接作为 AScout 源码或子文档的一一对应拓扑。Agent Control Gateway、Policy Engine、Speculation Coordinator、Effect Coordinator 与 Ledger 位于控制面内部；Interposer、ASC-Proxy 及 Product Integration 位于 Agent 接入职责。控制面定义 Runtime Adapter 的接入边界，但 Adapter 作者自行负责目标 Runtime 的能力映射、transport 与事件规范化。Runtime 是外部后端，不属于 AScout 可定义的实现模块。

Execution、Workspace、Input、Effect、Outcome 与 Interposer 的职责语义不绑定具体 crate、模块、进程或服务边界。

后续 crate、模块、进程和服务边界必须依据责任、依赖方向、权限、替换频率与部署需求决定，不能由“一侧一模块”或“一组件一 crate”直接推出。具体 Adapter transport、discovery、连接生命周期和 Runtime backend 属于 Adapter/部署实现选择。

**废弃记录**：原决定要求“实现组织服从四侧一级架构”，并把 Runtime backend 视为 AScout 待设计的实现部分。该解释已废弃；四侧现在只表示完整调用路径中的逻辑位置。

## D-025-I：项目名称统一为 AScout

2026-08 决定：项目现行名称统一为 **AScout**（Agent Scout），相关现行复合名称使用 `AScoutAgent`。正式设计、架构、使用和开发文档不再使用 `ASCout` 作为现行名称；本条仅保留旧拼写用于历史追溯。该名称变更不改变既有生命周期、Policy、Runtime 或接口语义。

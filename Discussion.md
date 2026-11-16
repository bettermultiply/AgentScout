# 待讨论事项

本文件只保留未决问题、必要前提与已收敛内容的入口。已确定的目标语义归对应设计文档，实现状态和验证证据归开发文档；选择理由和废弃历史归决策记录。已确定但尚未完成的能力标为实现或验证缺口，不重新当作设计问题。

讨论约定：一次只收敛一个问题，同时展开不超过三件；以下清单是排队记录，不表示同时启动。2026-11-13 用户批准按上述原则整理，并要求列出全部未决问题。本次任务不使用 subagent。

使用偏好：2026-11-10 记录 Sol 使用 mid（实际 variant 为 `medium`），不要 max；只有获准使用 subagent 时适用。

## 未决问题清单

2026-11-16 纯 FS 正常装配已修复，本地预检后唯一真实 FS 短 session 成功；覆盖 Authority 四步、候选隔离只读执行、自然 Discard、正常物化与清理。5 Candidate／5 Discard／0 Promote，晋级后续跑、候选写入发布和恢复仍未在本次覆盖，纯 FS 入口持久回归仍待补齐。旧 FS 脚本使用 NLE inner，不代替纯 FS 证据。原始记录与费用边界见 [参考负载](docs/development/reference-workload.md) 和 [测试讨论](docs/development/test-discussion.md)，不再将入口列为当前阻塞。

2026-11-16 Environment 身份边界已确定并落到实现：Runtime 创建并返回身份，控制面保存 Branch 独占绑定；创建前允许无绑定，恢复沿用保存身份并显式 load。正式语义见 [Runtime Adapter 契约](docs/design/runtime-adapter-contract.md#31-身份与关联)，实现及验证范围见 [Runtime 开发讨论](docs/development/runtime-discussion.md) 和 [恢复](docs/development/recovery.md)，不再作为待决问题。

2026-11-13 通用回放剩余范围：完整 Runtime 留档、初始环境自动重建、非文本用户输入、长历史同键碰撞和取消时序仍未保证。已实现范围与本轮证据见 [回放](docs/development/replay.md)，不再将正常模型切换、模型请求失败保留会话列为未实现。

用户已检查并接受当前直接构造连接画面；极端参数尚未穷举，验证范围见连接状态文档。

编号仅用于本清单定位，不是设计决策编号。“待复核”表示旧讨论与后续文档或代码已有变化，尚不能直接判为完成或继续沿用旧要求。

| 编号 | 主题 | 仍需处理的问题 | 状态与出处 |
|---|---|---|---|
| 6 | v0.1.0 文件样例 | 选择真实文件操作任务及验收用例，验证候选写入隔离、Promote 后继续、Discard 清理和派生进程覆盖。Overlay 方向及首版 Backend 已有，不再列作未选型。 | 待细化验收；[路线图](docs/development/roadmap.md)、[FS 现状](docs/fs-backend/current-state.md) |
| 7 | v0.1.0 NLE 展示 | 完整样例交付、地图与候选处置展示、成本统计指标和验收范围。真实 NLE 操作及跨 Promote 地图已有实现，不能描述成从零接入。 | 待细化验收；[路线图](docs/development/roadmap.md)、[地图](docs/development/nle-map.md) |
| 8 | 原型闭环验收 | 剩余为候选数量与嵌套的端到端覆盖、跨组件取消／超时及可重复演示。基础接受／丢弃、失败清理、迟到／重复消息和嵌套失效已有控制面回归，不重复列为未实现。 | 局部回归通过、剩余整体验收；[参考负载](docs/development/reference-workload.md) |
| 9 | v0.2.0 Policy 与效果 | Policy 作者接口、表示、编译和执行方式重设计；Effect Coordinator 支持的效果模式、Runtime 与组合／嵌套场景。 | 方向已定、具体方案待定；[路线图](docs/development/roadmap.md) |
| 10 | v0.3.0 验证 | 静态分析覆盖、动态检查点、能力声明实际验证与正确性回放的验收范围；回放不证明性能收益。 | 待设计；[路线图](docs/development/roadmap.md) |
| 11 | v0.4.0 论文实验 | 论文及场景选择、策略适配与必要模型调整、正确性和可复现实验基线；论文数量未定。 | 待设计；[路线图](docs/development/roadmap.md) |
| 12 | v0.5.0 发布边界 | 效率指标、稳定性与故障恢复程度、目标 coding Agent 接入完整性、支持平台／Runtime／效果模式和文档清单。 | 待细化；不预设完整恢复或第二个 Agent 产品；[路线图](docs/development/roadmap.md) |
| 13 | 源码责任边界 | 已有 `src/control_plane/`、`src/protocol/`、`src/runtime/`；仍需审查公开接口与依赖方向是否符合设计，不再讨论“是否开始归组”。 | 待复核；[架构](docs/architecture.md)、`src/lib.rs` |
| 14 | Gateway 与 UI 部署 | Gateway 的传输方式（进程内、IPC、网络）、UI 组件独立部署与否、启动协调归属。 | 通信范围与职责已定；部署与启动协调待定；[控制面](docs/design/control-plane.md) |
| 15 | Adapter 部署与配置 | 独立二进制不是语义要求；配置归属、环境网络参数是否进入 Adapter 配置仍需明确。 | 暂停讨论；[Runtime 接入](docs/development/runtime-adapter.md) |
| 16 | Runtime 按需组合 | 已有显式 Backend 路由、组合与 BaseRuntime 进程／数据面装配；剩余为按隔离需求动态取得和管理 Backend，以及逐层 Runtime 关系的完整语义。 | 部分实现、剩余设计；[Runtime](docs/design/runtime.md)、[Runtime 语义](docs/development/default-runtime-semantics.md) |
| 17 | Runtime 能力声明与验证 | 派生范围最低要求、延续能力在 Runtime／Agent 声明中的表达、Registry 首期对应检查和主动验证；模型服务业务拒绝是否构成验收通过。 | 待讨论；[Runtime 接入](docs/development/runtime-adapter.md) |
| 18 | Authority 物化 | 独立暂停入口的具体 UI 及 Ctrl-C 是否关闭整个程序待定；候选环境承接、终止／暂停入口对齐、task 完成信号、Runtime 物化回报及控制面成功报告仍有实现核查与验证缺口。候选继续使用自己的环境、Agent 常用 Ctrl-C／退出终止任务、AScout 独立 `/pause` 保留树和候选已确定；暂停不意味着退出程序，继续入口命令名称未定。属性变更首版仅支持权限位，其他属性不新增修改支持，未改属性及链接结构保留仍需实现验证，详见提案。 | 目标已定部分仍未实现；[物化提案](docs/fs-backend/materialize-proposal.md)、[恢复现状](docs/development/recovery.md) |
| 19 | Runtime 嵌套与压平 | 父候选 Promote 后，保留后代进展并调整底层快照／写时复制层引用；先查 Firecracker 派生方式，再明确何时适合压平及事实来源。 | 暂缓；触发于需要保留后代进展的深度嵌套 Promote；[Policy 实现](docs/development/policy-engine.md)、[FS 现状](docs/fs-backend/current-state.md) |
| 20 | FS 执行覆盖 | 剩余为嵌套挂载、进程组后备冻结路径对逃逸后代的覆盖及真实环境验证。BaseRuntime 已编排 FS fork／mark／accept 的冻结、操作、解冻；cgroup 冻结路径应单独验收，不能套用进程组限制。 | 编排回归通过、剩余实现／环境验证；[FS 现状](docs/fs-backend/current-state.md)、[Runtime 接入](docs/development/runtime-adapter.md) |
| 21 | 原子转换与失败 | Execution、Workspace、Input、Effect、Outcome 的可观察原子性、可见性、失败收敛及 cleanup result；不规定 Runtime 内部算法。 | 待细化契约；[Runtime 契约](docs/design/runtime-adapter-contract.md) |
| 22 | Control session 与 Workspace 身份 | 已有请求身份、会话内去重与有界结果缓存、等待超时、环境队列及断线 shutdown；剩余为跨重启幂等／恢复契约、完整事件顺序及 Workspace revision／object identity 与并发写线性化。 | 部分实现、剩余契约；[Runtime 生命周期](docs/development/runtime-discussion.md)、[Runtime 契约](docs/design/runtime-adapter-contract.md) |
| 23 | Branch 执行上下文 | 已有 generation、执行位置、Chance 归属、假设集、未决操作和执行状态，Chance 保存权威结果与证据；剩余为能力／约束及 Runtime 对齐契约是否充分的审查。 | 基础状态已实现、剩余契约复核；[Branch 语义](docs/design/branch-semantics.md) |
| 24 | 信息与约束传播 | 用户／parent 向 descendants 的传播、child 向 parent 的 proposal／outcome summary，以及 sibling 不直接协作的边界。 | 排队；[Branch 语义](docs/design/branch-semantics.md) |
| 25 | Resolution 与策略组合 | winner、旧 parent 和接受前提的关系；accept 选择后续有效执行，不等于宣布候选是客观真值；Policy、自定义操作与 Adapter 证明责任的组合边界。 | 待对照现有契约收敛剩余细节；[Branch 语义](docs/design/branch-semantics.md)、[Policy Decisions](docs/design/policy-decisions.md) |
| 26 | Chance 模型边界 | 流式输入尚未产完时如何部分消费；L3／Bypass 无真实输入时的措辞；sibling command reuse 与 Chance 的关系。 | 已记录、尚未展开；[设计模型](docs/design-model.md) |
| 27 | Agent 接入完整性 | 非标准上下文复制／恢复、复杂和多段结果及元数据转换、并行工具等待点专项验证、sub-agent 启动前 Hook。 | 实现／验证缺口；[Agent 接入](docs/development/agent-access.md)、[参考负载](docs/development/reference-workload.md) |
| 28 | Policy 原型差距 | 草案字段、输出声明、能力授权、领域 schema、Contract／Fidelity、增量预测、score、Reuse／Salvage、执行隔离和取消等剩余能力的范围及落实。rebalance 已按 epoch 调度并接通暂停／恢复／抢占，剩余扩展须明确具体缺口。 | 部分实现、剩余待审；以[Policy 实现](docs/development/policy-engine.md)为当前接口依据 |
| 29 | UI 交互完整性 | 已有 textarea 多行输入、Shift+Enter 换行和文字／思考／工具调用／结果基础展示；剩余为 Markdown、图片、产品特有交互及远程隔离环境展示的交付范围。 | 基础交互已实现、剩余范围待定；[心智模型](docs/mental-model.md)、[参考负载](docs/development/reference-workload.md) |
| 30 | 退出与接受竞态 | 启动握手阶段信号、Runtime 故障注入、独立环境端到端退出；Promote 在途时 Parent 开始下一操作、Agent 确认前候选取消。 | 验证缺口；已有 Shutdown／SIGINT／SIGTERM 验证不重复列为未实现；[参考负载](docs/development/reference-workload.md) |
| 31 | 历史与恢复 | Pi 原生完整持久化与单一最终 session 汇总、compaction/custom 引用与摘要处理、在飞推理协调及 rollback 外部资源关联；Ledger 历史查询索引、活动状态重建与恢复工具。 | 目标已定、实现差距；不承诺原始运行与整理后恢复上下文相同；[接入实现](docs/development/agent-access.md) §4、[回放](docs/development/replay.md) |
| 32 | 回放匹配可靠性 | 原生 session/events 导出还原已实现；长分支同键碰撞、并发／重试顺序、取消请求时序及最终回答交付仍待验证。缺失／耗尽报错且不得回退真实模型已确定；命中／消费不等于回答交付。 | 局部回归已有覆盖，不代表完整机制整链通过；[回放](docs/development/replay.md) |
| 33 | 嵌套端到端覆盖 | 活跃祖先 Candidate 被 Discard 后的嵌套 Chance 端到端执行、自然重复键与消费顺序；嵌套失效已有单元回归，不能替代端到端验证。 | 剩余端到端验证；[回放](docs/development/replay.md) |
| 34 | NLE 可复现性 | 独立随机 lgen 回归已修复；剩余长轨迹／并发／重试及完整 RNG、日期策略与初始环境复现范围。 | disp=11/22/99 仅核实短轨迹无差异；[回放](docs/development/replay.md) |
| 35 | 观察与统计 | 未命中造成的权威额外延迟、校验耗时与模型投机生产路径；长运行折叠／内存成本、任务列表及通用事件级回放界面。树布局缓存已有实现，剩余细粒度复用和长运行性能验证见 WebUI。 | 数据／实现缺口；[观察](docs/development/observation.md)、[WebUI](docs/development/web-ui.md#性能目标与实现差距2026-11-14) |
| 36 | 后续执行承载与资源身份 | 已有进程 PID／进程组／cgroup 身份、Branch generation 和取消／清理路径；剩余为 continuation branching、效果插件、完整资源租约与跨重启孤儿清理、输入边界及持久化。pidfd 是否需要仍属选型问题。 | 基础实现已有、扩展暂停讨论；[Runtime](docs/design/runtime.md) |
| 37 | 决策文档整理 | 两套设计记录的重复编号与引用、开发决策的标题层级和历史状态、runtime-discussion 文件名与内容职责；正式文档与历史记录的冲突需逐条核对。 | 本次完成 Discussion 清理后的后续项，尚未批量重写决策文档 |

### 必要前提

- Runtime 内部整理的判断权已归 Runtime。第 19 项早期“控制面决定压平”的提议未定案；其与第 18 项边界的适用关系必须先核对，不能把旧提议当作实施授权。
- 单 winner、无通用 merge、同级默认不协作、候选只读观察、端用户只操作权威主线均已确定。未来 DAG／多 winner／协作属于执行模型扩展，不是本清单默认目标。
- Agent 接入侧、Interposer、ASC-Daemon、端侧管理节点与 Instance 的职责已有正式文档；原“暂停讨论”中仅重复这些结论的条目不再占用未决清单。Agent 主动控制接口仍不在当前范围。
- 原“独立树形原型”讨论已转向以仓库 WebUI 为准；投机起点与父枝验证点的实现和验证限制见[观察](docs/development/observation.md)与 [WebUI](docs/development/web-ui.md#源侧起点与父枝截止2026-11-14)，布局性能与复用剩余验证由第 35 项承接。
- 本清单汇总原 Discussion 未决事项及其直接引用文档中的相关缺口，不把所有远期设想自动提升为当前交付要求。文档中的历史验收结果不是本轮重新运行的结果。

## 已收敛内容入口

2026-11-16 Promote 后等待问题已定位并最小修复：Promoting 不得让同 Branch 的在飞模型等待点失效；Rust 状态回归与真实 Pi→Runtime→NLE／本地伪模型五步回归先红后绿。唯一一次真实补验成功完成 5 个实际动作、Promote 后 3 次后续工具及完整最终回复，6 个 Branch 的独立 Runtime ID 和清理已核对；原失败证据保留，统计及边界见 [参考负载](docs/development/reference-workload.md#promote-在途模型等待点修复2026-11-16)。不扩展为通用嵌套／取消竞态已经全部验收。

2026-11-14 本机构建与使用已收敛：在本机运行控制面，用本机浏览器访问；根目录 `make` 为统一构建入口，先准备前端依赖并构建，再构建 Rust，`make -j` 同样保证顺序与失败停止。前端产物继续嵌入二进制且不入库；规则与验证见 [WebUI](docs/development/web-ui.md#生产构建)。Gateway 部署与启动协调仍由第 14 项保留。

2026-11-14 Gateway 共识入口：Gateway 统一承接 Agent 接入侧、UI 与 Runtime 的通信接入、消息分发和送达，并负责连接／会话管理及入口格式、身份、上下文与权限检查；消息对投机执行的变化由相应控制逻辑决定。Gateway 不拥有 Tree、Policy 或投机编排；启动实际由 Runtime 负责，启动协调归属未决。Registry 属于 Runtime，保留主动验证与独立证据职责。第 14 项仅保留 transport、UI 部署和启动协调问题。依据见[控制面 §2.1](docs/design/control-plane.md#21-gateway)、[Runtime Registry 职责](docs/design/runtime.md#adapter-registry)。

Gateway 实现补记：三侧通信入口已归入 `src/control_plane/gateway/`，保持 Unix socket、HTTP/SSE、现有进程部署和投机语义。类型化分发不拥有 Tree／Policy／Runtime 执行状态。既有 Rust、Pi 通信和恢复测试通过；完整 NLE 回放尝试仍失败，未声称整链验收完成。通用认证授权、Runtime 断线立即唤醒及启动阶段资源清理的剩余范围见 [Gateway 实现核查](docs/development/gateway.md)，部署与启动协调继续由第 14 项保留。

2026-11-14 候选环境及终止／暂停已收敛：接受后由候选环境承接后续执行，嵌套接受不越过祖先约束；Agent 常用 Ctrl-C／退出终止任务，AScout 以独立于原 Agent 的 `/pause` 入口暂停并保留树、候选和环境，暂停不意味着退出程序。目标见 [Runtime](docs/design/runtime.md)、[Agent 接入](docs/design/agent-integration.md)；现有退出保存入口调整及物化仍为实现缺口，保存恢复能力保留，见 [恢复](docs/development/recovery.md)。选择理由与废弃表述归[开发决策](docs/development/decisions.md)。

2026-11-16 Environment 归属已收敛：控制面独占 `Branch -> Environment`；Runtime 不接收 `branch_id`，内部用独立 Backend instance ID 维护 Environment 与 Backend 的多对多关联。实现与验证见 [Runtime 生命周期协议](docs/development/runtime-discussion.md)；不在本清单重复作为待讨论选型。

session／task 与物化职责已收敛：task 是 Agent 完成本轮工作并开始等待用户输入时结束的一轮工作；一个 session 对应一个 AScoutAgent 和一棵树。Runtime 报告物化完成／失败，控制面等待物化成功后宣告任务成功。目标见 [心智模型](docs/mental-model.md) 与 [物化提案](docs/fs-backend/materialize-proposal.md)；代码待核查，物化剩余事项归第 18 项。

首版范围已收敛：同一 session 加载的 Policy 固定，多轮 task 使用同一份 Policy，不实现修改或切换；长期修改能力保留，其切换规则留待后续，不再作为首版待定项。属性变更首版仅支持权限位，其他属性不新增修改支持，不允许物化破坏未改属性或链接结构；其他属性扩展范围不阻塞首版。能力限制见 [Policy 开发说明](docs/development/policy-engine.md) 与 [物化提案](docs/fs-backend/materialize-proposal.md)，选择理由及历史归[开发决策](docs/development/decisions.md)。

2026-11-14 Policy 目录整理已确定：每份策略独立拥有声明与算法，当前策略暂不 Git 跟踪；本地布局、配置限制与交付前提见 [Policy 实现](docs/development/policy-engine.md)，废弃路径归开发决策。

2026-11-13 通用回放收敛：用户重新批准 subagent 执行，主 agent 审查。指定源 run 每次现读原生 session/events，在内存提供旧回复；使用正常新 run 实际执行 Runtime/Policy，允许正常 session/events/日志/恢复资料持久化，禁止额外 recording 缓存或源记录副本。按真实 user 消息消费顺序自动送入，custom 通知不算用户；源 Policy artifact 优先，缺失原配置显式提供，不猜测试目录布局。完成或缺键后保留系统，通过普通模型切换继续。多输入/真实 bash/切换后继续/缺键保留/正常恢复均用本地伪模型实测。源码、命令、资料和限制归 [回放](docs/development/replay.md)，废弃历史归开发决策；用户独立树形改动不在本次提交范围。

2026-11-13 Policy 留档已收敛并实施：正常运行在 run 内原生保存完整 `policy.json` artifact，包含实际同次加载的原 YAML、全部声明源码/资源、编译绑定及原来源；恢复目录优先，缺失允许显式补充且核对原 digest，不回退默认策略、不覆盖已有资料。Runtime 引用保留原 cwd 解析关系，完整 Runtime 保存和交互回放不在本轮。实现、失败边界及无真实模型验证见 [Policy 实现](docs/development/policy-engine.md)；不新增项目概念、回放 session/events 字段或动态依赖保证。

叶茎完成位置、宽度及同侧 placement 的已定规则见[参数](docs/development/tree-parameters.md)；连接实现与本轮验证见[连接状态](docs/development/tree-connection-repair.md)。

通用比较与 NLE 修复事实：按键集合及每键完整回复队列比较，保留同键顺序；Policy 随机子集按配置顺序输出；NLE 使用 lgen=None，保留保存／恢复检查。现有小回归保留，短轨迹结果不构成长轨迹、完整环境或机制确定性的保证。依据见 [回放](docs/development/replay.md)。

2026-11-13 记录回放：记录是系统原生能力，测试套件不执行录制；已有 session.jsonl/events.jsonl 先导出，再回放核对。导出仅用原生 ID、Candidate/Chance/applied Promote 还原预测批次之后第一条记录的父边，处理并行／嵌套及 Pi compaction/custom/context_edit，不读取遗留专用键，不改运行期或源记录，不加旧格式兼容。B1、已有 B1-PI 的导出→真实 Pi provider 回放闭环及手构造还原回归通过；真实 NLE 单层／四层整链因新工具结果与源记录不同而缺键失败，严格断言保留，不声称完成整链验收。当前证据和边界归 [回放](docs/development/replay.md)。

根枝起点提前量规则及零首段方向回退已实现，见[参数](docs/development/tree-parameters.md)。

2026-11-13 主动退出恢复本轮收敛：FS + Pi 真实主入口、独立 lower、保存失败暂停重试与旧 manifest 不变、加载中途失败资源清理、package/glob 扩展变化拒绝、独立 rollback 保留 ref 及 Runtime 参与环境集合核对已实现并验证；未调用真实模型。snapshot-copy 与普通 overlay 的恢复后 fork／再次保存通过。原生 btrfs incremental backend 因本机探测不可用仍未验收；系统版本绑定、外部依赖覆盖与失败目录残留等精确限制归 [恢复](docs/development/recovery.md)，不再将上述已完成整链列为待办，不扩展为崩溃恢复承诺。

2026-11-13 树枝几何参数决定：子枝夹角范围默认 30–90°，以 `${id}/childAngle` 稳定采样；两个界限各范围 5–90°、步长 1°，交换界限不改变结果。优先按稳定偏向选侧，初始绝对方向超 90° 时选另一侧，水平允许。父枝宽度在条目位置取剩余工作量对应的值，条目之间用 smoothstep 平滑过渡；子枝根宽公式不变。

2026-11-13 实现核查：本轮依据当前源码缩小第 8、16、20、22、23、28、29、33、36 项范围；保留编号便于对照。现有控制面测试 22 项、Runtime 组合与冻结编排测试 13 项、Pi／地图／树形测试 14 项通过；未调用真实模型，未运行特权环境或深度回放验收。完整证据见 [实现核查](docs/development/implementation-audit-2026-11-13.md)。这些主题仍有剩余事项，不能按主题数量统计已完成功能数。

2026-11-14 根部连接收敛：用户已接受当前画面，完成小范围代码和文档清理；当前直接构造与验证限制见[连接状态](docs/development/tree-connection-repair.md)。既有默认值、范围和锚点规则保持，见[参数](docs/development/tree-parameters.md)；废弃记录归开发决策。

| 主题 | 当前依据 |
|---|---|
| 全局模型、分层、对象与术语 | [心智模型](docs/mental-model.md)、[设计模型](docs/design-model.md)、[术语表](docs/design/glossary.md) |
| Chance／Proposal／Branch、嵌套与提交资格 | [Branch 语义](docs/design/branch-semantics.md)、[Policy Decisions](docs/design/policy-decisions.md)、[Policy 实现](docs/development/policy-engine.md) |
| Tree 状态归属、Controller、Ledger 与只读观察 | [控制面](docs/design/control-plane.md)、[观察](docs/development/observation.md)、[开发决策](docs/development/decisions.md) |
| Agent 通用接入层、Pi、action 事实与模型用量 | [Agent 接入设计](docs/design/agent-integration.md)、[Agent 接入实现](docs/development/agent-access.md) |
| Runtime 契约、组合及生命周期 | [Runtime 契约](docs/design/runtime-adapter-contract.md)、[接入指南](docs/development/runtime-adapter.md)、[生命周期](docs/development/runtime-discussion.md) |
| FS 布局、状态目录、Firecracker 清理与物化边界 | [FS 现状](docs/fs-backend/current-state.md)、[特权检查](docs/fs-backend/priv-check.md)、[物化提案](docs/fs-backend/materialize-proposal.md) |
| 权威窗口、投机树、调参、面板调宽及验证证据 | [WebUI](docs/development/web-ui.md)、[参数](docs/development/tree-parameters.md) |
| NLE 真实闭环、退出及跨 Promote 地图 | [参考负载](docs/development/reference-workload.md)、[地图](docs/development/nle-map.md) |
| 权威 session 文件、消息引用与系统外回放 | [回放](docs/development/replay.md)、[工具说明](tools/replay/README.md) |
| 版本目标 | [路线图](docs/development/roadmap.md) |
| 选择理由与历史方案 | [设计决策](docs/design/decisions.md)、[底座设计决策](docs/design/speculation-decisions.md)、[开发决策](docs/development/decisions.md) |

2026-11-13 整理说明：已移除已下沉结论的重复全文、被后续记录取代的实施描述和逐轮验收日志；原讨论中的树形与 PTY 废弃方案补记于开发决策。源码归组依据 `src/lib.rs` 核实；版本目标依据 roadmap 核实。其余能力状态按链接文档保留限定，不宣称本轮重新验收。

## 2026-11-13 session 与状态交接讨论收敛

用户已批准：正确状态交接由机制保证；最终单一权威 session 是经过验证、用真实结果整理后的历史，保留预测输入，将预测输入后第一条后续记录改接真实结果 ID，保留原主／丢弃历史，不额外强制另存完整投机后续路径。真实结果通知由 Policy 决定，默认完全不追加；不新增 Policy 名称或类别。原始运行与整理后恢复上下文可能不同。

目标语义归 [Agent 接入设计](docs/design/agent-integration.md) §3、[Branch 语义](docs/design/branch-semantics.md)；Pi 原生完整持久化、多 session 过渡、接入侧汇总及临时文件删除条件归 [接入实现](docs/development/agent-access.md) §4。compaction/custom 必须保存；其引用／摘要、在飞推理、rollback 外部资源关联仍有开发差距。系统外导出已接通历史重挂还原，验证边界见回放文档。FS 共同快照差异合入 winner 仅为讨论方向。

### 保留的待议事项

2026-11-13 已收敛：既有 dispose／Promote 承载可选 `notification`；本轮用户明确回复“同意”，批准保留 `dispose.context.authority_outcome`，向 Policy 提供对应 Chance／operation 的真实结果，不自动给模型追加消息；保留 Match evidence 与 `context.match`。此前将该字段写成“用户同意”缺乏独立批准依据，不据本轮批准追认早先已确认。Pi 原生交付、缺省零记录、持久化及去重边界见 [接入设计](docs/design/agent-integration.md) §3，实现与验证缺口见 [接入实现](docs/development/agent-access.md) §4.1。compaction 摘要保留原文，不阻塞汇总或满足其他条件后的源清理；移除这两项待议问题，不新增 FS backend 讨论。

2026-11-13 独立测试审查：补充真实 main.ts 协议入口的通知 cleanup／EOF、完整命令重传及写失败验证，9 项新增、联合 Pi 共 24 项通过；未复现生产代码缺陷。验证边界见接入实现 §4.1。不改 display:false，不新增恢复能力。上述为既有审查记录，本轮文档收尾不重跑测试。批准依据已按本轮明确回复收敛，不再保留核对早先批准的待确认项，决策记录见 [开发决策](docs/development/decisions.md)。

- **多步投机的可行性**：区分“候选执行多步”与“一个 Proposal 由多步组成”；不因历史整理方案确认就宣布二者均已支持。
- **脱离 AgentScout 后继续原任务**：仍需证明工具、环境与外部资源能够完整交接；不以单一最终 session 替代验证。相关实现差距见第 31 项。
- **events 与全系统恢复日志**：首版范围按下方“主动退出恢复”记录；events 的恢复用途与保存技术方案仍待定。

## 保存与恢复实现入口

整棵有效树、原上下文与候选环境的保存恢复能力继续保留；当前 Shutdown／退出信号的实际行为、验证证据及其与终止／暂停目标的实现差距集中在 [恢复](docs/development/recovery.md)。既有能力不代表物化完成；入口语义已确定，剩余具体接口与实现事项归第 18 项。此前退出保留候选的目标记录归[开发决策](docs/development/decisions.md)。

## 2026-11-13 实现核查补记

- Pi 原生 session 分叉、compaction/custom 保留、原生 ID、checkpoint 分叉和单一最终文件整理已接通并由伪模型检查；源文件删除仍只发生在成功整理后，活跃 winner 不删除。
- 回放仅使用原生 session/events；正常运行不计算或保存回放专用键，也不依赖 tools/replay。键只在导出产物中生成。历史重挂按原生记录关系在导出内存还原；套件从已有 run 导出开始，真实 NLE 整链本轮因工具内容变化缺键失败。废弃方案归开发决策。
- `pi-code-rollback` 的快照关联仍是缺口：必须继续传递并验证 `targetId`、ref、repoRoot 与 Git 对象，session 文件复制不构成支持；本轮未对用户仓库执行恢复。
- 并行工具配对及在飞推理与外部状态交接仍需专门验收；compaction 摘要按已确认规则保留原文。
# Replay 测试套件的职责

用户确认：replay 套件的功能仅限于从已有运行中读取模型响应，并按顺序提供给被测系统；它本身不负责评估投机策略或机制。严格说，它提供的是顺序响应，而非恢复或重演原运行的全部状态。

套件本身的正确性要求是：在投机逻辑和策略不变的前提下，顺序提供的响应应使执行逻辑保持一致。借助该套件编写的测试，进一步检查投机执行、Promote／Discard、隔离环境等机制是否正确。后续需继续讨论“执行逻辑一致”的具体判定范围与依据。

2026-11-13 本轮共识补记：Replay 测试套件仅从已有运行读取模型响应，并按顺序向被测系统提供响应；只保证套件本身的正确性。上段所述“执行逻辑一致”是用户对回放的要求：在投机逻辑与策略不变的前提下，回放执行逻辑应保持一致。借助此套件编写的测试检查投机执行、Promote／Discard、隔离环境等机制是否正确。未决事项为“执行逻辑一致”的判定范围，以及所需的环境和时序前提；顺序响应的具体交付规则尚未讨论，不擅自确定为全局 FIFO。

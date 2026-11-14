# 待讨论事项

本文件只保留未决问题、必要前提与已收敛内容的入口。已确定的目标语义归对应设计文档，实现状态和验证证据归开发文档；选择理由和废弃历史归决策记录。已确定但尚未完成的能力标为实现或验证缺口，不重新当作设计问题。

讨论约定：一次只收敛一个问题，同时展开不超过三件；以下清单是排队记录，不表示同时启动。2026-11-13 用户批准按上述原则整理，并要求列出全部未决问题。本次任务不使用 subagent。

使用偏好：2026-11-10 记录 Sol 使用 mid（实际 variant 为 `medium`），不要 max；只有获准使用 subagent 时适用。

## 未决问题清单

2026-11-13 通用回放剩余范围：完整 Runtime 留档、初始环境自动重建、非文本用户输入、长历史同键碰撞和取消时序仍未保证。已实现范围与本轮证据见 [回放](docs/development/replay.md)，不再将正常模型切换、模型请求失败保留会话列为未实现。

是否需要通过 `build.rs` 或脚本自动构建前端，避免首次克隆后直接运行 `cargo build` 因缺少嵌入文件而失败（未定，本轮不实施）？

末端是否也需提前量仍待讨论，只记录，不在本轮扩展讨论或实施。根枝起点提前量的决定与实现见参数文档。

用户已检查并接受当前直接构造连接画面；极端参数尚未穷举，验证范围见连接状态文档。

编号仅用于本清单定位，不是设计决策编号。“待复核”表示旧讨论与后续文档或代码已有变化，尚不能直接判为完成或继续沿用旧要求。

| 编号 | 主题 | 仍需处理的问题 | 状态与出处 |
|---|---|---|---|
| 1 | 投机树形状选项 | 枝端和叶片形状是否可选；叶长是否继续按参考时长归一化。 | 待讨论；[参数](docs/development/tree-parameters.md) |
| 2 | 投机树方向与重叠 | 转角、回中尺度及整体形态是否满足要求；是否需要面积避让及整树左右配平。现有方向概率不等于面积避让。 | 待观察、待定范围；[WebUI](docs/development/web-ui.md)、[参数](docs/development/tree-parameters.md) |
| 3 | 投机开始时间 | Chance 分叉应表示投机开始；目前创建记录时刻不能冒充源侧准入时刻，无候选 Chance 缺少源侧字段。 | 已定目标、数据缺口；[观察](docs/development/observation.md)、[WebUI](docs/development/web-ui.md) |
| 4 | 父枝验证点截止 | 子枝被采纳后，父枝保留原调用归属，按对应 Proposal 的真实验证点截止并标记端点；公共事件缺少真实验证时刻。 | 已定目标、实现受阻；[WebUI](docs/development/web-ui.md) |
| 5 | 树形验证 | 用户已接受当前画面；极端参数及完整组合尚未穷举。 | 剩余验证范围；[WebUI](docs/development/web-ui.md)、[参数](docs/development/tree-parameters.md) |
| 6 | v0.1.0 文件样例 | 选择真实文件操作任务及验收用例，验证候选写入隔离、Promote 后继续、Discard 清理和派生进程覆盖。Overlay 方向及首版 Backend 已有，不再列作未选型。 | 待细化验收；[路线图](docs/development/roadmap.md)、[FS 现状](docs/fs-backend/current-state.md) |
| 7 | v0.1.0 NLE 展示 | 完整样例交付、地图与候选处置展示、成本统计指标和验收范围。真实 NLE 操作及跨 Promote 地图已有实现，不能描述成从零接入。 | 待细化验收；[路线图](docs/development/roadmap.md)、[地图](docs/development/nle-map.md) |
| 8 | 原型闭环验收 | 剩余为候选数量与嵌套的端到端覆盖、跨组件取消／超时及可重复演示。基础接受／丢弃、失败清理、迟到／重复消息和嵌套失效已有控制面回归，不重复列为未实现。 | 局部回归通过、剩余整体验收；[参考负载](docs/development/reference-workload.md) |
| 9 | v0.2.0 Policy 与效果 | Policy 作者接口、表示、编译和执行方式重设计；Effect Coordinator 支持的效果模式、Runtime 与组合／嵌套场景。 | 方向已定、具体方案待定；[路线图](docs/development/roadmap.md) |
| 10 | v0.3.0 验证 | 静态分析覆盖、动态检查点、能力声明实际验证与正确性回放的验收范围；回放不证明性能收益。 | 待设计；[路线图](docs/development/roadmap.md) |
| 11 | v0.4.0 论文实验 | 论文及场景选择、策略适配与必要模型调整、正确性和可复现实验基线；论文数量未定。 | 待设计；[路线图](docs/development/roadmap.md) |
| 12 | v0.5.0 发布边界 | 效率指标、稳定性与故障恢复程度、目标 coding Agent 接入完整性、支持平台／Runtime／效果模式和文档清单。 | 待细化；不预设完整恢复或第二个 Agent 产品；[路线图](docs/development/roadmap.md) |
| 13 | 源码责任边界 | 已有 `src/control_plane/`、`src/protocol/`、`src/runtime/`；仍需审查公开接口与依赖方向是否符合设计，不再讨论“是否开始归组”。 | 待复核；[架构](docs/architecture.md)、`src/lib.rs` |
| 14 | Gateway 与 UI 部署 | Gateway 是否统一及其接收／传输方式（进程内、IPC、网络）；UI 组件独立部署与否。职责已确定，不以部署方式代替责任边界。 | 待定；[控制面](docs/design/control-plane.md) |
| 15 | Adapter 部署与配置 | 独立二进制不是语义要求；配置归属、环境网络参数是否进入 Adapter 配置仍需明确。 | 暂停讨论；[Runtime 接入](docs/development/runtime-adapter.md) |
| 16 | Runtime 按需组合 | 已有显式 Backend 路由、组合与 BaseRuntime 进程／数据面装配；剩余为按隔离需求动态取得和管理 Backend，以及逐层 Runtime 关系的完整语义。 | 部分实现、剩余设计；[Runtime](docs/design/runtime.md)、[Runtime 语义](docs/development/default-runtime-semantics.md) |
| 17 | Runtime 能力声明与验证 | 派生范围最低要求、延续能力在 Runtime／Agent 声明中的表达、Registry 首期对应检查和主动验证；模型服务业务拒绝是否构成验收通过。 | 待讨论；[Runtime 接入](docs/development/runtime-adapter.md) |
| 18 | Authority 物化 | 宿主写者静止、锁与并发写入协调、基准引用保护、冲突处理及保留周期的具体策略。控制面只报操作间隙，Runtime 决定内部整理；任务结束即结束 authority 使命。 | 未实现、策略待定；[物化提案](docs/fs-backend/materialize-proposal.md) |
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
| 32 | 回放匹配可靠性 | 原生 session/events 导出还原已实现；长分支同键碰撞、并发／重试顺序与取消请求时序仍待验证。缺失／耗尽报错且不得回退真实模型已确定。 | 当前基线单层 5/5、四层 3/3 严格通过，旧基线仍失败；[回放](docs/development/replay.md) |
| 33 | 深度回放覆盖 | 活跃祖先 Candidate 被 Discard 后的嵌套 Chance 端到端回放、自然重复键与消费顺序；嵌套失效已有单元回归。四层当前基线已三次严格通过，但后续 Chance 均产生于祖先 Promote 之后。 | 剩余端到端验证；[回放](docs/development/replay.md) |
| 34 | NLE 可复现性 | 独立随机 lgen 回归已修复；当前短轨迹重复验收已通过，不兼容旧日期和旧权威历史。剩余长轨迹／并发／重试及完整 RNG／环境复现范围。 | disp=11/22/99 仅核实本组短轨迹无差异；[回放](docs/development/replay.md) |
| 35 | 观察与统计 | 未命中造成的权威额外延迟、校验耗时与模型投机生产路径；长运行折叠／内存成本、缓存、任务列表及通用事件级回放界面。树布局的更新、复用与资源要求归下方“树布局性能与求交讨论”。 | 数据／实现缺口；[观察](docs/development/observation.md)、[WebUI](docs/development/web-ui.md) |
| 36 | 后续执行承载与资源身份 | 已有进程 PID／进程组／cgroup 身份、Branch generation 和取消／清理路径；剩余为 continuation branching、效果插件、完整资源租约与跨重启孤儿清理、输入边界及持久化。pidfd 是否需要仍属选型问题。 | 基础实现已有、扩展暂停讨论；[Runtime](docs/design/runtime.md) |
| 37 | 决策文档整理 | 两套设计记录的重复编号与引用、开发决策的标题层级和历史状态、runtime-discussion 文件名与内容职责；正式文档与历史记录的冲突需逐条核对。 | 本次完成 Discussion 清理后的后续项，尚未批量重写决策文档 |

### 树布局性能与求交讨论

2026-11-14，归并第 5、35 项相关讨论。用户已接受当前直接构造画面，并要求代码和文档保持整洁、降低心智负担。

- 用户已明确要求：对话内容更新与树几何更新分离，只在几何输入改变时重新布局；利用增量事件避免重复计算，进入详情后返回树时，输入未变应复用已有结果。增量事件不能等同于当前已有增量布局。
- 用户已明确要求：同时优化速度、CPU 和内存。树仅用于辅助展示，不能挤占核心系统资源；缓存应有界，并在不再需要时释放。
- 待评估方向：共享已生成的轮廓和长度，具体复用方式尚未确定。
- 已实施：实际可见父侧位置映射、有限子侧终点及两条 cubic 的直接构造；保留锚点、路径、宽度、方向和 placement。2026-11-14 小范围清理完成，七案例所有保留字段逐值一致，布局／placement 14 项及构建通过。具体公式、证据与限制见[连接状态](docs/development/tree-connection-repair.md)。用户已亲自看图接受；这不代表穷举极端参数。后续增量性能工作仍保留上述范围。

本轮研究报告：`/tmp/opencode/layout-investigation/report.md`。

### 必要前提

- Runtime 内部整理的判断权已归 Runtime。第 19 项早期“控制面决定压平”的提议未定案；其与第 18 项边界的适用关系必须先核对，不能把旧提议当作实施授权。
- 单 winner、无通用 merge、同级默认不协作、候选只读观察、端用户只操作权威主线均已确定。未来 DAG／多 winner／协作属于执行模型扩展，不是本清单默认目标。
- Agent 接入侧、Interposer、ASC-Daemon、端侧管理节点与 Instance 的职责已有正式文档；原“暂停讨论”中仅重复这些结论的条目不再占用未决清单。Agent 主动控制接口仍不在当前范围。
- 原“独立树形原型”讨论已转向以仓库 WebUI 为准，现阶段剩余事项由第 1～5 项承接。
- 本清单汇总原 Discussion 未决事项及其直接引用文档中的相关缺口，不把所有远期设想自动提升为当前交付要求。文档中的历史验收结果不是本轮重新运行的结果。

## 已收敛内容入口

2026-11-13 通用回放收敛：用户重新批准 subagent 执行，主 agent 审查。指定源 run 每次现读原生 session/events，在内存提供旧回复；使用正常新 run 实际执行 Runtime/Policy，允许正常 session/events/日志/恢复资料持久化，禁止额外 recording 缓存或源记录副本。按真实 user 消息消费顺序自动送入，custom 通知不算用户；源 Policy artifact 优先，缺失原配置显式提供，不猜测试目录布局。完成或缺键后保留系统，通过普通模型切换继续。多输入/真实 bash/切换后继续/缺键保留/正常恢复均用本地伪模型实测，NLE 单层 4/4 与四层 13/13 回复通过。源码、命令、资料和限制归 [回放](docs/development/replay.md)，废弃历史归开发决策；用户独立树形改动不在本次提交范围。

2026-11-13 Policy 留档已收敛并实施：正常运行在 run 内原生保存完整 `policy.json` artifact，包含实际同次加载的原 YAML、全部声明源码/资源、编译绑定及原来源；恢复目录优先，缺失允许显式补充且核对原 digest，不回退默认策略、不覆盖已有资料。Runtime 引用保留原 cwd 解析关系，完整 Runtime 保存和交互回放不在本轮。实现、失败边界及无真实模型验证见 [Policy 实现](docs/development/policy-engine.md)；不新增项目概念、回放 session/events 字段或动态依赖保证。

叶茎完成位置、宽度及同侧 placement 的已定规则见[参数](docs/development/tree-parameters.md)；连接实现与本轮验证见[连接状态](docs/development/tree-connection-repair.md)。

2026-11-13 当前语义回放验收收敛：使用前轮已完成的 bIAF7f/run-1 与 K0uZLH/run-1 原生记录作为新基线，不调用真实模型录制、不改旧源、不兼容旧日期／旧权威历史。临时 bundle 字节核对并记录来源；复现并修复 Policy 随机排列完整候选集导致身份交换（随机子集选择保留、按配置顺序输出），导出比较按端点实际的每键队列严格核对，保留同键顺序和完整分支／回复。两项小回归先失败后通过，单层连续 5/5、四层连续 3/3 严格通过，分别每轮 4/4、13/13 命中和消费、零缺失／耗尽／未消费；事件事实、权威正文和真实 NLE 观察均一致，进程／端口清理通过。旧基线严格失败结论仍有效；disp 对这两条短轨迹无影响不构成全游戏保证。精确命令、来源哈希、失败证据及产物见 [回放](docs/development/replay.md)。

2026-11-13 NLE 回放调查：确认 e89533d 新增独立随机 lgen 改变了 seed=7 的地图；修复为 lgen=None，保留现有保存／恢复检查。小回归先失败后通过，已有单层／四层各三次独立回放恢复全部命中和零未消费，但严格断言仍拒绝初始日期消息差异，四层还拒绝源历史预测 `moved` 与当前真实结果整理差异。不重新录制、不伪造结果、不添加运行期匹配键或测试元数据，不弱化断言；剩余决策与证据见回放文档，未宣布成功。

2026-11-13 记录回放：记录是系统原生能力，测试套件不执行录制；已有 session.jsonl/events.jsonl 先导出，再回放核对。导出仅用原生 ID、Candidate/Chance/applied Promote 还原预测批次之后第一条记录的父边，处理并行／嵌套及 Pi compaction/custom/context_edit，不读取遗留专用键，不改运行期或源记录，不加旧格式兼容。B1、已有 B1-PI 的导出→真实 Pi provider 回放闭环及手构造还原回归通过；真实 NLE 单层／四层整链因新工具结果与源记录不同而缺键失败，严格断言保留，不声称完成整链验收。当前证据和边界归 [回放](docs/development/replay.md)。

根枝起点提前量规则及零首段方向回退已实现，见[参数](docs/development/tree-parameters.md)。末端提前量继续待议。

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

2026-11-13 整理说明：已移除已下沉结论的重复全文、被后续记录取代的实施描述和逐轮验收日志；原讨论中的树形与 PTY 废弃方案补记于开发决策。源码归组依据 `src/lib.rs` 核实；版本目标依据 roadmap 核实；回放数据以 replay 文档区分 N=3 的 10 条与 N=4 的 13 条，不沿用旧讨论“两者均为 10”的重复描述。其余能力状态按链接文档保留限定，不宣称本轮重新验收。

## 2026-11-13 session 与状态交接讨论收敛

用户已批准：正确状态交接由机制保证；最终单一权威 session 是经过验证、用真实结果整理后的历史，保留预测输入，将预测输入后第一条后续记录改接真实结果 ID，保留原主／丢弃历史，不额外强制另存完整投机后续路径。真实结果通知由 Policy 决定，默认完全不追加；不新增 Policy 名称或类别。原始运行与整理后恢复上下文可能不同。

目标语义归 [Agent 接入设计](docs/design/agent-integration.md) §3、[Branch 语义](docs/design/branch-semantics.md)；Pi 原生完整持久化、多 session 过渡、接入侧汇总及临时文件删除条件归 [接入实现](docs/development/agent-access.md) §4。compaction/custom 必须保存；其引用／摘要、在飞推理、rollback 外部资源关联仍有开发差距。系统外导出已接通历史重挂还原，验证边界见回放文档。FS 共同快照差异合入 winner 仅为讨论方向。

### 保留的待议事项

2026-11-13 已收敛：既有 dispose／Promote 承载可选 `notification`；本轮用户明确回复“同意”，批准保留 `dispose.context.authority_outcome`，向 Policy 提供对应 Chance／operation 的真实结果，不自动给模型追加消息；保留 Match evidence 与 `context.match`。此前将该字段写成“用户同意”缺乏独立批准依据，不据本轮批准追认早先已确认。Pi 原生交付、缺省零记录、持久化及去重边界见 [接入设计](docs/design/agent-integration.md) §3，实现与验证缺口见 [接入实现](docs/development/agent-access.md) §4.1。compaction 摘要保留原文，不阻塞汇总或满足其他条件后的源清理；移除这两项待议问题，不新增 FS backend 讨论。

2026-11-13 独立测试审查：补充真实 main.ts 协议入口的通知 cleanup／EOF、完整命令重传及写失败验证，9 项新增、联合 Pi 共 24 项通过；未复现生产代码缺陷。验证边界见接入实现 §4.1。不改 display:false，不新增恢复能力。上述为既有审查记录，本轮文档收尾不重跑测试。批准依据已按本轮明确回复收敛，不再保留核对早先批准的待确认项，决策记录见 [开发决策](docs/development/decisions.md)。

- **多步投机的可行性**：区分“候选执行多步”与“一个 Proposal 由多步组成”；不因历史整理方案确认就宣布二者均已支持。
- **脱离 AgentScout 后继续原任务**：仍需证明工具、环境与外部资源能够完整交接；不以单一最终 session 替代验证。相关实现差距见第 31 项。
- **events 与全系统恢复日志**：首版范围按下方“主动退出恢复”记录；events 的恢复用途与保存技术方案仍待定。

## 2026-11-13 主动退出恢复（实施中）

用户明确首版覆盖主动退出成功后重启恢复，授权参考 Pi 自主实现、不委派、不再询问方案。立即发出原生中断，等待取消确认与保存；不等待推理自然结束。意外崩溃与掉电不属于首版。

保存目标是整棵仍有效的投机树及其执行状态，包括尚未决出的 Candidate，不能为方便退出自行丢弃候选或仅保留权威主线。需协调控制面停止接收新工作、Pi 原运行上下文及其 pending 工具批次、Policy 异步工作与命令回执，以及 NLE／FS 环境的保存和重建。整理后的最终 session 是历史，不能直接替代活跃 Branch 的原运行上下文；恢复保存成功前不能沿用现有 cleanup 删除唯一原始源文件或环境数据。

Pi 基准：AgentSession.abort 立即触发 AbortSignal 并 waitForIdle；message_end 保存最终部分 assistant 与工具结果；重新打开 session 等待用户输入，不自动 continue。保存失败保留原文件与环境，不报告成功。活跃原上下文与最终历史分别保存，Promise／线程必须重建等待关系。活动工具与 sibling 候选先原生取消，收齐已发出操作／命令确定回执再保存；继续后并行重连，不重复 Runtime 副作用。多 backend 全部成功才发布 manifest，FS 每次恢复复制独立可写环境。已支持本地资源按身份和内容绑定，Git snapshot 保留独立 ref；不以进程内存或复制 session 代替外部资源。失败继续、重试、旧保存代以及 notification 的真实入口证据及未交付项见 docs/development/recovery.md。

## 2026-11-13 实现核查补记

- Pi 原生 session 分叉、compaction/custom 保留、原生 ID、checkpoint 分叉和单一最终文件整理已接通并由伪模型检查；源文件删除仍只发生在成功整理后，活跃 winner 不删除。
- 回放仅使用原生 session/events；正常运行不计算或保存回放专用键，也不依赖 tools/replay。键只在导出产物中生成。历史重挂按原生记录关系在导出内存还原；套件从已有 run 导出开始，真实 NLE 整链本轮因工具内容变化缺键失败。废弃方案归开发决策。
- `pi-code-rollback` 的快照关联仍是缺口：必须继续传递并验证 `targetId`、ref、repoRoot 与 Git 对象，session 文件复制不构成支持；本轮未对用户仓库执行恢复。
- 并行工具配对及在飞推理与外部状态交接仍需专门验收；compaction 摘要按已确认规则保留原文。
# Replay 测试套件的职责

用户确认：replay 套件的功能仅限于从已有运行中读取模型响应，并按顺序提供给被测系统；它本身不负责评估投机策略或机制。严格说，它提供的是顺序响应，而非恢复或重演原运行的全部状态。

套件本身的正确性要求是：在投机逻辑和策略不变的前提下，顺序提供的响应应使执行逻辑保持一致。借助该套件编写的测试，进一步检查投机执行、Promote／Discard、隔离环境等机制是否正确。后续需继续讨论“执行逻辑一致”的具体判定范围与依据。

2026-11-13 本轮共识补记：Replay 测试套件仅从已有运行读取模型响应，并按顺序向被测系统提供响应；只保证套件本身的正确性。上段所述“执行逻辑一致”是用户对回放的要求：在投机逻辑与策略不变的前提下，回放执行逻辑应保持一致。借助此套件编写的测试检查投机执行、Promote／Discard、隔离环境等机制是否正确。未决事项为“执行逻辑一致”的判定范围，以及所需的环境和时序前提；顺序响应的具体交付规则尚未讨论，不擅自确定为全局 FIFO。

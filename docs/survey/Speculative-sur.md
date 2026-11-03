# Agent 投机执行调研

核查更新：2026-10-22。日期区分 arXiv 首次提交与版本修订；作者机构不等于公司产品归属。下文的效果数字均为论文报告，未在本项目中独立复现。公开仓库存在也不代表实现完整或已经达到生产可用程度。

按主要贡献区分：

- **投机执行算法**：预测什么、提前推进多远、何时接受预测，以及如何权衡延迟与成本。
- **投机执行基础设施／沙箱**：如何隔离状态、追踪依赖、控制效果公开，以及提交或丢弃执行结果。
- 两者可以交叉；兼具预测和运行时机制的工作会明确标注，不强行视为纯算法或通用基础设施。普通异步并发另列，不自动视为投机。
- 基础设施的收录依据是直接处理尚未确认的执行及其状态／效果的接受、公开或丢弃，而不只是可供投机使用。资源预热、沙箱内存优化和通用快照／克隆能力本身不作为投机执行工作收录；仅研究相关安全语义的工作另列。

![[8537852f0ed52329cd19541bfa759436.jpg]]

## 投机执行算法

### Speculative Actions

[Speculative Actions: A Lossless Framework for Faster Agentic Systems](https://arxiv.org/abs/2510.04371)，哥伦比亚大学等，首次提交 2025-10-05，v2 为 2026-04-23。

借鉴处理器投机执行与 LLM speculative decoding，把 Agent 的推理、工具调用等行为建模为 action，由较快的投机路径提前推进，再由权威路径验证并复用匹配的执行。其框架表述较通用，但实验结论仍受具体负载、预测准确性、验证规则和效果处理能力约束，不能直接外推为任意外部工具均可无损投机。

### PASTE

[Parallelizing Tool Execution and LLM Generation for Low-Latency Agent Serving](https://arxiv.org/abs/2603.18897)，作者单位包括上海交通大学、Microsoft Research、Stevens、Google、HKUST。首次提交 2026-03-19，v3 为 2026-06-16。

提出 Pattern-Aware Speculative Tool Execution（PASTE），利用轨迹中可复用的工具调用模式和参数传递关系，在模型生成正式工具调用前提前执行候选。通过工具级资格策略限制副作用行为，例如禁止提前执行或转换为 dry-run；这不是对任意工具的自动隔离与回滚保证。该工作同时涉及预测算法和资源调度。

![[749f3b3ebc9551623dd9828b364b4de3.png]]

### SPAgent

[Reducing Latency of LLM Search Agent via Speculation-based Algorithm-System Co-Design](https://arxiv.org/abs/2511.20048)，作者单位包括清华、上海交通大学、Infinigence AI、联想。首次提交 2025-11-25。

使用动作级投机与系统调度降低搜索 Agent 的端到端延迟；除提前执行工具外，还在论文规定的接受条件下跳过部分推理。这里的接受规则属于具体算法设计，不等于任意预测均可跳过权威验证。

### Cost-Aware Speculative Execution

[Cost-Aware Speculative Execution for LLM-Agent Workflows: An Integrated Five-Dimension Method](https://arxiv.org/abs/2606.07846)，Faisal Fareed，正文署名机构为 AWS，首次提交 2026-06-05。

把投机选择建模为以货币计价的期望价值决策，权衡成功时节省的时间价值和失败时浪费的增量计算成本。按失败概率加权的是该成本模型下的浪费项，不意味着所有实际费用都只在失败时发生。论文主要提供方法与合成数值验证，不是 AWS 已部署系统的证据，也不覆盖运行时动态生成拓扑的一般问题。

### Speculative Interaction Agents

[Speculative Interaction Agents: Building Real-Time Agents with Asynchronous I/O and Speculative Tool Calling](https://arxiv.org/abs/2605.13360)，UC Berkeley、ICSI、LBNL。首次提交 2026-05-13，v2 为 2026-05-14。

面向低延迟交互，将 Agent 的思考／行动流与用户输入流、工具结果流解耦，同时处理“用户尚未说完”和“工具尚未返回”两种等待。信息足够时可提前推理和发起符合安全条件的读调用；写操作等待最终输入确认后再执行／提交。取消未采用的结果，不代表撤销已经发出的请求及其外部痕迹。致谢中的资助公司不作为作者机构或官方背书。

### Speculative Tool Calls

[Optimizing Agentic Language Model Inference via Speculative Tool Calls](https://arxiv.org/abs/2512.15834)，劳伦斯利弗莫尔国家实验室、马里兰大学。

研究 Agent 推理中的提前工具调用，应与单次模型调用内部的 token 级 speculative decoding 区分。

### Skim

[Skim: Speculative Execution for Fast and Efficient Web Agents](https://arxiv.org/abs/2605.16565)，Princeton、Microsoft Research。首次提交 2026-05-15，v2 为 2026-05-19。

面向 Web Agent 的投机执行工作；浏览器交互和验证条件构成其应用边界，不直接视为任意环境的通用沙箱。

### SPORK

[SPORK: Self-Speculative Forking to Accelerate Agentic LLM Inference](https://arxiv.org/abs/2607.03333)，清华、美团，首次提交 2026-07-03。

在主模型仍生成 CoT 时，从同一模型分出 probe，提前预测工具调用并执行，将工具延迟与剩余生成时间重叠。这里预测的是调用内容，不是工具实际返回的结果。置信度控制提前派发，后续机制还尝试把 probe token 用作 speculative-decoding draft。

负载包括 GAIA、HotpotQA、τ²-bench 等通用工具 Agent，并非专为语音流式场景设计。D1/D2 使用自托管 vLLM 的 completion、logprobs 等接口能力；D3 还涉及 draft-token 接入，不能笼统称为适配任意闭源模型 API。

[公开实现](https://github.com/baihuajun24/spork)包含核心代码和运行入口，但公开后端与论文实验并不完全相同：GAIA 搜索／浏览后端需自行补充，公开 τ² 评估采用单轮、无用户模拟器的 action-match 检查，不等同于官方完整 benchmark 分数。D3 的 HTTP 参考路径也不应与论文的引擎集成路径混写。

### Interactive Speculative Planning / Dynamic Speculative Agent Planning

[Interactive Speculative Planning: Enhance Agent Efficiency through Co-design of System and User Interface](https://arxiv.org/abs/2410.00079)使用快速 Agent 预测未来动作，由目标 Agent 验证；不把该规划算法视为通用副作用隔离／回滚方案。作者任职经历不作为公司官方项目归属的依据。

[Dynamic Speculative Agent Planning](https://arxiv.org/abs/2509.01920)针对固定投机步数难以适应运行状态的问题，根据当前部分轨迹估计后续预测可靠性，动态调整投机深度，并采用在线学习。重点是投机深度选择，而不是提供环境沙箱。

### SpecHop

[SpecHop: Continuous Speculation for Accelerating Multi-Hop Retrieval Agents](https://arxiv.org/abs/2605.21965)，首次提交 2026-05-21。

预测中间工具 observation，在真实工具并行运行时继续后续推理，异步验证并接受正确分支，验证失败则丢弃依赖错误预测的后续分支。区别于只预测下一次工具调用，该算法通过预测结果持续向前推进。摘要报告最高约 40% 延迟下降，适用范围是论文评估的多跳检索负载。

[项目仓库](https://github.com/mehrdadsaberi/spechop)截至核查仅包含 README 和图片，明确写有 “Code will be released soon”；不能标注为实现已开源。

### Speculative Macro Commit（SMC）

[Speculative Macro Commit for Faster Tool-Using Agents](https://arxiv.org/abs/2609.03236)，USC、Intel Labs，首次提交 2026-09-03；arXiv 标注 Accepted in MLSP2026。

从训练轨迹挖掘多动作模式，由较快的 drafter 在隔离环境快照上提前执行动作链。权威 actor 确认首个锚点调用后，运行时可按模式匹配和在线检查接受后续若干步骤，跳过这些步骤的 actor 决策。因此它是**近似算法，而非逐步权威验证的严格 lossless 方法**。

论文报告 τ² Telecom 延迟相对串行执行降低 18.59%；AppWorld 延迟降低约 44.9%，但成功率由 41.67% 降至 40.48%。串行基线与投机配置的 GPU 数量不同，与相同硬件的 SA 基线相比增益应另行比较，不能把全部收益都归于宏提交。

[公开实现](https://github.com/zeyuliu1037/speculative-macro-commit)包含 AppWorld、τ² Telecom 代码、冻结的模式库、环境配置与运行脚本；这是具体 benchmark 的研究实现，不等于通用外部服务事务底座。

### B-PASTE

[B-PASTE: Beam-Aware Pattern-Guided Speculative Execution for Resource-Constrained LLM Agents](https://arxiv.org/abs/2604.16469)，Yanfei Song，正文署名 Independent Researcher，首次提交 2026-04-09。

把单工具投机扩展为有界候选子图，按预期关键路径收益、资源竞争和安全限制选择执行前缀。属于值得跟踪的算法／调度构想：论文仅提供 Thor-class 环境中的初步内部结果，完整评估仍为未来工作。硬件名称不能作为 NVIDIA 官方归属的证据，当前不将其列为已充分验证的实现。

### Speculate with Memory

[Speculate with Memory: Lossless Acceleration for LLM Agents](https://arxiv.org/abs/2607.12236)，作者均署名 Salesforce Research，首次提交 2026-07-14。

为预测器加入三种在线记忆：记录动作转换统计的 transition table、检索相似历史片段的 episodic memory，以及抑制重复错误的 confusion tracker。覆盖 action、observation 和连续预测，让预测器能够跨任务积累经验，而非每次从零预测。

主要贡献是预测质量改善，不提供新的通用效果隔离机制。“Lossless”仍依赖相应投机框架的验证与效果处理条件；不能将预测准确率提升直接写成端到端加速倍数。截至核查未确认公开实现。

### Speculate While You Reason

[Speculate While You Reason: Teaching Agents to Predict Their Next Tool Call via Joint Agent-Speculator RL](https://arxiv.org/abs/2607.25816)，UCSB、LinkedIn，首次提交 2026-07-28。

同一模型以 Agent 与 speculator 两种模式运行，在 Agent 推理期间预测下一次工具调用；通过联合强化学习改善预测，并复用共享前缀的 KV cache。论文重点包括 Hit@1 等预测质量与执行效率，不应把模型训练方法视为通用工具沙箱。

LinkedIn 参与依据正文作者机构及实习说明，不推断为其产品已部署能力。截至核查未确认公开实现。

### AOSpec（兼具预测算法与运行时机制）

[AOSpec: Action and Observation Co-Speculation for Low-Latency Agent Serving](https://arxiv.org/abs/2608.00881)，Imperial College London，首次提交 2026-08-01。

联合预测未来动作和 observation，在隔离分支中提前推进；按预期可隐藏的执行时间选择投机，并以 Joint Action–State Verification 联合检查动作与状态。它不仅预取下一次调用，也尝试跨越尚未返回的工具结果继续执行。

论文报告平均延迟下降 11.8–32.5%，限于其评估负载和隔离、验证条件；不能推断任意外部调用都能安全提前执行。截至核查未确认自身公开实现，引用底层沙箱项目不等于算法实现已开源。

### DynBranch

[DynBranch: Speculative Subgraph Reuse for Dynamic Agentic LLM Serving](https://arxiv.org/abs/2609.31047)，NUS，首次提交 2026-09-25。

利用工作流模板中的稳定决策位置，在路由结果确定前执行候选子图，并跨请求复用匹配结果。虽面向动态 Agent 工作流，方法仍依赖可识别的模板与决策位置，不是对任意未知拓扑自动投机。

复用前提包括精确输入匹配、外部读取新鲜度，以及考虑资源负载的收益判断。它主要贡献子图投机与复用策略，不提供任意外部效果的事务隔离。截至核查未确认公开实现。

### LEAP

[LEAP: Learning Efficient Action Proposals For LLM Agents](https://arxiv.org/abs/2610.02670)，Chicago、Stanford、Together AI，首次提交 2026-10-02（UTC）。

训练 0.6B 的小型 action drafter 生成候选动作，由目标模型并发验证相应上下文，以降低逐步等待目标模型的开销。论文报告整任务 1.10–1.63× 加速；应与单次模型调用内部的 token 级 speculative decoding 区分。

其收益依赖候选质量、目标模型验证与执行负载，不意味着通用工具副作用已被隔离。截至核查未确认公开实现。

### ObsSpec

[ObsSpec: Accelerating Agentic Reinforcement Learning with Observation Speculation](https://arxiv.org/abs/2610.15246)，UCSC、ByteDance Inc.，首次提交 2026-10-11（UTC）。

面向 Agent 强化学习 rollout，预测工具 observation，在真实工具运行期间继续后续生成；只有预测与真实输出精确匹配时才保留依赖预测的生成，否则丢弃。这里的验证对象是 observation，不是仅检查下一次工具调用的名称或参数。

论文报告 8B／32B policy 的延迟分别下降 9.3%／14.2%，不外推为所有在线 Agent 的收益。ByteDance 参与依据正文机构与 Seed 实习说明。[公开实现](https://github.com/kylemontgomery1/verl-recipe/tree/obsspec/obsspec)位于 `obsspec` 分支，包含 Agent loop、轨迹处理、sandbox adapters、训练及复现目录，仓库许可为 Apache-2.0；本次未运行。

### SpecAgent

[Trajectory-Level Speculation with Semantic Verification for Tool-Using Agents](https://arxiv.org/abs/2610.21463)，LARK / HKUST(GZ)、HKUST、Cambridge、Yinwang，首次提交 2026-10-15。

让较快模型提前执行轨迹块，由较强模型进行语义审核，而非要求每一步都与强模型独立生成的动作精确匹配。因此接受规则不等于保持 strong-only 基线的逐步轨迹，不能归为严格逐步权威验证的无损方法。

[公开实现](https://github.com/LARK-AI-Lab/SpecAgent)采用 MIT 许可，包含 runtime gateway、HTTP server 和 benchmark runners。README 明确连续 K 个只读步骤可未经审核提交，遇到首个写操作再审核，不能概括为所有步骤均先经审核。论文报告 FLOPs 增加约 30–87%，时间收益来自减少昂贵模型的内存流量等因素，不应写成计算量与延迟同时下降。

## 投机执行基础设施／沙箱

### TomasuLLM（兼具预测算法与运行时机制）

[TomasuLLM: Out-of-Order Speculative Execution for LLM Agents](https://arxiv.org/abs/2609.38201)，HKUST、南京大学、KAUST、浙江实验室。首次提交 2026-09-22，v2 为 2026-10-01。

同时预测未来动作和 observation，但主要运行时贡献在于：在私有写时复制沙箱中提前执行工具，追踪依赖和效果，并按正式轨迹顺序验证、公开结果或提交效果。验证涵盖动作一致性、依赖新鲜度、结果记录完整性和效果安全性；无法保守追踪或隔离的调用构成投机屏障，退回串行路径。

论文报告 SWE-bench Verified、Terminal-Bench 2.0、SWE-Marathon 分别约 1.31×、1.35×、1.27× 的加速，以及 4,010 条提交验证审计记录中零 false accept。实验结果不是对任意工具的普遍正确性证明，保证依赖工具包装、保守追踪和受控效果范围。截至核查尚未确认公开实现仓库。

### TClone

[TClone: Low-Latency Forking of Live GUI Environments for Computer-Use Agents](https://arxiv.org/abs/2605.17320v1)，UC San Diego，首次提交 2026-05-17。

主要贡献是运行中 GUI 环境的低延迟分叉能力，为 Computer-Use Agent 的多路径探索提供环境支撑。探索可用于改善任务质量，也可支撑并行执行；不宜把它归为某种下一动作预测算法，也不宜将其价值限定为“只提升正确性、不涉及速度”。

### Atomix

[Atomix: Timely, Transactional Tool Use for Reliable Agentic Workflows](https://arxiv.org/abs/2602.14849)，马普所、EPFL。首次提交 2026-02-16，v2 为 2026-05-29。

关注工具效果何时可以永久公开／结算，处理投机残留、并发冲突与过期写入、不可撤销操作等问题。checkpoint 只能恢复内部状态，取消任务也不能撤回已发送邮件；补偿、锁和冲突检查分别覆盖部分问题，不能自动组成完整的效果治理保证。Atomix 应归入工具事务与效果控制基础设施，而不是动作预测算法；实际保证仍取决于工具和外部服务提供的接口能力。

### Fork, Explore, Commit / BranchFS

[Fork, Explore, Commit: OS Primitives for Agentic Exploration](https://arxiv.org/abs/2602.08199)，YuSheng Zheng，首次提交 2026-02-09，v2 为 2026-03-19。

论文提出 OS 级 branch context，把探索组织为 fork → explore → commit/abort，并讨论独立写时复制文件系统视图、进程上下文和 first-commit-wins 语义。

[BranchFS](https://github.com/multikernel/branchfs)提供 Rust/FUSE 实现，但必须与 proposed Linux `branch()` syscall 分开，不能称为已实现的内核能力。当前 README 描述叶子分支提交到直接父分支、丢弃叶子分支、访问嵌套分支等能力，并明确展示某个分支提交后 sibling 仍可继续工作；不能将论文的 single-winner 语义直接当作当前仓库保证。文件系统分支也不自动涵盖网络和其他外部效果。

### Chronos

[Chronos: Efficient Bolt-On Branching Across Data Stores for Stateful Agentic Applications](https://arxiv.org/abs/2609.14889)，MIT CSAIL、University of Arizona。首次提交 2026-09-14，v2 为 2026-09-17。

通过区间版本管理和统一分支元数据，为 PostgreSQL、SQLite、DuckDB、Qdrant 以及 DBMS-backed filesystem 提供分支、差异比较与合并。合并先暂存数据，再通过事务性元数据更新公开，目标是避免读者看到跨存储的部分合并。

这是状态基础设施，不是预测算法，也不是任意外部服务的通用回滚层。其前提包括受管理的数据访问经过适配层、底层存储支持所需的原子更新，以及至少一个事务性元数据存储；冲突解决仍由 Agent、用户或应用策略决定。论文报告的最高 16.7× 是特定 MCTS 风格探索工作流相对比较方案的结果，不是所有 Agent 的端到端加速。

### Ghost Tool Calls（调用发出时的隐私控制）

[Ghost Tool Calls: Issue-Time Privacy for Speculative Agent Tools](https://arxiv.org/abs/2606.02483)，马普所、EPFL，首次提交 2026-06-01。

关注尚未确认需要的工具请求是否已经把用户意图暴露给外部观察者。投机隐私风险发生在调用发出时，而不是结果最终被采用时；丢弃结果无法抹去访问痕迹。因此隐私策略需要在请求离开可信运行时边界之前执行。此工作属于投机调用的安全控制，不是动作预测或环境分叉算法。

### Kairos（安全用途的投机效果控制）

[Kairos: Transactional Security Enforcement at the System Boundary for LLM Agents](https://arxiv.org/abs/2610.18457)，Lübeck、UCL、MBZUAI，首次提交 2026-10-13。

面向间接 prompt injection 防护：单步动作不足以判定是否合法时，不立即将效果公开，而是在文件／网络隔离边界暂存未授权效果，允许不透明 Agent 继续投机推进；后续由 oracle 根据累积证据决定释放或拒绝。主要目标是延后安全决策、取得更充分的判断信息，而不是预测下一动作或主要追求延迟加速。

[公开 artifact](https://github.com/UzL-ITS/Kairos)采用 AGPL-3.0，包含 reference monitor、策略、provenance、投机状态机，以及 microVM、OverlayFS/FUSE/WAL 文件通道、mitmproxy 网络通道和评估记录。它直接处理未确认执行与效果可见性，因此纳入本节；保证应限定于受控通道和部署边界，不能理解为撤销任意已经送达外部服务的请求。本次未运行该实现。

## 相邻工作：并行与异步执行、执行安全语义

### W&D

[W&D: Scaling Parallel Tool Calling for Efficient Deep Research Agents](https://arxiv.org/abs/2602.07359)，Salesforce AI Research，首次提交 2026-02-07。

作为深度研究 Agent 的并行工具调用路线保留。并行执行多个已确定调用与预测尚未确定的调用不同，不应仅凭存在并行就视为同一种投机机制。

### AsyncFC

[Concurrency without Model Changes: Future-based Asynchronous Function Calling for LLMs](https://arxiv.org/abs/2605.15077)，UC Berkeley。首次提交 2026-05-14，v2 为 2026-10-09。

工具调用先返回 future 标识，真实函数在后台运行；模型可继续生成后续调用，由运行时处理依赖、等待真实值就绪。这使推理与执行重叠，但不必包含对未知值的预测与验证，因此列为相邻的异步并发路线，而不是直接归入投机算法。

### Checkpoint / Fork / Restore / Merge 的安全检查

[When Can Agents Safely Checkpoint, Fork, Restore, and Merge? Exact Checking for Execution Edits](https://arxiv.org/abs/2608.22928)，首次提交 2026-08-24。

研究运行时如何从执行记录推导 checkpoint、fork、restore、merge 必须保留的约束：执行状态恢复不能撤回已经发出的授权或工具请求，也不能导致同一外部操作重复执行、丢失仍必需的结果，或与在飞调用冲突。论文提出精确检查算法，并提供有限模型下的 Lean 证明与可执行检查。

[公开仓库](https://github.com/eunomia-bpf/agent-check-restore-safety)包含 Python／Lean artifact 和运行时原型；当前 README 说明项目正在向 Agent、VM 和微服务变更的共同控制层演进，不能把当前仓库所有能力归入原论文。精确性针对所建模的执行记录、策略和操作契约，不等于任意 API 自动具备 exactly-once 或事务保证。本条记录与投机分支相关的安全语义，不作为预测算法或通用投机沙箱。

### Tracking State Footprints

[Tracking State Footprints: How Agents Can Transact](https://arxiv.org/abs/2610.03140)，TU Delft、Ververica GmbH，首次提交 2026-10-02。

从数据管理角度分析多 Agent 的读写竞争、陈旧读取和依赖丢失，主张追踪 Agent 本地、编排器以及外部系统三个范围的状态读写，而不只记录任务控制流。论文讨论确定性编排机制应如何提供保证，以及 Agent 如何参与语义冲突解决。

这是设计愿景与问题分析，不是已完成的通用事务运行时，也不专门提出投机预测算法。[配套仓库](https://github.com/delftdata/agenti-ai-guarantees)主要包含失败数据分析、确定性执行拓扑实验和真实 Agent 实验，不能把存在 artifact 当作事务机制已经实现的证据。

## 公司工作与社区实现

### NVIDIA / LangChain：已知工作流图的分支投机

[LangChain 官方公告](https://www.langchain.com/blog/nvidia-enterprise)，2026-03-16，描述在条件路由确定前运行候选分支，路由确定后丢弃未选分支。

[NVIDIA/NeMo-Agent-Toolkit](https://github.com/NVIDIA/NeMo-Agent-Toolkit)中存在对应的公开机制代码：

- [`router_branch.py`](https://github.com/NVIDIA/NeMo-Agent-Toolkit/blob/main/packages/nvidia_nat_app/src/nat_app/speculation/strategies/router_branch.py)：识别路由分支，生成提前执行及保留／取消计划。
- [`safety.py`](https://github.com/NVIDIA/NeMo-Agent-Toolkit/blob/main/packages/nvidia_nat_app/src/nat_app/speculation/safety.py)：提供不安全节点标记和安全配置。

这是官方发布且可查到代码的工作流分支投机机制，兼有执行策略和框架运行时支持。它利用已知图结构，不等同于预测动态生成的任意工具调用；排除不安全节点和取消任务，也不等同于自动回滚已经发生的外部效果。本次未运行该实现，不宣称生产成熟度。

### speculative-tools：社区工具预测与缓存原型

[joelvarun/speculative-tools](https://github.com/joelvarun/speculative-tools)参考多篇论文，提供工具序列预测、参数模式匹配、异步提前执行、结果缓存和多种选择策略。

属于算法与轻量执行包装的社区原型，不是通用隔离沙箱。副作用工具需要显式排除；缓存 TTL 只能限制结果存放时间，不能证明读取时状态未变化，也不能保证结果与串行执行完全一致。不直接采纳 README 中对任意工具“行为不变”的概括性承诺。

### agent-lsp：编码候选的暂存与接受／丢弃

[blackwell-systems/agent-lsp/speculative-execution](https://github.com/blackwell-systems/agent-lsp/blob/main/docs/guide/speculative-execution.md)主要面向编码任务中的候选变更暂存，避免未接受的代码直接污染工作区。

按编码状态管理基础设施记录，不作为任意进程、数据库和外部服务隔离能力的证据。

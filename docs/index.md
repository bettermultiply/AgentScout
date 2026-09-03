# AScout 文档导航

AScout 文档组织成树：[心智模型](mental-model.md) 是提供全局视角的唯一根文档，其余文档从心智模型中的节点向下展开，承载相应的目标语义、设计与实现细节。尚未收敛的议题先进入仓库根目录的 [Discussion.md](../Discussion.md)，接受后再同步到正式文档。

## 根文档

- [心智模型](mental-model.md)：**概念层**。先给出 AScout 的定位（定义、问题、目标、边界），再介绍工作模型（可组合的投机执行过程、投机边、Policy/Mechanism、Speculation Contract）、支撑模型（Policy Decisions、状态与效果、Speculation Contract）和系统结构（四侧架构与核心对象所有权）。

心智模型当前直接展开为：

- [问题与目标](problems-and-goals.md)：投机动机、三个缺口（策略孤岛、隔离碎片化、上下层脱节）与 AScout 目标。
- [设计模型](design-model.md)：抽象概念层。工作模型（可组合的投机执行过程、投机边、Policy/Mechanism/Runtime 抽象分层）与支撑模型（Policy Decisions、状态与效果、Speculation Contract），以及跨领域设计约束。
- [架构](architecture.md)：系统层的四侧调用路径、责任与状态边界；四侧不表示都由 AScout 实现或拥有。

心智模型内部先区分工作模型和支撑模型：投机执行过程、Policy/Mechanism、Speculation Contract 与投机边构成工作模型；Policy Decisions、状态与效果、Speculation Contract 构成支撑模型。三者分别对应策略孤岛、隔离碎片化和上下层脱节三个问题。抽象概念模型见 [设计模型](design-model.md)，跨领域设计约束亦在其中。Fidelity 是跨层契约的导出不变量，shadow/replay 是验证和校准基础设施，不另列为独立模型。Policy 的语义对象与决策细节见专门的 [Policy Decisions 设计](design/policy-decisions.md)。投机边同时承载多投机之间的组合语义（嵌套、级联失效与信息流约束）。

## 设计文档

1. [Branch 语义设计](design/branch-semantics.md)：Branch 与 Speculative Tree、隔离与通信、Agent 最小感知模型、Workspace/Command 粒度、可见执行环境与输入传播的目标语义。
2. [设计决策](design/decisions.md)：AScout 核心 Speculative Tree 语义选择的速查手册与原始讨论记录。
3. [投机执行底座：设计决策](design/speculation-decisions.md)：投机执行底座的已接受语义速查手册；其中的历史研究分类与当前 Policy Decisions 的整理关系见 [Policy Decisions 设计](design/policy-decisions.md)（D-001..D-028）。
4. [术语表](design/glossary.md)：每个术语的一句话定义、权威出处，以及两套设计视角之间的术语映射。
5. [Policy Decisions 设计](design/policy-decisions.md)：Policy 可作出的决策集合、阶段映射、扩展边界与 Mechanism 约束。
6. [效果管理设计](design/effect-management.md)：世界状态分类、EffectClass、StateDomain、提交屏障与外部效果可见性。
7. [Speculation Contract 设计](design/speculation-contract.md)：约束 Policy 表达与 Runtime Adapter 控制面接入行为的语义对齐契约，及其校验、降级与运行期校验。
8. [Agent 接入侧设计](design/agent-integration.md)：Agent/product integration、ASC-Daemon、Interposer 与 Operation Integration 的责任边界。
9. [控制面设计](design/control-plane.md)：控制面作为 Mechanism 的实现主体，如何把 Contract 落到投机执行的控制路径。

## 使用文档

1. [Agent 接入状态](usage/agent-guide.md)：当前可用性和计划中的接入边界。
2. [FAQ](usage/faq.md)：不依赖具体 backend 的目标级问答。

## 开发文档

1. [开发决策](development/decisions.md)：已经接受的实现架构选择（含 D-024-I：四侧一级架构）。
2. [开发路线图](development/roadmap.md)：当前实现状态与剩余任务。
3. [验收状态](development/verification.md)：当前证据和后续 acceptance gates。
4. [首要验证 Workload](development/reference-workload.md)：delayed-choice coding 场景与指标。
5. [AScout 定位与设计思考](development/ascout-agent-positioning.md)：Agent、AScout 与 Runtime 的定位、阶段目标和长期演进方向。
6. [投机执行底座：抽象设计札记](development/spec-exec-abstraction.md)：投机执行底座的完整研究札记，包括论文调研、轴推导、机制/策略划线、嵌套投机、内部组件与部署视角、问题陈述和评测指标。一级架构以 [AScout 架构](architecture.md) 为准，研究结论的权威条目见 [设计文档 3](design/speculation-decisions.md)。
7. [Runtime Adapter 接入指南](development/runtime-adapter.md)：面向 Adapter 作者的接入责任、状态边界、实现建议与尚待定义的接口事项。

## 状态术语

- **目标语义**：不依赖具体实现、系统必须满足的行为。
- **开发决策**：已经接受的实现架构或验证选择。
- **计划中**：当前树中没有实现证据。
- **已实现**：当前树中同时存在代码和验证证据；当前没有任何 AScout lifecycle、隔离和真实 backend 处于该状态。

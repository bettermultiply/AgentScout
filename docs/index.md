# AScout 文档导航

AScout 文档组织成树：两个根文档提供全局视角，其余文档作为语义权威与实现细节。尚未收敛的议题先进入仓库根目录的 [Discussion.md](../Discussion.md)，接受后再同步到正式文档。

## 根文档

- [心智模型](mental-model.md)：**概念层**。先给出 AScout 的定位（定义、问题、目标、边界），再讲它赖以工作的核心模型（投机边、轴体系、Assumption DAG、效果管理、资源调度、保真度、Speculation Contract）。
- [架构](architecture.md)：**系统层**。定义 `Agent 接入侧 -> 控制面侧 -> Runtime Adapter 侧 -> Runtime Mechanism` 的四侧分层、各侧所有权与跨侧调用方向。

心智模型当前长出的叶子：

- [问题与目标](problems-and-goals.md)：投机动机、三个缺口（策略孤岛、隔离碎片化、上下层脱节）与 AScout 目标。
- [设计原则](design-principles.md)：贯穿设计的四条原则（Mechanism/Policy 划线、同一坐标系、跨层契约、Fidelity 可声明/可强制/可审计）。

其余核心模型叶子（投机边、轴体系、Assumption DAG、效果管理、资源调度、保真度、Speculation Contract）与架构四侧的叶子尚未建立。

## 设计文档

1. [状态与生命周期语义](design/overview.md)：目标行为的语义权威。
2. [设计决策](design/decisions.md)：AScout 核心 Speculative Tree 语义选择与不变量。
3. [投机执行底座：设计决策](design/speculation-decisions.md)：投机执行底座的已接受语义 —— 正交轴体系、Assumption DAG、效果管理、资源准入、嵌套投机与系统架构（D-001..D-028）。
4. [术语映射表](design/terminology.md)：Speculative Tree 机制视角与投机执行底座视角两套决策记录的术语对应关系与权威声明。
5. [术语表](design/glossary.md)：每个术语的一句话定义与权威出处。

## 使用文档

1. [Agent 接入状态](usage/agent-guide.md)：当前可用性和计划中的接入边界。
2. [FAQ](usage/faq.md)：不依赖具体 backend 的目标级问答。

## 开发文档

1. [开发决策](development/decisions.md)：已经接受的实现架构选择（含 D-024-I：四侧一级架构）。
2. [开发路线图](development/roadmap.md)：当前实现状态与剩余任务。
3. [验收状态](development/verification.md)：当前证据和后续 acceptance gates。
4. [首要验证 Workload](development/reference-workload.md)：delayed-choice coding 场景与指标。
5. [AScout Agent 定位与设计思考](development/ascout-agent-positioning.md)：AScout Agent、Harness 与 Runtime 的定位、阶段目标和长期演进方向。
6. [投机执行底座：抽象设计札记](development/spec-exec-abstraction.md)：投机执行底座的完整研究札记，包括论文调研、轴推导、机制/策略划线、嵌套投机、内部组件与部署视角、问题陈述和评测指标。一级架构以 [AScout 架构](architecture.md) 为准，研究结论的权威条目见 [设计文档 3](design/speculation-decisions.md)。

## 状态术语

- **目标语义**：不依赖具体实现、系统必须满足的行为。
- **开发决策**：已经接受的实现架构或验证选择。
- **计划中**：当前树中没有实现证据。
- **已实现**：当前树中同时存在代码和验证证据；当前没有任何 AScout lifecycle、隔离和真实 backend 处于该状态。

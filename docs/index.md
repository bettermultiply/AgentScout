# ASCout 文档导航

ASCout 文档按权威范围划分。设计文档定义已经确认的目标语义；开发文档描述已接受的实现架构、实现决策、路线图和验收状态；使用文档说明 Agent 接入者当前能够和不能使用什么。

设计文档与开发文档分别定义目标语义与已接受的实现选择。尚未收敛的议题先进入仓库根目录的 [Discussion.md](../Discussion.md)，接受后再同步到正式文档。

需要先建立全局视角时，从 [ASCout 一级架构](architecture.md) 开始；它定义 `Agent 接入侧 -> 控制面侧 -> Runtime Adapter 侧 -> Runtime Mechanism` 的四侧分层、所有权与调用方向，但不取代各语义权威文档。

## 设计文档

1. [状态与生命周期语义](design/overview.md)：目标行为的语义权威。
2. [设计决策](design/decisions.md)：ASCout 核心 Speculative Tree 语义选择与不变量。
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
5. [ASCout Agent 定位与设计思考](development/ascout-agent-positioning.md)：ASCout Agent、Harness 与 Runtime 的定位、阶段目标和长期演进方向。
6. [投机执行底座：抽象设计札记](development/spec-exec-abstraction.md)：投机执行底座的完整研究札记，包括论文调研、轴推导、机制/策略划线、嵌套投机、内部组件与部署视角、问题陈述和评测指标。一级架构以 [ASCout 架构](architecture.md) 为准，研究结论的权威条目见 [设计文档 3](design/speculation-decisions.md)。

## 状态术语

- **目标语义**：不依赖具体实现、系统必须满足的行为。
- **开发决策**：已经接受的实现架构或验证选择。
- **计划中**：当前树中没有实现证据。
- **已实现**：当前树中同时存在代码和验证证据；当前没有任何 ASCout lifecycle、隔离和真实 backend 处于该状态。

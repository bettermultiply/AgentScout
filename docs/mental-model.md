# AScout 心智模型

AScout（Agent Scout）是面向 Agent 投机执行的通用底座，Agent 能无修改地接入并获得投机执行能力。AScout 分离策略决策与具体运行时实现：上层仅设计不同的投机执行策略，而不关心具体实现；下层由 Runtime 提供投机执行所需的基本能力，而不会与某个特定投机执行策略耦合。

## 1. 定位 - 为什么需要 AScout

投机执行想要解决的问题是，Agent 在执行过程中经常需要等待一个慢结果——例如，远程 API、用户决策、CI 测试——它会阻塞后续推理执行，甚至主导整个执行流程。投机执行允许在结果尚未到达时按预测提前推进后续步骤，从而加速整个执行过程。而不同投机执行策略所依赖的不同的准入、预测、验证、隔离、取消与复用等能力，正是 AScout 抽象为统一机制的那组能力。

现有投机执行工作大多针对特定执行对象或特定状态域设计，能力与任务绑定，方案之间难以组合，也无法让不同任务选择不同策略。这些[问题](problems-and-goals.md)集中表现为策略孤岛、隔离碎片化、上下层脱节三个缺口。

AScout 通过统一的投机执行机制连接策略与运行时，承接不同策略在预测、验证、拓扑和处置等方面的差异，使策略能够独立选择和组合，而不绑定具体的执行实现。投机执行涉及的不同状态与效果也通过同一框架纳入管理，并由明确的契约约束策略要求与运行时能力之间的匹配；当运行时无法满足策略要求时，AScout 不会静默改变策略语义，而是拒绝执行或进行明确降级。

落在使用上：
- 端用户提交任务并得到结果，通常无需感知背后的投机分支。
- Agent 可以在不修改自身执行逻辑的情况下接入 AScout，并获得由 AScout 和 Runtime 共同提供的投机执行能力。
- 部署方负责选择和配置可用的 Runtime 以及相应的 Runtime Adapter，并根据部署条件选择可运行的 Policy。
- Policy 作者只需专注于算法设计，例如如何预测、如何判断命中、如何选择拓扑、成本和处置方式，不必负责底层机制的具体实现。

AScout 不是某个固定算法，也不是替代所有 Runtime 的统一运行环境。它也不承诺每个任务必然加速；是否投机、何时投机以及如何投机，由控制面根据所绑定的 Policy 决定。投机策略能否执行以及能够达到什么保证，则取决于 Runtime 的基本能力、具体能力和 Speculation Contract。

## 2. AScout 的工作模型

投机执行由一组可组合的阶段构成，而不是固定的线性流程：机会发现 -> 准入判断 -> 预测生成 -> 过程执行 -> 验证 -> 处置与提交。Policy 可以选择、替换和组合这些阶段，但被启用的阶段仍需遵守必要的数据依赖和 AScout 固定的正确性、隔离、资源与副作用约束。投机执行可以基于尚未确认的 Proposal 提前开展后续工作，并在获得权威依据或其他可接受性判断后继续、转正或结束。

Policy、Mechanism 与 Runtime 构成抽象责任模型：Policy 作者只需面对一组抽象的决策点；Mechanism 把抽象落实为执行并兜底正确性、隔离与资源约束；Runtime 只提供简单原语。Speculation Contract 约束 Policy 与 Adapter/Runtime 两侧的语义对齐；控制面作为 Mechanism 的实现方，通过编排与校验把约束落实为可执行的投机路径。

工作模型的完整权威描述见 [设计模型](design-model.md) §2。

## 3. 支撑模型

AScout 用三个支撑模型承接不同投机执行策略在预测、验证、拓扑和处置等方面的差异：

- **投机边与 Policy Decisions**：投机边 Edge 是 Policy 的作用对象；开发者可以根据任务、上下文、资源和 Runtime 能力，通过组合 Edge 上的 Policy Decision 设计可替换的决策 Policy，并处理多次投机的组合与嵌套。
- **状态与效果**：把投机执行过程中需要保护的文件、进程、输入、结果和外部服务交互放入统一的管理框架，根据状态域和效果纪律选择隔离、暂存、延迟、阻塞、补偿或清理方式；具体处理由 Effect Management 与 Runtime 落实。
- **Speculation Contract**：约束 Policy 与 Adapter/Runtime 两侧的语义对齐契约，使策略需求、后端接入通过同一条可检查、可执行的路径衔接，并让拒绝或降级原因可见。

支撑模型的完整权威描述见 [设计模型](design-model.md) §3；各模型的细节分别见 [Policy Decisions 设计](design/policy-decisions.md)、[效果管理设计](design/effect-management.md)、[Speculation Contract 设计](design/speculation-contract.md) 与 [控制面设计](design/control-plane.md)。

## 4. 系统结构

### 4.1 四侧架构

AScout 按以下四侧组织，控制请求向下流动，结果与事件向上返回：

```text
Agent 接入侧 -> 控制面 -> Runtime Adapter -> Runtime
```

四侧是逻辑责任边界，不等同于四个进程或 crate。各侧的所有权和边界见 [架构](architecture.md)。

控制请求沿四侧向下传递，结果和事件沿相反方向返回：

```text
Policy Action -> 控制面编排 -> Runtime Adapter 映射
  -> Runtime 执行 -> typed result/event -> 控制面更新控制状态
```

### 4.2 核心对象与所有权

```text
task
  -> AScoutAgent
      -> Speculative Tree
          -> Branch
              -> Branch Node
                  -> Instance
```

`task` 是 session 中一次独立的 AScout 管理单元。AScout 为每个 task 建立一个 AScoutAgent 控制域，并在其中维护一棵 Speculative Tree。task 准入时绑定 policy profile，并建立初始 Agent，称为 root Authority Instance。在运行过程中，随着投机执行的不断发生和结束，Authority Instance 不断变化，由此形成树形结构。树中的每个子树都称为 `Branch`。`Branch` 是树中的计算延续，也是投机世界的结构实体；Branch 对应的根节点称为 `Branch Node`。`Instance` 是 Runtime 中承载 Node 的实际执行单元，代表实际运行的单一 Agent。

对外，为只呈现一个单一 Agent，这个统一视图称为 `AScoutAgent`。它拥有 Speculative Tree 的控制状态和 Branch 之间的关系。由此，一个 task 对应一个 AScoutAgent，一个 AScoutAgent 拥有一棵 Speculative Tree；控制面拥有 Tree、Branch 关系和控制状态，Runtime 拥有 Instance 的执行、资源和隔离状态。

## 相关文档

- 问题与目标的完整论证见 [问题与目标](problems-and-goals.md)
- 抽象概念模型见 [设计模型](design-model.md)，逐条决策的权威出处见 [设计决策](design/decisions.md)、[投机执行底座设计决策](design/speculation-decisions.md) 与 [Branch 语义设计](design/branch-semantics.md)
- 系统结构见 [架构](architecture.md)
- 术语表见 [术语](design/glossary.md)。

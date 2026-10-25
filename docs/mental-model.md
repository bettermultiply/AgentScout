# AScout 心智模型

AScout（Agent Scout）是面向 Agent 投机执行的通用底座。Agent 可以在不修改自身执行逻辑的情况下接入，但接入实现应尽可能保持修改面小；具体产品仍可能需要 Product Integration、Hook 或其他拦截适配。AScout 分离策略决策与具体运行时实现：上层仅设计不同的投机执行策略，而不关心具体实现；下层由 Runtime 提供投机执行所需的基本能力，而不会与某个特定投机执行策略耦合。

## 1. 定位 - 为什么需要 AScout

投机执行想要解决的问题是，Agent 在执行过程中经常需要等待一个慢结果——例如，远程 API、用户决策、CI 测试——它会阻塞后续推理执行，甚至主导整个执行流程。投机执行允许在结果尚未到达时按预测提前推进后续步骤，从而加速整个执行过程。而不同投机执行策略所依赖的不同的准入、预测、验证、隔离、取消与复用等能力，正是 AScout 抽象为统一机制的那组能力。

现有投机执行工作大多针对特定执行对象或特定状态域设计，能力与任务绑定，方案之间难以组合，也无法让不同任务选择不同策略。这些[问题](problems-and-goals.md)集中表现为策略孤岛、隔离碎片化、上下层脱节三个缺口。

AScout 通过统一的投机执行机制连接策略与运行时，承接不同策略在预测、验证、拓扑和处置等方面的差异，使策略能够独立选择和组合，而不绑定具体的执行实现。投机执行涉及的不同状态与效果也通过同一框架纳入管理，并由明确的契约约束策略要求与运行时能力之间的匹配；当运行时无法满足策略要求时，AScout 不会静默改变策略语义，而是拒绝执行或进行明确降级。

落在使用上：
- 端用户提交任务并得到结果，通常无需感知背后的投机分支。
- Agent 可以在不修改自身执行逻辑的情况下接入 AScout，并获得由 AScout 和 Runtime 共同提供的投机执行能力；具体接入方式应尽可能保持修改面小。
- 部署方负责选择和配置可用的 Runtime 以及相应的 Runtime Adapter，并根据部署条件选择可运行的 Policy。
- Policy 作者只需专注于算法设计，例如如何预测、如何判断命中、如何选择拓扑、成本和处置方式，不必负责底层机制的具体实现。

AScout 不是某个固定算法，也不是替代所有 Runtime 的统一运行环境。它也不承诺每个任务必然加速；是否投机、何时投机以及如何投机，由控制面根据所绑定的 Policy 决定。投机策略能否执行以及能够达到什么保证，则取决于 Runtime 的基本能力、具体能力和 Speculation Contract。

## 2. AScout 的工作模型

投机执行由一组可组合的阶段构成，而不是固定的线性流程：机会发现 -> 待选准入 -> 启动选择 -> 预测生成 -> 过程执行 -> 验证 -> 处置与提交。AScout 可以暂存已经准入但尚未启动的机会；一个 task / AScoutAgent 绑定的 Policy 在该 task 内全部有效待选机会之间选择现在启动哪些，并可以调整已经运行或暂停的投机工作。Policy 可以选择、替换和组合这些阶段，但被启用的阶段仍需遵守必要的数据依赖和 AScout 固定的正确性、隔离、资源与副作用约束。投机执行可以基于尚未确认的 Proposal 提前开展后续工作，并在获得权威依据或其他可接受性判断后继续、转正或结束。

Policy、Mechanism 与 Runtime 构成抽象责任模型：Policy 作者只需面对一组抽象的决策点；Mechanism 横跨 Agent 接入侧和控制面，把抽象落实为执行并兜底正确性、隔离与资源约束；Runtime 是提供原生执行能力的外部后端。Speculation Contract 约束 Policy 的表达以及 Runtime Adapter 在控制面接入边界上的可观察行为；Agent 接入侧与控制面共同作为 Mechanism 的实现部分，通过拦截、编排与校验把约束落实为可执行的投机路径。AScout 不直接约束或定义 Runtime 内部实现。

工作模型的完整权威描述见 [设计模型](design-model.md) §2。

## 3. 支撑模型

AScout 用三个支撑模型承接不同投机执行策略在预测、验证、拓扑和处置等方面的差异：

- **投机边与 Policy Decisions**：投机边 Edge 是 Policy 的作用对象；开发者可以根据任务、上下文、资源和 Runtime 能力，通过组合 Edge 上的 Policy Decision 设计可替换的决策 Policy，并处理多次投机的组合与嵌套。
- **状态与效果**：把投机执行过程中需要保护的文件、进程、输入、结果和外部服务交互放入统一的管理框架，根据状态域和效果纪律选择隔离、暂存、延迟、阻塞、补偿或清理方式；Branch-local state 的环境承载由 Runtime 提供，Agent 造成且可能影响全局的外部效果（例如调用外部 API）由控制面内的 Effect Coordinator 实际处理。
- **Speculation Contract**：约束 Policy 表达与 Runtime Adapter 接入行为的语义对齐契约，使策略需求、后端接入通过同一条可检查、可执行的路径衔接，并让拒绝或降级原因可见。

支撑模型的完整权威描述见 [设计模型](design-model.md) §3；各模型的细节分别见 [Policy Decisions 设计](design/policy-decisions.md)、[效果管理设计](design/effect-management.md)、[Speculation Contract 设计](design/speculation-contract.md) 与 [控制面设计](design/control-plane.md)。

## 4. 系统结构

### 4.1 四侧架构

AScout 按以下四侧组织，控制请求向下流动，结果与事件向上返回：

```text
Agent 接入侧 -> 控制面 -> Runtime Adapter -> Runtime
```

四侧是完整调用路径中的逻辑责任边界，不等同于四个进程或 crate，也不表示都由 AScout 实现或拥有。Runtime 是外部能力来源；AScout 通过 Runtime Adapter 的控制面接入边界使用它。各侧责任见 [架构](architecture.md)。

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
  -> Branch Root
                  -> Instance
```

`task` 是 session 中一次独立的 AScout 管理单元。AScout 为每个 task 建立一个 AScoutAgent 控制域，并在其中维护一棵 Speculative Tree。task 准入时绑定 Policy 及其 task-level 配置，并建立初始 Agent，称为 root Authority Instance。在运行过程中，随着投机执行的不断发生和结束，Authority Instance 不断变化，由此形成树形结构。树中的每个子树都称为 `Branch`。`Branch` 是树中的计算延续，也是投机世界的结构实体；Branch 对应的根节点称为 `Branch Root`。`Branch Root` 具有稳定 identity 并直接拥有该根的 local state；`Instance` 是由外部 Runtime 承载 Branch Root 执行活动的实际执行单元。逻辑 Branch 可以在没有立即建立独立 Instance 的情况下先存在并推进，直到其状态或效果要求独立 Runtime 环境。

对外，为只呈现一个单一 Agent，这个统一视图称为 `AScoutAgent`。端用户通过主对话直接操作当前有效的权威执行主线；另可只读观察候选 Branch 的运行状态与明确标记为未验证的内容，以了解投机是否真实发生及其处理过程。观察不形成另一条用户对话，不表示候选已被接受，也不改变 Branch 的裁决权限或向其他 Branch 传播候选内容。一个 task 对应一个 AScoutAgent，一个 AScoutAgent 拥有一棵 Speculative Tree；控制面拥有 Tree、Branch 关系、逻辑推进和实际环境对齐状态，已建立 Instance 的实际执行、资源和隔离状态位于外部 Runtime，控制面只能通过 Adapter 观察所需结果。

用户端保持稳定的 AScoutAgent 交互入口，不绑定某个后端 Agent session。合法 Promote 改变该视图对应的权威执行及输入输出路由，不要求用户切换对话，也不以候选会话结束为前提；候选保留自己的执行状态和在飞进展，接受与效果约束仍然有效。

## 相关文档

- 问题与目标的完整论证见 [问题与目标](problems-and-goals.md)
- 抽象概念模型见 [设计模型](design-model.md)，逐条决策的权威出处见 [设计决策](design/decisions.md)、[投机执行底座设计决策](design/speculation-decisions.md) 与 [Branch 语义设计](design/branch-semantics.md)
- 系统结构见 [架构](architecture.md)
- 术语表见 [术语](design/glossary.md)。

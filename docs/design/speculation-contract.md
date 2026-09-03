# Speculation Contract 设计

Speculation Contract 是 AScout 用于规范用户自定义 Policy，并定义 Runtime Adapter 控制面接入边界的契约。它通过主动的静态校验和运行期校验，保证 Policy 意图与 Adapter 实际呈现的后端能力保持一致，避免 Policy 的制订或 Adapter 的接入结果破坏 AScout 的抽象。AScout 不通过这份 Contract 定义 Runtime 内部实现。

## 1. Contract 的定位

现有的投机执行工作大多把策略与运行时、甚至与 Agent 实现强耦合，一侧的修改常常迫使另一侧联动修改，运行环境无法被不同策略复用，策略之间也未考虑兼容性。这种耦合同时带来很高的切换心智负担：修改者必须同时了解投机策略、Runtime 如何运行、Agent 本身的执行阶段，才能判断一处改动是否会影响另一端完成正确的修改。

Contract 的首要目的，是协调与统合投机执行的策略和运行时，消除由此产生的语义错位与相互适配的困难，使 Runtime 的接入以及策略的接入、验证变得直接，且不需要联动修改。它同时规定几件事：Policy 如何表达意图，控制面如何把意图变成操作，Runtime Adapter 如何连接操作与后端，以及后端需要承载哪些实际执行状态。

```text
Policy 决策
    ↓ 只能通过 Contract 规定的决策接口表达
控制面机制
    ↓ 只能形成 Contract 规定的控制操作
Runtime Adapter
    ↓ 自行映射为目标 Runtime 的原生调用
Runtime
    ↓ 承载并返回实际执行状态
控制面状态更新
```

Contract 是上述约束的总称，实现这些约束的抽象责任称为 Mechanism。Mechanism 由 AScout 自身能实现、能强制的固定机制共同构成，不归属控制面内的任何具体组件，而是组件协同完成检查和翻译的过程。

Mechanism 的责任包括如何校验 Policy 和 Runtime 能力的匹配，以及如何将 Policy 翻译成按时序排列的 Adapter 必须实现并提供的接口组合。前者是校验 Policy 期望的隔离度能否被当前接入的 Runtime 所承载，也就是 Policy 的声明与 Adapter 报告的能力是否匹配，并在运行期根据真实结果确认声明是否达成。后者是把 Policy 的高层决策展开为按固定时序的受管操作，并维护这些操作之间的状态与不变量。Mechanism 先把它们编排成受管语义操作序列，再由 Adapter 按 Contract 规定的同一组操作，把每个语义操作映射为后端行为。

Contract 直接约束 Policy 的表达，以及 Runtime Adapter 在控制面边界上声明和返回的可观察行为。Runtime 是 Adapter 使用的外部能力来源，不是 Contract 能直接约束的实现对象。控制面是 Contract 的实现方，属于 Mechanism 的一部分，不是被 Contract 约束的对象；控制面如何组织校验与编排，见 [控制面设计](control-plane.md)。

## 2. Contract 约束什么

Contract 本质上规定 Policy 能表达什么，以及控制面接受怎样的 Adapter 能力、结果和事件。它通过受限入口和校验使策略表达与后端接入对齐。Contract 的固定语义包括以下相互联系的内容：

- Policy 已经通过 Policy Decision 约束其设计空间，但 Decision 取值的合法性仍需校验。同时随着 Runtime 的变化，其能提供的隔离能力也会有所不同，这就造成了可能出现语义不匹配，同样表现为 Decision 取值的合法性。
- Runtime Adapter 由固定的接口约束其输入输出，但相同接口约束下能提供的隔离度也并不相同，真实的隔离度需要进行校验。
- 失败时如何收敛：对尚未被接受的投机 Branch，Policy 与 Runtime 把它视作在 Speculative Tree 中从未被创建，且不重放该操作。被接受的 Branch 已完成使命，不适用这一语义。

## 3. 如何约束

Contract 的约束只有落实到固定执行点才具有约束力。AScout 不要求 Policy 或 Adapter 通过自觉遵守一份文档来获得正确性，而是让它们只能通过受限制的接口进入执行路径。

### 3.1 对 Policy：决策取值合法性的校验

Policy 通过 Policy Decision 表达策略，其设计空间已经被约束，但具体取值是否合法仍要由控制面在决策进入执行路径前主动校验。Policy Engine 检查的对象包括决策的输入输出、决策之间的组合、所依赖的验证依据，以及 Policy 对效果、资源和 Fidelity 作出的声明。

校验的合法性是相对当前 Runtime 的。同一个 Decision 取值在某个后端上可行，在另一个只能提供更弱隔离能力的后端上就不可行，这种语义不匹配同样表现为取值不合法。例如，Policy 要求一个需要隔离和清理能力的候选执行，而当前 Adapter 报告的后端能力不足以承载时，该取值没有合法路径。若 Policy 的目标超出 AScout 固定语义，或当前后端无法提供所需能力，控制面必须拒绝该决策或按既定语义报告降级。

检查通过后，Policy 的结果仍只是策略意图。实际 Branch、生命周期、效果与 Runtime 操作由控制面固定机制形成，Policy 不能通过返回一个特殊值绕过控制面状态转换。

### 3.2 对 Runtime Adapter：接入校验与运行期校验

Adapter 由固定接口约束其输入输出形态，但相同的接口约束下，后端实际能提供的隔离度并不相同。投机执行最终要改变真实执行环境——创建和停止进程、管理 workspace 与资源、隔离不同 Branch、处理影响 Branch 外部的效果——这些语义能否达成，取决于 Adapter 能用后端原生能力组合到什么程度。

AScout 不要求后端是专为 AScout 设计的 Runtime。Runtime 可以是通用执行后端，例如容器、虚拟机或一组本地进程管理能力。因此，Adapter 需要先声明它能在控制面边界上提供什么能力，以及它承诺用后端原生能力组合出 AScout 基本操作语义后能保证多大程度的隔离。Adapter 内部如何完成组合不由 Contract 规定。

但往往 Adapter 开发者本身也无法完全保证声称的隔离度符合需求，所以会通过仅接入时进行一次的基本语义测试，来规范其是否正确实现了所声明的隔离状态。接入校验的对象是 Adapter 而不是 Runtime 本身：控制面无法越过 Adapter 直接触碰后端，因此只要 Adapter 能忠实表达基本操作语义，就代表它背后的 Runtime 具备接入资格；反过来，一个后端能力再强，如果 Adapter 表达不出这些语义，也不能接入。无法完成这些语义时，该 Adapter 没有完整接入路径；对于基本语义之上的附加能力，Adapter 报告支持范围和限制，供 Policy Engine 判断具体策略是否有可行执行路径。

同时由于基本测试难以覆盖所有场景，同样会通过运行期校验来兜底：运行期间根据实际操作和事件确认 Contract 是否真的被兑现。运行期需要产生的至少包括 Policy 决策及其上下文、Branch 与 Proposal 的状态变化、验证依据、效果与 obligation 的处理结果、资源使用、取消与清理结果，以及 Adapter 映射或 Runtime 执行失败。Ledger 以异步旁路记录这些事实，用于状态对齐、问题定位、shadow/replay 和契约校验。接入时检查只能确认声明在形式上相容，运行期校验才能发现"声明与实际不符"并触发对应处理。

运行期间，Adapter 不能静默放宽 AScout 语义。它把 discard 翻译成后端真正停止对应执行并处理资源与效果的动作，而不是降成"删除一条记录"；把 accept 翻译成后端完成对应状态转换，而不是只返回"请求已发送"。后端确实无法完成某个语义时，Adapter 应返回明确失败或部分完成结果。

### 3.3 失败如何处理

跨层操作可能在不同位置失败：Policy 可能无法产生有效 Proposal，控制面可能发现当前状态不允许某项转换，Adapter 可能无法把语义操作映射到后端，Runtime 可能在执行中遇到资源或清理错误。失败结果需要说明操作处于什么状态、接下来如何收敛，不能让某一层用成功值掩盖。

对尚未被接受的投机 Branch，失败收敛到"该 Branch 从未被创建"：不保留它的部分结果作为成功，也不重放该投机操作。控制面沿固定状态入口撤销对应 Branch 的控制状态，并请求 Runtime 停止对应执行、处理资源与受管效果。Salvage 是可选的残值回收，可在这一默认之上把可移交权威路径的部分进展保留下来，不是失败处理的必要条件。

## 4. 完整控制闭环

一次投机执行分成两段：**创建投机 task 时的静态校验**只进行一次；之后投机内部每次具体操作运行遵循同一个闭环，运行中不再重复前置静态校验，而是以执行后的结果确认。

创建投机 task 时，做一次静态校验：

```text
Policy 产生投机决策（predict / admit / ...）
    → Policy Engine 校验决策合法性（表达、组合、声明相容）
    → 通过则建立 candidate Branch 的控制状态并创建受管执行
    → 不通过则拒绝或明确降级，不创建
```

这次校验确认 Policy 的决策在表达上合法、与 Adapter 声明的能力在声明层面相容（见 §3.1、§3.2）。它只证明"这个投机值得尝试、按声明应当可行"，不证明运行中会真的按语义完成。

投机 task 建立后，内部的具体操作（一次读取、一次工具调用、一次效果处理等）遵循运行闭环：

```text
Policy 产生推进或处置决策
    → 控制面机制检查状态并形成语义操作
    → Runtime Adapter 检查并映射操作
    → Runtime 执行操作并返回结果或事件
    → Runtime Adapter 规范化结果
    → 控制面根据结果更新控制状态
```

这里的"控制面机制检查状态"是状态机层面的前置条件检查（该操作在当前控制状态下是否允许），不是重复创建时的静态校验。一个操作是否真的按语义完成，要到 Runtime 实际执行并返回结果或事件后才能确认，控制面据实更新控制状态。

以接受和丢弃为例，Authority 或 validator 提供依据后，控制面通过 `match` 和 `dispose` 得到处置意图。处置意图经过固定状态入口检查后，转换成 `accept` 或 `discard` 操作。Runtime 处理相应的执行、资源、效果和清理，Adapter 返回结果，控制面再更新 Branch 和 Proposal 的状态。这样，"Policy 选择了一个候选"和"候选已经成为可继续使用的执行"之间有清楚的操作和事件边界。

## 相关文档

- [控制面设计](control-plane.md)：控制面如何作为 Mechanism 落实 Contract
- [Policy Decisions 设计](policy-decisions.md)：Policy 决策面
- [Branch 语义设计](branch-semantics.md)：Branch 生命周期
- [效果管理设计](effect-management.md)：状态与效果
- [架构](../architecture.md)：四侧所有权

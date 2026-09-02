# Speculation Contract 设计

本文定义 Policy 与 Runtime 之间的 Speculation Contract 目标语义。Contract 是双向能力匹配机制，不是新的生命周期模型，也不是某一种固定的投机算法。

## 1. 目的

投机策略的成立依赖两类条件：上层策略对预测、验证和失败的要求，以及下层 Runtime 对隔离、效果和资源的实际支持。Contract 把这两类条件显式化并进行匹配，避免策略在能力不足的 Runtime 上被静默执行，并使拒绝或降级的原因可解释、可审计。

Contract 只描述“策略要求”和“执行能力”是否匹配，不重新定义 AScout 的 Branch、生命周期、状态所有权或效果语义。

## 2. 双向声明

### 2.1 Policy 声明

Policy 注册时声明一个策略 profile 的最低要求和允许范围，至少包括：

- 预测要求：Proposal 的来源、置信度或数量要求；
- 验证要求：需要的 Authority 输入、validator 能力、验证条件和时机；
- 失败代价：可接受的浪费、取消、重执行或其他失败成本；
- 隔离要求：投机工作所需的隔离粒度；
- 效果纪律：允许使用的 `EffectClass × StateDomain` 组合；
- Fidelity 声明：`Trajectory-equivalent`、`Outcome-equivalent`、`Bounded-degradation` 或 `Best-effort`。

Policy 声明的是需求和承诺，不是对 Runtime 内部实现的指定。

### 2.2 Runtime 能力声明

Runtime 通过 Runtime Adapter 声明当前后端可以提供的能力、成本和限制，至少包括：

- 可用的隔离粒度和状态域；
- 可执行的效果纪律及其限制；
- fork、暂停、恢复、抢占、取消和清理等机制能力；
- 资源预算、资源压力和能力对应的成本；
- 可支持的验证、Authority 输入和结果交付路径；
- 能力的限制条件、保证范围和降级方式。

Runtime 能力声明必须描述实际可提供的机制，不得把未来能力或未验证能力当作已满足。

## 3. 匹配与校验

控制面负责协调 Contract 校验，Runtime Adapter 负责把 Runtime 的能力转换成稳定的声明。校验至少分为三步：

1. **静态约束检查**：比较 Policy 要求与 Runtime 能力，推导当前组合允许达到的 Fidelity 上界，并检查效果、隔离、验证和资源约束。
2. **生成已验证 profile**：匹配成功后生成绑定了 Policy、Runtime 能力和约束结果的 policy profile，供任务准入和运行期组件使用。
3. **处理不匹配**：无法满足最低要求时拒绝加载或拒绝任务准入；如果策略声明允许较低保证，则生成明确标注降级原因和实际 Fidelity 等级的 profile。

静态检查只能证明能力上界，不能证明运行中的实际行为已经达到声明等级。

## 4. 运行期校验与审计

运行期间，底座记录与 Contract 相关的事实，包括 Authority 是否提供输入、validator 的判断、效果处理结果、资源使用和最终处置。Ledger 以异步旁路记录这些事件，不改变关键路径上的控制结果。

Shadow/replay 用于把声明的 Fidelity 与实际行为进行对照：shadow 同时运行投机路径和串行基准，只提交基准结果；replay 用于确定性重放和问题定位。审计发现声明无法持续满足时，系统应暴露偏差并触发既定的降级、停用或重新配置流程；具体 kill-switch 和运营策略不由本文件固定。

## 5. 结果与边界

Contract 校验的结果是以下之一：

- **Accepted**：Policy 要求与 Runtime 能力匹配，按声明 profile 运行；
- **Degraded**：只能满足较低 Fidelity 或受限能力，profile 必须携带可见的降级原因；
- **Rejected**：无法满足最低要求，不能加载或不能准入。

Contract 不决定何时 `branch`、`commit`、`accept` 或 `discard`，不持有 Branch 状态，不替代 Assumption Manager、Scheduler、Effect Coordinator 或 Runtime 的职责。

## 相关文档

- [Branch 语义设计](branch-semantics.md)：Branch 生命周期语义
- [设计决策](decisions.md)：决策速查
- [效果管理设计](effect-management.md)：状态与效果
- [架构](../architecture.md)：系统结构

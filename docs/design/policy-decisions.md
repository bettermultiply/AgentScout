# Policy Decisions

Policy Decision 是 Policy 根据任务、执行上下文、可用信息、资源条件和 Runtime 能力作出的可替换决策。Policy Decisions 定义策略选择空间；它们不是 Runtime 状态，也不是可以绕过 AScout 固定语义的自由执行脚本。

## 阶段映射

一次投机执行可以按以下阶段理解。表中的决策是当前最重要的代表性内容，不是完整清单。

| 阶段 | 代表性 Policy Decisions |
|---|---|
| 机会发现与 Site 注册 | 观察对象、识别信号、是否登记投机 Site |
| 准入与权威启动 | 是否投机、何时启动权威路径、预算与资源上限 |
| 预测生成 | Predictor、Proposal 数量、置信度与来源 |
| 拓扑展开与投机执行 | candidate 数量、深度、扩展时机、执行优先级 |
| 权威获取与验证 | Authority 来源、Equivalence 层次、验证时机与条件 |
| 处置与提交 | Reuse、Promote、Discard、Preempt、Re-execute、Compensate、Salvage 等 |

机会发现与 Site 注册是 Policy 与 Mechanism 的接缝：Policy 可以决定观察和提议方式，但 Site 的身份、admissibility 和后续生命周期必须由 Mechanism 校验。准入、资源、效果和 Speculation Contract 也会限制各项决策的可行范围。

## 语义对象

Policy Decisions 作用于一组核心语义对象：

- **Proposal**：投机路径使用的预测值。
- **Authority**：产生真实结果或权威判断的主体；Policy 可以选择或等待其来源，但实际执行可能由 Runtime 提供。
- **Equivalence**：判断 Proposal 与权威依据是否兼容的关系。
- **Disposition**：验证后对投机工作采取的处置意图；实际清理和状态转换由 Mechanism 强制完成。

这些对象属于 AScout 的执行语义，不等同于 Policy 组件，也不表示其状态全部由 Policy 持有。

## 可扩展性

Policy Decision 集合不固定为上表六项。后续可以加入 resolver delegation、等待与恢复、失败 fallback、调度与优先级、Runtime Service 选择、效果 materialization 等决策。新增决策必须说明所属阶段、输入输出和与既有决策的依赖，并遵守 AScout 的 Branch、single-winner、隔离、capability、obligation、commit/discard/accept 和 cleanup 语义。

## 分层边界

- Policy 选择和组合决策，不拥有 Runtime mutable state，也不能伪造 Runtime 已完成的操作。
- Mechanism 校验决策是否可执行，并强制正确性、隔离、资源、副作用和生命周期不变量。
- Runtime 提供具体后端原语；后端能力不足时，控制面 必须拒绝、降级或选择其他合法决策，不能静默改变 Policy 语义。

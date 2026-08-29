# 效果管理设计

本文承载 AScout 效果管理的详细语义。心智模型只保留 [状态与效果](../mental-model.md) 的高层说明。

## EffectClass

EffectClass 描述效果在投机期间可以采用的纪律：

| EffectClass | 含义 |
|---|---|
| PURE | 无外部效果，允许直接投机 |
| IDEMPOTENT | 重放收敛到同一状态 |
| STAGEABLE | 效果可缓冲，在 commit barrier 后释放 |
| TRANSFORMABLE | 存在非变异的等价执行（dry-run / staging） |
| COMPENSABLE | 事后可补偿（refund / replace） |
| OVERWRITABLE | 允许 last-write-wins 收敛 |
| IRREVERSIBLE | 不可安全投机，必须硬闸 |

## StateDomain

StateDomain 描述效果所作用的状态范围。EffectClass 与 StateDomain 正交，二者的组合决定可接受的隔离和释放方式：

| StateDomain | 典型纪律 | 隔离或适配方式 |
|---|---|---|
| filesystem | STAGEABLE | overlayfs / union mount |
| process, memory | STAGEABLE | namespace / COW fork / VM 快照 |
| GUI | TRANSFORMABLE | 影子渲染 / 离屏合成 |
| network (read) | PURE | 直通 |
| network (write) | TRANSFORMABLE / STAGEABLE | 录制回放 / 代理拦截 |
| database | STAGEABLE | 未提交事务 / savepoint |
| email, payment | COMPENSABLE / IRREVERSIBLE | outbox 延迟投递 / 硬闸 |

具体组合不是静态白名单的替代品。Policy 的效果选择必须经过 Contract、资源和 Runtime capability 校验；不能安全处理的组合必须拒绝或降级。

## 提交与可见性

可缓冲效果在 **commit barrier** 后才释放。外部效果只在 service adapter 声明的 **observation surface** 与 **guarantee horizon** 内获得 sibling non-interference 与 winner materialization 保证。真实 staged effect、obligation 和 materialization 状态属于 Runtime Mechanism；控制面 Effect Coordinator 只持有效果分类、计划和协调视图。

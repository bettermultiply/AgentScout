# Authority 物化提案（仅设计，未实现）

## 背景

当前文件系统 Backend 的 accept 把结果写入目标环境的 upper；权威环境（authority）
的写入也停留在 authority 的各个挂载 upper 中。宿主真实 workspace 不会自动收到
这些写入。只要 authority 仍存活或仍有投机分支引用其状态，就不能把结果直接写回
宿主 Base；否则后续 fork/diff 的基准会被宿主变化污染。

本提案描述一个未来的 materialize 操作，本轮不实现。

## 触发条件

用户确认的触发偏好（不代表本轮实现）是：

- 默认在 Agent 等待用户输入，或达成阶段目标而停止活动时触发；
- 更积极的模式可在没有存活的投机分支时提前触发，以分摊物化成本。

“没有存活的投机分支”不等于“没有写者”。无论采用哪种触发偏好，物化只能在
authority 已静止并完成写者协调后发生：

1. authority 环境处于 `Suspended`，ProcessBackend 已真实冻结并确认
   `writers_quiesced=true`；或 authority 没有运行进程且 filesystem 状态为
   `Suspended`。
2. 所有 candidate/投机分支已 discard 或 terminate+cleanup；没有其它环境把
   authority 的 snapshot/upper 作为 base 或 lower 引用。
3. 控制面已经确认 authority 的结果是本轮最终结果，并且不会再有新的 fork。
4. 宿主侧没有未协调的并发写入，或已取得宿主 workspace 的独占锁（部署层责任）。

建议由**控制面**决定触发，因为它拥有 Branch 生命周期与“最终权威”语义；Runtime
Adapter 只执行 `FilesystemMaterialize` 并报告结果。Runtime 不应自行猜测何时物化。

## 操作语义

对 authority 环境的每个挂载点：

1. 把 authority 挂载的 **upper 视图**作为 source。
2. 把宿主真实挂载点作为 target Base。
3. 以 authority 进入投机前的快照（authority base）作为三方合并的 common ancestor。
4. 将 source 的增量重放到宿主 Base：
   - 创建/修改：写入宿主对应路径；
   - 删除：从宿主删除；
   - opaque/redirect/metacopy：先按 overlay 语义还原，再删除/替换宿主内容。
5. 每个会覆盖或删除宿主内容的操作，先写入同一套 accept replay journal
   （reflink 优先；不支持时复制；>64MiB 单文件告警），以支持逆序回滚。
6. 全部挂载都成功后，清理 journal；任何挂载失败都逆序回滚全部已应用操作。

物化成功后的权威环境如何处置由控制面决定（建议保留到下一次显式 cleanup，或标记
`materialized`；不要在物化操作内部删除 authority，以免丢失回滚依据）。

## 契约变更建议

不要在 `FilesystemAccept` 上偷偷加“写宿主”语义。建议新增独立操作，避免调用方
误把普通 accept 当成物化：

```rust
Operation::FilesystemMaterialize {
    environment_id: String,       // authority environment
    base_snapshot_id: String,     // authority 进入投机前的基准
    path_scope: Vec<PathScope>,   // 默认全部挂载
}
```

`BackendContext` 继续携带 operation id/generation，用于幂等重放；不需要
`target_mount_pid`，因为物化目标是宿主 Base，而不是 live sandbox mount。

`FilesystemResult.confirmation` 建议新增：

```rust
materialized: bool,
host_base_updated: bool,
```

或复用 `published` 并在文档中明确其含义是“已写入宿主 Base”。两者只能选一个，
避免同时出现两个近义字段。

与 `FilesystemAccept` 的关系：

- accept 只改目标 Backend 环境的 upper/live mount，不碰宿主 Base；
- materialize 是唯一允许把 authority upper 重放到宿主 Base 的操作；
- 两者共享三方合并、journal 和回滚实现，但触发条件、target 和确认字段不同。

## 失败与回滚

- journal 记录宿主 Base 的原内容，重放失败时按逆序恢复，报告未恢复路径。
- 物化不是原子的：宿主读者可能在中间态看到部分结果；只能在文档中写“可回滚、
  非原子”。
- 回滚失败时 `materialized=false`，返回 `cleanup_failures`，并保留 journal
  目录供人工恢复。
- 物化成功后删除 journal，但保留 authority 环境直到控制面确认。

## 宿主并发修改的冲突处理

宿主 Base 可能在投机期间被编辑器、git 或用户修改。物化必须检测：

1. 用 authority base 快照记录宿主在投机开始时的状态（每个挂载一份）。
2. 对每个 source delta 路径，比较宿主当前内容与 authority base：
   - 宿主未变：直接应用；
   - 宿主已变且与 source 相同：视为已满足；
   - 宿主已变且不同：产生冲突，默认**不写**该路径并报告冲突；不得静默覆盖。
3. 冲突策略由控制面决定：失败并保留 journal、人工合并、或按 Policy 选择
   authority-wins。默认建议 fail-closed。
4. 宿主目录 rename/删除跨越多个路径时，冲突检测按路径集合处理，不能只看顶层。

部署层仍应提供宿主 workspace 锁；文件系统 Backend 只能检测，不能阻止其它进程
在检查与写入之间修改宿主文件。

## 对 authority 行为的影响

物化前，authority 行为与无投机执行的差异是“结果只存在于 upper，宿主观察者看不到”。
物化后，宿主 Base 收到 authority 的最终视图，才真正接近“无投机直接运行”的外部
可见性。物化仍然不是原子的：中间态可见，且失败时可能已经部分写入再回滚。

## 仍需用户决定的问题

1. 物化由控制面、Runtime 还是显式用户命令最终发起（触发偏好已记录）？
2. authority 在物化后是否保留？保留多久？谁负责最终 cleanup？
3. 宿主并发修改默认策略：fail-closed、人工合并，还是 Policy 可配置 authority-wins？
4. materialize 是否复用 `FilesystemResult.confirmation.published`，还是新增
   `materialized` 字段？
5. 是否需要为物化新增专门的宿主 workspace 锁契约，还是由部署层保证？
6. 多挂载物化时，是否允许部分挂载成功、其余冲突后整体回滚？本提案建议整体回滚。

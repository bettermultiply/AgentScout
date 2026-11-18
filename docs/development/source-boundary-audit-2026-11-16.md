# 源码责任边界核查（2026-11-16）

本次执行 Discussion 第 13 项，对照 [心智模型](../mental-model.md) §4、[架构](../architecture.md) §1.1／§3.1、[control plane 设计](../design/control-plane.md) §2.1／§2.4、[Runtime 设计](../design/runtime.md) 和 [runtime proxy 契约](../design/runtime-adapter-contract.md) §1／§3.1。范围为三个顶层模块的公开接口、生产依赖、三侧 Gateway 与三个二进制的装配责任；不重新设计目录、技术选型或部署，也不等于完整架构或全部契约验收。

## 核查结果与证据

| 范围 | 当前源码证据 | 判断 |
|---|---|---|
| 顶层与协议 | `src/lib.rs` 仅公开 control_plane／protocol／runtime；`protocol/{contract,transport}.rs` 只使用标准库及 serde／serde_json | 生产依赖方向符合架构；协议不读取 Tree 或 Backend 实现。Agent 线消息定义在 agent_control，不能误称所有线协议都已集中到 protocol。 |
| control plane → Runtime | `gateway/runtime.rs::Runtime` 只传 Operation／OperationResult／Event，coordination 使用该 trait；control_plane 不引用 runtime 模块 | 通过契约使用 Runtime，没有直接访问 Backend、进程句柄或挂载状态。 |
| Runtime → control plane | `RuntimeProxy` 依赖 protocol 和 `BaseRuntime`，`BaseRuntime` 持有有序 Backend 实例；runtime 的测试使用 control plane 会话驱动请求 | 生产实现没有反向依赖 control plane。测试借用真实控制会话驱动 runtime proxy 是测试装配，不是生产依赖。 |
| Gateway | agent 管连接与线消息；runtime 管请求、结果缓存与事件；ui/http 管 HTTP/SSE 与 Presentation | 不持有 Tree／Policy／调度预算。Tree 合法性检查仍在控制逻辑；LedgerReader 是只读事实访问，不能算第二份 Tree。 |
| Runtime 二进制 | `ascout-runtime-adapter/config.rs` 读取并规范化 JSON，登记实现后调用 `BaseRuntime::from_registry` 并装配 `RuntimeProxy`。`ascout-guest-agent.rs` 调用 `guest_agent::serve` | 配置读取归入口，实例创建和宿主、环境资源的协调归 base runtime。 |
| control plane 二进制 | main 调用 `coordination::session::{prepare,start,connected}`；execution 调用 `Session::{request,interaction,agent,timer,progress,settle,fail_and_exit}` | root／恢复／Agent 启动、本地 Policy 交付、控制请求、模型切换及保存／物化协调已进入库接口。入口仅解析与装配、接收事件、派发异步工作、送达命令及呈现／写 Ledger。 |
| Tree 公开接口 | `SpeculativeTree` 与 `BranchRecord` 的领域字段均不再为 pub；只读方法返回借用／值，无公开可变 getter | 二进制及外部调用方不能直接赋值或取得内部可变记录。身份设置经 `set_execution_id` 限制在启动／恢复阶段；Environment 仍经 `bind_environment` 检查，转换继续使用 Tree 方法。库内协调及恢复使用 crate 可见性，序列化格式未变。 |
| workload 边界 | `chances.rs::record_public_nle_map_action` 过滤公开 chars／blstats／message，发布 NleMapAction；`coordination/actions.rs` 调用它 | 是 [现行地图实现](nle-map.md) 已明确的只读事实投影，不计算移动判断、不裁决 Policy；不能把这处已有展示适配擅自扩展成通用新协议或视为必须移除的运行策略。 |

公开 API 是库面向各二进制／外部调用方的接口，不是 UI 可直接访问 Tree 的授权。HTTP/SSE 继续通过受限观察接口。恢复反序列化保留原格式和既有校验，不通过重新绑定改变保存身份。

## 完成的边界实现

增加 `SpeculativeTree::bind_environment`，统一检查 Branch 存在、返回身份非空、身份未被其他 Branch 绑定、目标 Branch 尚无绑定。root 建立结果和 Candidate 成功／失败创建结果共用该方法；替换 main 对 `branches.get_mut(...).environment_id` 的直接赋值以及 Candidate 路径重复的检查和赋值。不预定身份，不把绑定等同于环境已 Ready，不改变 Runtime 实际启动 Agent 的责任。创建失败返回了环境身份时仍先保存身份，后续清理继续使用它。

`coordination/session.rs` 负责已有 session 的语义协调，不保存第二份 Tree：`prepare` 读取恢复点或建立树；`start` 登记 root／加载 Runtime、绑定实际环境并要求 Runtime 启动 Agent；连接接纳后 `connected` 记录 Ready 并按 LocalPolicy → StartBranch／Load 顺序返回命令。AgentLaunch 的产品路径、参数和配置仍由入口装配，实际进程仍由 Runtime 启动。

`Session` 统一处理 UI 请求与 Agent 事实，保持恢复暂停／继续、交互应答合法性、取消、在飞工作／Promote 期间禁止模型切换、后代保留自身模型等既有规则。保存／物化成功失败与结束前提由 `settle` 协调；二进制只送达返回命令、唤醒事件循环和关闭连接。Tree／BranchRecord 的只读访问及字段可见性收口已完成。未新增测试或项目术语。

## 范围与限制

- 第 13 项以上实现缺口已完成并从根 Discussion 删除；不把此结论扩大为所有控制语义与 Runtime 契约已经全面验收。
- backend registry 已承担本地实现登记与接口检查。独立能力验证及其状态仍待实现，Hello 只报告实例声明。该能力缺口由 [Runtime 接入](runtime-adapter.md) 与 Discussion 第 17 项承接，不在本次另造 Registry。
- Gateway 与 UI 的部署及统一启停以[control plane §2.1](../design/control-plane.md#21-gateway)为准；实现及关闭验证见 [Gateway 核查](gateway.md)。Adapter 部署及配置组织仍由 Discussion 第 15 项承接，不因 Gateway 的生命周期结论关闭。

本次继续实现开始时已有 40 个受跟踪文件的未提交差异及未跟踪文件。编辑前已查看相关 diff；全部使用局部 patch 整合，保留交互接口新增路径以及进行中新增的旧 Authority 提示取消修正，未提交、切分支、reset 或覆盖并行文档重命名。仅格式化新增文件及自己的编辑范围，没有全局重写。未调用真实模型，未做深度回放或 Firecracker 环境修复。

## 本轮完成验证

| 命令 | 结果 | 日志 |
|---|---|---|
| `cargo check --all-targets` | 通过 | `/tmp/opencode/boundary13-check.log` |
| `cargo build --bins` | 通过，整链回归使用重建的二进制 | `/tmp/opencode/boundary13-build.log` |
| `cargo test --lib` | 171 通过，覆盖 control plane、恢复与 Runtime | `/tmp/opencode/boundary13-lib-tests.log` |
| `cargo test --lib --test policy`（最终复核） | 171 项库测试及 1 项 Policy 集成测试通过 | `/tmp/opencode/boundary13-final-tests.log` |
| `node --test tests/pi-access.mjs tests/pi-recovery-binding.mjs tests/pi-recovery-sources.mjs` | 8 通过；真实 Pi 协议、资源绑定与独立恢复源，无真实模型 | `/tmp/opencode/boundary13-pi-tests.log` |
| `node --test tests/recovery-active.mjs tests/recovery-filesystem.mjs` | 6 通过；在飞工具和候选、保存失败重试、继续、多 Backend、文件系统恢复及代码变化拒绝；真实 Pi／Runtime，本地伪模型 | `/tmp/opencode/boundary13-recovery-tests.log` |
| `cargo test` | 库／二进制测试通过；Firecracker 集成 7 项因当前绝对 Unix socket 路径过长在建立环境阶段失败，未到达模型请求，不声称全量通过 | `/tmp/opencode/boundary13-all-tests.log` |
| `git diff --check` | 通过；未执行全局格式化 | — |

以上结果基于共享工作区，不证明 HEAD 单独具备这些能力，也不替代完整架构验收。

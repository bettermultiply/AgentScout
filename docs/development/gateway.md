# Gateway 实现核查

本轮异步反馈、接收回执、取消与发送失败清理及有界身份窗口见[异步反馈与接收确认](feedback-delivery.md)。Agent send 等待 received，实际 Pi 启动器和公共接入层均已支持 delivery；Runtime 与 Pi 均明确拒绝已退休身份。恢复 retry/continue 两项失败及其证据也记录于该文档，不能视为全部验收通过。下文旧阶段的“发送 5 秒”只描述每次写入超时，不能作为当前交付成功定义。

目标依据：[control plane §2.1](../design/control-plane.md#21-gateway)。Gateway 属于 control plane，不独立部署，由 control plane 统一启动和停止；UI 同样由 control plane 启动和停止。Gateway 与 Runtime 网络通信；Agent 与接入进程的实际启动、监视和回收仍由 Runtime 执行。不规定必须同进程或自动打开浏览器，不实现 Registry，不改变投机语义。

## Agent 协议与 Tree 的源码边界

`src/control_plane/gateway/agent/protocol.rs` 定义 Agent 消息、命令、握手能力和线封装。Gateway 校验握手、收发消息并处理交付回执；`src/control_plane/speculative_tree/messages.rs` 检查并应用影响 Tree 状态的消息。此次迁移未改变线格式、快照格式或交付行为。`cargo check --all-targets`、控制面库测试（60 项）、控制面二进制与 Ledger 投影测试（6 项）、交付与恢复集成测试（9 项）通过；模型响应均来自本地伪模型。

## 2026-11-16：统一启动和停止

当前实现仍由 `src/bin/ascout-control-plane.rs` 装配三侧通信，不增加统一总对象或重写协议：

- `gateway::ui::start` 返回 `Server`，control plane 持有并显式 `stop`；Drop 也停止。停止标志唤醒 SSE 的 Ledger 等待，不产生观察事件、不改变 Ledger 的 connected 事实。listener 释放后，中断空闲／不完整 HTTP 读取，再等待全部连接线程；已完成线程在运行期回收，不积累到退出。写入仍保留两秒超时。正常观察断开后仍交付末尾事件；运行中直接停止只结束通信，不伪造业务完成。
- listener 使用 nonblocking accept，每 20ms 检查停止标志；这是通信停止检查，不驱动 Tree／Policy。Ledger 增加内部可取消等待及唤醒入口，原 `events_after` 的序号、补发与 15 秒等待不变。
- `ControlSession` 保存 Runtime 读线程句柄；shutdown 关闭 cloned socket 两侧并 join，重复 shutdown 和 Drop 可重复调用。EOF／读错误唤醒等待者的逻辑在本轮编辑前已存在，不计作本轮新增能力。
- Agent Connection 原已有双向关闭、读线程 join，以及 finish 写侧半关闭／stop_receiving 的分离，本轮沿用。Agent listener 由作用域释放；运行目录及 socket 路径仍留给调用方检查，不扩展为目录清理。
- Adapter 接入等待发生 accept／try_wait 错误时 kill/wait 自己启动的进程；握手失败仍按既有路径清理。成功握手后的能力检查、Agent 路径解析、日志打开等全部 fallible 装配步骤纳入同一退出路径，先关闭 Runtime 会话并 join，再等待 Adapter 自行退出，五秒未退出则强制回收。运行阶段继续先请求 Runtime 清理环境、发布断开事实、停止 UI，再关闭 Runtime 会话。没有把 Agent 进程所有权移给 control plane。

编辑前快照及基线：`/tmp/opencode/gateway-lifecycle-20261116-214948/`，`before/` 保存相关文件，`baseline.diff`、`baseline-staged.diff`、`baseline-status.txt` 记录已有工作。该工作区同时有其他修改，当前总 diff 不等于本轮增量；该目录的 `incremental.diff` 及 `incremental-stat.txt` 是快照至结束的文件差异，也含期间并行修改，须结合 `scope.txt` 审查，不能整体归功或整体应用。不提交、不切分支、不还原其他改动。

本轮没有新增仓库测试，也没有调用真实模型。手构造探针仅在 `/tmp/opencode/`，依赖当前生产库。

| 验证 | 本轮结果／证据（日志均在 `/tmp/opencode/`） |
|---|---|
| `cargo check --all-targets` | 最终通过，`gateway-lifecycle-check-final.log`。第一次因并行工作中的 tests/commands.rs 缺 json! 导入失败，本轮未修该文件；后续工作区变化后重跑通过。 |
| `cargo check --lib --bins`、`cargo build --bins` | 通过；`gateway-lifecycle-production-check.log`、`gateway-lifecycle-build.log`。 |
| `cargo test --lib control_plane::gateway` | 2 通过，`gateway-lifecycle-tests-final.log`。 |
| `cargo test --lib` | 172 通过，`gateway-lifecycle-lib-tests.log`；不归功于本轮新增测试。 |
| Pi 既有接入测试 | `node --test tests/pi-access.mjs tests/pi-notification-adapter.mjs`，15 通过，`gateway-lifecycle-pi.log`。 |
| 既有恢复测试 | `node --test tests/recovery-active.mjs`，5 通过，本地伪模型／真实 Pi、Runtime 和 NLE，含文件系统组合、保存重试及恢复清理；`gateway-lifecycle-recovery.log`。这些测试用 SIGKILL 结束暂停后的 control plane，不代替 graceful Shutdown 验证。 |
| HTTP/SSE／Runtime 手构造通信 | `gateway-lifecycle-probe.log`：活 SSE、空闲 HTTP 下显式 stop、重复 stop、Drop 均退出约 10ms，UI 端口可立即重新 bind；Runtime 显式 shutdown、重复 shutdown、Drop 均 join 返回且对端收到 EOF。源码 `gateway-lifecycle-probe.rs`，临时 Cargo 工程 `gateway-lifecycle-probe/`。未改 HTTP 校验、状态码和正常 SSE 补发循环。 |
| 正常 main 退出 | 快照目录 `normal-exit/result.txt`、`stderr.log`：无模型请求的空闲会话，HTTP Shutdown 返回 202，control plane 退出 0，所属进程消失，UI 端口重新 bind 成功。 |
| main 装配错误退出 | 快照目录 `early-error/`：假 Adapter 完成 hello 后，缺失 ASCOUT_AGENT_ENTRY 导致非零退出；Adapter 收到 EOF，PID 已回收。Runtime 端口的立即 bind 检查遇到 Address already in use，不据此宣称立即复用通过（TCP 连接关闭状态未排除）。该例不建立 Agent 或环境。 |
| 格式检查 | `gateway-lifecycle-local-fmt.log`：main、Runtime／HTTP／Ledger 使用 skip_children 的定向 rustfmt 检查通过；全仓 `cargo fmt --all -- --check` 仍失败，`gateway-lifecycle-fmt-final.log`，差异来自并行交互工作涉及的 tui、当时的 `agent_control`（现 `speculative_tree`）、coordination/session、gateway/ui.rs、observation 等，本轮不批量格式化它们。 |

剩余范围：通用身份认证授权、Registry 验证、启动握手阶段信号快速响应、所有资源耗尽／线程 panic 注入、阻塞写及停止期间在飞交互回执、SSE 全量重连故障矩阵仍未完成专项验收。Runtime 读线程的错误类型尚未保存并向外作为独立错误传播；现有断线唤醒不等于错误分类完整。跨重启幂等和完整事件顺序不在本轮。Adapter 部署及配置组织继续由 Discussion 第 15 项承接，Gateway 生命周期结论不关闭它。

## 既有迁移的验证记录（2026-11-14）

## 实际调用与责任

| 当前调用链 | 保留在控制／展示侧的责任 |
|---|---|
| 前台入口装配 `gateway::agent::Listener`，协调侧 accept 得到 `Connection`，通过 receive 接收类型化 AgentMessage、send 交付 AgentCommand、finish 半关闭、stop_receiving 回收读线程 | Tree 身份／Branch／task／命令关联和合法转换，LocalPolicy 的生成，恢复保存顺序，错误后的关闭与清理决策 |
| `gateway::runtime::{ControlServer, ControlSession}`；SpeculationCoordinator.perform 与运行循环依赖 `&dyn Runtime` 的 Operation / OperationResult / Event | 环境选择、结果 Completed 判定、运行状态映射、Policy、Promote／Discard、预算与调度 |
| main 装配共享 UI Connection、嵌入静态资源及只读 Presentation；HTTP 和 TUI 直接调用 Connection.request → execution 事件 | `ui.rs` 对话／输入状态读取和展示折叠；TUI 绘制与终端输入；Ledger 原始事实保存、序号和补发读取 |

Runtime 通信调用统一使用 `gateway::runtime`。三侧统一的是源码通信责任边界，不增加一个无业务作用的总对象，也不统一三侧线协议。Gateway 不持有 Tree、Branch 生命周期、Policy、调度预算或 Runtime 执行状态。

`Presentation` 仅提供 `input_status` 与 `conversation` 展示查询，`Interface` 不持有请求连接。Runtime 请求统一由 `send` 构造，保留 `call_with_id`、`wait_event` 等既有测试调用入口。`Runtime::establish()` 不传预定身份，`Runtime::branch(parent_environment_id, agent)` 只传父环境和启动参数，创建结果返回新环境身份。后续 `Runtime::call(environment_id, operation)` 使用已保存的绑定，不接收逻辑 `branch_id`；不能用 `environment_id` 代填任何 Branch 字段。结果缓存读取仅克隆一次。

Runtime 仍通过 OperateEnvironment 的 AgentLaunch 实际启动 Agent，负责监视和回收。control plane 负责启动协调，main 保持 Adapter 进程启动／退出协调与配置装配；启动协调不移入 Gateway。部署与统一启停见[control plane §2.1](../design/control-plane.md#21-gateway)。

## 失败与关闭路径

- Agent：连接等待 30 秒、握手读取 10 秒、发送 5 秒；握手消息类型、agent_id 和六项既有能力检查在 Gateway。错误直接返回，局部 stream 释放。运行期非法方向的 wire Command 在 Gateway 拒绝；EOF／读错误作为事件送入控制逻辑。控制逻辑只在 draining 时接受 EOF 为正常结束，其他 EOF 不声称 cleanup 已确认。发送失败向上传播，不冒充命令执行成功。finish 仅半关闭写侧；读线程继续收取末尾事实，退出时 Read shutdown＋join，Connection Drop 关闭两侧。
- Runtime：ControlSession 保持原请求 ID、30 秒结果等待、有界结果缓存和事件队列；send 失败返回 io error，未完成结果不转换为成功。正常关闭 cloned socket 并 join 读线程，Runtime 自行 shutdown，main 等待／回收 Adapter。握手超时分支关闭会话并 kill/wait 自己启动的 Adapter。EOF／读错误会设置 disconnected 并唤醒 hello、result 和 event 等待者。
- UI：保留 loopback、Host、同源 Origin、JSON 类型、请求体大小和格式、空 prompt、SSE 序号检查与原状态码。Gateway Connection 拒绝已断开的 control plane 请求；202 仅表示排队，业务完成结果仍由控制逻辑产生。HTTP 读取／写入各 2 秒超时；SSE 写失败结束该连接，正常断开后交付剩余事件再结束。listener 和连接线程的受管停止见上方当前实现。
- 恢复：仍经同样 Agent handshake、LocalPolicy、Load/RecoveryLoaded、Suspend/Save/CancelSave、Runtime LoadRuntime/SaveRuntime 通道；Gateway 不解释恢复资料、不改变保存发布顺序。

## 验证与证据

本轮不调用真实模型，不新增测试，不改回放工具／断言／语义。初始快照：`/tmp/opencode/gateway-20261114-before/`，包含工作树／暂存 diff、状态和全部未跟踪文件；files 子目录另存本轮相关文件的编辑前内容。该快照已含更早中断任务的部分 Agent／Runtime Gateway 迁移，不是完整重构前基线。审查完整迁移另对照 git HEAD 的相关源文件；已有 TS 迁移、Policy 路径和树图等改动不计作本轮成果。

| 命令 | 结果与日志 |
|---|---|
| `cargo check --all-targets` | 通过；`/tmp/opencode/gateway-check-final.log` |
| `cargo test --lib` | 148 通过，含控制／Runtime／通信既有测试；`/tmp/opencode/gateway-rust-lib-final.log` |
| `cargo check --all-targets`（审查收尾含 session 路径清理） | 通过；`/tmp/opencode/gateway-review-check.log` |
| `cargo test --lib`（审查收尾含 session 路径清理） | 148 通过；`/tmp/opencode/gateway-review-lib.log` |
| `node --test tests/recovery-active.mjs tests/recovery-filesystem.mjs` | 5 通过，本轮收尾复跑；`/tmp/opencode/gateway-review-recovery.log` |
| `node /tmp/opencode/replay-ts-run1.mjs` | 已有兼容 run-1 两轮各 40/40 hits/consumed、misses/exhausted/unconsumed/cancel_timeouts=0；两输入完成、23 成功/17 取消；正常退出码 0。`/tmp/opencode/gateway-compatible-replay-final.log`，产物 `/tmp/opencode/replay-ts-run1-oxCwIm`。本验证说明 Gateway 与已兼容请求格式的输入通信正常，不替代机制的全部端到端验收。 |
| 当时的 Policy 集成检查 | 1 通过；`/tmp/opencode/gateway-rust-policy.log` |
| `cargo build --bins` | 通过；`/tmp/opencode/gateway-build-final.log` |
| `node --test tests/pi-access.mjs tests/pi-notification.mjs tests/pi-notification-adapter.mjs tests/pi-recovery-binding.mjs tests/pi-recovery-sources.mjs` | 18 通过；`/tmp/opencode/gateway-pi.log`，包括 Instrumentation EOF／写失败，不等于 control plane Gateway 的全部故障注入 |
| `node --test tests/recovery-active.mjs tests/recovery-filesystem.mjs` | 5 通过，最终代码重跑仍 5 通过；`/tmp/opencode/gateway-recovery-final.log`，本地伪模型、真实 Pi／Runtime、保存重试与恢复、进程清理 |
| `node tools/replay/verify/b1-pi.ts /tmp/opencode/ascout-replay-b1-pi-hyZBIv` | 通过；`/tmp/opencode/gateway-replay-pi.log`；产物 `ascout-replay-b1-pi-sh5No7`，4 子进程及动态端口清理通过 |

产物名称均位于 `/tmp/opencode/`。完整回放失败保留，不修改源记录、不 fallback 模型、不把既往通过报告当本轮通过。Gateway 三侧正常通信与恢复已有上述覆盖，握手超时／错误身份、control plane 侧发送失败、Runtime 运行中断线及 SSE 重连的完整专项故障注入未在本轮完成。

## 收尾审查与首错归因

2026-11-14 迁移审查核对了请求编号、环境映射、等待／缓存及队列，未重写这些协议语义。Agent Connection 显式 shutdown＋join；finish 只关闭 Write，draining 后 EOF 才正常结束。UI 的 connected 检查位于 Gateway Connection。HTTP/SSE 的校验、状态码及补发规则保留；当前连接和线程生命周期以本文上方 2026-11-16 实现为准。

不是纯搬移的局部差异明确保留：Agent bind 由无条件尝试删路径改为只替换 socket、拒绝普通文件／符号链接；Connection Drop 关闭两侧并回收读线程、ControlSession Drop 关闭会话；Runtime hello 超时分支新增已启动 Adapter 的 kill/wait。这些是连接资源保护，不增加投机规则；超时错误不再附完整 run_dir，只提示 agent.log，属于诊断文字变化。非法方向的 Agent Command 由控制线程拒绝改为读线程拒绝并停止读取，正常协议行为不变，但该非法协议路径的完整关闭故障注入尚未完成，不能声称逐项异常行为完全等同。

当时的首错为 replay 校验器在匹配和请求日志之前拒绝合法纯工具调用消息，而非 NLE 观察缺键。格式拒绝不计入报告的 total_requests，不能将报告误读为实际 HTTP 请求总量。

直接因果证据：`/tmp/opencode/gateway-request-probe.mjs` 从两份 session 只读取 user、首个 assistant 与三个工具结果，经当前 Pi provider 向独立 loopback HTTP 捕获服务发送六次请求（无 Gateway、无模型）。六次唯一不符合 `isMessage` 的消息均为 assistant `content:null` 加合法 tool_calls；Pi `openai-completions.ts:1284` 正是这样初始化纯工具调用消息。git HEAD `61c3f09` 的 `types.ts` 不接受 null；后续已由 [replay 格式修复](replay.md#2026-11-14assistant-null-正文兼容修复)补齐，并有 HTTP 回归覆盖。六次 requestKey 对应源 recording 中的 moved、blocked 和真实工具结果键（11a578d9…、ab63bfc1…、05ee5348…），不是此处工具内容漂移。捕获正文及日志：`/tmp/opencode/gateway-request-probe-0dDZ1C/requests.json`、`/tmp/opencode/gateway-request-probe.log`。探针不修改源文件，未重建或冒称完整重构前运行对照；格式修复不等于机制整链验收通过。

移除冗余通信路径的依据及弃用记录见 [开发决策](decisions.md#2026-11-14gateway-审查收尾)。审查未把已存在的 Policy 目录迁移、树图、恢复入口目标变化计为 Gateway 成果。

## 剩余缺口

### 2026-11-14 精简复核与独立提交验证

快照与日志根目录：`/tmp/opencode/gateway-slim-20261114/`。提交前验证阶段未新增测试、未调用真实模型、未运行 B3，也未修改现有暂存区。独立候选从 HEAD 构造，只提取 Gateway 及相关文档片段，排除 Policy 迁移、任务生命周期／物化、树图与 replay 修改。

- 工作区 `cargo check --all-targets`、`cargo build --bins` 通过；Gateway 既有库测试通过，Pi 接入 18 项通过。
- 工作区全量库测试 146 通过、2 失败：任务继续断言受并行生命周期修改影响；物化复制宿主内容触发空间不足。工作区恢复 5 项失败：Shutdown 已被并行任务修改为终止／物化，旧恢复测试依赖的保存入口不再成立，文件系统例另报告 Runtime cleanup 失败。日志分别为 `lib-workspace.log`、`recovery-workspace.log`；不计作 Gateway 通过。
- 独立候选 `cargo check --all-targets`、`cargo build --bins`、`cargo test --lib`（148 项）通过，恢复 5 项与 Pi 接入 18 项通过；恢复同时覆盖 HTTP 请求、展示查询及恢复后的连续输入。日志为 `check-candidate-final.log`、`build-candidate.log`、`lib-candidate.log`、`recovery-candidate.log`、`pi-candidate.log`。
- 独立候选保留 HEAD 的 Shutdown 保存行为及原 Policy 路径，不纳入新的 Pause／物化协议；Gateway 的现有工作区 Request::Pause 属于任务生命周期改动。编译需要既有、未跟踪的本地 Policy YAML 和预构建 `webui/dist`，这些只作为本地验证前提提供，不进入候选提交。没有对其他未提交 Rust 源码的依赖。

当前剩余缺口以本文上方 2026-11-16 小节为准；Branch/task/command 合法性仍由 Tree 校验，Gateway 不复制其控制责任。

当前界面读取经 UI Projection，Gateway 不持有观察缓存；Projection 管理界面连接的读取位置并转发系统运行日志报警，Ledger 只从已保存记录按序号读取。实现与验证见 [UI Projection](ui-projection.md)。

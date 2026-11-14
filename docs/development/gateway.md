# Gateway 实现核查（2026-11-14）

目标依据：[控制面 §2.1](../design/control-plane.md#21-gateway)。本轮保持现有协议、Unix socket、HTTP/SSE 和进程部署；不实现 Registry，不改变投机语义或决定启动协调归属。

## 实际调用与责任

| 当前调用链 | 保留在控制／展示侧的责任 |
|---|---|
| main 装配 `gateway::agent::Listener` → accept → `Connection`；execution 通过 receive 接收类型化 AgentMessage、send 交付 AgentCommand、finish 半关闭、stop_receiving 回收读线程 | Tree 身份／Branch／task／命令关联和合法转换；LocalPolicy 的生成；恢复保存顺序；错误后的关闭与清理决策 |
| `gateway::runtime::{ControlServer, ControlSession}`；AgentController.perform 与运行循环依赖 `&dyn Runtime` 的 Operation / OperationResult / Event | 环境选择、结果 Completed 判定、运行状态映射、Policy、Promote／Discard、预算与调度 |
| main 装配共享 UI Connection、嵌入静态资源及只读 Presentation；HTTP 和 TUI 直接调用 Connection.request → execution 事件 | `ui.rs` 对话／输入状态读取和展示折叠；TUI 绘制与终端输入；Ledger 原始事实保存、序号和补发读取 |

Runtime 通信调用统一使用 `gateway::runtime`。三侧统一的是源码通信责任边界，不增加一个无业务作用的总对象，也不统一三侧线协议。Gateway 不持有 Tree、Branch 生命周期、Policy、调度预算或 Runtime 执行状态。

`Presentation` 仅提供 `input_status` 与 `conversation` 展示查询，`Interface` 不持有请求连接。Runtime 请求统一由 `send` 构造，保留 `call_with_id`、`wait_event` 等既有测试调用入口；请求仍为 `task_id: None`、`branch_id: Some(environment_id)`、`generation: 0`，结果等待仍为 30 秒。结果缓存读取仅克隆一次。

Runtime 仍通过 OperateEnvironment 的 AgentLaunch 实际启动 Agent，负责监视和回收。main 保持现有 Adapter 进程启动／退出协调与配置装配；未把它移入 Gateway，也未将未决部署问题定案。

## 失败与关闭路径

- Agent：连接等待 30 秒、握手读取 10 秒、发送 5 秒；握手消息类型、agent_id 和六项既有能力检查在 Gateway。错误直接返回，局部 stream 释放。运行期非法方向的 wire Command 在 Gateway 拒绝；EOF／读错误作为事件送入控制逻辑。控制逻辑只在 draining 时接受 EOF 为正常结束，其他 EOF 不声称 cleanup 已确认。发送失败向上传播，不冒充命令执行成功。finish 仅半关闭写侧；读线程继续收取末尾事实，退出时 Read shutdown＋join，Connection Drop 关闭两侧。
- Runtime：ControlSession 保持原请求 ID、30 秒结果等待、有界结果缓存和事件队列；send 失败返回 io error，未完成结果不转换为成功。正常关闭关闭 cloned socket，Runtime 自行 shutdown，main 等待／回收 Adapter。新增握手超时分支会关闭会话并 kill/wait 自己启动的 Adapter，避免该分支留下进程。Runtime EOF／读错误仍结束读取线程，但不会立即唤醒结果／hello 等待者；现有调用方直到超时才发现，这不是本轮新实现的断线通知能力。
- UI：保留 loopback、Host、同源 Origin、JSON 类型、请求体大小和格式、空 prompt、SSE 序号检查与原状态码。Gateway Connection 拒绝已断开的控制面请求；202 仅表示排队，业务完成结果仍由控制逻辑产生。HTTP 读取／写入各 2 秒超时；SSE 写失败结束该连接，正常断开后交付剩余事件再结束；listener／连接线程仍随当前控制面进程结束，没有新增独立 UI 服务部署。
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
| `cargo test --test policy` | 1 通过；`/tmp/opencode/gateway-rust-policy.log` |
| `cargo build --bins` | 通过；`/tmp/opencode/gateway-build-final.log` |
| `node --test tests/pi-access.mjs tests/pi-notification.mjs tests/pi-notification-adapter.mjs tests/pi-recovery-binding.mjs tests/pi-recovery-sources.mjs` | 18 通过；`/tmp/opencode/gateway-pi.log`，包括接入侧 EOF／写失败，不等于控制面 Gateway 的全部故障注入 |
| `node --test tests/recovery-active.mjs tests/recovery-filesystem.mjs` | 5 通过，最终代码重跑仍 5 通过；`/tmp/opencode/gateway-recovery-final.log`，本地伪模型、真实 Pi／Runtime、保存重试与恢复、进程清理 |
| `node tools/replay/verify/b1-pi.ts /tmp/opencode/ascout-replay-b1-pi-hyZBIv` | 通过；`/tmp/opencode/gateway-replay-pi.log`；产物 `ascout-replay-b1-pi-sh5No7`，4 子进程及动态端口清理通过 |

产物名称均位于 `/tmp/opencode/`。完整回放失败保留，不修改源记录、不 fallback 模型、不把既往通过报告当本轮通过。Gateway 三侧正常通信与恢复已有上述覆盖，握手超时／错误身份、控制面侧发送失败、Runtime 运行中断线及 SSE 重连的完整专项故障注入未在本轮完成。

## 收尾审查与首错归因

对照 git HEAD 与当前文件，Runtime 会话主体除位置、说明、格式、`Runtime` trait 委托与 Drop 关闭外保持一致；请求编号、环境映射、等待／缓存及队列没有重写。Agent 原 scoped 读线程的 Read shutdown 后隐式 join，变为 Connection 中显式 shutdown＋join；finish 仍只关闭 Write，draining 后 EOF 才正常结束。主循环信号线程解除、工作线程 scope 等待及最终 Runtime／Ledger 清理顺序保持。UI 的 connected 检查原在 Interface.request，现移入 Gateway Connection，不是新增拒绝规则。HTTP/SSE 的校验、状态码、补发循环与 listener 生命周期保持；Runtime EOF 直到超时才被等待者发现、UI 无停止句柄均为既有边界。

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

通用认证、凭证校验与授权未实现；当前 agent_id／能力和本地 HTTP 检查不是通用安全机制。Branch/task/command 合法性依然由 Tree 校验，未复制到 Gateway。Runtime Hello 仍保留既有接入行为，未完成 Registry 的契约版本／能力验证；Runtime 读线程尚未有可等待的 join／断线错误传播。UI listener 仍无显式停止／线程回收句柄。main 在 Adapter 启动后、进入执行保护区之前的配置／能力不匹配等提前返回路径仍需统一资源清理审查；本轮仅覆盖并清理握手超时分支。启动阶段信号处理、跨重启幂等、完整事件顺序及部署／启动协调继续未定或未验收。

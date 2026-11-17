# 异步反馈与接收确认实现记录

本轮聚焦 Agent 交付等待、取消与发送失败清理。Gateway 的接收确认与业务完成仍是独立事实；未扩展暂停、中断、恢复的业务语义。

`select` 只接受原请求中的 Chance。Select、Validate、Rebalance、Predict、Launch 均在调度时记录输入，在完成时检查，不额外调用 Policy。Branch 状态变更使用单调计数；Chance 可变访问与 stage 变更分别记录单调计数，能识别状态恢复为旧值的情况。Rebalance 仅比较对应 Branch 的生命周期、角色、chance_id，逐项过滤；无关 Branch 不影响结果。Predict 只比较原 Chance 的 stage、kind、stage 计数、父 Branch generation 和假设，不因同一 Branch 的无关 transcript/usage 报告丢弃预测。运行中的 Policy 上下文不写入恢复资料。

实际输入依赖：Select 保存原 frontier 中 Chance 的内容与计数及父 Branch 生命周期；scheduling_context 当前为空。Predict 的 tool/arguments/operation 来自不可变 ToolPending，observation 是本次执行取得的值；检查假设、Chance kind/stage、generation 与 Runtime capability_view。Validate 保存候选与权威 Branch、Proposal、关联 Chance 及计数、action_facts、model_usage；authority 身份变化也使其失效。Launch 保存原 authority_state、candidate_view 各 Branch 和候选成员列表；新增/删除候选也会使其失效。Rebalance 检查各条结果对应的 Branch 身份、状态、角色、Chance 及生命周期计数。Validate 失效后不使用旧判定；若权威已经越过原等待点，仍沿既有 Discard 路径清理。

操作终态只允许第一次写入。ActionFinished 在检查 task 身份后保存第一份事实，重复事实不再推进，冲突事实保留第一份。操作及 Agent 终态冲突进入 `feedback_conflict`，由执行循环写 ERROR 系统日志。complete_work 在消费 pending_work 时保存结果的 SHA-256 指纹，只保留最近 256 个已消费操作；窗口内相同反馈不推进，冲突反馈进入同一 `feedback_conflict` 路径，窗口外旧操作仍因不在 pending_work 中而不能再次执行，但不再判断冲突。指纹随内存中的协调树销毁，不作为恢复资料。Runtime 冲突由 Gateway 收集并唤醒控制循环，写入 ERROR 系统日志。

Agent wire 新增 delivery（id、command）与 received（id）；公共 AgentAccessCore 和实际 Pi 启动器均接收 delivery。两者在连接内使用连续已受理序号和 256 指纹窗口，乱序身份暂存上限 1024；重复交付确认接收但不再次执行，冲突或过期身份明确报错并关闭连接，不确认未知内容。command_id 的关联也有 1024 条硬上限，标准 command-N-* 序号连续后淘汰旧记录；无可压缩序号及长期缺口达到上限时拒绝新增命令。Pi 恢复资料保存 commandPrefix/commandAhead 和当前命令窗口，load 后已知受理但没有结果记录的身份明确拒绝，不重新执行。同一业务命令以新 delivery id 再次发送仍视为新交付，原有 command_id 关联继续有效。旧 command 入口仍可被现有测试直接使用，但 Gateway 的 send 使用 delivery，只有 received 才返回成功。对端确认的是进程内接收，不能据此宣称业务完成或跨进程重启持久化。

Runtime 新增 received(request_id)，Adapter 校验请求身份与内容指纹并接受后发送。同 id 不同内容返回独立的 rejected 消息，不作为原操作的业务结果，避免覆盖仍在执行的请求；Gateway 收到 rejected 后唤醒对应发送并立即返回错误，同时把拒绝记录交给控制循环写系统日志。Adapter 的请求身份采用连接内有界窗口：标准 req-N 序号连续受理进度配合最近 256 个序号的指纹；未完成请求保留身份，总受理身份上限 1024。只跨过已经逐个受理的序号，不把先到达的大序号当成可以拒绝尚未受理小序号的依据。窗口外旧序号明确 rejected/expired，不发送内容已经确认的 received，也不再执行。窗口内同 id 不同内容 rejected。非标准字符串 id 无法压缩，保留到本连接容量上限；序号缺口长期不闭合或未完成请求占满容量时，新请求明确拒绝，调用方不能假定已执行。结果缓存淘汰与身份淘汰独立：仍保留指纹的旧请求可确认内容，并明确返回结果过期；已退休身份不再确认内容。业务 Result 也证明已接收，但不改变业务完成与接收确认的协议区别。Gateway 的 send 至多发送三次，每次最多等待 200ms 接收回执，重试保持原请求/交付 id。每次 socket 写入设 5 秒上限，写失败可能已发送部分帧，关闭连接并返回错误，绝不接着重放完整帧。三次均耗尽写超时的理论上界约 15.6 秒；正常写入、无回执约 600ms。

迟到的 Environment 创建成功继续通过现有 complete_establish 绑定真实资源身份；取消后不发 StartCandidate，由已有 progress 的 Terminate/Cleanup 流程处理。没有把资源创建结果套入 Policy 的丢弃过滤。

Agent 的首批命令、控制动作及 settle 命令统一进入每连接一个串行发送工作线程。等待及三次有界重试不占用控制循环，交付结果携带原命令返回事件队列；成功不再次推进业务状态。控制循环收到取消或 Branch 的 Abort/Discard/Cleanup 时，标记相关排队启动消息及确认等待，尚未发送的消息不再开始，已发送消息不再重试。检查令牌时已经开始的帧写入不能撤回，迟到业务事实仍由 Coordinator 接收。取消、Discard、Cleanup 使用独立令牌，不被启动取消禁用。退出前等待交付结果，避免提前半关闭写端。

发送错误交给 Tree 的命令/Branch 失败入口：有待处理 command_id 时使用原操作身份完成失败，已完成的命令不再推进；启动成功事实已被接收时，随后到达的接收超时不再把该启动改成失败。权威 Branch 交付失败结束 task 并请求退出清理；连接断开也进入该入口，Abort 交付失败后转入 shutdown 清理，不重复产生 Abort。Agent 无法确认清理时仍返回失败，由外层既有 Runtime 清理负责回收，不能宣称 Agent 已确认清理。

保证边界：交付窗口属于连接；Runtime Adapter 当前不自动重连后重放未知状态请求。Pi load 恢复 command_id 去重进度，delivery 序号则属于新连接。没有承诺进程崩溃、跨重启或旧格式恢复文件的 exactly-once。容量耗尽是明确拒绝，不静默遗忘身份。

未覆盖所有系统级重复/迟到/断线/截止组合；后续增加故障场景时应沿用当前边界。

2026-11-17 补齐验证：标准 Runtime 请求连续执行 599 次后重放 req-1，明确 rejected/expired，后端调用数不变；窗口测试以 req-2 先于 req-1 的顺序受理，并验证约 2000 个请求后身份表仍有界。TypeScript 窗口同样覆盖乱序、过期及非序号身份容量耗尽。已有重复内容冲突、结果缓存淘汰、取消等待确认、断线清理测试继续通过。真实 Pi 启动器原先收到 delivery 会报 Unexpected message direction；已补齐接收及 received，并用伪模型单步和五步 E2E 验证。

整合提交 `1c6eec7` 的恢复回归已完成验证：重建实际 bins 后串行运行 `tests/recovery-active.mjs` 与 `tests/recovery-filesystem.mjs`，6 项全部通过。retry 恢复后全部 Branch 继续推进；continue 成功收尾后保存且恢复的树仅保留一个权威 Branch。暂停期间普通 Prompt 仅入队，必须显式 `/continue`。Pi turn finalizer 保留 saving 期间 suspendedHooks 对应的动作，不提前报告取消；恢复后接回原工具等待。证据：`/tmp/ascout-recovery.log`，retry `/tmp/opencode/recovery-active-ZcUSAK`，continue `/tmp/opencode/recovery-active-mGEbBQ`。

本次整合验收：`cargo check --all-targets`、`cargo build --bins` 通过；串行 `cargo test --lib --bins -- --test-threads=1` 为 205 个库测试、4 个 control-plane 测试通过；Gateway delivery、身份窗口、Pi access、通知适配、恢复绑定与恢复源、ASC proxy 共 23 项 Node 测试通过；相邻 Pi checkout 的 `tsc --noEmit` 通过。日志分别为 `/tmp/ascout-check.log`、`/tmp/ascout-build.log`、`/tmp/ascout-tests.log`、`/tmp/ascout-node.log`、`/tmp/ascout-pi-tsc.log`。使用伪模型和伪连接，真实模型调用为 0；未改变 Ledger 恢复用途及 Pi session 目录。

本聚焦步骤验证：用伪 UnixStream 验证确认等待中取消会停止启动重试且 Cleanup 仍可交付；关闭取消检查时该回归失败。用伪 TCP 连接验证 rejected 立即返回对应错误；恢复旧的忽略 rejected 行为时该回归失败。另覆盖接收确认/业务 Result 两种顺序、重复 Result、队列等待中仍可处理取消、迟到创建资源保留以及 Branch 发送失败入口。最终 `cargo test --lib` 195 项、control-plane binary 4 项、Gateway/Pi 接入与通知 Node 测试 12 项全部通过，`cargo check --all-targets` 通过（已有 IsTerminal 未使用警告）。该聚焦步骤当时未修改 TS。当前补齐实际 Pi 启动器 delivery 接收，并运行 Pi 全仓 tsc --noEmit；上述队列与连接回归仍不等于完整进程故障组合测试。

验证：先用伪造事件复现 select 越出原请求和操作终态覆盖，两项修复后通过；新增 Branch 局部 Rebalance 有效性回归通过。Agent UnixStream 伪连接丢弃首次回执，确认第二次发送复用相同 id 后返回成功。control_plane 库测试、runtime::adapter 库测试、Pi 接入/通知测试及 cargo check 已运行；最终数字见执行报告。未调用真实模型，未提交，未修改暂停/中断/恢复语义或 Ledger 恢复用途。

# Agent 接入侧通用层

本文记录 Agent 接入侧通用层的当前实现、接口与限制；目标语义见 [Agent 接入侧设计](../design/agent-integration.md)。

## 1. 分层

- `agent-access/core/`：与具体 Agent product 无关的通用接入逻辑，包括本地 Policy 求值调用、`extent` 的 step 计数与求值、`effect` 闸门、Blocked/Resumed 上报，以及等待控制面处置的 hold。
- `agent-access/pi/main.ts`：Pi 的 Product Integration，只负责把 Pi 的工具执行前、模型推理前等执行点转成通用层的执行点事件，执行返回的处置，并上报 Pi 专有的消息、checkpoint、session 与用量事实。

通用层不读取 Pi 的消息结构、checkpoint、session、SDK、模型对象，也不读取 NLE 专有字段。Pi 把 `checkpoint` 作为不透明的 admission fact 交给通用层，通用层只把它原样放进 `speculative_admission_request`，不解释其内容。

## 2.1 模型投机（`branch_model`）当前语义

这条路径与工具等待路径不同，但已经有明确的接入顺序：

1. Pi 在真正调用 `streamSimple` 前执行模型推理前 Hook，登记模型 action、checkpoint 和模型上下文；`message_start` 只是 SDK 的消息边界，不能替代这个 Hook。
2. Hook 通过当前 Policy 的 `admit.opportunity_pass`／`admit.feasibility_pass` 判断是否有机会。Policy 的 `predict` 返回 `branch_model` 及候选模型（当前实现从 Policy artifact 的场景选择该模型），而不是由控制面或 Pi 硬编码模型选择。
3. 控制面先按该等待点建立候选 Branch，再让候选使用指定模型继续推理；候选模型产生的 assistant 结果中，完整的 `tool_calls` 列表是比较输入。当前语义不是先创建一个只作占位的 Proposal 再补候选产出：候选先启动，模型完成后才以完整调用列表形成可比较的结果。
4. 完整列表参与匹配：调用名称、参数以及调用数量／顺序均属于候选与权威模型调用列表的比较范围；不能只比较第一个调用，也不能用普通文本、思考文本或其他 assistant 输出判定命中。未满足确定等价规则的调用列表不能 Promote。
5. 候选的模型推理和工具调用在 Promote 前仍受候选生命周期约束。候选不能把工具效果写入权威路径；Promote 只保留一个胜者并切换后续权威会话，旧 Parent 与其他候选按既有清理流程处置；Discard 取消并清理该候选，未验证输出不进入主对话。

上述语义只说明当前已接通的模型投机接缝，不表示已经解决所有模型投机策略问题。调用列表等价的规范化／语义相似规则、多模型成本与失败回退仍由 `Discussion.md` 第 40 项保留为未决问题。

当前实现中 Pi 是宿主上的单一进程，Authority 与全部 Candidate session 都由它托管；每个 Branch 的 Runtime 环境另外持有一个常驻 namespace-holder，文件工具通过该 Branch 专属 socket 在已有 mount namespace 中执行，Pi 不进入 Branch namespace。控制面持有逻辑 transcript，并在创建 Candidate 时继承父 Branch 的逻辑 transcript；Pi 的 Branch session 继续保留原生记录，以便最终汇总和从原生 checkpoint 续跑。`/app` 这类 workload 路径由 FilesystemBackend 的 `isolated_paths: [{path, target}]` 映射，host 目录仍是 overlay lower 与 publication 目标。holder 的命令各自独立执行，直接 shell 退出即返回，后台后代留在 Branch namespace 直到环境结束；物化轮换 execution generation 时旧 holder 结束，发布后建立新 holder。

## 2. 接口

`agent-access/core/index.ts` 导出 `createAgentAccess(options)`。调用方提供：

| 选项 | 含义 |
| --- | --- |
| `taskId` | 上报事实中的 task 身份 |
| `report(message)` | 向控制面发送一条已构造好的消息 |
| `evaluate(artifact, decision, data, signal)` | 执行一次编译 Policy 的 Decision 求值；由接入进程注入，通用层不绑定具体 runner 位置 |

返回的接口：

| 方法 | 用途 |
| --- | --- |
| `beforeAction(point)` | 产品在真实执行前调用。执行 effect 闸门、rebalance pause、`admit`、`extent` 与相应上报，最后返回 `allow` 或 `admit` 处置 |
| `control(command)` | 按控制面命令维护 hold、本地 Policy 和 extent 计数；产品适配在命令处理入口调用 |
| `releaseAll()` | 进程退出时释放全部 hold |

`ExecutionPoint` 只承载通用事实：`branch`、`actionId`、`actionKind`、`name`、`arguments`、Policy 的 `op` 与 `contractContext`、是否允许准入、附加的 admission facts、取消信号。产品用函数形式的 `admit` / `admissionFacts` 表达“等到闸门放行后再判断是否是权威路径”和“准入成立时才生成产品事实”。

## 3. 当前覆盖

- `admit.opportunity_pass` 与 `admit.feasibility_pass` 在通用层求值；准入成立时上报 `speculative_admission_request`。
- `extent` 按 Branch 和 action identity 计数，只在工具执行点求值；结果为 `stop` 时上报 `branch_suspended(extent_reached)` 并等待 `resolve_pending` 或取消释放。
- `effect` 只对 `tool_execution` 执行点求值；`deny` 直接抛错，`block` 上报 `branch_suspended(effect_gate)` 并等待 Promote、Discard 或清理释放。模型推理是 PURE 步骤，不经过 effect 闸门。
- `rebalance` 的 pause/resume 与 extent/effect hold 都由通用层保存和释放；`branch_suspended` / `branch_resumed` 由通用层上报。
- 控制面命令的 hold 生命周：`start_candidate` 建立 effect hold，`promote` 释放 effect/extent，`discard`、`cleanup` 释放全部 hold 并清理本地状态，`resolve_pending` 释放 extent。

## 4. session 持久化与状态交接：已定方向及实现差距

2026-11-13 确认：运行分支采用 Pi 原生完整持久化，允许多 session 作为过渡，由接入侧汇总为单一最终权威 session。源分支停止写入且成功汇总后，才可删除对应临时文件；Promote 后仍活跃的 winner 不能删除。compaction、custom 等记录必须保存，不只保存 `message_end`。这些是已定实现方向，不表示当前适配器已经完成。

实现入口为 `agent-access/pi/main.ts` 与 `session-tree.ts`；当前原生持久化、真实结果父链整理和停止写入后的汇总删除进展及验证范围见本节末。

必要状态交接的正确性由机制保证。文件系统可讨论以共同快照为基准，将真实执行的差异合入获胜候选；这只是讨论方向，不是已实现 Backend，也不是通用状态 merge 的授权。

尚未解决的实现差距：

- 完整持久化及汇总已保留原生身份、预测与原主／丢弃历史；剩余为并发配对、冲突与失败的完整验收。未额外要求另存完整投机后续路径。
- compaction/custom 记录必须保存；原生父引用与已知 `targetId`／`firstKeptEntryId`／`fromId` 已有校验，任意 custom payload 的外部引用仍不能通用解释。compaction 摘要保留原文已确定，不阻塞汇总或满足其他条件后的源清理，不再列作待议事项。
- 在飞推理如何与历史整理、状态交接协调仍未定；不擅定重启、强制取消或替换运行中上下文。
- rollback 与外部资源状态如何关联仍未定；最终 session 能恢复对话历史不等于已证明工具、环境和外部资源可完整继续。
- 历史重挂后，系统外导出按原生 session/events 还原请求关系；正常运行不计算或保存回放专用键，还原算法与验证边界见 [回放](replay.md)。

原始运行上下文与整理后恢复上下文可能不同，是用户接受的历史含义变化，不把二者相同列为验收承诺。是否向 winner 追加真实结果通知交由 Policy，默认完全不追加，不发送空消息；既有 dispose／Promote 承载可选 `notification`，不新增 Policy 名称或类别。

2026-11-13 实现进展：Pi 分支现在由独立的原生持久 `SessionManager` 文件承载，复制原生 entry 后在独立 manager 选择 checkpoint，保留原生 entry ID 及 compaction/custom 等非 message entry。Pi 的 `createBranchedSession` 会修改 receiver 并重建 label ID，不能在活跃 manager 上调用；此处保留完整索引以免丢失旁支引用。候选在待决工具 checkpoint 建立，不等待整个 turn。`message_end` 只在 Pi 已落盘后读取对应 native entry，避免重复写入 message。接入侧先校验父引用，再写入临时汇总文件并替换最终文件；源分支失败时保留，活跃 winner 不删除。回放导出不能直接使用重挂后的父链。

验证：`node --test tests/pi-access.mjs tests/pi-session-history.mjs` 共 7 项通过，覆盖 checkpoint 原生记录、原生 ID、真实结果重挂、运行 manager 不变、活跃源保留、停止后清理、失败保留和已安装 `pi-code-rollback` 的同仓库快照关联。快照在 `/tmp/opencode` 临时 Git 仓库生成，核对 `targetId`、`repoRoot`、`ref^{tree}`、Git blob 可读及重开后的启动 GC；没有执行用户仓库恢复。`node tools/replay/verify/b1-pi.ts` 通过，使用本地回放服务，产物 `/tmp/opencode/ascout-replay-b1-pi-ENK910`。

后续审查已补上同仓库独立真实 Pi 文件任务继续及临时 Git 实际 rollback 恢复，并修复结果误选、候选重挂路径、独立重开位置和异步 shutdown 记录遗漏；完整命令、范围和剩余项见 [Pi session 审查修复](pi-session-review-2026-11-13.md)。跨仓库 Git ref/object、NLE 外部资源迁移仍未验收，不能把复制 session 文件当作完整 `pi-code-rollback` 支持。compaction 摘要已确认保留原文，不阻塞汇总或满足其他条件后的源清理，不重新生成摘要。

### 4.1 Promote notification

`Notifications` 跟踪已接受但未确认原生落盘的通知，以 `command_id` 关联 pending 与原生 details。发送使用 `triggerTurn:false`，不是 prompt／steer。读取源文件核对完整原生 entry 后才移除 pending 并报告 transcript；stopSession 在停止和 idle 后重试检查，失败不删除源文件、不报告 released。EOF 失败非零退出。Promote 已完成后通知失败报告既有 `adapter_capability_gap`，不伪称 Promote Failed。pending 只在进程内，不提供跨崩溃恢复保证。

真实 Pi SDK 加本地伪 HTTP 模型验证见 `tests/pi-notification.mjs`：缺省零记录、完整双工具批次后交付、下一自然请求可见、空闲无新请求、去重、汇总重开 ID／正文／关联及注入写失败后保留 pending 和重试。Rust 检查 schema／缺省序列化、真实结果与 evidence 分别进入 Policy、Runtime 失败不发通知与重复完成不重发。原实现验证为 `cargo test --lib` 144 项、`cargo check --all-targets` 通过；Pi notification/access/history-regressions/session-history/standalone 五组共 15 项通过。未调用外部模型或操作用户仓库恢复。全仓格式检查存在既有差异，未批量格式化。

2026-11-13 独立 adapter 审查新增 `tests/pi-notification-adapter.mjs`：真实 `main.ts` 子进程、Unix 帧协议和原生 `SessionManager`，SDK double 只模拟 `triggerTurn:false` 的运行中队列、abort、idle 及落盘事件，不模拟真实推理。9 项覆盖排队后的 cleanup／EOF／abort／自然 turn_end、缺省零通知、完整相同 `command_id` 重传（交付前后）及冲突 payload 拒绝；空闲与排队两种写失败均核对源文件保留、无通知 transcript、无虚假 released、EOF 非零退出，空闲接收失败另核对 `adapter_capability_gap(notification, failed)` 而 Promote 仍 Applied。正常 Promote 不 abort，不增加模型执行；通知重开核对正文、原生 ID 和 command_id。联合原五组共 24 项通过。Rust 协调层 8 项通过，新增两个 Chance 的 outcome／evidence 不串用检查；真实 Policy 的 context 输入与 Runtime 失败不交付沿用现有测试。本轮未发现需要生产代码修复的问题。

边界：SDK double 的 shutdown／故障注入不是完整原生 Pi adapter 端到端验收；原生 Pi 双工具队列与自然下一请求由独立 SDK 测试覆盖。没有验证跨崩溃去重、全系统恢复、跨仓库／外部环境资源迁移。`display:false` 只关闭 Pi 本地展示，不阻止模型上下文或 transcript 持久化；已记录目标语义未要求强制 display=true，本轮未改动。关于 `dispose.context.authority_outcome` 的批准来源，早先“已确认”的表述缺乏独立批准依据；2026-11-13 本轮用户明确回复“同意”，批准保留该字段，向 Policy 提供对应 Chance／operation 的真实结果，不自动给模型追加消息。此批准不追认早先已确认；本轮仅文档收尾，不改实现、不重跑测试，见 `decisions.md`。

上述审查的全量 `cargo test --lib` 为 144 通过、1 失败：无关的 `send_dump_drains_large_send_stderr` 在启动临时可执行文件时报 `Text file busy (os error 26)`；立即单项复跑通过。未修改该测试或声称上述审查全量全过。`Discussion.md` 保留既有验证记录，批准来源待核对项已由本轮明确批准收敛；该文件当前被 Git 忽略，未强制纳入版本控制。

## 5. 限制

- 只有 Pi 一个 Product Integration；sub-agent 启动前仍无统一 Hook。
- effect 只实现 `block` / `deny` 两种最小计划，没有 Reuse、Salvage 或更多效果模式。
- extent 计数的是工具执行点的 action 次数，不是模型语义上已完成步骤数；子 Chance 的计数独立于父 Chance。
- hold 的 Promise 只存在于接入进程内；主动退出保存 hold 所属 Branch 与工具／候选配对描述，跨进程重新建立等待，用户继续后才推进。控制面仍是 Tree、Chance 和提交资格的唯一权威。实际验收与 backend 边界见 [恢复](recovery.md)。
- 通用层不等待“控制面返回的任意处置”；它只把控制面命令落实为 hold 的释放，真正的继续/阻止/使用结果仍由 Product Integration 在本次调用返回路径上完成。
- `node --test tests/pi-access.mjs` 覆盖 Pi 接入行为，使用伪造 SDK 与手工事件，不调用真实模型；本轮没有新增测试。

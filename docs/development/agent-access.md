# Instrumentation 通用层

Product Integration 对应 `instrumentation/pi/`，Interposer 对应 `instrumentation/core/` 的 `createInterposer`，导出类型为 `Interposer`。`instrumentation/pi/asc-proxy.ts` 的进程内 ASC-Proxy 同时管理控制 Gateway Unix socket 和独立的 Runtime 执行连接。control plane 从 Environment 建立、派生和恢复启动结果取得访问地址。Instrumentation 保留 Branch 到 Environment 的映射，执行端点按 Environment 与 operation 关联执行、取消与确认。`instrumentation/pi/tools.ts` 独立适配 Pi 参数、schema 和格式，一次完整工具调用对应一次 Runtime read_file/write_file/replace_text/list_directory/exec；完整编辑和 canonical path 队列由目标 backend 执行。NLE 等声明工具保留相同生命周期。

Runtime 仍在状态目录写入 `tool-plans/<environment>.json`，BaseRuntime 读取内部 holder 地址及 workspace host/target 映射，连接当前环境 holder；不存在逐工具 Node 进程或 Runtime 内的 Pi SDK 依赖。Firecracker 通过已有 vsock guest agent 原生处理同一组文件／命令请求，guest 不需要 Node 或 Pi。执行访问方式沿 Runtime → control plane → Instrumentation 交付，ASC-Proxy 不读取计划或直接连接 holder。每次完整工具操作有独立身份，保留 tool_call_id 与取消归属。

### 2026-11-18 通用环境操作验证

- `cargo check --all-targets`、`cargo build --bins`、Pi `asc-proxy.ts/tools.ts` 的定向 `tsc --noEmit` 通过。
- `cargo test --lib -- --test-threads=1`：212/212 通过，覆盖四条工具／模型启动路径、只放行一次、候选 Promote、暂停消息排队、接收端取消代际和重放窗口。此前默认并行运行 211/212，额外挂载测试报 OverlayFS EBUSY；该测试单独复跑和全库串行复跑均通过，未修改测试断言，也未证明并行冲突已修复。
当前完整操作回归由 tests/pi-filesystem-tools.mjs 与 tests/pi-asc-proxy.mjs 承载；不依赖原生 SDK 工具实现。
- 重建后 `node --test --test-concurrency=1 tests/authority-filesystem.mjs tests/recovery-filesystem.mjs tests/recovery-active.mjs`：8/8 通过。最终证据为 `/tmp/opencode/authority-filesystem-e6R7R4`、`/tmp/opencode/recovery-filesystem-Wca7Bk`、`/tmp/opencode/recovery-active-1SafyW`（filesystem）。全部模型响应为本地 fixture。
- `cargo test --test firecracker_e2e real_microvm_completes_the_full_lifecycle_over_vsock -- --exact --nocapture`：真实 VM 1/1 通过，增加统一执行接收端 write/read/exec 检查。未运行整个 Firecracker 组或模型服务测试。guest 本地回归同时验证二进制文件、2 MiB 流输出和 detached writer 取消。

验证边界见 [Runtime 开发文档](runtime-adapter.md)。内置 FS／Firecracker 尽力终止该操作的进程组，脱离进程组的后代可能存活。独立可执行 backend 由程序接收单操作取消并报告实际终态。旧 SDK 注入实现的通过记录不作为当前实现验收。

当前控制握手核对 `agent_id` 与能力，Tree 校验 Branch/task/action 归属，执行端点检查 Environment 状态、generation 和请求身份窗口。Agent session 是 Product Integration 管理的执行上下文，不是另一个通信对端。这些检查不等于通信凭证认证：当前 ASC-Proxy 没有完整的双向凭证认证或防伪机制，不能以 Unix socket、本地握手或身份窗口推定通用认证授权已完成；相关缺口见 [Gateway 实现核查](gateway.md)。

2026-11-17：完整脚本模型 benchmark 的 match、mismatch、authority_first、fast_failure 均通过；证据分别为 `/tmp/opencode/benchmark-task-gT3UtV`、`/tmp/opencode/benchmark-task-y4bhE3`、`/tmp/opencode/benchmark-task-JaHLiL`、`/tmp/opencode/benchmark-task-JaZYQN`。恢复文件场景通过，证据 `/tmp/opencode/recovery-filesystem-f2BUCi`。provider 在首个流事件前失败时保留 action 身份，供 Pi 随后生成的失败消息关联 Usage；恢复后的运行重新发布当前连接状态，避免历史断开事件阻止 `/continue`。未调用真实模型。

执行协议包含接受、增量 output、终态、cancel 和 confirm。取消等待端点接受后转发并等待 backend 返回终态。NLE 工具模拟延迟在 backend 内可取消等待，并用独立动作锁维持共享世界动作顺序，不再占住 bridge 阻塞候选环境创建；请求已进入 bridge 后仍发送取消并等待其终态，动作已进入同步游戏调用时等待调用返回。Branch 取消还调用 holder 的 `drain(cancel:true)`，清理已完成工具留下的后代。确认释放结果正文与增量输出；请求使用已有 `req-N` 连续序号窗口，保留最近身份指纹及未确认操作，超窗旧序号拒绝重放，长期序号缺口或未确认操作占满 1024 槽位时明确拒绝新增操作。回归覆盖连续 1299 个已确认操作，不能将窗口有界理解为终身最多执行 1024 次。

Pi 目前保留 SDK 宿主，不启动原生 CLI：原生 RPC 的 fork/clone 切换单一当前 session，缺少并行候选、待决工具恢复与当前 continue 的对应命令。原生 `context` 可等待并用 `ctx.abort()` 在 fake transport 前取消；`before_provider_request` 的取消仍可能进入 provider，不能把它当作请求前强制阻止保证。生产命令发现已通过原生 inline extension 的公开 `getCommands()` 实现，保留默认资源加载。Pi 原生从配置目录读取主 `models.json`，应用内置 provider 覆盖。普通入口给 Agent 和 Policy provider 传入一致的配置目录。优先采用非空的 `PI_CODING_AGENT_DIR`，其次采用 AScout 配置项 `models` 指向的主 `models.json` 所在目录，最后采用 Pi 默认目录。集成参数 `--models` 加载额外 provider JSON，重名 provider 仍会报错。Pi 原生 CLI 的模型列表参数具有独立语义。

Agent 与 Instrumentation 宿主在所有 BaseRuntime 路径都位于 Branch 隔离外，由 Runtime 启停；有 holder 的文件系统路径仅把 holder 和受管命令放进环境。Adapter 的直接启动路径（包括 Firecracker）同样在宿主运行；Guest Agent 留在 guest 提供 Runtime 命令服务。扩展直接使用 Node 的工作区读写、进程启动或直接 shell 不会自动得到 Branch 视图；只有已适配的文件工具和 Runtime 工具受到当前环境约束。内建文件工具的覆盖不等于任意自定义工具已隔离。所有网络无需统一绕行 Runtime。


本文记录 Instrumentation 通用层的当前实现、接口与限制；目标语义见 [Instrumentation 设计](../design/agent-integration.md)。

## 本轮验证与能力边界

2026-11-17 最终 focused correction（下列早先整组检查未重跑）：先以手工 Rust 事件复现工具空预测／预测失败未放行，以及 speculative-first 候选不能进入 Policy 裁决；分别复用 `authorize_authority`／`resolve_pending` 和既有 `Validate → match/dispose` 修复。扩展回归又复现预测失败后的模型结果无法配对及终态 Chance 过早回收，修复结果过滤及等待点释放前的回收条件。`cargo test --lib held_authority -- --test-threads=1` 三项通过；`node --test --test-concurrency=1 tests/pi-access.mjs tests/pi-asc-proxy.mjs` 九项通过，均无真实模型调用。

四路径回归 `held_authority_four_launch_paths_promote_or_fallback_once` 覆盖工具／模型 × eager／speculative-first × 接受／拒绝的控制状态、执行授权次数、Promote Applied 后父 Discard、无父结果注入及回退一次。它注入 `ControlResult::Disposition`，不是实际 Policy 求值或 Pi SDK 四路径端到端验收。现有 Pi access doubles 覆盖工具执行前等待、独立候选 session、Promote 后执行和各等待点取消；不据此声称模型 held hook 与工具 held hook 的完整矩阵均已端到端验证。

相邻定向检查：`ordinary_prompt_is_queued_while_paused_until_continue`、`cancellation_fences_admission_until_writers_stop_and_old_generation_stays_dead`、`promoting_branch_keeps_its_inflight_model_waiting_point_live` 各一项通过；`cargo test --lib validation -- --test-threads=1` 三项通过；最终 `git diff --check` 通过。未重复父轮完成的 202 lib、37 Pi、7 recovery、6 VM 整组，也未重新运行全目标 check。

speculative-first 在已有 Proposal、launch 已决定且没有在飞 launch 时进入现有 Policy `match/dispose`；没有权威结果时输入为 null，由 Policy 的明确可接受性规则裁决。默认需要真实比较的 Policy 不因此自动具备独立 validator；`Indeterminate` 会保留等待，是否转为 eager 仍由 `launch` 决定。未完成替代模型不伪造 Proposal，候选失败沿 BranchFailed／Discard 收敛并放行父调用。

暂停普通输入只入队，不自动恢复；`/continue` 先恢复原任务，原轮结束后按队列顺序交付消息。恢复时以当前 `execution_accesses` 覆盖 session 和待启动 candidate setup 的旧访问地址；ASC-Proxy 的执行 generation 属于当前 Runtime 端点，重建端点后从其初始 generation 开始，不从保存文件复用。Branch 命令 generation 和执行 generation 是不同协议字段，前者随 Tree 恢复，后者由 `cancel_environment` 的 Runtime 回执更新。

2026-11-17 最终定向复核：`cargo check --all-targets` 无警告通过；`cargo test --lib -- --test-threads=1` 202/202 通过。`cargo test --test firecracker_e2e -- --test-threads=1 --skip codex_starts_and_reaches_the_model_service` 真实 VM 6/6 通过，覆盖生命周期、独立 sibling 快照、退出事件、重试、SSH 和在飞操作中断。模型服务用例排除，因为本轮禁止真实模型请求；早先未排除的整组运行在该用例超时，不能宣称 7/7。fixture 状态目录使用工作区祖先的短 `afc/` 路径，既避开 Unix socket 长度上限，也不把内存快照放入容量不足的 `/tmp` tmpfs。namespace 清理严格接受 `/run/netns` 和 `/var/run/netns` 的缺资源诊断；成功 cleanup 用 backend 回执报告 Cleaned，不依赖可能已移除的 backend 记录。

`node --test --test-concurrency=1 tests/pi*.mjs tests/predictor-fact-e2e.mjs` 37/37 通过，包括 notification adapter 9 项及 predictor 单步／五步 2 项；顺序扩大运行未重现此前的 Policy 求值失败，不据此猜测根因。predictor 证据 `/tmp/opencode/predictor-fact-e2e-tOnzRM`、`/tmp/opencode/predictor-fact-e2e-IBvClg`。notification fixture 补齐 Pi 工具定义模块，并使用独立执行 socket 的接受／取消／终态／确认流程。模型 admission 不使工具验证过期的回归为 `next_model_admission_does_not_expire_tool_validation`；checkpoint 已有工具结果时不重复登记恢复调用。

此前 benchmark 独立重跑 4/4：`/tmp/opencode/benchmark-task-YpE6yi`、`/tmp/opencode/benchmark-task-zGiA9N`、`/tmp/opencode/benchmark-task-iZAXXl`、`/tmp/opencode/benchmark-task-0hANuI`；本次未重新执行 benchmark。全仓 `cargo fmt --check` 仍有既有／其他工作差异，原始证据 `/tmp/opencode/instrumentation-fmt-before.log`；只格式化相关 Rust，不批量覆盖共享工作树。

`node --test --test-concurrency=1 tests/authority-filesystem.mjs tests/recovery-active.mjs tests/recovery-filesystem.mjs` 7/7 通过。活跃恢复五场景已连续两组全部通过；最新证据 none `/tmp/opencode/recovery-active-TsQSEH`、retry `/tmp/opencode/recovery-active-RQCi3k`、continue `/tmp/opencode/recovery-active-MV0IBH`、multiple `/tmp/opencode/recovery-active-Rn2sMG`、filesystem `/tmp/opencode/recovery-active-ZAcmf9`。fixture 在实际 before-tool/checkpoint 后、执行前阻塞 sibling，不向生产代码添加 op 编号顺序；continue 场景等 task_completed 后再保存。恢复中的 candidate setup 使用当前 Runtime 下发的 execution access，不复用保存时已失效的 socket。断言保留待决 sibling、候选配对结果及实际动作次数，未放宽重复执行检查。authority 文件测试覆盖后台写者、跨 task 复用、中断和失败重试。

Branch cancellation 与 Execute 使用同一 admission 锁：开始取消时推进 execution generation，关闭新 admission，等待已有操作终态后 drain 写者；成功后开放新 generation，旧 generation 不能复活。drain 失败仍关闭执行，但允许重试 drain，不再永久卡死。确定性回归覆盖活跃操作取消、清理期间新增执行拒绝、drain 失败／重试、延迟旧请求拒绝及存活 authority 的新 task 执行；1299 次确认回归仍通过。

补充检查：当时的 Rust 二进制和集成测试 9/9 通过；ASC-Proxy 的独立 `tsc --noEmit --target es2022 --module nodenext --moduleResolution nodenext --skipLibCheck ../pi/node_modules/@types/node/index.d.ts instrumentation/pi/asc-proxy.ts` 通过（使用 `../pi/node_modules/.bin/tsc`）。`git diff --check` 通过。全仓格式最终记录 `/tmp/opencode/instrumentation-fmt-final.log`，剩余 24 个文件均已出现在本轮格式化前记录中。ASC-Proxy 的个别取消失败不会跳过 branch-wide drain，新增 socket 回归核对该流程。

`cargo check --all-targets`、`cargo test --lib`（178 项）、`node --test tests/pi*.mjs`（32 项）、`tests/predictor-fact-e2e.mjs`（2 项，正常单步及 Promote 后五步）、相邻 Pi 原生边界与摘要验证（20 项）、Pi `npm run check` 通过。端到端仅使用 loopback fake SSE；证据目录为 `/tmp/opencode/predictor-fact-e2e-CAb2ux` 和 `/tmp/opencode/predictor-fact-e2e-FtNgKj`。本轮无真实模型请求。Firecracker 实机和不受本机支持的隔离 backend 未做端到端验收，不由库测试通过推定全部 backend 已验证。

新增可信宿主回归验证无 holder 的 BaseRuntime 不将宿主登记为 Environment writer，物化后仍能复用宿主；旧冻结/物化写者 fixture 改为显式 Environment 进程，保留写者清理语义。已有 FS holder 回归覆盖 Branch 视图及物化轮换。模型完成回归验证完成回调等待期间不能交付 `result()`；摘要 OAuth 验证保留。

工具 checkpoint 仍仅支持进入 transcript 的顶层调用。Pi 原生扩展 `tool_call` 能观察 `ctx.executeTool()` 嵌套调用，而当前 Agent hook 包装不能据此为嵌套调用建立独立投机 checkpoint；本轮未宣称该能力已等价替换。任意插件自带 shell、Node 文件访问及自定义工具的宿主副作用仍属可信扩展的限制，不能将正常资源加载说成全面隔离。

## 1. 分层

- `instrumentation/core/`：与具体 Agent product 无关的通用接入逻辑，包括本地 Policy 求值调用、`extent` 的 step 计数与求值、`effect` 闸门、Blocked/Resumed 上报，以及等待 control plane 处置的 hold。
- `instrumentation/pi/main.ts`：Pi integration，只负责把 Pi 的工具执行前、模型推理前等执行点转成通用层的执行点事件，执行返回的处置，并上报 Pi 专有的消息、checkpoint、session 与用量事实。

通用层不读取 Pi 的消息结构、checkpoint、session、SDK、模型对象，也不读取 NLE 专有字段。Pi 把 `checkpoint` 作为不透明的 admission fact 交给通用层，通用层只把它原样放进 `speculative_admission_request`，不解释其内容。

## 2.1 模型投机（`branch_model`）当前语义

这条路径与工具等待路径不同，但已经有明确的接入顺序：

1. Pi 在真正调用 `streamSimple` 前执行模型推理前 Hook，登记模型 action、checkpoint 和模型上下文；`message_start` 只是 SDK 的消息边界，不能替代这个 Hook。
2. Hook 通过当前 Policy 的 `admit.opportunity_pass`／`admit.feasibility_pass` 判断是否有机会。Policy 的 `predict` 返回 `branch_model` 及候选模型（当前实现从 Policy artifact 的场景选择该模型），而不是由 control plane 或 Pi 硬编码模型选择。
3. control plane 先按该等待点建立候选 Branch，再让候选使用指定模型继续推理；候选模型产生的 assistant 结果中，完整的 `tool_calls` 列表是比较输入。当前语义不是先创建一个只作占位的 Proposal 再补候选产出：候选先启动，模型完成后才以完整调用列表形成可比较的结果。
4. 完整列表参与匹配：调用名称、参数以及调用数量／顺序均属于候选与权威模型调用列表的比较范围；不能只比较第一个调用，也不能用普通文本、思考文本或其他 assistant 输出判定命中。未满足确定等价规则的调用列表不能 Promote。
5. `branch_model` 的 `predict` 可以返回 `candidate_guidance`。Pi 只将该文本加入候选模型的单次请求，不把它写入公共提示或会话历史。候选恢复为权威模型后（包括 Promote），后续推理不再附带该文本。
6. 候选的模型推理和工具调用在 Promote 前仍受候选生命周期约束。候选不能把工具效果写入权威路径；Promote 只保留一个胜者并切换后续权威会话，旧 Parent 与其他候选按既有清理流程处置；Discard 取消并清理该候选，未验证输出不进入主对话。

上述语义只说明当前已接通的模型投机接缝，不表示已经解决所有模型投机策略问题。调用列表等价的规范化／语义相似规则、多模型成本与失败回退仍由 `Discussion.md` 第 40 项保留为未决问题。

当前实现中 Pi 是宿主上的单一进程，Authority 与全部 Candidate session 都由它托管；启用文件系统隔离的 Branch 环境另外持有一个常驻 namespace-holder。Pi 通过 ASC-Proxy 请求 Runtime 执行文件工具，Runtime 内部适配器再通过该环境的 holder socket 在已有 mount namespace 中执行，Pi 不进入 Branch namespace，也不直接访问 holder。control plane 持有逻辑 transcript，并在创建 Candidate 时继承父 Branch 的逻辑 transcript；Pi 的 Branch session 继续保留原生记录，以便最终汇总和从原生 checkpoint 续跑。`/app` 这类 workload 路径由 FilesystemBackend 的 `isolated_paths: [{path, target}]` 映射，host 目录仍是 overlay lower 与 publication 目标。holder 的命令各自独立执行，直接 shell 退出即返回，后台后代留在 Branch namespace 直到环境结束；物化轮换 execution generation 时旧 holder 结束，发布后建立新 holder。

## 2. 接口

`instrumentation/core/index.ts` 导出 `createInterposer(options): Interposer`。调用方提供：

| 选项 | 含义 |
| --- | --- |
| `taskId` | 上报事实中的 task 身份 |
| `report(message)` | 向 control plane 发送一条已构造好的消息 |
| `evaluate(artifact, decision, data, signal)` | 执行一次编译 Policy 的 Decision 求值；由接入进程注入，通用层不绑定具体 runner 位置 |

返回的接口：

| 方法 | 用途 |
| --- | --- |
| `beforeAction(point)` | 产品在真实执行前调用。执行 effect 闸门、rebalance pause、`admit`、`extent` 与相应上报，最后返回 `allow` 或 `admit` 处置 |
| `control(command)` | 按 control plane 命令维护 hold、本地 Policy 和 extent 计数；产品适配在命令处理入口调用 |
| `releaseAll()` | 进程退出时释放全部 hold |

`ExecutionPoint` 只承载通用事实：`branch`、`actionId`、`actionKind`、`name`、`arguments`、Policy 的 `op` 与 `contractContext`、是否允许准入、附加的 admission facts、取消信号。产品用函数形式的 `admit` / `admissionFacts` 表达“等到闸门放行后再判断是否是权威路径”和“准入成立时才生成产品事实”。

## 3. 当前覆盖

- `admit.opportunity_pass` 与 `admit.feasibility_pass` 在通用层求值；准入成立时上报 `speculative_admission_request`。
- `extent` 按 Branch 和 action identity 计数，在工具与模型执行前求值；结果为 `stop` 时上报 `branch_suspended(extent_reached)` 并等待 `resolve_pending` 或取消释放。
- `effect` 只对 `tool_execution` 执行点求值；`deny` 直接抛错，`block` 上报 `branch_suspended(effect_gate)` 并等待 Promote、Discard 或清理释放。模型推理是 PURE 步骤，不经过 effect 闸门。
- `rebalance` 的 pause/resume 与 extent/effect hold 都由通用层保存和释放；`branch_suspended` / `branch_resumed` 由通用层上报。
- control plane 命令的 hold 生命周：`start_candidate` 建立 effect hold，`promote` 释放 effect/extent，`discard`、`cleanup` 释放全部 hold 并清理本地状态，`resolve_pending` 释放 extent。

### 3.1 交互接口（2026-11-16）

目标语义见 [Agent 接入设计](../design/agent-integration.md) §3.1。2026-11-16 已实现：`instrumentation/pi/main.ts` 四处创建 session 后通过共同的 `bindInteraction` 调用 `session.bindExtensions({ uiContext })`，上下文由 `interaction.ts` 提供。`confirm/select/input/editor` 生成 `prompt_id` 并等待 control plane 的回答；AbortSignal、取消、Branch discard/cleanup/abort、恢复暂停及连接退出都结束等待。结束等待一律 resolve 而不 reject：取消、中止、断开都按 Pi RPC 宿主同样的默认答案收尾（`confirm` 为 `false`，其余为 `undefined`），只有不支持的 `custom` 直接 reject。这样扩展拿到的是“未确认／未作答”而非异常，避免未捕获 rejection 终止 Pi 进程；取消之后的结果如何处理仍归 Agent。绑定本身可能触发 `session_start` 提示，因此不占住命令队列；执行等待绑定完成，回答和取消能够独立送达。提示只传文字，答案原样交回 Pi，不增加 Policy Decision 或修改 extent/effect。

命令清单在初始权威 session 的 `bindExtensions` 完成后读取一次：这包括 `session_start` 和随后的 `resources_discover` 资源加载。启动提示先独立发布、可立即回答，不等待清单；执行等待绑定完成，但命令队列不等待提示。清单保存在 Pi 接入进程的 `commandCatalog`，control plane `command_catalog` 保存已发布清单，Ledger 保存 `commands_listed` 供 WebUI/TUI 重连折叠；权威切换不清空、不重取，输入 `/` 只筛选缓存。旧 `get_commands` 入口仅返回缓存，不触发 Pi 资源读取；不引入自动过期或轮询。项保留 `name/description/source/sourceInfo`；生产接入使用原生 inline extension 的公开 `getCommands()`，在绑定与资源发现完成后读取；SDK double 仍可提供 `session.getCommands()` 或原有资源来源。没有硬编码命令表或修改 Pi。选中命令原文交给 session.prompt；Pi 原生终端内建命令不另行复制。

协议：Agent 消息 `ui_prompt_requested(branch_id,prompt_id,kind,title,message_text,options,placeholder,prefill)`、`ui_prompt_cancelled(branch_id,prompt_id)`、`ui_notification(branch_id,text,level)`、`commands_listed(branch_id,request_id,commands)`；命令 `ui_prompt_answer(branch_id,prompt_id,answer,cancelled,reason)`、`get_commands(branch_id,request_id)`。HTTP action 为 `ui_prompt_answer(prompt_id,answer,cancelled?)` 和 `get_commands(request_id)`。回答在 control plane 事件循环同步校验当前权威与未回答状态，未知、重复、迟到回答返回明确错误（HTTP 409）；不为提示增加超时或默认答案。

`SpeculativeTree.ui_prompts` 按 Branch 保存提示，候选内容不进入观察事件。`change_authority` 在确认 Promote 后发布该 Branch 的挂起提示，同时取消换出权威的未答提示：提示已经展示过，但换出后任何回答都会被拒绝，继续显示只会留下无法操作的控件。`change_status` 进入丢弃／清理、interrupt、shutdown、失败及释放时关闭提示并排队取消命令。未知 Branch 或重复 `prompt_id` 也回一条取消命令，不把 Agent 的提示错误升级为连接失败。执行循环先发送这些取消，再发送原有生命周期命令。WebUI 仍通过原有 SSE 读取 `ui_prompt_requested/ui_prompt_resolved/ui_notification/commands_listed`，无第二条推送通道。

本轮验证：先用手造交互事件稳定复现回答后 Promise 不完成，再移除外层提前 `pending.delete`，由内部 finish 统一删除并 resolve；回答、明确取消、分支取消、AbortSignal 与无人回答保持等待均通过。实际 TUI 输入循环的手工字节回归覆盖 confirm/select/input/editor、中文退格、多行提交及 Esc 取消，不把答案当聊天。控制树 13 项相关测试通过，含候选提示挂起→换权威发布→回答、迟到／错类型拒绝、换权威取消与缓存查询。`tests/pi*.mjs` 全部 32 项通过；SDK double 的 session_start 先等待回答，再完成资源加载，断言清单读取一次、重复请求只回缓存。WebUI 手工事件核对换权威保留清单、关闭旧提示、重连折叠恢复缓存。`cargo check --all-targets`、WebUI typecheck/build、`git diff --check` 通过；构建保留现有大包体提示。未调用真实模型、未执行特权环境测试，也未完成真实终端／浏览器／真实扩展的人工全链路验收。

整轮中断中止权威执行、丢弃候选，并把仍在请求阶段的工具操作标记为失败。文件、shell 和 Runtime 工具通过同一执行协议转发取消，只有 backend 终态和遗留写者清理确认后才结束 Branch 取消；本地 Promise 结束不作为 backend 停止证据。

本轮补充验证：`cargo test --lib control_plane::` 27 项通过；`interaction.ts` 独立 TypeScript 检查通过，手工构造事件核对 confirm 的 false 原样返回、AbortSignal、取消及 custom reject 均通过。以上不等于真实扩展与浏览器全链路验收。

## 4. session 持久化与状态交接：已定方向及实现差距

2026-11-13 确认：运行分支采用 Pi 原生完整持久化，允许多 session 作为过渡，由 Instrumentation 汇总为单一最终权威 session。源分支停止写入且成功汇总后，才可删除对应临时文件；Promote 后仍活跃的 winner 不能删除。compaction、custom 等记录必须保存，不只保存 `message_end`。这些是已定实现方向，不表示当前适配器已经完成。

实现入口为 `instrumentation/pi/main.ts` 与 `session-tree.ts`；当前原生持久化、真实结果父链整理和停止写入后的汇总删除进展及验证范围见本节末。

Policy 的 `predict` 通过现有 proposal `payload` 交付预测值。Pi 在实际工具 checkpoint 后追加原生 `toolResult`。支持单个原生 `text`／`image` 块、非空的原生块列表，以及带原生 `content` 列表的结果体。结果体可携带 `isError`、`details`、`usage`、`nestedCalls`、`timestamp`、`durationMs`，调用 ID 和名称取自 checkpoint。没有显式状态时按成功处理，因此布尔 `false` 是成功的文本 `false`。字符串按原样交付，包括描述图像的字符串。其他 JSON 值（包括裸空列表）以 JSON 文本交付；结果体中的 `content: []` 保留为空。模型端的图像支持及请求格式由 Pi provider 决定，原生历史保留块顺序。任意工具专有字段没有通用映射。

`tests/pi-prediction-result.mjs` 通过正常 YAML Policy、control plane、Runtime 和真实 Pi session 接入本地伪 HTTP 模型。回归先复现了结构化结果被保存为 JSON 文本，再验证候选原生历史中的块顺序、失败状态和消息详情，以及下一请求中的文本和图像。同一轮只执行一次权威工具。候选续跑和恢复沿用原生 session 记录，尚未专项验证带图像候选的跨进程恢复。

必要状态交接的正确性由机制保证。文件系统可讨论以共同快照为基准，将真实执行的差异合入获胜候选；这只是讨论方向，不是已实现 Backend，也不是通用状态 merge 的授权。

尚未解决的实现差距：

- 完整持久化及汇总已保留原生身份、预测与原主／丢弃历史，未额外要求另存完整投机后续路径。并行工具处理沿用当前规则（2026-11-16 确认）：候选预测目标调用，等待父分支同批其他调用的真实结果齐全后再继续；规则确认不代表冲突、失败及全部并发场景已验收。
- compaction/custom 记录必须保存；原生父引用与已知 `targetId`／`firstKeptEntryId`／`fromId` 已有校验，任意 custom payload 的外部引用仍不能通用解释。compaction 摘要保留原文已确定，不阻塞汇总或满足其他条件后的源清理，不再列作待议事项。
- 在飞推理与历史整理的顺序已定（2026-11-16）：被接受的那一支自己的在飞推理和它的后代继续；同一 Chance 下其他候选及其子树失效，先停止写入、再清理；每一支的历史在它自己停笔后才整理。取消确实发生在清理之前尚未专门验证。
- rollback 的跨仓库 Git ref／object 与 NLE 外部运行资源迁移暂不在当前范围，同仓库快照关联与恢复已验证；最终 session 能恢复对话历史不等于已证明工具、环境和外部资源可完整继续。
- 历史重挂后，系统外导出按原生 session/events 还原请求关系；正常运行不计算或保存回放专用键，还原算法与验证边界见 [回放](replay.md)。

原始运行上下文与整理后恢复上下文可能不同，是用户接受的历史含义变化，不把二者相同列为验收承诺。是否向 winner 追加真实结果通知交由 Policy，默认完全不追加，不发送空消息；既有 dispose／Promote 承载可选 `notification`，不新增 Policy 名称或类别。

2026-11-13 实现进展：Pi 分支现在由独立的原生持久 `SessionManager` 文件承载，复制原生 entry 后在独立 manager 选择 checkpoint，保留原生 entry ID 及 compaction/custom 等非 message entry。Pi 的 `createBranchedSession` 会修改 receiver 并重建 label ID，不能在活跃 manager 上调用；此处保留完整索引以免丢失旁支引用。候选在待决工具 checkpoint 建立，不等待整个 turn。`message_end` 只在 Pi 已落盘后读取对应 native entry，避免重复写入 message。Instrumentation 先校验父引用，再写入临时汇总文件并替换最终文件；源分支失败时保留，活跃 winner 不删除。回放导出不能直接使用重挂后的父链。

验证：`node --test tests/pi-access.mjs tests/pi-session-history.mjs` 共 7 项通过，覆盖 checkpoint 原生记录、原生 ID、真实结果重挂、运行 manager 不变、活跃源保留、停止后清理、失败保留和已安装 `pi-code-rollback` 的同仓库快照关联。快照在 `/tmp/opencode` 临时 Git 仓库生成，核对 `targetId`、`repoRoot`、`ref^{tree}`、Git blob 可读及重开后的启动 GC；没有执行用户仓库恢复。`node tools/replay/verify/b1-pi.ts` 通过，使用本地回放服务，产物 `/tmp/opencode/ascout-replay-b1-pi-ENK910`。

后续审查已补上同仓库独立真实 Pi 文件任务继续及临时 Git 实际 rollback 恢复，并修复结果误选、候选重挂路径、独立重开位置和异步 shutdown 记录遗漏。跨仓库 Git ref/object、NLE 外部资源迁移不在当前范围（暂缓），不能把复制 session 文件当作完整 `pi-code-rollback` 支持。compaction 摘要已确认保留原文，不阻塞汇总或满足其他条件后的源清理，不重新生成摘要。

### 4.1 Promote notification

`Notifications` 跟踪已接受但未确认原生落盘的通知，以 `command_id` 关联 pending 与原生 details。发送使用 `triggerTurn:false`，不是 prompt／steer。读取源文件核对完整原生 entry 后才移除 pending 并报告 transcript；stopSession 在停止和 idle 后重试检查，失败不删除源文件、不报告 released。EOF 失败非零退出。Promote 已完成后通知失败报告既有 `adapter_capability_gap`，不伪称 Promote Failed。pending 只在进程内，不提供跨崩溃恢复保证。

真实 Pi SDK 加本地伪 HTTP 模型验证见 `tests/pi-notification.mjs`：缺省零记录、完整双工具批次后交付、下一自然请求可见、空闲无新请求、去重、汇总重开 ID／正文／关联及注入写失败后保留 pending 和重试。Rust 检查 schema／缺省序列化、真实结果与 evidence 分别进入 Policy、Runtime 失败不发通知与重复完成不重发。原实现验证为 `cargo test --lib` 144 项、`cargo check --all-targets` 通过；Pi notification/access/history-regressions/session-history/standalone 五组共 15 项通过。未调用外部模型或操作用户仓库恢复。全仓格式检查存在既有差异，未批量格式化。

2026-11-13 独立 adapter 审查新增 `tests/pi-notification-adapter.mjs`：真实 `main.ts` 子进程、Unix 帧协议和原生 `SessionManager`，SDK double 只模拟 `triggerTurn:false` 的运行中队列、abort、idle 及落盘事件，不模拟真实推理。9 项覆盖排队后的 cleanup／EOF／abort／自然 turn_end、缺省零通知、完整相同 `command_id` 重传（交付前后）及冲突 payload 拒绝；空闲与排队两种写失败均核对源文件保留、无通知 transcript、无虚假 released、EOF 非零退出，空闲接收失败另核对 `adapter_capability_gap(notification, failed)` 而 Promote 仍 Applied。正常 Promote 不 abort，不增加模型执行；通知重开核对正文、原生 ID 和 command_id。联合原五组共 24 项通过。Rust 协调层 8 项通过，新增两个 Chance 的 outcome／evidence 不串用检查；真实 Policy 的 context 输入与 Runtime 失败不交付沿用现有测试。本轮未发现需要生产代码修复的问题。

边界：SDK double 的 shutdown／故障注入不是完整原生 Pi adapter 端到端验收；原生 Pi 双工具队列与自然下一请求由独立 SDK 测试覆盖。没有验证跨崩溃去重、全系统恢复、跨仓库／外部环境资源迁移。`display:false` 只关闭 Pi 本地展示，不阻止模型上下文或 transcript 持久化；已记录目标语义未要求强制 display=true，本轮未改动。关于 `dispose.context.authority_outcome` 的批准来源，早先“已确认”的表述缺乏独立批准依据；2026-11-13 本轮用户明确回复“同意”，批准保留该字段，向 Policy 提供对应 Chance／operation 的真实结果，不自动给模型追加消息。此批准不追认早先已确认；本轮仅文档收尾，不改实现、不重跑测试，见 `decisions.md`。

上述审查的全量 `cargo test --lib` 为 144 通过、1 失败：无关的 `send_dump_drains_large_send_stderr` 在启动临时可执行文件时报 `Text file busy (os error 26)`；立即单项复跑通过。未修改该测试或声称上述审查全量全过。`Discussion.md` 保留既有验证记录，批准来源待核对项已由本轮明确批准收敛；该文件当前被 Git 忽略，未强制纳入版本控制。

## 5. 限制

- 2026-11-16：Pi `custom` 提示立即 reject 并说明不支持；终端组件、主题、状态与编辑器装饰接口仅最小空实现，`notify` 为文字观察事件。AScout TUI 提供 confirm/select/input/editor 回答与取消入口，多行 editor 可用，操作见 [TUI 使用说明](reference-workload.md)。无人回答保持等待，不补默认答案或超时；明确取消才按 Pi 的取消值结束。挂起提示不提供跨崩溃恢复，主动恢复暂停会取消提示。
- 只有 Pi 一个 Product Integration；sub-agent 启动前仍无统一 Hook。
- effect 只实现 `block` / `deny` 两种最小计划，其他效果模式由独立的效果协调能力承载。
- extent 计数的是工具执行点的 action 次数，不是模型语义上已完成步骤数；子 Chance 的计数独立于父 Chance。
- hold 的 Promise 只存在于接入进程内；主动退出保存 hold 所属 Branch 与工具／候选配对描述，跨进程重新建立等待，用户继续后才推进。control plane 仍是 Tree、Chance 和提交资格的唯一权威。实际验收与 backend 边界见 [恢复](recovery.md)。
- 通用层把 control plane 命令落实为 hold 的释放；Product Integration 负责放行或取消原调用，以及候选 session 的创建和接替。候选预测输入不用于替旧父调用完成。
- `node --test tests/pi-access.mjs` 覆盖 Pi 接入行为，使用伪造 SDK 与手工事件，不调用真实模型；本轮没有新增测试。

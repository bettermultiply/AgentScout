# 主动退出恢复

## 本次实现复核：发布前关联校验

本轮验证：`cargo check --all-targets`、控制面 55/55、Ledger/Projection 2/2、持久化完成记录部分写入回退 1/1；Pi 历史／通知／访问／恢复引用 19/19（`/tmp/ascout-ledger-check.log`），TS 检查通过（`/tmp/ascout-ledger-ts.log`）；恢复 7/7（`/tmp/ascout-ledger-recovery-final.log`）。缺失 Pi 引用明确报 ENOENT 的回归已覆盖。整链保留保存中间失败／重试及继续检查；旧兼容索引替换用例更新为确认 Ledger 仍选择最新代。实际发现并修复释放 Branch 删除 checkpoint 源，以及汇总父链替代原始父链造成恢复冲突。

当前 `ascout-control-plane <pi-root> --resume <run-id>` 按原 id 和运行记录目录打开两种状态。Ledger 有最新的有效保存时恢复原 task、树与活跃分支，普通消息排队，直到显式 `/continue`。最新 task 已结束时使用该次执行的 `AgentReleased` 权威引用，通过 Pi `SessionManager.open` 打开同一原生会话，显示历史并等待下一条 prompt 创建新 task。旧记录可使用 `session.jsonl` 引用和 `final-tree.json` 中的启动绑定。旧 `session-*` 目录照用。相同 run 已被打开时，第二个进程在修改记录和启动 Agent 前报错。

已结束路径只复用会话和启动绑定。新的执行身份与 Branch 身份将新操作和旧 Ledger 事实分开，后续暂停保存会记录这次执行的身份。旧命令、工具等待和 Branch 执行状态不进入新上下文。启动时原生源保持只读，下一条 prompt 到来后才允许合并新会话内容。正常终结优先于较早的保存，较新的开始记录或保存错误也不能被当作旧 task 已结束。

正常 CLI 回归 `tests/resume-ended.mjs` 使用隔离配置、真实 Pi 和本地假 provider。修正前在首次打开已结束 run 时失败，见 `/tmp/ascout-resume-before.log`。修正后覆盖无 checkpoint 的已结束 run、旧记录兼容引用、界面会话接口的历史、重复和并发打开、引用丢失、下一条 prompt 携带旧上下文、旧工具执行恰一次、下一 task 保存暂停与显式继续，以及终结后跳过旧 checkpoint。最终证据目录为 `/tmp/as-resume-Mfgjp2`，回归和原暂停恢复 7 项纳入 `/tmp/ascout-resume-node-final.log`（18 项通过）。`cargo check --all-targets`、串行 `cargo test --lib --bins`（221 项）和兄弟 Pi 的 SessionManager 文件及上下文测试（51 项）通过。

前台 control plane 一次管理一个 run。TUI 的 Ctrl-D、显式 Shutdown 或 SIGTERM 请求正常收尾并退出进程。SIGINT 与 TUI Ctrl-C 请求中断当前 task 并保留会话。Web 页面关闭只断开页面与仍在前台运行的进程之间的连接。暂停保存使用 `/pause`。Pi 源副本目前无自动回收，保存失败可能留下无引用的安全副本。任意外部等待、进程内存恢复和不确定命令重放仍超出恢复范围。

`recovery::publish` 在追加新控制快照与替换 `recovery.json` 前，复用加载阶段的
组件关联校验。缺失或不一致的 task／Authority、Branch Session、Environment、
Chance／Proposal 关联直接保存失败，沿既有失败路径保持暂停并保留旧标记。
新增回归先证明旧实现会把不一致组件发布为完成，再验证拒绝发布。
Ledger 现在追加 `recovery_complete`：包含完整控制快照、Agent／Runtime 附件路径及 SHA-256；agent 附件记录分支、checkpoint、hook 状态和 Pi session 引用。加载直接从完成记录的 control 重建 Tree，不再生成或要求 tree.json；旧代携带的 tree.json 也不作为恢复依据。附件与关联检查成功后，直接附件和保存目录同步，再在共享写入器内追加并同步完成记录。recovery.json 仅保留兼容索引，替换失败通过系统运行日志警告，已持久化完成不误报失败。加载从 Ledger 倒序选择最新 task 的有效保存。保存错误可以回查同一 task 的较早有效保存。遇到 task 开始或结束边界时停止回查。若 task 已结束，打开其权威历史。

同一进程中，同一规范路径的 Ledger::open 复用原写入器及失败状态；普通追加、观察序号发布、归档重试、完成记录追加／sync／回退在同一互斥边界内完成。等待观察事件只持 reader，不持写入锁；写入器锁不跨 Runtime 调用或附件准备。此处不增加跨进程多写者支持。故障回归先复现第二句柄绕过部分写入失败，再检查共享失败和 durable 回退期间另一句柄的成功追加不会被截断。

保存保证以已校验附件的进程重启恢复为范围。直接 agent.json、runtime.json、保存目录、运行目录和 Ledger 调用 sync_all；这不等于递归同步 Pi 引用的每个文件、Runtime 外部资源、Git 对象或所有后端的保存数据。因此不承诺任意断电后全部引用仍在；加载仍须校验引用，缺失或损坏时回退合法旧代或拒绝，不能宣称全链路 fsync 保证。兼容索引无需构成恢复依据。

本轮审查验证：`/tmp/ascout-review-repro.log` 记录修复前第二句柄绕过失败的复现；`/tmp/ascout-review-node.log` 为恢复整链 7/7 与 Pi 引用／绑定／历史／通知适配合计 20/20。最终 Rust 检查（控制面 59/59、Projection 2/2、cargo check --all-targets）见 `/tmp/ascout-review-final.log`；附件同步调整后再次运行恢复 7/7，见 `/tmp/ascout-review-recovery-final.log`。该次审查未调整待确认的 CLI 命名，也未实现 EEC。

Pi 源通过公开 SessionManager.create/forkFrom/open 管理；保存及恢复各建立独立 Pi 源，ASCout 不拼接 Pi 目录。run 内 session.jsonl 现在只是指向 Pi 管理文件的符号链接，兼容已有读取入口。AgentReady/AgentReleased 在 Ledger 中携带原生文件、id、cwd 引用。释放 Branch 不删除仍可能由 checkpoint 引用的源。恢复验证源摘要后才调用 open，缺失引用明确失败。保存附件不重复存储完整 Pi index，恢复使用 Pi 接口取回 entries；保留原始父链控制映射以避免最终权威汇总父链污染模型上下文。

完成记录的追加失败会修复追加边界，保留旧完成记录。兼容索引预备写失败也不提交；完成记录已落盘但索引 rename 失败只报警，Ledger 保存仍有效。当前完成写复用进程内共享 Ledger 写入器，不依赖“当前无并发”的假设。跨进程同时写同一运行仍未实现。

下一拦截点暂停现已替换取消式暂停：`suspend_recovery` 只设置暂停意图，
模型／工具执行前 hook 阻止下一动作；已经开始的调用继续，保留真实结果。
已在执行前等待的分支可计为停稳；自然结束的分支不要求再到达一个 hook。
已有候选初始化允许完成结果配对，但暂停期间不启动候选推理。
全部活动分支停稳后回报 `recovery_suspended`，保存再等待必要的初始化收敛，
通过既有 Pi SessionTree 和恢复记录保存 hook 位置、调用及结果事实，不保存 Promise。
同进程 continue 释放原 hook；重新加载后 continue 重建保存位置，已完成动作不重放。
普通 Prompt 仍由控制层排队；保存失败保持暂停。Interrupt 仍使用 abort／Runtime
cancel 主动取消执行，不等待暂停屏障完成，也不把中断后的暂停申请报告为成功。

夹具已改为先断言 Pause 等待且未取消已开始 sibling，再受控释放并确认真实结果、
下一推理未开始，然后才允许保存；两次 Runtime action 均应在保存中，替代旧的
“只保存第一次、取消第二次”断言。文件系统夹具的未结束模型流同样由测试受控结束，
覆盖保存 before-tool 位置、continue 执行一次，以及模型直接结束而无下一动作。
旧实现失败证据 `/tmp/ascout-pause-boundary-before.log`，断言为
`Pause must not cancel the already started sibling`。
新增等待中 SIGINT 用例无需释放 sibling 即完成取消回收。
本次验证：Rust 库 206/206（`/tmp/ascout-pause-rust.log`），
`cargo check --all-targets`（`/tmp/ascout-pause-check.log`）；Pi 恢复／通信／历史／
交互 13/13（`/tmp/ascout-pause-pi.log`），额外 Pi 检查 22/22
（`/tmp/ascout-pause-pi-extra.log`）。恢复六项及新增中断用例 7/7，最终日志
`/tmp/ascout-pause-recovery-final.log`；TS 检查启用 allowImportingTsExtensions
（`/tmp/ascout-pause-ts.log`），`git diff --check` 通过。
全部使用本地伪模型和受控工具，不调用真实模型；未修改 Pi 原生代码。

尚未实现：异常退出清理闭环，以及 Runtime External Effect Coordinator。检索现有 Runtime
发现 Firecracker 网络 namespace 配置，未发现通用外部 API 请求暂存／模拟适配；
namespace 本身不能提供 API 级拦截。未选择新网络技术、未增加模拟服务协议、
未绕过真实请求路径。本轮不据既有恢复回归通过宣称以上能力已实现。

验证：旧发布逻辑的新增回归失败，日志 `/tmp/ascout-implementation-publish-before.log`；
修复后库测试 206/206、Ledger/Projection 2/2，通过日志
`/tmp/ascout-implementation-rust-final.log`。恢复六项、通信取消、Pi 恢复绑定／源、
历史及交互联合 14/14，通过日志 `/tmp/ascout-implementation-node-final.log`。
`cargo check --all-targets`、bin 构建、TS 检查（启用 allowImportingTsExtensions）、
修改的 Rust 文件格式及 diff 空白检查通过。全仓 `cargo fmt -- --check` 仍报告其他
文件格式差异，未批量重排。仅使用本地伪模型及既有 Runtime 夹具，无真实模型请求。

## 当前范围

2026-11-16 审查补验：Authority 的首个后台写者现为持续写 heartbeat 的无限循环，
不存在外部 release；测试先确认写者仍活着且未发布，SIGINT 后要求 task 为 cancelled，
并确认完成后 heartbeat 不再增长。加强后的测试在本轮修改实现之前已经通过
（`/tmp/opencode/sigint-review-before.log`，证据 `authority-filesystem-cUpUV1`）；
因此本轮不把它描述为新复现的实现失败，也不再用旧夹具中 SIGINT 后写 release
的通过结果证明主动停止。既有 Abort → holder cancel drain 路径保留。

信号补验先确定性消费唤醒字节，再设置 pending 标志，不再补写字节；旧信号线程
在 500ms 门槛内失败（`sigint-review-signal-before.log`）。当前信号线程以 20ms
读取超时重新检查标志，标志使用 release/acquire 同步；pipe 仅为唤醒提示。
三个相继的晚到 SIGINT 与随后 SIGTERM 均能转发，不依赖后续 UI/Runtime 输入；
退出用 socket 写半部关闭，避免结束字节因非阻塞缓冲区满而丢失。
标准信号及布尔 pending 标志允许同类信号合并，不保证逐次计数；取消保持幂等。

缺失 authority、Branch 或 Environment 时不再 unwrap／直接传播启动错误，
而是记录 `materialization_failed` 并显示可重试失败，禁止报告 task 成功及放行下一任务。
测试先失败于缺失 Environment（`sigint-review-authority-before.log`），修复后覆盖三种缺失。
缺失绑定不会自动重建；状态未修复时重试仍失败，保留 task_finishing 是明确失败门槛，
不是等待一个永不到达的 Runtime 回执。此处不承诺损坏状态下 SIGTERM 一定正常退出。

本轮最终联合回归 `sigint-review-joint-final.log` 7/7，无跳过或取消，Authority 证据
`/tmp/opencode/authority-filesystem-dh4Lnm`。全库默认并发首次 175 通过、1 项
`extra_mount_fork_diff_accept_discard_round_trip` 失败；该项单独复跑通过
（`sigint-review-fs-recheck.log`），降低并发复跑 176/176（`sigint-review-lib-final.log`）。
本轮未修复或确定该挂载用例的偶发失败原因，不能称默认并发全库首次通过。
`cargo fmt -- --check`、`git diff --check`、`cargo check --all-targets`、
`cargo build --bins` 和 bin 测试（3/3）通过，最终日志为
`sigint-review-{check-final,build-final,signal-final}.log`，日志前缀均为
`/tmp/opencode/sigint-review-`；全程本地伪模型，未调用真实模型。

2026-11-16 物化等待中断补验：物化请求从控制事件循环移到工作线程，结果回到
同一循环处理；task 收尾期间只允许一个物化请求在飞。SIGINT 可在 holder 后台
写者等待期间发出 Abort；Abort 使用命令编号、代数与终态回执，不把普通
`turn_completed` 当作取消完成。物化结果与全部命令／操作回执收齐后才完成 task，
随后才放行已排队输入；Shutdown 等同样收齐后退出，Pause 不抢占收尾。
holder 的被动 drain 与取消 drain 串行确认，取消停止其拥有的后代并确认停止后
回复，避免被动物化先关闭 holder 而丢失取消回执。不使用任意超时宣称活写者结束。

失败→成功证据：`/tmp/opencode/materialize-interrupt-before.log` 在保持门控关闭时
稳定失败于 `SIGINT must terminate a resident background writer during materialization`，
目录 `/tmp/opencode/authority-filesystem-4x4TSg`；当时同一断言修复后通过，但旧夹具仍写
release，主动停止证据以后续无 release 补验为准。最终联合回归
最终复跑 `/tmp/opencode/materialize-interrupt-joint-final.log` 7/7、无跳过／取消，Authority
证据 `/tmp/opencode/authority-filesystem-0pPXAy`，含等待中 SIGINT 留会话、下一轮、
自然结束的后台写者、延后输入、Pause 交错、物化失败重试及等待中 SIGTERM。
库测试 175/175（`materialize-interrupt-lib-final.log`），Pi 接入等 16/16
（`materialize-interrupt-pi.log`），`cargo check --all-targets` 与 `cargo build --bins`
通过，日志均在 `/tmp/opencode/`。恢复 continue 夹具最后的 Pause 改为等待新 task
物化完成，避免将收尾期间按既定语义忽略 Pause 当作保存失败；保留活动任务暂停断言。
全部使用本地伪模型，无真实模型调用。任意不受管扩展写者、深层候选组合和原生
btrfs incremental 未由本次补验覆盖；不可终止的内核等待仍不得冒充写者已结束。

2026-11-16 入口增量：库测试 174/174、`cargo check --all-targets`、
`cargo fmt -- --check`、`cargo build --bins`、`git diff --check` 通过；Pi 接入、
交互和 notification 回归 16/16。日志 `/tmp/opencode/task-entry-{lib-final,check-final,build,pi}.log`。
Authority/恢复联合验证 7/7（无跳过、取消）通过，日志
`/tmp/opencode/task-entry-node-final3.log`。Authority 证据 `/tmp/opencode/authority-filesystem-fqtxBD`，
恢复五模式及 FS 主入口目录见该日志。覆盖 SIGINT 后同 session 下一轮、空闲控制、
暂停后中断、失败重试、后台写者等待、候选继续、晋级后写入及保存源不变。
补充“物化失败在 UI 状态可见”断言后单独重跑 Authority 1/1 通过，
日志 `/tmp/opencode/task-entry-authority-final.log`，证据 `/tmp/opencode/authority-filesystem-Bul8Vn`。
WebUI `npm run build` 通过（`/tmp/opencode/task-entry-web-build.log`）；未进行屏幕交互验收。
先用库测试复现 idle `/continue` 被算作用户输入（`task-entry-lib-before.log`），
再修复；Authority 门控回归复现物化早于后台写者结束（`task-entry-sigint-before.log`、
`task-entry-node.log` 中 `background writer prevents publication`），最终通过。不据局部通过宣称全部 Authority 完成。
环境后台写者需先结束再物化；等待期间的 Ctrl-C 验收与其余范围限制见本节顶部补验。

2026-11-16 确定：暂不要求程序异常退出后重新启动并继续原任务。已有主动保存、加载和恢复能力保留；本文件中的验证不因此扩大为崩溃恢复保证。异常退出后的受管进程停止及残留资源清理仍需处理，与任务是否能恢复分开，见 [Runtime 生命周期](runtime-lifecycle.md#当前范围执行承载资源清理与恢复)；本轮未选择或实现新的异常清理机制。

## 2026-11-16 Environment 身份与生命周期修正

本轮 Runtime 创建身份链路最终验证：`cargo fmt -- --check`、`cargo check --all-targets`、`cargo test --lib`（170/170）通过；先 `cargo build --bins`，再串行执行 `node --test --test-concurrency=1 tests/recovery-active.mjs`，五种模式全部通过，无取消或跳过。证据目录依次为 `/tmp/opencode/recovery-active-cy90lI`、`/tmp/opencode/recovery-active-3LMIAo`、`/tmp/opencode/recovery-active-R2uHDY`、`/tmp/opencode/recovery-active-QfT7ro`、`/tmp/opencode/recovery-active-tns9kK`。使用本地伪模型，保留动态端口、独立目录及原有超时和断言；本轮未单独重跑 `recovery-filesystem.mjs`，五模式脚本包含 filesystem 恢复场景。

Environment id 由 Runtime 在建立或派生时创建并返回，control plane 收到后持久化在 BranchRecord；恢复使用保存的绑定并显式加载 Runtime，不重新创建环境或根据恢复目录推导身份。创建中的 Branch 可以无绑定；已有绑定必须非空且不被其他 Branch 共用，存活环境必须存在于保存的 Runtime 状态中。Runtime 请求不传 Branch id，Agent 接入命令单独传递 Branch→Environment 绑定以选择对应文件工具计划。

通过 `BackendPolicy` 路由时，即使只有一个 Backend，`BaseRuntime` 也保存和校验参与记录。恢复按存活 Branch 的 Environment 重建 holder；Runtime 事件唤醒 control plane，避免 sibling 结果到达后仍等待其他输入。holder socket 经打开目录的 `/proc/self/fd` 路径连接，保留完整 Environment 身份且不受 Unix socket 全路径长度限制。

最终验收使用本地伪模型，没有真实模型请求：`cargo check --all-targets`、`cargo test --lib`（168/168）、Python bridge 语法检查通过；每个恢复脚本前均执行 `cargo build --bins`，串行运行 `tests/recovery-active.mjs`（5/5）及 `tests/recovery-filesystem.mjs`（1/1）。日志位于 `/tmp/opencode/agent-scout-validation/`，分别为 `check-final.log`、`lib-final.log`、`recovery-active-final.log`、`recovery-filesystem-final.log`。身份唯一性及共享世界生命周期回归均先复现失败再修正；特权项的环境条件与覆盖限制不变。

## 2026-11-14 当前契约与验证

最终联合验证：`node --test --test-concurrency=1 tests/authority-filesystem.mjs tests/recovery-filesystem.mjs tests/recovery-active.mjs`
7/7 通过，日志 `/tmp/opencode/authority-e2e-final7.log`；库测试
`cargo test --lib -- --test-threads=2` 161/161，日志 `/tmp/opencode/authority-lib-final5.log`。
`cargo check --all-targets`、`cargo build --bins` 和 `git diff --check` 通过。
这不把环境条件跳过的特权项算作实际特权验证，也不覆盖任意扩展宿主代码。

恢复使用恢复时的宿主普通 bottom lower，不保证其历史内容，不复制整个挂载。
受管 upper、层、候选、Agent 和固定 Policy 状态独立保存；加载创建独立可写 upper，
继续执行不修改保存代。代码、配置、source dependency 绑定与保存资料摘要校验保留。

Pi 常驻会话宿主不持有 Authority overlay。Branch 内建 read/edit/write/bash 每次使用
对应 Runtime 计划创建工具挂载；shell supervisor 在私有 PID namespace 中作为 PID 1，
等待包含脱离 shell 的后台后代全部结束。取消杀死该 namespace 的执行，收齐 action
回执后物化；物化成功新建 upper/work 并更新计划，不清空 live upper。

实测：FS 主入口恢复通过；生命周期验证覆盖后台写者、两个 task 不重放、取消、失败
重试及 SIGINT。filesystem 场景的暂停、跨进程恢复、候选晋级、旧父环境清理、晋级后
工具写入及再次保存通过。全部使用本地伪模型，未调用真实模型。

组合能力按所有状态域汇总：各状态域能建立独立身份与生命周期时才允许
branch_environment；内部资源可以共享。候选实际建立 Backend 环境引用，
不能以回退 authority 文件计划规避。清理原 Authority 文件系统资源时不终止拥有整棵树的
常驻 Pi，宿主进程留到 Runtime shutdown 统一退出。证据：
`/tmp/opencode/authority-active-final5.log`，`/tmp/opencode/recovery-active-8nPRJI`；
联合运行 `/tmp/opencode/authority-e2e-final6.log` 中 FS 活候选与普通 FS 恢复均通过。

尚未覆盖任意扩展工具的写者约束、深层候选组合和全部存储机制。不能据此宣称
所有接入与所有存储机制已完成；原生 btrfs incremental 未在本机验收。

2026-11-16 目标与实现差距：Agent 常用的 Ctrl-C 或退出均终止任务，须结束候选并
等待任务相关写入完成后物化。Ctrl-C 保留程序和 session，留在对话中，待上述处理
完成后可接下一轮；Runtime 报告物化失败时须显示失败，control plane 不能宣告任务成功。
暂停任务由 AScout 提供独立于原 Agent 的 `/pause` 入口，明确继续命令为 `/continue`，
恢复原任务执行，保留原树、候选和环境关系。暂停期间的普通消息先留着，不解除暂停；`/continue` 之后再按原顺序交给模型。
它与原 Agent 的 Ctrl-C／退出终止语义分开，暂停不意味着退出程序。独立暂停入口的
具体 UI 未定。task 是 Agent 的一轮
工作，Agent 完成本轮工作并开始等待用户输入时完成。

当前 SIGINT 与 TUI Ctrl-C 走 `Interrupt`，终止当前 task 后保留 control plane 和 session；
SIGTERM 与显式 Shutdown 仍退出。Pause／`/pause` 保存整树；`/continue` 经专用
控制命令恢复原任务而不新增模型用户消息。暂停期间（包括保存尚未完成时）的普通消息进入待处理队列，不解除暂停；`/continue` 之后再按原顺序交给模型。保存尚未完成时 `/continue` 不启动工作；终止期间不再允许 Pause
阻塞物化，重复取消不重复发出中断。物化失败保留重试状态且不报告 task 成功。
保存记录包含 full 后的 Branch 与目标环境绑定，以及 part 尚待交给下一次模型调用的内容。保存和物化分别由各自回执确认。见
[物化提案](../fs-backend/materialize-proposal.md)。

回放工具终端的 `pause` 仅暂停后续自动输入，不是此处的任务暂停入口；
已有 Policy／Runtime 的 `pause` 操作也不能据同名视为任务保存入口。

Pi 源码依据（相邻 ../pi checkout）：

- `packages/coding-agent/src/core/agent-session.ts` 的 `abort()` 立即调用 `agent.abort()`，取消 retry／compaction／branch summary，再等待 idle。
- 同文件 `_handleAgentEvent` 在 `message_end` 追加原生 assistant、toolResult、custom message；部分 assistant 的终态由 agent loop 提供。
- `packages/agent/src/agent-loop.ts` 将 stream 的最终部分消息交付 `message_end`，工具准备阶段检查 AbortSignal，已开始工具通过同一 signal 协作取消。取消不撤销已发生的外部副作用。
- `packages/agent/src/agent.ts` 的 `continue()` 不允许从 assistant 直接继续；重新打开 session 不自动启动推理。恢复等待下一条用户输入。

保存恢复能力：直接中断、保存整棵有效树及原上下文、校验后恢复。保存失败不得触发原文件或环境删除，也不得报告保存成功。最终单一 Pi 历史与运行恢复元数据职责不同。

## 当前实现

control plane 的 Pause 等待下一拦截点并收齐已开始调用的真实结果，再保存原生会话引用、接入状态及控制快照；Interrupt 保留 abort 语义。NLE 保存初始 RNG/configuration、动作序列及逐步校验值。资料写入 run 内独立目录，经组件关联和摘要校验后追加 Ledger 完成记录。恢复还校验 Policy、Runtime、资源和模型绑定，等待明确继续。保存失败保留暂停及旧完成状态。本能力不保证任意外部等待跨崩溃恢复，也不自动重放不确定外部命令。

### 活动工具与候选等待（2026-11-13）

`SuspendRecovery` 先令 Pi 原生取消，`RecoverySuspended` 确认后，control plane 继续接收既有异步操作结果及 Promote／ResolvePending／Discard／Cleanup 回执；这些动作不盲重做。待 pending work 和 pending commands 均清空才发送 Save。正在执行的真实 Runtime 动作收齐结果再保存，不承诺无限阻塞的 Runtime 能立即退出。尚未发出的动作保存 Chance 后由继续调度启动。

Instrumentation 保存 `suspendedHooks`（Branch、toolCall 身份、原 assistant 与参数、是否已准入）、`candidateSetups`（原 StartCandidate 命令及 sibling 配对关系）、中断中的 Branch 集合，以及原有 checkpoint/results/access holds。没有序列化 Promise。原生工具取消的 toolResult 仍保存在原生记录中；保存期间的取消 ActionFinished 不将 control plane 工具等待置为 Failed，不丢弃 Chance。已开始的非 Runtime 原生工具遵循 Pi 取消终态，不重放外部副作用；未开始的工具可重新过原 hook。

用户显式继续后，Instrumentation 重新建立 Runtime result waiters 与 sibling result waiters，使用 Pi `runToolCall` 接回工具边界；已准入工具保留 effect/pause/extent 等待但不重复准入或计数。Runtime execute 只等待／取已保存结果，不重新发出真实动作。一个 assistant 的未完成工具并行重连，避免“先等待预测工具导致 sibling 永远未启动”的死锁。工具完成后在原生历史追加真实结果，当前模型上下文仅保留该调用最新结果；候选配对完成后原生 continue，所有分支只在用户继续后推进。暂停期间的工具及候选启动截止时间顺延。

已经释放的 Branch 仍可能被 checkpoint／后代输入引用。保存时若其原生源已由成功汇总清理，则从仍保留的原生 SessionManager 完整 entries/header 生成保存副本；其内容校验仍按保存副本计算，恢复继续复制成独立可写源。

入口（run-dir 为原运行目录，保留原生 artifact、恢复资料、Runtime 配置及工作区）：

```sh
cargo build --bins
target/debug/ascout-control-plane <pi-root> --resume <run-id> --web 127.0.0.1:8080 --model <provider/model>
```

保存通过 Pause／`/pause`，成功后程序保持暂停等待继续；目录包含 `recovery-<timestamp>/{agent,runtime}.json`，Ledger 的完成记录保存 Tree 并决定恢复代。Pi 保存分支的原上下文，ASCout 保存引用。NLE 重建只在固定版本和固定配置下校验，不声称公共 observation/RNG 校验是任意隐藏状态等价的数学证明。任意外部工具副作用不能靠此重放。

### 保存恢复与生命周期的库边界（2026-11-14）

- `src/control_plane/recovery.rs` 保存组件摘要、原子发布 manifest，读取并校验树／Agent／Runtime 关联及启动配置绑定；读取完成后通过 Tree 方法进入恢复状态。
- `src/control_plane/speculative_tree/tree.rs` 拥有恢复状态、暂停时刻、继续时期限顺延及任务结束状态转换；字段仅库内可写，二进制使用只读查询。
- `src/control_plane/coordination/recovery.rs` 组织 SuspendRecovery → 回执及在途工作收齐 → Save → SaveRuntime → 发布 → CancelSave；失败保留暂停状态与旧 manifest。该对象只保存当前保存尝试的路径和回执进度，不复制 Tree。
- `src/control_plane/coordination/task.rs` 组织终止与物化，物化记录写出后才结束 task／发出关闭命令；`coordination/execution.rs` 解释 Runtime 完成结果并组织共享／独立环境的最终清理。
- 二进制保留参数和进程装配、信号与事件等待、异步工作派发、Gateway 发送、Ledger／UI 呈现；不把 UI 文件整体移入库。不改 Gateway 三侧边界。

本轮最终验收（2026-11-14）：`cargo check --all-targets`、`cargo build --bins`、`git diff --check` 通过；`cargo test --lib` 153/153，Node 联合回归 25/25（命令见文末，增加 `--test-concurrency=1`）通过。日志分别为 `/tmp/opencode/control-plane-refactor-verify-{check-complete,build-latest,lib-complete,node-complete}.log`。所有模型请求均为本地伪模型，没有调用真实模型。

恢复整链现使用 Pause 发布保存代，并断言 control plane 仍存活；随后仅 SIGKILL 本 fixture control plane，等待 Adapter EOF 清理其拥有的进程，再以 `--resume` 跨进程恢复。保留候选／sibling、原上下文、绑定拒绝、真实 Runtime 动作恰一次、notification 恰一次、保存源不变及旧代独立恢复断言。四种活动工具流程证据为 `/tmp/opencode/recovery-active-pXQuHa`（正常）、`recovery-active-KQI4sG`（保存失败后重试）、`recovery-active-WnTUS4`（保存失败后继续）、`recovery-active-WnMj2Z`（两个 NLE backend）。FS 完整流程证据为 `/tmp/opencode/recovery-filesystem-gh0VVb`，并独立验证 Shutdown 退出码 0、无任务进程残留且不覆盖已有 recovery manifest。Pause 本身不退出；此验收不声称任意崩溃中的状态均可恢复，也不解决本文开头记录的保存全量 lower 限制。

本轮修复：保存代统一建在原 run-dir 下，避免重复 Pause 嵌套到上一代；恢复后输入先解除恢复状态，再等待上一 task 的物化，避免 `task_finishing` 阻塞继续；取消期间 Promote 选出新权威后由库再次停止写者；关闭期间晚到的 TurnCompleted 不再触发重复物化。错误退出和清理完成判定收在 `coordination/task.rs`，bin 只派发动作并关闭连接；coordination 未引用 UI 类型。

归因依据为 `/tmp/opencode/control-plane-refactor-baseline/{staged,unstaged}.diff`：`unstaged.diff` 第 1666–1727 行已包含任务继续的 `task_finishing` 门槛，因此原继续测试需保留晋级与路由断言，同时先断言物化前阻塞、完成后放行。FS cleanup 的既有 BaseRuntime 聚合将 NLE 保留的 Cleaned 与 FS 删除记录后的 None 误报为部分可用；先用新增回归测试复现失败（`/tmp/opencode/control-plane-refactor-cleanup-repro.log`），再修正聚合。FS 多目标布局测试改用两个自有 lower 目录，保留 fork／rename／旧格式拒绝断言，避免遍历宿主权限受限内容。复制宿主内容的空间问题由并发 FS 物化实现改动解决，本轮不将该实现认领为生命周期修复；检查时 `/home` 约 73G、`/tmp` 约 2.5G 可用，未大清理或删除其他任务文件。

正常启动原生保存完整 `policy.json`，恢复默认使用它，外部原 YAML/脚本不再必需；显式传入 Policy 时核对相同 digest。缺失 artifact 必须显式补充原 YAML，不能采用当前默认策略；损坏或绑定不符直接失败。Runtime 未显式覆盖时使用原 binding，路径/内容/能力校验保持。精确保存范围与验证见 [Policy 实现](policy-engine.md)；完整 Runtime 配置保存不在此轮范围。

## 实际验证（2026-11-13）

- `cargo check --bins`、`cargo build --bins`、`cargo test --lib --no-run` 通过（最后一项是编译，不是执行全部 Rust 测试）。
- `node --test tests/pi-access.mjs tests/pi-session-history.mjs tests/pi-notification.mjs tests/pi-notification-adapter.mjs`：18/18。
- `node /tmp/opencode/check-recovery.mjs`：真实 Pi + 本地假 OpenAI SSE 服务 + 真实 NLE；模型流永不自然结束，Shutdown 触发取消并保存退出；跨进程 `--resume` 不请求模型，用户输入后请求第二次并再次保存退出。通过，证据 `/tmp/opencode/control-recovery-wKIQIV`。测试只请求本地假模型。
- 真实 NLE 手工 JSON-lines 驱动：30 动作、3 环境引用、保存时重建校验、进程退出、加载、observation 连续、下一动作、再次保存，通过；证据 `/tmp/opencode/nle-recovery-final-34_tgbvl`。
- 原生 SessionTree snapshot/restore 手工检查通过，证据 `/tmp/opencode/recovery-native-z0ck82`。
- 最后重建后再次执行 `node /tmp/opencode/check-recovery.mjs` 通过，证据 `/tmp/opencode/control-recovery-3xEDZs`；`cargo test --lib runtime::adapter -- --nocapture` 5/5，通过，包括预期注入 panic 的队列恢复。

## 验收与剩余边界

- `node --test tests/recovery-active.mjs` 先在原实现复现 Shutdown 不退出（证据 `/tmp/opencode/recovery-active-8zpJIk`），实现后通过真实 Pi + 真实 NLE + 本地假模型：authority 同一 assistant 发两个工具，两个有效候选等待 sibling，真实 sibling NLE 动作在途时退出，收齐一项动作后保存，所有任务进程关闭；重启不请求模型，用户继续后三个 Branch 实际请求模型、两个候选获得真实 sibling 配对结果，原两个动作最终恰执行两次，再次保存退出且保存源不变。最终联合回归证据 `/tmp/opencode/recovery-active-6rDa6i`。这验证了核心等待恢复，不代表全部 backend／extension 恢复。
- FS portable overlay 与 btrfs snapshot-copy 保存、独立可写恢复、恢复后 fork 和再次保存已通过真实环境验证；FS + Pi control plane 主入口已通过下述整链。原生 btrfs incremental backend 本机探测失败，不能据 snapshot-copy 结果宣称其验收通过。不保存任意子进程内存。
- 恢复检查 task/authority、存活 Branch 的原生 session、Chance/Proposal 引用、Runtime 参与环境集合和 NLE world 索引；不据此宣称任意复杂部署及深层语义均已验证。无限阻塞 Runtime 或故障导致的回执无法收齐目前不能保存成功；外部原生工具只保留 Pi 取消终态，不承诺恢复子进程内存。
- Pi 原生源通过公开 SessionManager.forkFrom 保存和恢复为独立可写源，checkpoint 按保存源身份重新关联；继续原运行或恢复运行不会改写该代 Pi 源。package-managed 扩展、glob、配置与代码绑定及外部 Git snapshot 保留见下方。
- Promote notification 已在真实 main 入口、真实 Pi + NLE + 假模型的保存失败后继续／晋级／再次保存／恢复中验证最终历史恰一次；尚未验证所有通知交付时刻的跨恢复组合。

早期 `cargo build --bins`、`cargo test --lib control_plane -- --nocapture`（26/26）、Pi 联合回归含新整链用例（20/20）通过。既有无工具、永不自然结束模型流 `/tmp/opencode/check-recovery.mjs` 通过，证据 `/tmp/opencode/control-recovery-wiBhe4`。没有调用真实模型；后续扩展验证见下方。

## 原生源独立性修复（2026-11-13）

`node --test tests/pi-recovery-sources.mjs` 先复现继续原运行后 `Recovery source changed`；修复后验证原运行追加、恢复后追加及再次从同一代恢复，保存副本内容不变。恢复前检查源唯一性、完整记录数、引用、父链环、Branch tail 与预测源关系，checkpoint 仅引用的源也纳入保存。该用例使用真实 Pi SessionManager，不调用模型。

`node --test tests/pi-recovery-sources.mjs tests/pi-access.mjs tests/pi-session-history.mjs tests/pi-notification.mjs tests/pi-notification-adapter.mjs`：19/19；`node /tmp/opencode/check-recovery.mjs`：真实 Pi + 真实 NLE + 本地假模型跨进程中断／等待输入／继续／再次退出通过，证据 `/tmp/opencode/control-recovery-Pzx5zV`。本轮尚未验证 authority + 两候选含阻塞等待。

新增保存代还绑定解析后的模型对象、模型选择参数、thinking level 和 Runtime tools；恢复在建立 session 前检查上述内容及 authority/checkpoint owner/调用身份与原生末条消息。旧格式缺少该字段时仍按旧格式读取，不追认其配置保证。新增绑定后再次执行 `node /tmp/opencode/check-recovery.mjs` 通过，证据 `/tmp/opencode/control-recovery-Z1vV5T`。

## 多组件、失败继续与资源绑定（2026-11-13）

BaseRuntime 在 `*.components/` 中按登记身份存放组件，文件名由组件索引提供。所有组件保存成功后写入索引，最后 control plane 发布总 manifest。索引绑定 backend 顺序、身份、组件 SHA-256 与参与环境集合。加载先检查同级所有组件字节，再逐个重建，并核对参与者与实际环境集合；中途失败对所有组件执行 shutdown，错误包含清理失败信息，不启动 Agent 推理。不提供已加载组件的内存事务回滚。

FS 复制存活环境的受管 upper、lower/snapshot 与已执行操作回执；普通宿主 bottom lower 不复制，恢复使用恢复时内容。保存数据树校验文件内容、类型/权限、symlink、OverlayFS opaque/whiteout/redirect/metacopy。OverlayFS 内部 work 目录不复制，恢复重建；btrfs 子卷布局记录后在恢复目录重建。BaseRuntime 复制期间冻结拥有的执行进程，失败也尝试 thaw。每次恢复另建 `restored-*` 可写副本；保存代和旧运行目录均不覆盖。可用 `filesystem.isolated_paths` 显式指定独立工作目录，`passthrough_rw` 为宿主直通，不具有保存独立性。系统只读依赖绑定 pacman/dpkg 的已安装版本列表与 sh/env/bwrap 内容摘要，不复制整个宿主 root；这不是对未受包管理文件或任意系统文件原地篡改的完整证明。

Pi SDK 默认使用 DefaultResourceLoader 自动加载全局/项目扩展，因此保存绑定 Node 版本、Pi 与接入代码内容、模型配置、全局和项目 settings/extension/skill/prompt/system prompt/AGENTS 资源。使用 SDK `DefaultPackageManager.resolve()` 解析 package 与 glob，绑定有效资源、package root 及其已安装 node_modules；验证期间不安装缺失包或执行扩展代码。恢复在创建 session、执行扩展代码前验证；变化明确拒绝。任意目录外动态依赖、第三方服务及全部间接依赖发现不在保证范围，不能作为完整 extension 环境部署方案。

`rollback-snapshot` 绑定 repoRoot、Git directory、原 ref、tree/commit，并建立独立 `refs/ascout/recovery/<digest>` 保留对象可达性；只执行读取和 update-ref，不 reset/checkout 工作区。恢复验证保留 ref、commit/tree；原 ref 删除或移动不影响独立保留对象，保留 ref 缺失则明确失败。保留 ref 目前没有自动回收，保存失败可能留下安全但冗余的 ref。复制 session 不代表复制 Git 对象。

故障整链通过占用 `recovery.tmp` 让中断后最终发布失败：进程留在暂停，manifest 字节不变；分别验证直接重试及用户继续、全部三个 Branch 继续、两个原动作总共恰执行两次，再次保存退出、旧保存代独立恢复。该用例还复现并修复已晋级候选保留原生预测源、父 Branch/其他候选已清理时被错误拒绝恢复的问题；重试开始显式清除旧 saved 标记。

实际命令：`cargo test --lib recovery_overlay_executes -- --nocapture`（1/1，隔离 `/tmp/opencode/fs-recovery-*` 真实 shell）；`node --test tests/recovery-active.mjs tests/pi-recovery-binding.mjs`（5/5）。最新整链证据分别为 `/tmp/opencode/recovery-active-kkS0PZ`（原路径）、`recovery-active-t4UK1Q`（失败重试及模型配置变化拒绝）、`recovery-active-xRogpp`（失败继续与 notification）、`recovery-active-qFyc9n`（两个 NLE backend）。所有模型请求均为本地假服务。

补充执行 `cargo test --lib recovery -- --nocapture`（2/2）：第二项在真实 sandbox writer 运行时冻结复制、注入复制失败、验证 thaw 后计数继续增长及上一代恢复；`cargo test --lib control_plane -- --nocapture`（26/26）；Pi access/history/notification/recovery-sources/recovery-binding 联合回归（20/20）。

## FS 主入口与保存独立性验收（2026-11-13）

`tests/recovery-filesystem.mjs` 使用两个真实 NLE backend、一个普通登记的 FS backend、Pi bash 和本地假 SSE 服务：实际隔离写入，暂停保存后可跨进程恢复；中间 FS 组件保存失败时旧 manifest 不变、运行保持暂停并可重试；加载中途 FS 内容损坏时清理先前已加载组件且不请求模型；package 扩展代码变化在推理前拒绝。恢复不自动推理，用户继续后读取当前宿主 lower 与保存的受管写入并继续写；再次恢复旧代有效，旧保存文件摘要不变。

实际命令：

```sh
cargo build --bins
cargo test --lib runtime -- --nocapture
node --test tests/recovery-filesystem.mjs tests/recovery-active.mjs tests/pi-access.mjs tests/pi-session-history.mjs tests/pi-notification.mjs tests/pi-notification-adapter.mjs tests/pi-recovery-binding.mjs tests/pi-recovery-sources.mjs
```

Runtime 118/118（含现有环境条件跳过的用例）、Node 联合回归 25/25 通过；最终 FS 主入口证据 `/tmp/opencode/recovery-filesystem-65JoGp`。所有模型请求均为本地假服务。`recovery_btrfs_and_snapshot_copy_layouts` 在隔离 `/var/tmp/ascout-recovery-btrfs-*` 验证 snapshot-copy 和普通 overlay 的保存、恢复、fork、再次保存；本机探测报告 “btrfs create/snapshot works but incremental diff is unavailable; falling back to overlay”，原生 btrfs incremental backend 仍是环境验证缺口，不修改宿主挂载绕过探测。

剩余限制：原生 btrfs incremental backend 未完成环境验收；保存失败可能留下未发布的 generation 子目录，加载失败的目录清理错误尚未完整上报；系统绑定不覆盖任意未受管文件和全部可执行依赖；任意外部扩展依赖／服务、无限阻塞 Runtime、崩溃恢复及任意进程内存恢复均不在本次保证范围。

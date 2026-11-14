# 主动退出恢复

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

组合能力按所有状态域汇总：全部可共享才允许 shared_environment；各状态域支持
独立分支或共享引用时才允许 branch_environment。候选实际建立 Backend 环境引用，
不能以回退 authority 文件计划规避。清理原 Authority 数据面时不终止拥有整棵树的
常驻 Pi，宿主进程留到 Runtime shutdown 统一退出。证据：
`/tmp/opencode/authority-active-final5.log`，`/tmp/opencode/recovery-active-8nPRJI`；
联合运行 `/tmp/opencode/authority-e2e-final6.log` 中 FS 活候选与普通 FS 恢复均通过。

尚未覆盖任意扩展工具的写者约束、深层候选组合和全部存储机制。不能据此宣称
所有接入与所有存储机制已完成；原生 btrfs incremental 未在本机验收。

2026-11-14 目标与实现差距：Agent 常用的 Ctrl-C 或退出均终止任务，须结束候选并
等待任务相关写入完成后物化；Runtime 报告失败时，控制面不能宣告任务成功。
暂停任务由 AScout 提供独立于原 Agent 的 `/pause` 入口，保留树、候选和环境等待继续；
它与原 Agent 的 Ctrl-C／退出终止语义分开，暂停不意味着退出程序。具体 UI、继续入口的
命令名称及 Ctrl-C 是否同时关闭整个程序尚未确定。task 是 Agent 的一轮
工作，Agent 完成本轮工作并开始等待用户输入时完成。

当前 `ascout-control-plane.rs` 将 SIGINT／SIGTERM 转为 Shutdown 请求；TUI
捕获的 Ctrl-C 走 `Interrupt`。工作区已接通 Pause／`/pause` 的保存入口，
Shutdown／Interrupt 则走终止、等待写入及物化路径，Shutdown 随后关闭程序。
已有保存不构成物化完成，候选被接受后继续使用候选环境的关联与交接仍需核查，
不能以能恢复环境证明接受路径已对齐。见
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

控制面的 Pause 请求 Pi 同时 abort 所有 session，停止调度新工作，收齐已发出控制操作和命令的确定回执，然后保存原生消息、原 Branch 源、接入状态和 Tree；NLE 保存完整初始 RNG/configuration、动作序列及逐步校验值。保存使用独立目录，最后发布包含三个组件 SHA-256 的 `recovery.json`。恢复校验组件、Policy digest、Runtime 配置内容和能力后加载，等待用户输入，不自动推理或处置候选。保存失败报告失败并保留状态，原生执行已取消，后续输入重新连接等待；不删除唯一原生源。下方既往验证使用当时的保存入口，不代表当前入口已经通过同样验收。

### 活动工具与候选等待（2026-11-13）

`SuspendRecovery` 先令 Pi 原生取消，`RecoverySuspended` 确认后，控制面继续接收既有异步操作结果及 Promote／ResolvePending／Discard／Cleanup 回执；这些动作不盲重做。待 pending work 和 pending commands 均清空才发送 Save。正在执行的真实 Runtime 动作收齐结果再保存，不承诺无限阻塞的 Runtime 能立即退出。尚未发出的动作保存 Chance 后由继续调度启动。

接入侧保存 `suspendedHooks`（Branch、toolCall 身份、原 assistant 与参数、是否已准入）、`candidateSetups`（原 StartCandidate 命令及 sibling 配对关系）、中断中的 Branch 集合，以及原有 checkpoint/results/access holds。没有序列化 Promise。原生工具取消的 toolResult 仍保存在原生记录中；保存期间的取消 ActionFinished 不将控制面工具等待置为 Failed，不丢弃 Chance。已开始的非 Runtime 原生工具遵循 Pi 取消终态，不重放外部副作用；未开始的工具可重新过原 hook。

用户输入后，接入侧重新建立 Runtime result waiters 与 sibling result waiters，使用 Pi `runToolCall` 接回工具边界；已准入工具保留 effect/pause/extent 等待但不重复准入或计数。Runtime execute 只等待／取已保存结果，不重新发出真实动作。一个 assistant 的未完成工具并行重连，避免“先等待预测工具导致 sibling 永远未启动”的死锁。工具完成后在原生历史追加真实结果，当前模型上下文仅保留该调用最新结果；候选配对完成后原生 continue，所有分支只在用户继续后推进。暂停期间的工具及候选启动截止时间顺延。

已经释放的 Branch 仍可能被 checkpoint／后代输入引用。保存时若其原生源已由成功汇总清理，则从仍保留的原生 SessionManager 完整 entries/header 生成保存副本；其内容校验仍按保存副本计算，恢复继续复制成独立可写源。

入口（run-dir 为原运行目录，保留原生 artifact、恢复资料、Runtime 配置及工作区）：

```sh
cargo build --bins
target/debug/ascout-control-plane <run-dir> <pi-root> --resume --web 127.0.0.1:8080 --model <provider/model>
```

保存通过 Pause／`/pause`，成功后程序保持暂停等待继续；目录包含 `recovery-<timestamp>/{tree,agent,runtime}.json` 和 `recovery.json`。恢复不能只复制最终 `session.jsonl`；`sessions/` 保存各 Branch 的原上下文。NLE 重建只在固定版本和固定配置下校验，不声称公共 observation/RNG 校验是任意隐藏状态等价的数学证明。任意外部工具副作用不能靠此重放。

### 保存恢复与生命周期的库边界（2026-11-14）

- `src/control_plane/recovery.rs` 保存组件摘要、原子发布 manifest，读取并校验树／Agent／Runtime 关联及启动配置绑定；读取完成后通过 Tree 方法进入恢复状态。
- `src/control_plane/agent_control/tree.rs` 拥有恢复状态、暂停时刻、继续时期限顺延及任务结束状态转换；字段仅库内可写，二进制使用只读查询。
- `src/control_plane/coordination/recovery.rs` 组织 SuspendRecovery → 回执及在途工作收齐 → Save → SaveRuntime → 发布 → CancelSave；失败保留暂停状态与旧 manifest。该对象只保存当前保存尝试的路径和回执进度，不复制 Tree。
- `src/control_plane/coordination/task.rs` 组织终止与物化，物化记录写出后才结束 task／发出关闭命令；`coordination/execution.rs` 解释 Runtime 完成结果并组织共享／独立环境的最终清理。
- 二进制保留参数和进程装配、信号与事件等待、异步工作派发、Gateway 发送、Ledger／UI 呈现；不把 UI 文件整体移入库。不改 Gateway 三侧边界，不调整 `agent_control` 整体命名。

本轮最终验收（2026-11-14）：`cargo check --all-targets`、`cargo build --bins`、`git diff --check` 通过；`cargo test --lib` 153/153，Node 联合回归 25/25（命令见文末，增加 `--test-concurrency=1`）通过。日志分别为 `/tmp/opencode/control-plane-refactor-verify-{check-complete,build-latest,lib-complete,node-complete}.log`。所有模型请求均为本地伪模型，没有调用真实模型。

恢复整链现使用 Pause 发布保存代，并断言控制面仍存活；随后仅 SIGKILL 本 fixture 控制面，等待 Adapter EOF 清理其拥有的进程，再以 `--resume` 跨进程恢复。保留候选／sibling、原上下文、绑定拒绝、真实 Runtime 动作恰一次、notification 恰一次、保存源不变及旧代独立恢复断言。四种活动工具流程证据为 `/tmp/opencode/recovery-active-pXQuHa`（正常）、`recovery-active-KQI4sG`（保存失败后重试）、`recovery-active-WnTUS4`（保存失败后继续）、`recovery-active-WnMj2Z`（两个 NLE backend）。FS 完整流程证据为 `/tmp/opencode/recovery-filesystem-gh0VVb`，并独立验证 Shutdown 退出码 0、无任务进程残留且不覆盖已有 recovery manifest。Pause 本身不退出；此验收不声称任意崩溃中的状态均可恢复，也不解决本文开头记录的保存全量 lower 限制。

本轮修复：保存代统一建在原 run-dir 下，避免重复 Pause 嵌套到上一代；恢复后输入先解除恢复状态，再等待上一 task 的物化，避免 `task_finishing` 阻塞继续；取消期间 Promote 选出新权威后由库再次停止写者；关闭期间晚到的 TurnCompleted 不再触发重复物化。错误退出和清理完成判定收在 `coordination/task.rs`，bin 只派发动作并关闭连接；coordination 未引用 UI 类型。

归因依据为 `/tmp/opencode/control-plane-refactor-baseline/{staged,unstaged}.diff`：`unstaged.diff` 第 1666–1727 行已包含任务继续的 `task_finishing` 门槛，因此原继续测试需保留晋级与路由断言，同时先断言物化前阻塞、完成后放行。FS cleanup 的既有 BaseRuntime 聚合将 NLE 保留的 Cleaned 与 FS 删除记录后的 None 误报为部分可用；先用新增回归测试复现失败（`/tmp/opencode/control-plane-refactor-cleanup-repro.log`），再修正聚合。没有 data-plane 时直接返回 inner 状态。FS 多目标布局测试改用两个自有 lower 目录，保留 fork／rename／旧格式拒绝断言，避免遍历宿主权限受限内容。复制宿主内容的空间问题由并发 FS 物化实现改动解决，本轮不将该实现认领为生命周期修复；检查时 `/home` 约 73G、`/tmp` 约 2.5G 可用，未大清理或删除其他任务文件。

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
- FS portable overlay 与 btrfs snapshot-copy 保存、独立可写恢复、恢复后 fork 和再次保存已通过真实环境验证；FS + Pi 控制面主入口已通过下述整链。原生 btrfs incremental backend 本机探测失败，不能据 snapshot-copy 结果宣称其验收通过。不保存任意子进程内存。
- 恢复检查 task/authority、存活 Branch 的原生 session、Chance/Proposal 引用、Runtime 参与环境集合和 NLE world 索引；不据此宣称任意复杂部署及深层语义均已验证。无限阻塞 Runtime 或故障导致的回执无法收齐目前不能保存成功；外部原生工具只保留 Pi 取消终态，不承诺恢复子进程内存。
- Pi 原生源在保存时复制到该代 `sources/`，恢复时再复制为独立可写源，checkpoint 按保存源身份重新关联；继续原运行或恢复运行不会改写该代 Pi 源。package-managed 扩展、glob、配置与代码绑定及外部 Git snapshot 保留见下方。
- Promote notification 已在真实 main 入口、真实 Pi + NLE + 假模型的保存失败后继续／晋级／再次保存／恢复中验证最终历史恰一次；尚未验证所有通知交付时刻的跨恢复组合。

早期 `cargo build --bins`、`cargo test --lib control_plane -- --nocapture`（26/26）、Pi 联合回归含新整链用例（20/20）通过。既有无工具、永不自然结束模型流 `/tmp/opencode/check-recovery.mjs` 通过，证据 `/tmp/opencode/control-recovery-wiBhe4`。没有调用真实模型；后续扩展验证见下方。

## 原生源独立性修复（2026-11-13）

`node --test tests/pi-recovery-sources.mjs` 先复现继续原运行后 `Recovery source changed`；修复后验证原运行追加、恢复后追加及再次从同一代恢复，保存副本内容不变。恢复前检查源唯一性、完整记录数、引用、父链环、Branch tail 与预测源关系，checkpoint 仅引用的源也纳入保存。该用例使用真实 Pi SessionManager，不调用模型。

`node --test tests/pi-recovery-sources.mjs tests/pi-access.mjs tests/pi-session-history.mjs tests/pi-notification.mjs tests/pi-notification-adapter.mjs`：19/19；`node /tmp/opencode/check-recovery.mjs`：真实 Pi + 真实 NLE + 本地假模型跨进程中断／等待输入／继续／再次退出通过，证据 `/tmp/opencode/control-recovery-Pzx5zV`。本轮尚未验证 authority + 两候选含阻塞等待。

新增保存代还绑定解析后的模型对象、模型选择参数、thinking level 和 Runtime tools；恢复在建立 session 前检查上述内容及 authority/checkpoint owner/调用身份与原生末条消息。旧格式缺少该字段时仍按旧格式读取，不追认其配置保证。新增绑定后再次执行 `node /tmp/opencode/check-recovery.mjs` 通过，证据 `/tmp/opencode/control-recovery-Z1vV5T`。

## 多组件、失败继续与资源绑定（2026-11-13）

CompositeRuntime 多连接与 BaseRuntime inner/data-plane 分别写入本代私有 `*.components/`；所有子组件成功才写父组件索引，最后控制面发布总 manifest。索引绑定 backend 顺序、身份、组件 SHA-256 与参与环境集合。加载先检查同级所有组件字节，再逐个重建，并核对参与者与实际环境集合；中途失败对所有组件执行 shutdown，错误包含清理失败信息，不启动 Agent 推理。不提供已加载组件的内存事务回滚。

FS 复制存活环境的受管 upper、lower/snapshot 与已执行操作回执；普通宿主 bottom lower 不复制，恢复使用恢复时内容。保存数据树校验文件内容、类型/权限、symlink、OverlayFS opaque/whiteout/redirect/metacopy。OverlayFS 内部 work 目录不复制，恢复重建；btrfs 子卷布局记录后在恢复目录重建。BaseRuntime 复制期间冻结拥有的执行进程，失败也尝试 thaw。每次恢复另建 `restored-*` 可写副本；保存代和旧运行目录均不覆盖。可用 `filesystem.isolated_paths` 显式指定独立工作目录，`passthrough_rw` 为宿主直通，不具有保存独立性。系统只读依赖绑定 pacman/dpkg 的已安装版本列表与 sh/env/bwrap 内容摘要，不复制整个宿主 root；这不是对未受包管理文件或任意系统文件原地篡改的完整证明。

Pi SDK 默认使用 DefaultResourceLoader 自动加载全局/项目扩展，因此保存绑定 Node 版本、Pi 与接入代码内容、模型配置、全局和项目 settings/extension/skill/prompt/system prompt/AGENTS 资源。使用 SDK `DefaultPackageManager.resolve()` 解析 package 与 glob，绑定有效资源、package root 及其已安装 node_modules；验证期间不安装缺失包或执行扩展代码。恢复在创建 session、执行扩展代码前验证；变化明确拒绝。任意目录外动态依赖、第三方服务及全部间接依赖发现不在保证范围，不能作为完整 extension 环境部署方案。

`rollback-snapshot` 绑定 repoRoot、Git directory、原 ref、tree/commit，并建立独立 `refs/ascout/recovery/<digest>` 保留对象可达性；只执行读取和 update-ref，不 reset/checkout 工作区。恢复验证保留 ref、commit/tree；原 ref 删除或移动不影响独立保留对象，保留 ref 缺失则明确失败。保留 ref 目前没有自动回收，保存失败可能留下安全但冗余的 ref。复制 session 不代表复制 Git 对象。

故障整链通过占用 `recovery.tmp` 让中断后最终发布失败：进程留在暂停，manifest 字节不变；分别验证直接重试及用户继续、全部三个 Branch 继续、两个原动作总共恰执行两次，再次保存退出、旧保存代独立恢复。该用例还复现并修复已晋级候选保留原生预测源、父 Branch/其他候选已清理时被错误拒绝恢复的问题；重试开始显式清除旧 saved 标记。

实际命令：`cargo test --lib recovery_overlay_executes -- --nocapture`（1/1，隔离 `/tmp/opencode/fs-recovery-*` 真实 shell）；`node --test tests/recovery-active.mjs tests/pi-recovery-binding.mjs`（5/5）。最新整链证据分别为 `/tmp/opencode/recovery-active-kkS0PZ`（原路径）、`recovery-active-t4UK1Q`（失败重试及模型配置变化拒绝）、`recovery-active-xRogpp`（失败继续与 notification）、`recovery-active-qFyc9n`（两个 NLE backend）。所有模型请求均为本地假服务。

补充执行 `cargo test --lib recovery -- --nocapture`（2/2）：第二项在真实 sandbox writer 运行时冻结复制、注入复制失败、验证 thaw 后计数继续增长及上一代恢复；`cargo test --lib control_plane -- --nocapture`（26/26）；Pi access/history/notification/recovery-sources/recovery-binding 联合回归（20/20）。

## FS 主入口与保存独立性验收（2026-11-13）

`tests/recovery-filesystem.mjs` 使用两个真实 NLE backend、FS 数据面、Pi bash 和本地假 SSE 服务：实际隔离写入，暂停保存后可跨进程恢复；中间 FS 组件保存失败时旧 manifest 不变、运行保持暂停并可重试；加载中途 FS 内容损坏时清理先前已加载组件且不请求模型；package 扩展代码变化在推理前拒绝。恢复不自动推理，用户继续后读取当前宿主 lower 与保存的受管写入并继续写；再次恢复旧代有效，旧保存文件摘要不变。

实际命令：

```sh
cargo build --bins
cargo test --lib runtime -- --nocapture
node --test tests/recovery-filesystem.mjs tests/recovery-active.mjs tests/pi-access.mjs tests/pi-session-history.mjs tests/pi-notification.mjs tests/pi-notification-adapter.mjs tests/pi-recovery-binding.mjs tests/pi-recovery-sources.mjs
```

Runtime 118/118（含现有环境条件跳过的用例）、Node 联合回归 25/25 通过；最终 FS 主入口证据 `/tmp/opencode/recovery-filesystem-65JoGp`。所有模型请求均为本地假服务。`recovery_btrfs_and_snapshot_copy_layouts` 在隔离 `/var/tmp/ascout-recovery-btrfs-*` 验证 snapshot-copy 和普通 overlay 的保存、恢复、fork、再次保存；本机探测报告 “btrfs create/snapshot works but incremental diff is unavailable; falling back to overlay”，原生 btrfs incremental backend 仍是环境验证缺口，不修改宿主挂载绕过探测。

剩余限制：原生 btrfs incremental backend 未完成环境验收；保存失败可能留下未发布的 generation 子目录，加载失败的目录清理错误尚未完整上报；系统绑定不覆盖任意未受管文件和全部可执行依赖；任意外部扩展依赖／服务、无限阻塞 Runtime、崩溃恢复及任意进程内存恢复均不在本次保证范围。

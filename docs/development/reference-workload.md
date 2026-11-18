# 首要验证 Workload

## 纯 FS 真实模型短 session 成功（2026-11-16）

用户授权的唯一一次 `openai/gpt-6.1-sol --thinking low` 真实 FS session 已成功，主动耗时 **21.333 秒**，未使用 NLE、替代 Adapter 或宿主文件工具。证据根目录 `/tmp/opencode/fs-real-20261116-mfUC97`；`acceptance.json` 为汇总，`timeline.json`、`session/events.jsonl`、`session/session.jsonl` 为可独立核对的记录，`final-reply.txt` 保存完整最终回复。旧证据不动，未提交、未派生 subagent，已有工作区修改保留。

首次请求前确实遇到入口缺口。原拒绝配置、stderr、`entry-results.json` 与当时的 `acceptance.json` 保留于 `/tmp/opencode/fs-real-20261116-preflight`；其中“未运行、请求为 0”仅描述初始预检，**不是本次最终结论**。修复前单 FS 配置误走 Firecracker、多 Backend 拒绝 filesystem、空 Backend 也被拒绝。按“可离线复现先修复”的授权，`src/bin/ascout-runtime-adapter.rs` 新增 `backend=filesystem` 的正常装配，复用现有 `BaseRuntime` 和 `FilesystemBackend`；`src/runtime/base.rs` 将物化写者检查扩至持有 namespace holder 的纯 FS 环境。未改变既有身份、晋级、匹配或物化语义，未添加 NLE 依赖。现有 FS 生命周期及恢复脚本仍是 NLE inner 加 FS 数据面，不能当作本次纯 FS 入口证据。

先完成离线构建和文件工具回归 5/5；修复后构建通过、Runtime 单元测试 141/141。本地伪模型经正常 control plane→Runtime→真实 Pi→真实 FS 完成同一任务，证据 `/tmp/opencode/fs-local-20261116-MGZZSb`，9 次本地模型请求。临时 runner 与统计脚本保留于预检目录；尚未将纯 FS 入口用例纳入仓库持久回归，这是测试交接缺口，不影响已有原始验收证据。

真实任务读取 `input.csv`，计算并生成 `result.json`，再读回并独立重算断言。Authority **4 次工具**，候选 candidate-5 **1 次只读工具**，整场共 5 次工具。期望与实际逐字节一致：`{"total_cents":2021,"quantity":12}\n`；输入与 baseline 均未改，最终中文回复完成。`input.csv`、`expected/result.json`、`actual-result.json`、脱敏 `command.json` 与 `runtime.json` 均已保存。

文件工具使用 Runtime 为各 Environment 生成的计划及 holder socket，计划按真实 Environment ID 留于 `plans/`。Authority 与 candidate-5 的工具输出分别报告 mount namespace `mnt:[4026533058]`、`mnt:[4026533238]`。`snapshots/` 与 `host-samples.json` 记录 overlay 已产生结果、宿主尚不存在结果；随后正常 `materialization_completed` 回执 `published=true`，没有私自复制到工作区充当物化。

沿用旧 FS `branch_model` Policy 声明和当前默认算法，仅将候选模型绑定到本次指定模型，未禁用投机或强制晋级。自然产生 **5 Candidate、5 Discard、0 Promote**。candidate-5 在独立环境提前完成只读核验，但命令结构与 Authority 不同，严格匹配不通过而自然丢弃。因此本次覆盖 Authority 多步、候选隔离只读执行及 Discard；**不覆盖 Promote 后续跑、候选写入发布或恢复**。

模型请求前 action 启动 **11 次**（全部保守计入，含取消），原生记录有 **9 个 provider response ID、7 个 completed 回执**。原生 assistant **5 条标记 aborted，其中 4 条零 usage，1 条已有 completed 回执及完整 usage**；这些分类有重叠，不可相加。candidate-3 的 action failed 不代表 provider 未完成。Usage 合计 input 2183、output 752、reasoning 0、cacheRead 37888、cacheWrite 0、totalTokens 40823；Pi 配置估算 **$0.0156748**，非服务账单，取消的零 usage 也不能证明服务端不收费。120 秒主动预算、18 次 action 停止阈值、目标不超过 20 次请求，关闭重试与压缩；不是外部硬费用上限。沿用凭证，不输出密钥。

任务先记录 `task_completed=succeeded`；正常 Shutdown 随后又产生 cancelled 记录，后者是退出阶段事实，不覆盖前者。进程退出码 0，监控记录的自有 PID 无残留；Web 36905、Runtime 37271 均可重新 bind。完整回执与清理核对见 `acceptance.json`、`run-result.json`。

## Promote 在途模型等待点修复（2026-11-16）

原 `/tmp/opencode/runtime-id-real-success-20261116` 失败证据保持不变。原事件中，candidate-2 的 model-2 在 12.56 秒开始，13.52 秒 Branch 进入 Promoting 时，其 chance-3 被 sweep 标为 Unavailable／Discarded，17.35 秒模型成功结束时不再进入 `record_authority_outcome` 的非终态 Chance 路径，故正常 `ResolvePending(phase=continue)` 永远不会产生。Pi 在 assistant 结束时创建的 `modelResultHolds` 随后挡住工具 Hook。Promote Applied 已释放 effect／extent gate，现场没有 pause hold；因此不能靠重复 Promote、手动放行或修改 Pi 权限解决。

最小修复位于 `src/control_plane/agent_control/chances.rs`：`chance_waiting_live` 将 `Promoting` 也视为现有等待点仍存活。晋级是角色过渡，不是终止该 Branch 的在飞操作；Discarding 及之后的失效传播保持原规则。未修改 Policy、模型准入、暂停语义、Runtime 身份生成或 NLE 单世界。

先红后绿证据集中于 `/tmp/opencode/promote-continuation-20261116`（检查日志原路径为 `/tmp/opencode/promote-*.log`）。新增 Rust 回归先证明 sweep 错删在飞模型 Chance，修复后证明真实 ActionFinished 推进到 Arrived 并由正常 progress 生成 continue。`tests/predictor-fact-e2e.mjs` 新增本地 SSE 延迟响应场景，保留原完整 Policy，真实 Pi→Runtime→NLE；原版在 `/tmp/opencode/predictor-fact-e2e-v6Ti3K` 超时，修复后 `/tmp/opencode/predictor-fact-e2e-yL408f` 完成 5 次实际动作、首次 Promote 后 4 次工具、完整原生最终回复及正常退出（7 次本地伪模型请求、5 个 Candidate）。首次绿色尝试的最终回复断言误查 Ledger output（Ledger 不保存该正文），改为检查原生 session 完整 stop 回复；该脚本失败证据 `/tmp/opencode/predictor-fact-e2e-C3gJPQ` 保留。

真实模型补验 **成功**：`/tmp/opencode/promote-continuation-20261116/real/acceptance.json`。沿用宿主凭证、`openai/gpt-6.1-sol --thinking low`、原 5 步任务、NLE seed 7／1 秒延迟、原 Policy、关闭重试／压缩，120 秒上限和 18 次 action 停止阈值，最多 20 请求预算。仅一个 session；10 次请求前模型 action（保守全部计请求），6 个 provider response ID，另 4 次启动后被丢弃取消、无成功回执。Usage input 6841、cacheRead 30336、output 305、reasoning 8、totalTokens 37482；Pi 配置估算 **$0.0197656**，非服务账单。原失败费用 $0.0060204 保留，两次合计估算 $0.0257860。

真实动作恰好 5 次（向西 4 次、向南 1 次），最终公开位置 `(22,6)`、HP 14/14、turn 5，完整中文最终回复已保存于原生 session。5 个 Candidate 自然产生 1 Promote Applied、4 Discard；首次 Promote 后再完成 3 次真实工具。任务先 `task_completed=succeeded`，然后正常 Shutdown；Shutdown 又记录 cancelled task_completed，为退出阶段事实，不覆盖成功任务。全部 6 个创建的 Branch 绑定各自 Runtime 返回的不同 Environment ID，完整映射见 acceptance 的 `branch_bindings_all_created`；最终全部 Environment cleaned，自有进程无残留，退出码 0。8080／18180 无 listener，SO_REUSEADDR bind 成功；普通 bind 暂受 TIME_WAIT 影响，未误报成残留服务。

监控工具插曲也保留在证据中：首个 background shell 被显式 1000ms 工具超时终止，仅杀掉监控脚本，control plane 仍是同一个 idle session，事件只有 branch_ready、没有 prompt／模型请求。`continue-monitor.py` 接回同一 PID，才发送唯一 prompt，并从发送前维持 120 秒监控；任务约 52.53 秒完成，没有第二次启动或模型重试。原生 t_ms 包含此前空闲间隔，不应与主动任务耗时混淆。

检查：Rust lib 171/171、Pi 接入及 predictor 10/10、本地 E2E 2/2、`cargo build --bins`、`cargo check --all-targets`、`cargo fmt -- --check`、`git diff --check` 均通过。没有提交；全部既有修改保留。

## 用户重新授权后的单次真实模型验证（2026-11-16）

**结果：真实模型成功返回，但多步 E2E 仍被阻塞。** 本次独立证据目录为 `/tmp/opencode/runtime-id-real-success-20261116`（目录名不代表验收成功）；原 `/tmp/opencode/runtime-id-real-e2e-20261116` 及其他旧失败证据未改动。没有创建 subagent、提交或修改实现，没有禁用 Policy／投机，没有用伪模型或回放替代本次运行。

- 无付费预检：`cargo build --offline --bins` 成功（`build.log`），系统 Node v26.13.0、NLE 1.3.0、Policy 存在，8080／18180 空闲。认证仅检查现有文件存在，不复制或输出秘密；未设置独立 Pi 配置目录。沿用宿主认证、`openai/gpt-6.1-sol --thinking low`、现有 NLE seed 7／1 秒延迟和 NLE Policy。workspace 设置关闭重试和压缩。未重跑主 Agent 已通过的测试集。
- 仅运行一个真实 session，输入要求恰好 5 次、最多 6 次单步动作；120 秒截止，18 次模型 action 停止阈值（含候选，不是外部服务硬配额）。脱敏启动参数、输入、监控与清理脚本分别为 `command.json`、`prompt.txt`、`run.py`。
- 真实模型请求按请求前 `action_started` 与非零 provider 回复交叉核对为 **2 次，成功 2、失败 0**（Authority／Candidate 各一次，原生 transcript 含两个 response ID）。另有 2 条本地零 token 取消 usage，不算外部请求。没有代理截获或服务方账单，计数依据明确留在 `acceptance.json`。
- Usage 合计：input 1996、cacheRead 9984、output 103、reasoning 11、totalTokens 12083；reasoning 不额外重复加入总数。Pi 按配置估算费用 **$0.0060204**，`totalFromProvider=false`，不是服务记录费用。
- 真实 NLE 仅完成 1 次 `nle_action(action=4)`：位置 `(26,5)` → `(25,5)`，turn 2，HP 14/14，游戏 message 为空。候选预测 `moved`，比较 `compatible`，**1 次 Promote Applied**；旧 Authority 的 Discard Applied 1 次，不能称作预测不匹配丢弃。Candidate 模型生成第二次西移调用后没有进入工具 action／Runtime invoke，故不能算第二次真实动作。
- Runtime `req-1` 返回 `env-2d10e5-18e87efa5ff61e2b-1`，绑定 `authority`；`req-4` 返回 `env-2d10e5-18e87efd8a2c011b-2`，绑定 `candidate-2`。最终 Tree 与返回值一致，两个 Branch 的 ID 不同；该共享 NLE 世界的独立身份不代表世界复制。
- 约 13.57 秒 Promote Applied，17.35 秒 Candidate 模型成功完成后持续停滞；120 秒监控截止后 Interrupt／Shutdown，最终 `task_completed=cancelled`，**没有最终自然语言总结**。最后模型说明为“第1步返回‘moved’，未报告障碍；第2步继续向西。”，不是最终回答。退出码 0 仅证明正常清理，不代表任务成功。
- 离线核对的具体挂起边界：`instrumentation/pi/main.ts:347` 在模型 assistant 结束时设置 `modelResultHolds`，`:265–270` 在工具 Hook 等待该 hold，`:629` 仅收到 `ResolvePending(outcome.phase=continue)` 时释放。本次日志存在 `model-2` 完成，但没有之后针对它的 continue 命令，直到中断出现 `Model continuation cancelled`。上游为何未发送 continue 尚需 control plane 回归定位，不能将此直接认定为旧 Chance 路由错误或宣称已修复；付费开始后的阻塞未自动重试。
- 两 Branch 最终均 Cleaned／Agent Released／Runtime Cleaned，`turn_active=false`；记录到的自有子进程无残留，8080／18180 再次可绑定。只向本次 control plane 发 Interrupt／Shutdown，没有操作无关进程或端口。

独立核对入口：`acceptance.json` 汇总；`timeline.json` 为原事件过滤视图；`session/events.jsonl`、`session/session.jsonl`、`session/final-tree.json`、`end-state.json` 保留事实，`run-result.json` 保存进程清单及清理结果，`source-sha256.json` 保存本次关键源码／构建指纹。当前证明真实模型、真实动作、自然比较／Promote、多 Branch 返回身份绑定及清理；**未证明 4–6 步任务成功与最终回答**。

## Runtime 返回 Environment ID 修正后的单次真实路径验收（2026-11-16）

本次结果为 **失败，尚未完成真实模型 E2E**。用户授权一次真实模型 session；仅启动一次正常 control plane → NLE Runtime → Pi session，失败后只做离线定位，没有重试真实运行。没有提交或修改既有实现。

- 证据目录：`/tmp/opencode/runtime-id-real-e2e-20261116`。`command.json` 保存脱敏启动参数，`prompt.txt` 保存输入，`session/events.jsonl`、`session/session.jsonl`、`session/final-tree.json` 保存原始事实；`acceptance.json` 与 `run-result.json` 保存统计和清理结果。
- 选择现有配置模型 `openai/gpt-6.1-sol --thinking low`，认证仍用宿主 Pi 配置，未复制或输出凭证；使用现有 `nle-runtime.json`（seed 7、1 秒动作延迟）及 `assets/policy/nle/policy.yaml`。输入要求 5 次实际动作，最多 6 次，监控 18 次模型 action 停止阈值和 120 秒截止；workspace `.pi/settings.json` 关闭自动重试及压缩，不修改宿主配置。模型 action 监控不是外部 API 请求硬配额。
- 无付费预检：`cargo build --bins`、`cargo check --all-targets`、`cargo test --lib --bins`（170 个 lib 测试、1 个 bin 测试）、`/usr/bin/node --test tests/pi-access.mjs` 通过；NLE 1.3.0 可导入，Node v26.13.0，18180／8080 启动前空闲。已有 Runtime identity 测试覆盖返回独立身份、重试复用原创建结果及失败创建的身份保留，但不替代真实候选验收。
- Runtime 的 `req-1` 真实建立 NLE Environment，返回 `env-2c4e69-18e87ad0c800b262-1`，与最终 Tree 的 `authority.environment_id` 一致。没有 Candidate，故本次只验证单 Branch 的返回 ID 绑定，未真实验证多 Branch 独占绑定。
- session 在约 1.75 秒内出现 `task_completed: failed`；首个模型请求前即 `turn_failed: Policy evaluation failed`。模型 action 开始数为 0，实际外部模型请求为 0，实际 NLE 工具动作为 0，Candidate／比较／Promote／Discard 均为 0。原生 session 中 assistant 文本为空、`stopReason=error`；唯一 Usage 是本地失败消息的零 token／零费用记录，不是成功的 provider 回执，也没有服务方账单证据。
- 离线根因：`instrumentation/pi/main.ts` 的 `beforeModelInference` 在赋值 `op.has_predictor` 之前调用 `access.hasPredictor`；`instrumentation/core/index.ts` 将尚无该字段的 `op` 交给 `admit.opportunity_pass`，现有 NLE Policy 直接读取 `op["has_predictor"]`。使用本次保存的 Policy 源与相同输入离线复现 `KeyError: 'has_predictor'`（`offline-policy-traceback.txt`）；同一输入经 Rust Policy Engine 被拒为 `op must contain kind and the predictor fact`（`offline-admit.stderr`）。后续需修正 predictor 事实的提供与准入求值顺序；本次不以默认真假值绕过准入，也不宣称已修复。
- Shutdown 正常退出，control plane 退出码 0；Runtime terminate／cleanup completed，最终 Authority 为 Cleaned／Agent Released／Runtime Cleaned，`turn_active=false`。已记录的本 session 子进程均无残留，未操作无关进程。退出码 0 只说明退出清理成功，不代表任务通过。

覆盖仅限真实 NLE 建立、Runtime 返回身份绑定、Agent 启动、失败报告和清理；没有证明真实模型推理、4–6 步动作、Candidate 比较或 Promote／Discard。该失败 session 已消耗本轮一次运行额度，未因实际请求数为 0 而自动补跑。

首个真实接入对象是 Pi，当前源码位于相邻的 `pi`。本文只记录当前交互实现，不定义完整 Runtime 或 coding workload。

## 首次请求 predictor 事实修复的离线与本地伪模型验证（2026-11-16）

上述失败记录及 `/tmp/opencode/runtime-id-real-e2e-20261116` 原始文件保持原样。后续修复原因和事实来源见 [Policy Engine 开发记录](policy-engine.md)。本轮没有真实模型 API 请求、付费预检或重跑；**尚未真实模型 E2E 通过**。

`tests/predictor-fact-e2e.mjs` 默认从仓库 fixture `tests/fixtures/predictor-fact-policy.json` 读取原失败 Policy 的 YAML 与完整源码，不依赖临时失败 session。fixture 只脱敏不进入 digest 的来源路径；共享 helper 验证留档 digest，E2E 在独立 run 走普通 control plane 启动编译并检查语义和摘要（不同 Python build 仅允许 Python identity 解释变化）。不修改 Policy 函数、配置、预测或选择。独立 Pi 配置仅注册 `fixture/fixture`，端点为本地 loopback SSE。Runtime 使用真实 NLE、seed 7 和 1 秒动作延迟，正常 control plane Web prompt → Runtime 启动 Pi → 首次推理 → `nle_action(action=4)` → 真实西移动作及公开 observation → 任务成功 → Shutdown。最新收口结果、文件工具根因与 Rust 扩展回归失败证据见 [测试讨论](test-discussion.md)。

首次通过证据 `/tmp/opencode/predictor-fact-e2e-SV88y9`：`command.json`、`policy.json`、`requests.jsonl`、`events.jsonl`、`session.jsonl`、`agent.log`、`adapter.log`、`control.log`、`acceptance.json`。3 次本地伪模型请求，3 个模型 action 开始，4 次准入请求，1 个 Candidate；实际西移工具成功并返回 `position=(25,5)` 与地图，模型/工具均经过真实 Policy 准入，任务 succeeded，退出码 0。Ledger 另记录 compatible 后 Discard，不能把“生成候选”或“compatible”写成 Promote 验收。

在完成最终文档和检查后再次运行同一 E2E 的证据 `/tmp/opencode/predictor-fact-e2e-YmqJax`：2 次本地伪模型请求，2 个模型 action，4 次准入，1 个 Candidate，真实 NLE 工具及正常退出同样通过。最终证据的验收 JSON 明确标记未重跑真实模型。

第一轮脚本证据 `/tmp/opencode/predictor-fact-e2e-h4Jtwk` 保留：实际路径已执行成功，验收脚本错误地在 Agent 公开文本结果中查找内部 `blstats` 字段而失败。只把断言修正为真实公开坐标和地图后重新运行本地伪模型，未改生产结果格式。

检查命令和日志统一位于 `/tmp/opencode/predictor-fact-fix-20261116`：

- `node --experimental-strip-types --test tests/predictor-fact.mjs`：原版 0/3（`red.log`），首次修复后 3/3（`green.log`）；最终另加异常传播覆盖。
- `node --experimental-strip-types --test tests/pi-access.mjs tests/predictor-fact.mjs`：最终 10/10，`focused.log`。
- `node --test tests/predictor-fact-e2e.mjs`：两次通过均为 1/1，`e2e-green.log`、构建后再次运行的 `e2e-after-build.log`；首轮脚本断言失败见 `e2e.log`。
- `cargo test --test policy`：1/1，`policy-integration.log`；`cargo test --lib control_plane::policy`：3/3，`policy-unit.log`；`cargo test --lib control_plane`：26/26，`control-plane.log`。
- `cargo check --all-targets`、`cargo build --bins --example policy`：通过，`check.log`、`build.log`。
- `git diff --check`：通过，`diff-check.log`。退出后进程检查未发现本次三个 E2E run 的残留进程。
- `python3 tests/policy-proposal-order.py`：1/1，`proposal-order.log`。
- `node --experimental-strip-types --test tests/pi-*.mjs tests/predictor-fact.mjs`：当时 33/34（新增异常测试前），`pi-core.log`。唯一失败为 `tests/pi-filesystem-tools.mjs:100` 的后代进程等待断言 `1 !== 0`；`node --test tests/pi-filesystem-tools.mjs` 独立重跑仍 4/5（`pi-filesystem-rerun.log`）。该文件工具实现和测试本轮未修改，入口也未调用本次修改的 access core 或 main；保留给主审检查已有工作，不宣称 Pi 全部通过。

## 当前可运行范围

### 通用事实协议迁移（2026-11-06）

Pi 握手中的能力字段是接入原语声明，不是模型推理或 sub-agent Hook 的证明。当前值为：工具执行前 Hook、模型推理前 Hook、local Decision evaluation、checkpoint continuation、independent candidate session、cancel completion、tool result injection 为 `true`；sub-agent 前 Hook 为 `false`。模型 action 由真实 `streamSimple` 请求前的 Hook 创建，普通 action 仍发送 `action_started` 事实；工具 Hook 按下发 Policy 的 `admit` 结果决定是否发送 `speculative_admission_request`。不要把 `message_start` 当作模型推理前 Hook。

Rust 协议、控制循环与 Pi 接入使用 `ActionStarted` / `ActionFinished`。action 包含 task/Branch/action identity、类型、可选父 action、结果终态和 evidence，并携带 Instrumentation 测量的 `timestamp_ms`；`SpeculativeAdmissionRequest` 登记执行前的等待身份和准入时刻，不伪造实际开始。`Usage` 关联模型 `action_id` 并提供 Instrumentation 起止时间。Tree 保存活动 action，完成事实进入 Ledger。Pi 从真实请求前 Hook 报告模型开始，从工具 Hook 报告工具 action，输出与使用量可关联模型 action。Candidate 推理不使用角色专用完成事件。Agent session 与 Branch 执行释放分开报告；命令接收/开始回执不提前完成 Tree 操作。Runtime Adapter 独立报告操作开始/结果及环境事件，环境建立失败不伪装成 Agent 失败。

能力边界：通用协议可表达 sub-agent 等 action，但 Pi 当前只采集模型推理与工具执行；二者的真实执行前 Hook 均执行下发的本地 Decision，Policy 允许时产生 `speculative_admission_request`，通过既有 launch 控制 eager／speculative-first。sub-agent 启动前 Hook 仍为 false；不要把 `message_start` 当作模型推理前 Hook，也不宣称任意 action 均可投机。空预测或预测失败放行原权威调用一次，不保留挂起的 Proposal。模型开始时间来自真实请求前 Hook，而非首次 assistant message。extent stop 支持工具与模型边界；discard、cleanup、取消和合法恢复释放等待，断线不伪造完成。Agent 自身超时须实际观察后报告。分阶段回执覆盖已有身份管理的 Promote、ResolvePending、Discard、Cleanup；其他命令依其 Branch／回合事实。Runtime 资源清单随 OperationResult 返回，独立资源／效果事件尚未接通。

### 模型投机 `branch_model`（当前已接通的语义）

模型请求在真正进入 `streamSimple` 前经过模型推理前 Hook。Hook 先登记 action 和 checkpoint，再由当前 Policy 选择是否投机；`branch_model` 及其候选模型来自 Policy 的场景选择，不由 Pi 或 control plane 写死。候选建立在该等待点之后先于 Proposal 的可比较产出：候选使用指定模型继续推理，模型完成后才以完整的 assistant `tool_calls` 列表形成结果，而不是创建一个“Proposal 只作占位”再补结果。

匹配使用完整工具调用列表，而不是只看第一项或其他输出文本；调用数量、顺序、名称和参数都属于列表比较输入。候选在 Promote 前不把工具效果写入权威路径，候选工具调用受 AScout 生命周期控制；单一胜者 Promote 后切换后续权威会话并清理旧 Parent／其他候选，Discard 则取消并清理候选，未验证输出不进入主对话。调用列表的规范化／语义相似口径、模型成本和失败回退仍未定，见 `Discussion.md` 第 40 项。

当前承载语义：Pi 是宿主上的单一进程，不按 Branch 启动多份 Pi；启用文件系统隔离的 Branch 环境有常驻 namespace-holder。Pi 经 ASC-Proxy 请求通用 Runtime 执行端点，Runtime 内部文件工具适配器再经 holder socket 在该 mount namespace 中执行；Pi 不读取工具计划或直接访问 holder。`/app` 映射由 `isolated_paths` 的 `{path, target}` 提供，host 目录是 overlay lower 和 publication 目标；物化轮换 generation 会结束旧 holder 并建立新 holder。工具命令并发执行，直接 shell 退出即返回，后台后代保留到环境结束。

### 四个本地脚本场景（不是付费模型证据）

`tests/benchmark-task.mjs` 用本地 scripted SSE provider 驱动同一 FS/Pi/control-plane workload，循环覆盖四个场景：`match`（候选调用与权威调用一致，验证 Promote）、`mismatch`（候选调用不同，验证不能 Promote 且候选写入不落地）、`authority_first`（权威先完成，验证时序竞争）、`fast_failure`（候选模型请求返回 500，验证失败路径）。这四项是本地脚本模拟，不是付费模型调用，也不能证明真实模型的推理质量、成本或服务稳定性；脚本中的 `fixture`／`fast` 只是本地端点模型标识。

该脚本还检查完整任务完成、Candidate 创建、处置结果、丢弃写入不物化以及最终 FS materialization。脚本在 `materialization_started` 前轮询 host workspace，确认工具写入只落在 Branch overlay，而不是宿主工作目录；这验证了 holder 路径实际被执行。它证明的是当前接缝和生命周期在可控输入下的行为，不替代真实模型端到端验收或 Runtime 故障矩阵。

本轮修复后的真实模型验证使用 `/tmp/opencode/deepseek-flash-final-FTGsuO`：宿主默认 `~/.pi/agent`、未设置 `PI_CODING_AGENT_DIR`、未复制 credentials，一次原始 instruction prompt，3 个真实 `branch_model` Candidate 全部因 Authority 先返回而被取消并清理，`task_completed` succeeded、materialization published、fixture 原始验收 3 项通过，物化前 host workspace 无工具写入。

此前 NLE 验收：`/usr/bin/node` v26.12.1、`deepseek/deepseek-flash --thinking low`、NLE 策略（当前本地路径 `assets/policy/nle/policy.yaml`），运行目录 `/tmp/opencode/nle-auth-run-4`。共 10 次真实 `nle_action`，18 次模型推理 action；模型与工具 action 均 28/28 配对，0 未完成、0 孤儿完成。7 个候选全部 `Promote`，0 次 `BranchFailed`；8 个 Branch 全部清理，退出码 0。此前 10 步失败时发现的 `evaluatePolicyArtifact` 未定义、Policy 输出通道、空预测挂起和验证 evidence 取值错误均已修复。检查通过：`cargo test --lib`（14 项）、`cargo check --all-targets`、`cargo clippy --all-targets --all-features -- -D warnings`、Pi TypeScript 检查、`/usr/bin/node --test tests/pi-access.mjs`（3 项）。未提交、未 push，未运行 Firecracker E2E。

运行方式见 [README](../../README.md)。control plane 提供权威 Tree、Policy 和生命周期事实；Pi integration 使用 Pi 公开 SDK 管理 AgentSession，报告消息、思考、工具调用、结果和执行事件。浏览器 UI 与 TUI 通过共同接口读取这些事实并各自排版。Runtime 负责启动唯一的宿主 Pi 接入进程（不按 Branch 重复启动），control plane 只决定用哪个产品并把路径和版本交给它；文件系统 Branch 的工具命令由该环境的 namespace-holder 在 Branch namespace 内执行。Pi 接入没有 Tree 或 Policy 裁决权。

`/ascout` 选择 Branch 后，默认显示由 AScout 根据 SDK 消息排版的对话内容；Tab 切换 AScout 运行事实，↑/↓ 滚动，Esc 返回主对话。候选窗口不接收用户输入。当前仍使用 AScout 输入栏，不支持 Pi 完整编辑器、命令菜单或图片，也不承诺显示 Pi 原生终端画面。候选的独有输出分隔来自 Pi integration 的真实消息边界，不再复制 Rust 展示文本。

### TUI 文字提示与命令

真实入口是 `src/bin/ascout-control-plane/tui.rs`，由 control plane 启动时接入共享 Gateway Connection 和 LedgerReader。当前权威的 confirm/select/input/editor 提示出现时临时接管输入区域；候选提示不发布，Promote 后才展示。↑/↓ 选择是／否或选项，Enter 提交；input 可输入、退格并 Enter 提交。editor 显示预填内容，Enter 插入换行、退格编辑末尾、Ctrl-S 提交完整多行文本。Esc 取消当前提示（单独按键，等待约 100ms）；Ctrl-C 仍请求整轮中断，Ctrl-D 仍退出。回答、取消、换权威或断开后关闭提示，普通聊天草稿保留；答案只走 `UiPromptAnswer`，不向模型发送聊天。提示期间导航键用于提示，结束后恢复原观察导航。

主对话输入 `/` 展示匹配的缓存命令，Tab 补全第一项后 Enter 原文执行；继续输入可缩小范围。清单由初始 Pi session 完成扩展绑定及资源发现后取得一次，重连与权威切换复用，不重新读取 Pi。启动扩展提问不依赖清单完成；无人回答不自动补值。`custom` 组件暂不支持，不嵌 Pi 原生终端组件。

- 运行目录由调用方创建，control plane 只使用它；Agent 日志、Adapter 日志和 `events.jsonl` 都在其中。
- 初始 Authority 在运行目录的工作区中建立，连续请求使用同一 AgentSession。
- 模型选择与认证使用 Pi 现有配置，不写死 provider。
- 模型文本、工具启动与工具结果经接入协议实时回传，由 AScout 展示；模型错误如实显示。
- `/ascout` 展示当前 control plane Tree，可按编号深入只读查看 Branch 的记录和执行输出；选择观察对象不改变权威路径。
- Ctrl-C 请求中断并丢弃在飞候选；Ctrl-D 退出。退出时 control plane 清理环境，Runtime 回收 Agent 进程；运行目录保留用于诊断。
- 仅在 Pi 报告工具 action 的执行前等待边界时创建 Candidate；Candidate 使用复制的消息上下文与预测工具结果继续，不把预测或最终模型回答充当 Parent operation outcome。

## 手动测试步骤

### Coordinator 状态与操作收敛（2026-11-03）

活动控制事实集中在 `SpeculativeTree`：Chance 保存 Proposal、关联 Branch、权威值取得状态与验证依据；Branch 保存所属 `chance_id`、回合是否仍在进行、失败原因、操作请求与时限；Tree 保存命令回执身份与待执行用户输入。外围循环只转交事件、执行 Speculation Coordinator 发起的 Runtime 请求并发布界面／Ledger 记录，不再维护第二套候选集合、就绪和执行完成标记。

Candidate 创建通过一次入口登记完整控制上下文。Policy 预测、结果比较和处置由 Speculation Coordinator 协调。Runtime 晋级确认与 Agent 权威切换确认分开记录；仅 Agent Promote Applied 后切换 Authority，并丢弃旧 Parent 与其他候选的依赖子树。晋级等待中不发送下一条用户输入；确认后按新 Authority 的回合事实决定是否续跑。Agent 清理确认后才回收独立环境；共享环境仍由整个 run 退出清理。

本轮本地验证：`cargo check --all-targets`、`cargo test --lib --bins`（11 项）及 `/usr/bin/node --test tests/pi-access.mjs`（3 项）。新增回归针对实际发现的“请求晋级就提前清理旧 Parent／派发后续输入”问题，覆盖确认前不切换、确认后剪枝与已结束候选接收后续输入。没有执行真实 NLE 模型验收。一次未限定范围的 `cargo test` 启动 Firecracker 端到端测试，7 项失败，首个错误为 Unix socket 路径过长；不计作本轮通过证据，也未修改后端以绕过。

Ledger 归档失败按一次重试、最终失败先报警再释放的规则处理，见 [UI Projection](ui-projection.md)；不宣称已实现全部调度、预算、嵌套接入或独立环境端到端能力。

补充真实模型验收（用户明确要求后执行，代码版本 `e9694c4`）：两轮均使用 `openai/gpt-6-luna --thinking low`、现有 seed 7／1 秒延迟 NLE 配置，没有修改默认模型或 Policy。

- `/tmp/opencode/nle-coordinator-ugfou9lr`：5 次真实动作结果、5 次 Promote Applied；旧 Authority 的 5 次 Discard 和全部 6 次 Cleanup 均 Applied。最终 Authority `candidate-6` 接收后续无工具用户输入并准确回复“续跑确认”。Shutdown 退出码 0。
- `/tmp/opencode/nle-coordinator-83xt0z35`：要求连续向西 5 次，第 5 次确实撞墙，Policy 正确预测 `blocked`，仍为 5 次成功晋级。后续输入和 Shutdown 同样通过，退出码 0。
- 两轮均无 BranchFailed、失败命令、Policy 处置错误或 Runtime 晋级错误；`agent.log` 为空。`final-tree.json` 中 6 个保留 Branch 均为 Cleaned／Agent Released／Runtime Cleaned，`turn_active=false`，命令和输入队列均为空。证据目录包含 `acceptance.json`、动作结束与后续输入结束的运行视图、事件及最终 Tree。

覆盖边界：上述两轮没有出现预测不匹配导致的 Candidate Discard，不能把旧 Authority 的 Discard 当作该路径的真实验收。未注入 Runtime 错误、超时或覆盖独立环境后端。

补充 DeepSeek 十步试运行（用户要求，代码版本 `a3c9a17`）：`deepseek/deepseek-flash --thinking low`，其余 NLE 配置与 Policy 不变，证据在 `/tmp/opencode/nle-deepseek-12mobqwy`。

- 模型实际执行 11 次动作（提示要求 10 次，未强约束成功）：8 次 Promote Applied、2 次预测不匹配 Discard，旧 Authority 的 10 次 Discard 与全部 11 次 Cleanup 均 Applied。
- 这次真实验证了不匹配丢弃路径：`candidate-7`、`candidate-9` 预测 `moved`、实际 `incompatible`，被丢弃、清理并从活动 Tree 归档到 Ledger；两者不再出现在 `final-tree.json`。
- 无 BranchFailed、失败命令、Policy 处置错误或 Runtime 晋级错误，`agent.log` 为空；Shutdown 退出码 0。最终 9 个保留 Branch 均为 Cleaned／Agent Released／Runtime Cleaned，`turn_active=false`，命令与输入队列为空。
- 该次未验证候选在 Agent 确认前被取消、Runtime 故障或独立环境后端。

### control plane 树重构后的十步验证（2026-11-05）

用户明确要求后执行一次真实模型验证，未提交代码。代码为当前未提交的 control plane 重构：活动状态归 `SpeculativeTree`，控制循环由消息和到期事件触发，`SpeculationCoordinator` 只负责 Policy 求值与外部操作。

- 运行 `/tmp/opencode/nle-deepseek-ulhc2dif`，模型 `deepseek/deepseek-flash --thinking low`，NLE seed 7／1 秒动作延迟与 NLE 策略不变；提示要求共 10 次 `nle_action`。策略当前本地路径为 `assets/policy/nle/policy.yaml`。
- 结果：10 次真实动作结果、7 次 Candidate Promote Applied，旧 Authority 的 7 次 Discard 与全部 8 次 Cleanup 均 Applied，退出码 0。
- 7 次 Candidate 均为 `compatible`，本次没有出现预测不匹配的 Candidate Discard。原因在样例 Policy：确定性格与 `wait` 动作不产生候选，本次未遇到未见过格子的双预测。不能据此声称不匹配路径在本代码版本上已验证。
- 无 BranchFailed、失败命令、Policy 处置错误或 Runtime 晋级错误，`agent.log` 为空；`final-tree.json` 中 8 个保留 Branch 均为 Cleaned／Agent Released／Runtime Cleaned，`turn_active=false`，命令与输入队列为空；无遗留进程。
- 复查发现并修复：旧候选的核对在 Parent 已进入下一次工具调用后才返回时，原实现仍可能 Promote，导致权威回到过时的执行位置。现在 `can_promote` 要求权威仍停在候选对应的工具等待点；权威已请求另一个工具操作时，该候选按 Discard 处置。回归 `validation_cannot_promote_after_parent_enters_a_later_operation` 先复现失败，修复后通过。
- 未覆盖：异步 Promote 在途期间 Parent 启动下一工具操作的窗口；候选在 Agent 确认前被取消、Runtime 故障注入和独立环境后端。共享 NLE 世界中，Parent 已造成的真实效果不能由 Promote 回滚。

### 退出验证（2026-11-03）

当前 SIGINT 与 TUI Ctrl-C 走 Interrupt，终止 task 并物化后保留程序和 session；SIGTERM、Shutdown 与 Ctrl-D 退出。保存整树由独立 `/pause` 入口触发，`/continue` 明确继续原任务且不进入模型。Adapter 使用独立进程组，终端发给 control plane 前台进程组的信号不会直接终止子进程。实际验收边界见[恢复](recovery.md)，不将局部入口通过视为全部 Authority 已完成。

Agent 会话释放和 Runtime 环境清理分开记录；共享环境清理成功时更新 Tree 中仍引用它的所有 Branch。退出后 `final-tree.json` 保存最终状态，已归档 Branch 的历史事实不回写。

验证记录（均为真实 NLE，模型启动参数 `openai/gpt-6-luna`，未改默认配置）：

- `/tmp/opencode/nle-graceful-bkm0dgpn`：5 次真实动作结果后 Shutdown，退出码 0、无失败命令、Agent 日志为空。
- `/tmp/opencode/nle-sigterm-a3osqvyj`：活跃执行 2 次真实结果后 SIGTERM，3 个保留 Branch 均为 Agent Released／Runtime Cleaned，退出码 0。
- `/tmp/opencode/nle-sigint-vkvamocm`：空闲 SIGINT，退出码 0，最终状态正确。
- `/tmp/opencode/nle-process-group-pm8myr0l`：向 control plane 的整个前台进程组发送 SIGINT，正常完成清理，退出码 0、Agent 日志为空。

模型服务过载是外部服务错误，不属于退出修复范围；预期取消的 Request aborted 不等于控制命令失败。仍未覆盖启动握手期间收到信号、Runtime 故障注入和独立环境后端的完整端到端退出。

前提：在仓库根目录执行；`/usr/bin` 排在 `PATH` 最前（Node ≥ 22.19）；NLE 虚拟环境与 `nle-runtime.json` 已就绪。

```sh
# 1. 全量编译（Adapter 是独立二进制，必须一起编译）
cargo build

# 2. 建运行目录（由调用方创建，control plane 不创建也不删除）

# 3. 启动（相对路径按当前目录解析，所以要在仓库根目录）
PATH=/usr/bin:$PATH \
ASCOUT_RUNTIME_CONFIG=nle-runtime.json \
./target/debug/ascout-control-plane ../pi --policy nle
```

终端里出现 AScout 界面后，首次消息使用 [NLE 任务说明](../../examples/nle-task.md) 中的完整文本（多行可合并成一行粘贴）。不要只依赖下面这句简短目标：它没有解释动作编号、公开观测和接口限制。

```text
You are playing NetHack. Use nle_action for one game action per turn, and nethack_guide to read the official Guidebook when you need rules. Explore, fight when needed, and find the stairs down.
```

界面操作：

| 按键 | 作用 |
|---|---|
| `/ascout` | 查看当前 Tree |
| 数字 | 深入查看某个 Branch |
| Esc | 回到主对话 |
| Ctrl-C | 中断当前轮 |
| Ctrl-D | 退出 |

正常运行会看到：候选先推理完，权威执行动作，然后 `Candidate promoted`；偶尔 `Discard`。每次动作约 1 秒（配置的动作延迟）。

退出后检查运行目录：

```sh
ls /tmp/opencode/run-1
# agent.sock  workspace  agent.log  adapter.log  events.jsonl

# 每个动作的裁决
grep -o '"disposition":"[A-Za-z]*"' /tmp/opencode/run-1/events.jsonl | sort | uniq -c

# token 与花费
python3 - <<'PY'
import json
rows=[json.loads(l) for l in open('/tmp/opencode/run-1/events.jsonl')]
u=[r for r in rows if r.get('message')=='usage' and r['usage'].get('total')]
print('模型调用', len(u), '总花费', round(sum(r['usage'].get('cost') or 0 for r in u), 4))
c=[r for r in u if r['branch_id'] != 'authority']
print('候选最大单次输入', max(r['usage']['input'] for r in c))
PY
```

判断是否正常：

- `候选最大单次输入` 应在 1000 token 上下；出现上万说明整段前缀缓存被破坏。首次建立权威会话另有约 5～6K 的系统提示开销，属正常。
- 应同时存在 `Promote` 和 `Discard`；只有 Promote 说明预测没有遇到不确定格。
- 退出后 `ps -eo comm | grep -E 'ascout|node'` 应为空。

## 实际检查（2026-10-25）

在 tmux 真实终端启动，确认 Pi 接入成功、Authority 启动、主对话输入送达、模型错误实时回传、Tree 展示和 Branch 详情显示。Pi 默认模型 `gpt-5.5` 的当前服务返回 HTTP 503 `model_not_found`；没有伪造模型成功回答。中断后收到本轮结束事实。Rust 编译与已有两个控制状态测试通过，Pi 完整 `npm run check` 通过。

随后以独立临时 Pi 配置连接本地可控模型接口（脚本，不是真实大模型），使用真实 Pi 工具 `sleep 6; printf 'example'`：Candidate 在工具返回前续跑，返回后在仍流式输出时 Promote，后续输出及 follow-up 使用保留的 Candidate 会话；下一轮 `sleep 6; printf 'different'` 不命中时丢弃候选，Parent 用真实结果继续。Tree 显示旧 Parent／候选经 Instrumentation 清理确认后进入 Cleaned。此验证证明执行接缝和控制路径，不证明真实大模型效果、预测质量或加速收益。

## 当前等待点与使用范围

第三个参数可指定本地 Policy YAML，例如 `assets/policy/pi-tool/policy.yaml`：该示例声明字符串枚举 `example`、`true`、`false`，无固定种子地抽取其中两个不同预测。布尔类型也可由 Policy 声明为 `value_type: boolean`，其候选必须是 JSON `true`/`false`，与字符串枚举成员 `"true"`/`"false"` 不同。默认 `assets/policy/policy/policy.yaml` 仍禁用候选（`max_chances: 0`）。编译在终端进入 raw mode 前完成，运行期分别求值 admit/select/predict/launch/match/dispose；Policy 错误回退 Parent，不把未知结果当作接受。

该次历史验证在工具等待边界后执行真实工具，并经 Pi 原有 afterToolCall 路径回传真实结果。比较使用 Parent 与 operation 的明确归属。Candidate 在独立目录和独立会话管理器中保存共同消息及配对预测工具结果，以会话的 `continue()` 路径续跑。control plane 只在实际 BranchStarted 后接受 Candidate；缓冲输出只在合法 Promote 后进入主对话，未命中输出不发布。Promote 保留胜出会话并取消旧 Parent 下全部其他 Candidate，含尚在创建的候选。当前 Runtime 结果交付路径以本文开头的实现范围为准。

目前转换标准 system/user/assistant/toolResult 消息与字符串、布尔预测；真实结果携带原始 content，匹配与错误结果是否兼容由 Policy 决定。Candidate 的工具执行由 AScout 的 Branch 生命周期统一控制：未 Promote 前等待，Promote 后放行，Discard 时取消；Pi 不按工具名或命令格式筛选。并行消息中，候选自己的调用使用预测，其余调用等待并复用真实结果，全部配对后才请求模型，不复制工具效果；此路径尚未完成专项端到端验证。非标准上下文问题仅记录、暂缓处理。

Candidate 不再由 Pi 判断工具是否安全；所有未验证 Candidate 工具调用统一等待 AScout 的生命周期处置。control plane 不为 Branch 复制工作目录：一次 run 只有一个工作目录，Branch 只是同一执行状态的别名，见下节“单工作目录”。它不构成 Runtime 隔离，Runtime 能力、Policy 决定和 AScout 的正确性约束仍需接入到正式的执行控制路径。

## 随机三结果场景

Workload 是具体运行场景，Policy 不选择业务场景，只对等待点做准入、预测和处置。

- Parent 真实命令：`sleep 6; python3 -c 'import secrets; print(secrets.choice(["example", "true", "false"]), end="")'`。
- 两个 Candidate 分别执行 `sleep 1; printf 'Predicted{n}: {预测值}'`。
- Parent 收到真实结果后默认执行 `sleep 1; printf 'spec fail'`，只有 Promote 会中断旧 Parent。
- Policy 使用系统随机源抽取两个不同值；结果不固定，不保证每次命中。

`examples/random-workload-provider.py` 提供单独的本地脚本模型接口（不是大模型），通过时间区分本场景的提前预测请求和六秒后的 Parent 请求；只用于演示，不是产品路由或通用正确性依据。业务命令只在该脚本和示例 Policy 中，control plane 不识别 `Predicted` 或 `spec fail`。

启动脚本：`python3 examples/random-workload-provider.py`。使用独立临时 Pi 配置（不要覆盖用户配置），在 `models.json` 中添加 `openai-completions` provider，baseUrl 为 `http://127.0.0.1:18764/v1`，apiKey 为任意非空本地占位值，model id 为 `boundary-check`，contextWindow 为 `32000`，maxTokens 为 `1000`；`settings.json` 指定对应 defaultProvider/defaultModel。随后：

```sh
PI_CODING_AGENT_DIR=/tmp/opencode/ascout-check-config \
cargo run --bin ascout-control-plane -- ../pi --policy pi-tool
```

输入任意请求触发场景。2026-10-25 实测日志 `/tmp/opencode/ascout-f7tYJy/events.jsonl`：两个 Candidate 均在 Parent 返回前执行完成，真实值 `example` 后一个 Promote、另一个 Discard，旧 Parent 开始默认命令后被取消，Tree 确认旧 Parent 与 loser 已 Cleaned。这是脚本模型接口加真实工具验证，不是真实大模型成功证据。

同一保留会话继续运行，第六轮真实值为 `false`、两个预测均不匹配：两个 Candidate Discard，Parent 的 `spec fail` 实际完成并作为主对话结果，未发布失败候选输出。Rust 编译、既有两个 agent_control 测试和 Pi 完整 check 通过。检查全仓空白时发现原有 `AGENTS.md` 末尾空行提示，本轮未修改该用户文件。

## 尚未完成

嵌套、并行工具专项验证、非标准消息、Pi 原生消息字段之外的工具专有元数据、Extent/Rebalance 的持续运行管理和完整依赖/效果提交仍未完成。当前不能推广为任意 coding workload 的正确 Promote。普通预测值以文本交付，原生文本和图像块、显式失败状态及适用的消息详情由 Pi 会话保留，详见 [Agent 接入](agent-access.md#4-session-持久化与状态交接已定方向及实现差距)。Candidate 的会话由独立原生 SessionManager 文件承载，可在最终汇总与恢复流程中使用；在飞模型请求和候选推理上下文仍不构成跨进程恢复协议。

`branch_model` 的模型候选、在飞模型请求和候选推理上下文目前不构成可跨进程保存的恢复协议；保存恢复会保存已定义的 Tree、原生 session、Policy artifact、模型绑定和 Runtime 绑定，但恢复后不会自动重新推理或自动处置候选。无限阻塞 Runtime、任意外部工具副作用、子进程内存和任意旧 run 的隐藏环境等价性仍不保证；详细限制见 [恢复](recovery.md)。

完整多行编辑、Markdown 和工具专门排版、宽字符排版、产品特有交互、断线恢复、资源硬限制与确认式退出清理尚未实现。当前是简单终端原型，不宣称已达到原 Agent 全部交互能力。

环境管理已由 Runtime Adapter 接通文件系统与进程 Backend；NLE 路径的不同 Environment 引用同一 workload 世界，各自生命周期独立。目录不代表任意 shell 或外部效果已隔离，尤其不隔离扩展自身的代码及网络行为。环境结束时 holder 会按 cgroup/process group 和 mount namespace 结束工具后代，但退出另一个 namespace、改变 cgroup 或依赖外部服务的进程不在该保证内；接入进程退出也不证明任意外部效果已撤回。文件系统 Backend 的覆盖范围、已知偏差和需高权限验证的项目见 [文件系统 Backend 当前实现状态](../fs-backend/current-state.md)。

## 真实游戏端到端验证方向（2026-10-28）

当前首个真实多步 Agent workload 优先尝试真实的二维迷雾地图游戏，而不是继续扩展随机脚本 workload。目标是让真实模型通过多次观察、移动、计算和重新观察逐步探索地图；地图信息不能在开局一次性提供，Candidate 的命中或不命中必须由游戏本身的隐藏状态和揭示机制自然产生，不能由测试额外故意制造。

NLE 是 AScout 的外部测试 workload，不属于 AScout 实现。下面描述的是 NLE 专用 Runtime／测试适配的行为，不是 AScout 核心契约；AScout 核心只要求 Branch 不影响权威路径和其他 Branch，具体如何满足由该适配声明。

候选接入顺序如下：

1. **NetHack + NLE**：NLE 提供 Gym 风格的 `reset()` / `step(action)` 与结构化观察，实际基础游戏是 NetHack 3.6.6。它不是 DCSS 的接口；接入前必须核实本地构建、可见区域限制、动作结果和状态复制／恢复能力。
2. **DCSS + dcss-ai-wrapper**：DCSS 有独立的 AI wrapper，但当前资料显示它依赖本地或 Docker 中的 DCSS Web 服务。若采用，必须单独核实逐步动作、观察、权限和状态分支能力，不能把它与 NLE 混称。
3. **简化二维迷雾游戏**：只有在真实游戏接口不能提供本次验证所需的动作、观察或分支能力时才制作。简化游戏借鉴真实游戏的迷雾、二维地图、逐步揭示和回合机制，但作为 workload adapter 存在，不改变 Runtime 的一般语义。

本机已在独立的 Python 3.13 环境中安装 NLE 1.3.0，并验证了 `NetHackScore-v0` 的 `reset()`、结构化观察和 `step(action)`。NLE 不提供公开的状态克隆接口，也不应当在环境侧为投机执行增加分支能力：环境是外部 workload，不能为了适配 AScout 而修改。在 NLE 这一测试适配中，非干扰由“共享游戏环境 + 作用前阻止”承担——环境只有一个真实实例，Candidate 不执行动作，只在与预测结果一致的假设下推演一步；效果在 Promote 前不进入环境。

### NLE workload bridge（当前外部适配原型）

`examples/nle-runtime-bridge.py` 在不修改通用 Runtime 契约的前提下，把 NLE 的公开 `reset`／`step` 能力包成 JSON Lines workload 接口。它支持建立、增加世界引用、执行一步、读取最近公开观测、恢复、终止和清理，也提供 `invoke` 工具入口：`nle_action` 返回可读摘要和结构化 evidence，`nethack_guide` 读取官方 Guidebook。它不读取隐藏地图或 NLE 内部状态。`branch` 为新 Environment 保留同一世界的引用；Environment 身份与生命周期由 NleBackend 分别管理。

NLE workload 保持单世界方案，不改变 control plane 的 Branch→Environment 独占绑定；Candidate 创建仍必须得到不同的 Environment id。NleBackend 声明 `branch_environment: true`，表示建立独立身份与生命周期，不表示复制世界。暂停一个 Environment 不暂停其他引用；终止或清理一个引用不销毁其他使用者的世界，最后一个引用释放时才关闭世界。保存恢复继续按唯一世界记录 seed 与公开动作历史，重建世界后恢复各 Environment 原身份和生命周期。Runtime 内部如何组合 Backend instance 不暴露给 control plane。Candidate 的动作在 Instrumentation 被暂存，Promote 前不进入环境；这是该 workload 的共享资源约束，不是 AScout 核心的自动隔离保证。动作前可配置固定延迟（`action_delay_seconds`），用来模拟慢动作并把投机推理的窗口显性化。种子必须通过 `env.unwrapped.seed(seed)` 再 `reset()` 设置；NLE 忽略 `reset(seed=...)`。Agent 权限和 Policy 准入不由这个 bridge 代替。

Rust 的 `NleBackend` 通过已有 `Backend` 接口调用该 bridge。运行时 Adapter 配置使用 `"backend": "nle"`、Python 解释器和 bridge 路径。Runtime 在握手中声明 `nle_action` 和 `nethack_guide` 工具；control plane 为每个 Branch 维护独占 Environment，用通用 `tool_context` 查询预测上下文；工具执行由 Instrumentation 经独立连接直接请求 Runtime。NLE backend 的 `invoke_tool` 返回 `result` 与 Policy 匹配证据 `evidence`，两者都只含公开观察。游戏观察的摘要和 `Guidebook` 章节由 bridge 生成，`NleBackend` 只转发；control plane 不解析 `blstats`、`chars` 或 `message`。

已完成的实际验证：

```sh
../.venv-nle/bin/python examples/nle-runtime-bridge.py --delay 2 <<'EOF'
{"id":"1","op":"establish","environment":"parent","seed":7}
{"id":"2","op":"branch","environment":"parent","child":"candidate"}
{"id":"3","op":"operate","environment":"candidate","action":1}
{"id":"4","op":"observe","environment":"parent"}
{"id":"5","op":"suspend","environment":"parent"}
{"id":"6","op":"operate","environment":"parent","action":1}
{"id":"7","op":"resume","environment":"parent"}
{"id":"8","op":"terminate","environment":"candidate"}
{"id":"9","op":"cleanup","environment":"parent"}
EOF
```

实测：分支出 `candidate` 后在它上面执行的动作，`parent` 的 `observe` 立即看到同一位置，确认是同一个世界；一次动作耗时约 2 秒；暂停期间的动作被拒绝，恢复后才可继续。同一 `seed` 的两次独立建立都从 `(26,5)` 开始，确认种子生效。

### AScout Runtime Adapter 接入

`src/runtime/backends/nle.rs` 通过既有 `Backend` 接口调用该 bridge，`src/bin/ascout-runtime-adapter.rs` 根据配置中的 `"backend": "nle"` 选择它；Firecracker 配置路径保持不变。每个 Branch 独占一个 Environment，不同 Environment 在 NLE backend 内引用同一个游戏世界，派生不复制世界。候选动作由 Instrumentation 的 effect 闸门阻塞，Promote 前不进入游戏调用。Adapter 退出时会主动终止 bridge 子进程。

`src/runtime/backends/nle.rs` 目前仍在核心 crate 中；它作为外部 workload 的 Runtime 适配保留，不应把 NLE 语义加入通用 Agent Adapter 或 control plane 工具路径。

### control plane 等待点接入（当前实现）

control plane 收到投机准入请求后按 Policy 创建 Candidate，并用 `tool_context` 查询预测上下文。`launch=true` 通过 `ResolvePending` 的 `phase: execute` 放行原权威调用，`false` 则保持执行前等待，不是先执行再延迟读取结果。Pi 通过 ASC-Proxy 的独立执行连接请求 Runtime，真实 `result` 沿原工具调用返回；Instrumentation 上报 action outcome 与 `evidence`，供 control plane 和 Policy 匹配使用。control plane 不发送工具 Invoke、不把真实工具结果注入等待器，也不识别 `nle_action` 或动作编号。

eager 路径的顺序是：

```text
工具执行前的投机准入请求
→ Policy 用当前公开观测枚举该动作可能的结果
→ 每个结果建立 Candidate Branch（Environment 独占，backend 内共享游戏世界）
→ Candidate 在假设结果下推演一步；它的工具调用被暂存
→ launch 放行原权威调用，由 Instrumentation 直接请求 Runtime 执行工具
→ 公开观察作为真实结果返回
→ 原工具调用返回，Instrumentation 上报 ActionFinished
→ Policy match
→ Promote 或 Discard
```

Candidate 在 Promote 前可推进推理，游戏工具调用在 effect 闸门等待，不改变世界；接受时 Promote 候选并放行其后续调用，拒绝时丢弃。speculative-first 则先保持原权威调用不执行：候选被接受后取消旧父调用，全部候选不可接受时原调用放行一次；不向旧调用注入候选结果。因此被拒绝的候选不会消耗游戏回合。

Promote 后 control plane 的权威 Branch 切换到 Candidate，旧 Parent Branch 被清理；候选继续使用自己的 Environment，NLE backend 内仍引用同一个游戏世界，不发生世界复制或切换。Discard 清理对应候选；候选在 Promote 前的游戏工具调用保持挂起，不伪造观察。

Rust 全目标编译、库测试和 Pi TypeScript 类型检查通过。Firecracker 全量 E2E 中仍有既有的宿主 Unix socket 路径过长失败（测试目录位于较深路径），不把这些失败归因于 NLE 接入；NLE Adapter 的正式 Unix 协议 smoke test 已通过。


### 真实模型 + NLE（2026-10-29）

真实模型闭环已跑通，模型为 DeepSeek V4.1 Flash（模型 ID `deepseek-flash`）。当前 Pi 接入不再写死 provider/model：Authority 使用 Pi 配置选定的模型，Candidate 继承 Parent 的模型与 thinking level。

实测两轮（每次 `nle_action`）：

```text
工具 action 等待：authority
branch_started candidate-1 / candidate-2
action 结果（Runtime 真实观察，payload = moved|blocked）
Disposition: Promote 命中者 / Discard 未命中者
接入清理确认：未命中者与旧 authority
```

第二轮在 Promote 后的 candidate 上继续执行并再次 Promote，说明权威切换和后续动作可以有效衔接。Promote 判定使用 Runtime 的观察结果；Pi 回报的工具文本只作为通道表示，不覆盖可比较的 payload。

NLE 专用 Policy 当前本地路径为 `assets/policy/nle/policy.yaml`：`predict` 用固定程序从公开观测枚举动作结果。目标格在记忆里是墙时只给 `blocked`，是空地时只给 `moved`，其他字符（怪物、宠物、门等）同时给出两种，`match` 仍用同一 enum 比较。`moved`/`blocked` 由 NLE 的 `blstats` 位置变化决定，命中与不命中因此由游戏自身状态自然产生。这个判定属于 Policy 的 verifier：Runtime 只交回动作前后的两份公开观测，`match_proposal` 自己从 `blstats` 读出实际结果再和预测比较；control plane 不解释坐标、格子或移动。

因为相邻格始终在当前视野内，`moved`/`blocked` 在绝大多数移动上是确定的，通常只有一个候选；只有目标格字符无法判定时才会出现两个候选。投机的主要收益不来自“猜得准”，而来自推理在动作延迟期间已经完成。

### 暂存式投机与计时验证（2026-10-31）

用真实模型跑同一任务（四步移动），对比启用和禁用投机两种配置：

```text
启用投机（每步命中后直接沿用已完成的推理）
  工具等待 → action 结果 ≈ 2.2s
  相邻两步之间 ≈ 2.26s

禁用投机（Policy 不让位，权威每次都重新决策）
  工具等待 → action 结果 ≈ 2.1s
  相邻两步之间 ≈ 4.0s
```

每步节省约 1.8 秒，上限就是 2 秒的动作延迟，和设计预期一致。两候选实例也实测通过：宠物格无法判定时同时建立 `moved` 与 `blocked` 两个候选，真实结果为 `moved`，命中者 Promote、另一个 Discard 并清理。

实测中还发现并修正了两处实现错误：NLE 的对角键 `u` 是东北、`y` 是西北（原先写反，导致一次 `moved` 预测对 `blocked` 结果的误判）；`reset(seed=...)` 不生效，必须用 `unwrapped.seed()`。

运行依赖不再放在临时目录：

```text
../.venv-nle        NLE 1.3.0 虚拟环境，依赖见 requirements-nle.txt
nle-runtime.json    Runtime Adapter 配置
```

Node 使用系统 `/usr/bin/node`（v26.12.1，满足 Pi 要求的 ≥22.19），不再需要单独下载 Node。若 shell 里 nvm 默认是旧版本，启动时把 `/usr/bin` 放到 PATH 前面。

### 单工作目录与提示词前缀（2026-11-01）

正式全程验证第一次起跑时发现成本异常：48 个动作花费 $0.0971，其中 87% 花在 12 次整段重读上（260,219 个 token 按原价计），单动作成本 $0.0020，是缓存命中时（约 $0.0002）的十倍量级。

原因不在记账，而在提示词：当时每个 Branch 有自己的目录，Pi 把该目录的绝对路径写进系统提示，而系统提示位于对话最前面。逐条比对两次相邻请求的请求体，差异只有这一行：

```text
/tmp/opencode/ascout-LSNELW/authority
/tmp/opencode/ascout-LSNELW/candidate-1
```

系统提示一改，其后的整段对话都不再命中前缀缓存，每开一个新 Branch 就重读一次完整上下文，且重读量随对话增长。

已确认改为**一次 run 只有一个工作目录**：所有 Branch 共用同一个路径，系统提示全程恒定，前缀缓存连续命中。候选在当前实现中不执行工具（工具调用暂存到 Promote 才放行），因此共享目录不带来额外影响；需要目录级隔离时应由 Runtime 提供，而不是靠 Branch 目录名。

同一配置下的实测对比：

```text
                    动作数   总花费    单动作    最大单次输入
每 Branch 一个目录      48   $0.0971   $0.0020   20,580 token
一个 run 一个目录      36   $0.0166   $0.00046   1,456 token
```

第二组没有出现任何整段重读，命中缓存的输入随对话增长（3.5K → 18.7K），单次输入始终在 1.5K 以内。

### 接入核心与 Runtime 工具（2026-11-02）

使用本地脚本模型接口（`examples/random-workload-provider.py`，不是真实大模型）重新验证当前代码路径。运行参数：

```sh
PATH=/usr/bin:$PATH \
ASCOUT_RUNTIME_CONFIG=nle-runtime.json \
./target/debug/ascout-control-plane ../pi --policy pi-tool
```

输入三轮后取得命中结果。`/tmp/ascout-run6/events.jsonl` 中的事实：

```text
candidate-1 / candidate-2 branch_started（前两轮各两个候选）
candidate-5 disposition=Promote verdict=compatible
candidate-6 disposition=Discard reason=Parent replaced
candidate-5 command_result operation=promote status=applied
authority / candidate-6 command_result operation=discard status=applied
authority / candidate-6 command_result operation=cleanup status=applied
```

Promote 后继续输入第四轮，`candidate-5` 作为当前权威 Branch 接收用户消息，并产生 `candidate-7`、`candidate-8`；没有出现 `branch_failed`。这次验证覆盖了同一 `operation_id` 的重复 resolve 不再触发协议冲突、`CheckpointHandle` 校验、Runtime 工具声明、Promote 后会话续用和未命中候选清理。

这次使用的模型接口是脚本，不是真实大模型；它证明接入核心、工具通道和 Branch 生命周期可以实际运行，不证明真实模型预测质量或加速收益。

### 真实模型 + NLE（2026-11-02）

真实模型路径也已跑通。模型为 DeepSeek V4.1 Flash（`deepseek-flash`，thinking level `low`），通过隔离的临时 Pi 配置指定；用户级配置中的 `deepseek-v4-flash` 已不存在，Pi 回退到了不可用的 `gpt-5.5`，临时配置没有修改用户文件。

运行参数：

```sh
PATH=/usr/bin:$PATH \
ASCOUT_RUNTIME_CONFIG=nle-runtime.json \
./target/debug/ascout-control-plane ../pi --policy nle
```

首次消息使用 `examples/nle-task.md` 的完整任务说明。模型连续执行了 10 次动作，`/tmp/ascout-real2/events.jsonl` 的结果：

```text
9 次 disposition=Promote，verdict=compatible
1 次 disposition=Discard，verdict=incompatible
10 次 discard 命令 applied（含被替换的旧 Parent）
10 次 cleanup 命令 applied
0 次 branch_failed
26 次模型调用，模型均为 deepseek-flash，总花费约 $0.0060
```

候选在 Promote 后继续作为当前权威 Branch 执行下一步，旧 Parent 进入 Discard／Cleanup。最后一次真实结果命中 `blocked` 候选，说明预测并非只产生 `moved`。退出后 control plane、Adapter、Agent 和 NLE bridge 均无残留进程。

真实游戏代码和其运行环境属于外部 workload／接入边界，不直接成为 AScout Runtime 核心。Runtime 的一般契约仍然是建立、操作、分支、静止、恢复、终止和清理，并如实声明能力。对于地图游戏，动作暂存、在 Candidate 分支中执行，或 Promote 后提交，都只是具体 Policy、Runtime 能力和 workload 共同选择的实现策略。

首段验收至少需要证明：

- Parent 使用 Agent 原有权限执行真实游戏动作；Candidate 不得扩大这些权限；
- Candidate 的动作同时满足 Agent 权限、Policy 准入和 Runtime 能力，任一不满足都阻止并记录真实原因；
- Candidate 只能使用当时可见的游戏观察和自己的预测，不能读取隐藏地图或伪造观察结果；
- Parent 的真实动作结果作为权威依据，驱动预测比较及 Promote／Discard；
- 游戏状态、动作、观察、Candidate 启动／阻止、比较、处置和清理均可观测；
- 如果某个游戏接口不能提供独立状态或真实结果核对，则不能声称已完成真实投机验证，应转入简化游戏方案。

### Control Plane 事件处理与状态所有权（2026-11-05）

2026-11-14 目标调整、代码待核查：一个 session 对应一个 AScoutAgent 和一棵 Speculative Tree，task 是 Agent 完成本轮工作并开始等待用户输入前的一轮工作。以下实现记录中的 `task`／`task_id` 命名尚需逐处核查其实际生命周期，不能据其名称推导“一轮工作创建一棵树”。session 建立、多轮输入、树与预算归属、完成事实、物化报告及对外成功报告路径仍需对齐；本轮未核查代码或重跑以下历史验证，不能据此宣称目标已实现。

`SpeculativeTree` 直接拥有 Branch 状态、编号、命令回执、等待点及未完成外部操作。`SpeculationCoordinator` 无 task 状态，通过显式传入的树只读事实调用 Policy 函数或 Runtime。树的方法检查并落实操作，Policy 求值器不修改树。

`src/bin/ascout-control-plane/execution.rs` 接收 Agent 消息、UI 请求、操作返回及到期检查，同一 task 顺序处理。运行期间不再每 100 毫秒轮询；等待使用树给出的最近期限，期限到达才触发检查。Policy 与 Runtime 操作在工作线程完成，不阻塞控制事件处理。读取树的副本仅用于一次异步求值，不成为另一份可变控制状态。取消、结束或状态变化后，返回结果重新检查有效性；仍有环境操作在进行的 Branch 不得提前归档或清理。

Gateway 的 Agent socket、UI HTTP/SSE 和 Runtime ControlSession 通信代码已统一归入 `src/control_plane/gateway/`。协调逻辑依赖类型化 Runtime 操作／结果接口，UI 绘制与对话折叠、Ledger 保存、Tree 转换仍在原责任处；详情、验证及剩余缺口见 [Gateway 实现核查](gateway.md)。部署与生命周期以[control plane §2.1](../design/control-plane.md#21-gateway)为准：Gateway 属于 control plane，与 UI 均不独立部署，由 control plane 统一启动和停止；Gateway 与 Runtime 网络通信，Agent 与接入进程实际由 Runtime 启动、监视和回收。Gateway 不承担 Tree、Policy 或投机编排。

验证结果和支持边界见本轮验收记录；脚本 Agent／Runtime 替身验证控制路径，不代表真实模型或 NLE 的端到端效果。完整 Policy 调度、嵌套与 Registry 能力验证仍属既有后续范围。

本轮验收（2026-11-05）：`cargo test --lib --bins`、`cargo test --test policy`、`/usr/bin/node --test tests/pi-access.mjs`、`cargo check --all-targets`、`cargo clippy --lib --bin ascout-control-plane -- -D warnings` 通过。新增 3 项回归前已复现取消后的迟到预测、环境建立期间提前归档、工具超时重复立即唤醒问题，修复后通过。

脚本 Agent／Runtime 替身经正式 Unix 协议和现有 Policy 求值路径完成：命中接受、不命中丢弃、接受后输入、慢 Runtime 期间中断，以及有序退出。共享环境验证目录 `/tmp/ascout-event-probe-f6mwzji2`，真实 SIGINT 验证 `/tmp/ascout-event-probe-bdwx8p3u`，独立环境协议路径验证 `/tmp/ascout-event-probe-vdx7fvxl`。替身只证明控制流程，不证明真实隔离或真实 Agent 任务收益。未调用真实模型、未运行真实 NLE／Firecracker。

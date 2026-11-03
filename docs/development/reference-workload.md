# 首要验证 Workload

首个真实接入对象是 Pi，当前源码位于相邻的 `pi-code`。本文只记录当前交互实现，不定义完整 Runtime 或 coding workload。

## 当前可运行范围

运行方式见 [README](../../README.md)。控制面提供权威 Tree、Policy 和生命周期事实；Pi 接入侧使用 Pi 公开 SDK 管理 AgentSession，报告消息、思考、工具调用、结果和执行事件。浏览器 UI 与 TUI 通过共同接口读取这些事实并各自排版。Runtime Adapter 在环境内启动 Pi 接入进程，控制面只决定用哪个产品并把路径和版本交给它。Pi 接入没有 Tree 或 Policy 裁决权。

`/ascout` 选择 Branch 后，默认显示由 AScout 根据 SDK 消息排版的对话内容；Tab 切换 AScout 运行事实，↑/↓ 滚动，Esc 返回主对话。候选窗口不接收用户输入。当前仍使用 AScout 输入栏，不支持 Pi 完整编辑器、命令菜单或图片，也不承诺显示 Pi 原生终端画面。候选的独有输出分隔来自 Pi 接入侧的真实消息边界，不再复制 Rust 展示文本。

- 运行目录由调用方创建，控制面只使用它；Agent 日志、Adapter 日志和 `events.jsonl` 都在其中。
- 初始 Authority 在运行目录的工作区中建立，连续请求使用同一 AgentSession。
- 模型选择与认证使用 Pi 现有配置，不写死 provider。
- 模型文本、工具启动与工具结果经接入协议实时回传，由 AScout 展示；模型错误如实显示。
- `/ascout` 展示当前控制面 Tree，可按编号深入只读查看 Branch 的记录和执行输出；选择观察对象不改变权威路径。
- Ctrl-C 请求中断并丢弃在飞候选；Ctrl-D 退出。退出时控制面清理环境，Runtime 回收 Agent 进程；运行目录保留用于诊断。
- 仅在 Pi 报告真实 `ToolPending` 时创建 Candidate；Candidate 使用复制的消息上下文与预测工具结果继续，不把预测或最终模型回答充当 Parent operation outcome。

## 手动测试步骤

前提：在仓库根目录执行；`/usr/bin` 排在 `PATH` 最前（Node ≥ 22.19）；NLE 虚拟环境与 `nle-runtime.json` 已就绪。

```sh
# 1. 全量编译（Adapter 是独立二进制，必须一起编译）
cargo build

# 2. 建运行目录（由调用方创建，控制面不创建也不删除）
mkdir -p /tmp/opencode/run-1

# 3. 启动（相对路径按当前目录解析，所以要在仓库根目录）
PATH=/usr/bin:$PATH \
ASCOUT_RUNTIME_CONFIG=nle-runtime.json \
./target/debug/ascout-control-plane /tmp/opencode/run-1 ../pi-code assets/policy/nle.yaml
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

随后以独立临时 Pi 配置连接本地可控模型接口（脚本，不是真实大模型），使用真实 Pi 工具 `sleep 6; printf 'example'`：Candidate 在工具返回前续跑，返回后在仍流式输出时 Promote，后续输出及 follow-up 使用保留的 Candidate 会话；下一轮 `sleep 6; printf 'different'` 不命中时丢弃候选，Parent 用真实结果继续。Tree 显示旧 Parent／候选经 CleanupCompleted 后进入 Cleaned。此验证证明执行接缝和控制路径，不证明真实大模型效果、预测质量或加速收益。

## 当前等待点与使用范围

第三个参数可指定 Policy YAML，例如 `assets/policy/pi-tool.yaml`：该示例声明字符串枚举 `example`、`true`、`false`，无固定种子地抽取其中两个不同预测。布尔类型也可由 Policy 声明为 `value_type: boolean`，其候选必须是 JSON `true`/`false`，与字符串枚举成员 `"true"`/`"false"` 不同。默认 `assets/policy/policy.yaml` 仍禁用候选（`max_edges: 0`）。编译在终端进入 raw mode 前完成，运行期分别求值 admit/select/predict/launch/match/dispose；Policy 错误回退 Parent，不把未知结果当作接受。

接入报告 ToolPending 后立即执行真实工具；真实结果经 Pi 原有 afterToolCall 处理后报告为 ParentOutcome，立即交回 Parent，不等待 ResolvePending。比较使用 Parent 与 operation 的明确归属。Candidate 在独立目录和独立会话管理器中保存共同消息及配对预测工具结果，以会话的 `continue()` 路径续跑。控制面只在实际 BranchStarted 后接受 Candidate；缓冲输出只在合法 Promote 后进入主对话，未命中输出不发布。Promote 保留胜出会话并取消旧 Parent 下全部其他 Candidate，含尚在创建的候选。

目前转换标准 system/user/assistant/toolResult 消息与字符串、布尔预测；真实结果携带原始 content，匹配与错误结果是否兼容由 Policy 决定。Candidate 的工具执行由 AScout 的 Branch 生命周期统一控制：未 Promote 前等待，Promote 后放行，Discard 时取消；Pi 不按工具名或命令格式筛选。并行消息中，候选自己的调用使用预测，其余调用等待并复用真实结果，全部配对后才请求模型，不复制工具效果；此路径尚未完成专项端到端验证。非标准上下文问题仅记录、暂缓处理。

Candidate 不再由 Pi 判断工具是否安全；所有未验证 Candidate 工具调用统一等待 AScout 的生命周期处置。控制面不为 Branch 复制工作目录：一次 run 只有一个工作目录，Branch 只是同一执行状态的别名，见下节“单工作目录”。它不构成 Runtime 隔离，Runtime 能力、Policy 决定和 AScout 的正确性约束仍需接入到正式的执行控制路径。

## 随机三结果场景

Workload 是具体运行场景，Policy 不选择业务场景，只对等待点做准入、预测和处置。

- Parent 真实命令：`sleep 6; python3 -c 'import secrets; print(secrets.choice(["example", "true", "false"]), end="")'`。
- 两个 Candidate 分别执行 `sleep 1; printf 'Predicted{n}: {预测值}'`。
- Parent 收到真实结果后默认执行 `sleep 1; printf 'spec fail'`，只有 Promote 会中断旧 Parent。
- Policy 使用系统随机源抽取两个不同值；结果不固定，不保证每次命中。

`examples/random-workload-provider.py` 提供单独的本地脚本模型接口（不是大模型），通过时间区分本场景的提前预测请求和六秒后的 Parent 请求；只用于演示，不是产品路由或通用正确性依据。业务命令只在该脚本和示例 Policy 中，控制面不识别 `Predicted` 或 `spec fail`。

启动脚本：`python3 examples/random-workload-provider.py`。使用独立临时 Pi 配置（不要覆盖用户配置），在 `models.json` 中添加 `openai-completions` provider，baseUrl 为 `http://127.0.0.1:18764/v1`，apiKey 为任意非空本地占位值，model id 为 `boundary-check`，contextWindow 为 `32000`，maxTokens 为 `1000`；`settings.json` 指定对应 defaultProvider/defaultModel。随后：

```sh
PI_CODING_AGENT_DIR=/tmp/opencode/ascout-check-config cargo run --bin ascout-control-plane -- /tmp/opencode ../pi-code assets/policy/pi-tool.yaml
```

输入任意请求触发场景。2026-10-25 实测日志 `/tmp/opencode/ascout-f7tYJy/events.jsonl`：两个 Candidate 均在 Parent 返回前执行完成，真实值 `example` 后一个 Promote、另一个 Discard，旧 Parent 开始默认命令后被取消，Tree 确认旧 Parent 与 loser 已 Cleaned。这是脚本模型接口加真实工具验证，不是真实大模型成功证据。

同一保留会话继续运行，第六轮真实值为 `false`、两个预测均不匹配：两个 Candidate Discard，Parent 的 `spec fail` 实际完成并作为主对话结果，未发布失败候选输出。Rust 编译、既有两个 agent_control 测试和 Pi 完整 check 通过。检查全仓空白时发现原有 `AGENTS.md` 末尾空行提示，本轮未修改该用户文件。

## 尚未完成

嵌套、并行工具专项验证、非标准消息、图像等复杂结果转换、多种结果元数据的一致性、Extent/Rebalance 的持续运行管理和完整依赖/效果提交仍未完成。当前不能推广为任意 coding workload 的正确 Promote。字符串枚举和布尔预测已支持，但 Pi 仍把它们转换成工具接口所需的文本内容；图像、结构化多段结果和工具专有元数据尚未转换。Candidate 共同上下文保存在独立内存 SessionManager 中，不提供进程重启后的恢复保证。

完整多行编辑、Markdown 和工具专门排版、宽字符排版、产品特有交互、断线恢复、资源硬限制与确认式退出清理尚未实现。当前是简单终端原型，不宣称已达到原 Agent 全部交互能力。

环境管理暂不接通完整 Runtime Adapter 是已确认的开发顺序；目录不代表任意 shell 或外部效果已隔离，尤其不隔离扩展自身的代码及网络行为。接入进程退出只确认该进程退出，不证明任意工具后代均已停止或环境资源已清理。

## 真实游戏端到端验证方向（2026-10-28）

当前首个真实多步 Agent workload 优先尝试真实的二维迷雾地图游戏，而不是继续扩展随机脚本 workload。目标是让真实模型通过多次观察、移动、计算和重新观察逐步探索地图；地图信息不能在开局一次性提供，Candidate 的命中或不命中必须由游戏本身的隐藏状态和揭示机制自然产生，不能由测试额外故意制造。

NLE 是 AScout 的外部测试 workload，不属于 AScout 实现。下面描述的是 NLE 专用 Runtime／测试适配的行为，不是 AScout 核心契约；AScout 核心只要求 Branch 不影响权威路径和其他 Branch，具体如何满足由该适配声明。

候选接入顺序如下：

1. **NetHack + NLE**：NLE 提供 Gym 风格的 `reset()` / `step(action)` 与结构化观察，实际基础游戏是 NetHack 3.6.6。它不是 DCSS 的接口；接入前必须核实本地构建、可见区域限制、动作结果和状态复制／恢复能力。
2. **DCSS + dcss-ai-wrapper**：DCSS 有独立的 AI wrapper，但当前资料显示它依赖本地或 Docker 中的 DCSS Web 服务。若采用，必须单独核实逐步动作、观察、权限和状态分支能力，不能把它与 NLE 混称。
3. **简化二维迷雾游戏**：只有在真实游戏接口不能提供本次验证所需的动作、观察或分支能力时才制作。简化游戏借鉴真实游戏的迷雾、二维地图、逐步揭示和回合机制，但作为 workload adapter 存在，不改变 Runtime 的一般语义。

本机已在独立的 Python 3.13 环境中安装 NLE 1.3.0，并验证了 `NetHackScore-v0` 的 `reset()`、结构化观察和 `step(action)`。NLE 不提供公开的状态克隆接口，也不应当在环境侧为投机执行增加分支能力：环境是外部 workload，不能为了适配 AScout 而修改。在 NLE 这一测试适配中，非干扰由“共享游戏环境 + 作用前阻止”承担——环境只有一个真实实例，Candidate 不执行动作，只在与预测结果一致的假设下推演一步；效果在 Promote 前不进入环境。

### NLE workload bridge（当前外部适配原型）

`examples/nle-runtime-bridge.py` 在不修改通用 Runtime 契约的前提下，把 NLE 的公开 `reset`／`step` 能力包成 JSON Lines workload 接口。它支持建立、逻辑视图、执行一步、读取最近公开观测、暂停、恢复、终止和清理，也提供 `invoke` 工具入口：`nle_action` 返回可读摘要和结构化 evidence，`nethack_guide` 读取官方 Guidebook。它不读取隐藏地图或 NLE 内部状态。这里的 `branch` 是 NLE 测试适配内部的逻辑视图，不是 `branch_environment`；它不派生独立环境。

环境只有一个真实实例。NLE 适配用 `branch` 给同一个世界增加一个逻辑名字，不复制状态，也不把它声明为 `branch_environment`；Candidate 与权威看到的是同一份世界，区别只在 AScout 是否允许它动作。Candidate 的动作在 Agent 接入侧被暂存，Promote 前不进入环境；这是 NLE 专用 Runtime 的“共享环境 + 作用前阻止”实现，不是 AScout 核心契约。动作前可配置固定延迟（`action_delay_seconds`），用来模拟慢动作并把投机推理的窗口显性化。种子必须通过 `env.unwrapped.seed(seed)` 再 `reset()` 设置；NLE 忽略 `reset(seed=...)`。Agent 权限和 Policy 准入不由这个 bridge 代替。

Rust 的 `NleBackend` 通过已有 `Backend` 接口调用该 bridge。运行时 Adapter 配置使用 `"backend": "nle"`、Python 解释器和 bridge 路径。Runtime 在握手中声明 `nle_action` 和 `nethack_guide` 工具、`shared_environment: true` 和 `branch_environment: false`；控制面只使用通用的 `tool_context` 和 `invoke_tool` 操作，不再识别工具名或动作编号。NLE 的 `invoke_tool` 返回工具通道结果 `result` 与 Policy 匹配证据 `evidence`，两者都只含公开观察。游戏观察的摘要和 `Guidebook` 章节都由 bridge 生成，`NleBackend` 只转发；控制面不解析 `blstats`、`chars` 或 `message`。

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

`src/nle.rs` 现在通过既有 `Backend` 接口调用该 bridge，`src/bin/ascout-runtime-adapter.rs` 根据配置中的 `"backend": "nle"` 选择它；Firecracker 配置路径保持不变。已经用正式的 4 字节大端长度前缀 Unix 协议完成跨边界 smoke test。2026-11-02 起，NLE 不再声明或调用 `branch_environment`：Candidate 与权威共用同一个底层环境，Candidate 动作只在 Agent 接入侧暂存，Promote 前不进入环境。Adapter 退出时会主动终止 bridge 子进程。

`src/nle.rs` 目前仍在核心 crate 中；它作为外部 workload 的 Runtime 适配保留，不应把 NLE 语义加入通用 Agent Adapter 或控制面工具路径。

### 控制面等待点接入（当前实现）

`ResolvePending` 携带可选 `outcome`。控制面收到 Runtime 已声明工具的 `ToolPending` 后，先依据当前策略创建 Candidate 分支，再通过通用 `ControlSession` 的 `tool_context`／`invoke_tool` 操作调用 Runtime；Runtime 返回的 `result` 交给 Pi 等待中的工具，`evidence` 留给 Policy 匹配。若结果先于 Pi 工具等待器建立到达，Pi 接入侧会暂存该结果，工具建立等待器后立即消费。通用控制面不识别 `nle_action` 或动作编号。

当前顺序是：

```text
ToolPending
→ Policy 用当前公开观测枚举该动作可能的结果
→ 每个结果建立一个逻辑 Candidate Branch（共享游戏环境，不复制世界）
→ Candidate 在假设结果下推演一步；它的工具调用被暂存
→ 权威通过 Runtime 的通用 invoke_tool 真实执行工具（NLE 动作前有固定延迟）
→ 公开观察作为真实结果返回
→ ResolvePending / ParentOutcome
→ Policy match
→ Promote 或 Discard
```

Candidate 在 Promote 前只产生推理和一次被暂存的工具调用，不改变世界；命中时它的暂存调用直接放行，未命中时丢弃。因此猜错不消耗游戏回合。

Promote 后控制面的权威 Branch 切换到 Candidate，旧 Parent Branch 被清理；NLE 的游戏环境仍是同一个真实实例，不发生世界复制或切换。Discard 只清理 Candidate。Candidate 在 Promote 前调用 `nle_action` 会明确失败并记录为阻止，不会伪造观察。

Rust 全目标编译、库测试和 Pi TypeScript 类型检查通过。Firecracker 全量 E2E 中仍有既有的宿主 Unix socket 路径过长失败（测试目录位于较深路径），不把这些失败归因于 NLE 接入；NLE Adapter 的正式 Unix 协议 smoke test 已通过。


### 真实模型 + NLE（2026-10-29）

真实模型闭环已跑通，模型为 DeepSeek V4.1 Flash（模型 ID `deepseek-flash`）。当前 Pi 接入不再写死 provider/model：Authority 使用 Pi 配置选定的模型，Candidate 继承 Parent 的模型与 thinking level。

实测两轮（每次 `nle_action`）：

```text
tool_pending authority
branch_started candidate-1 / candidate-2
parent_outcome（Runtime 真实观察，payload = moved|blocked）
Disposition: Promote 命中者 / Discard 未命中者
cleanup_completed 未命中者与旧 authority
```

第二轮在 Promote 后的 candidate 上继续执行并再次 Promote，说明权威切换和后续动作可以有效衔接。Promote 判定使用 Runtime 的观察结果；Pi 回报的工具文本只作为通道表示，不覆盖可比较的 payload。

NLE 专用 Policy 为 `assets/policy/nle.yaml`：`predict` 不再抽样，而是用固定程序从公开观测枚举动作结果。目标格在记忆里是墙时只给 `blocked`，是空地时只给 `moved`，其他字符（怪物、宠物、门等）同时给出两种，`match` 仍用同一 enum 比较。`moved`/`blocked` 由 NLE 的 `blstats` 位置变化决定，命中与不命中因此由游戏自身状态自然产生。这个判定属于 Policy 的 verifier：Runtime 只交回动作前后的两份公开观测，`match_proposal` 自己从 `blstats` 读出实际结果再和预测比较；控制面不解释坐标、格子或移动。

因为相邻格始终在当前视野内，`moved`/`blocked` 在绝大多数移动上是确定的，通常只有一个候选；只有目标格字符无法判定时才会出现两个候选。投机的主要收益不来自“猜得准”，而来自推理在动作延迟期间已经完成。

### 暂存式投机与计时验证（2026-10-31）

用真实模型跑同一任务（四步移动），对比启用和禁用投机两种配置：

```text
启用投机（每步命中后直接沿用已完成的推理）
  tool_pending → parent_outcome ≈ 2.2s
  相邻两步之间 ≈ 2.26s

禁用投机（Policy 不让位，权威每次都重新决策）
  tool_pending → parent_outcome ≈ 2.1s
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
PI_CODING_AGENT_DIR=/tmp/ascout-config \
ASCOUT_RUNTIME_CONFIG=nle-runtime.json \
./target/debug/ascout-control-plane /tmp/ascout-run6 ../pi-code assets/policy/pi-tool.yaml
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
PI_CODING_AGENT_DIR=/tmp/ascout-real-config \
ASCOUT_RUNTIME_CONFIG=nle-runtime.json \
./target/debug/ascout-control-plane /tmp/ascout-real2 ../pi-code assets/policy/nle.yaml
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

候选在 Promote 后继续作为当前权威 Branch 执行下一步，旧 Parent 进入 Discard／Cleanup。最后一次真实结果命中 `blocked` 候选，说明预测并非只产生 `moved`。退出后控制面、Adapter、Agent 和 NLE bridge 均无残留进程。

真实游戏代码和其运行环境属于外部 workload／接入边界，不直接成为 AScout Runtime 核心。Runtime 的一般契约仍然是建立、操作、分支、静止、恢复、终止和清理，并如实声明能力。对于地图游戏，动作暂存、在 Candidate 分支中执行，或 Promote 后提交，都只是具体 Policy、Runtime 能力和 workload 共同选择的实现策略。

首段验收至少需要证明：

- Parent 使用 Agent 原有权限执行真实游戏动作；Candidate 不得扩大这些权限；
- Candidate 的动作同时满足 Agent 权限、Policy 准入和 Runtime 能力，任一不满足都阻止并记录真实原因；
- Candidate 只能使用当时可见的游戏观察和自己的预测，不能读取隐藏地图或伪造观察结果；
- Parent 的真实动作结果作为权威依据，驱动预测比较及 Promote／Discard；
- 游戏状态、动作、观察、Candidate 启动／阻止、比较、处置和清理均可观测；
- 如果某个游戏接口不能提供独立状态或真实结果核对，则不能声称已完成真实投机验证，应转入简化游戏方案。

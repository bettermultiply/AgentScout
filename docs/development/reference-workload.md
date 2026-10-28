# 首要验证 Workload

首个真实接入对象是 Pi，当前源码位于相邻的 `pi-code`。本文只记录当前交互实现，不定义完整 Runtime 或 coding workload。

## 当前可运行范围

运行方式见 [README](../../README.md)。Rust 控制面自动启动 Pi 接入进程，并提供 AScout 自己的终端界面；Pi 只管理本地 AgentSession、执行控制命令和报告事实，没有 Tree 或 Policy 裁决权。

- 初始 Authority 在本次独立管理目录中建立，连续请求使用同一 AgentSession。
- 模型选择与认证使用 Pi 现有配置，不写死 provider。
- 模型文本、工具启动与工具结果经接入协议实时回传，由 AScout 展示；模型错误如实显示。
- `/ascout` 展示当前控制面 Tree，可按编号深入只读查看 Branch 的记录和执行输出；选择观察对象不改变权威路径。
- Ctrl-C 请求中断并丢弃在飞候选；Ctrl-D 退出。运行目录、`agent.log` 和 `events.jsonl` 保留用于诊断。
- 仅在 Pi 报告真实 `ToolPending` 时创建 Candidate；Candidate 使用复制的消息上下文与预测工具结果继续，不把预测或最终模型回答充当 Parent operation outcome。

## 实际检查（2026-10-25）

在 tmux 真实终端启动，确认 Pi 接入成功、Authority 启动、主对话输入送达、模型错误实时回传、Tree 展示和 Branch 详情显示。Pi 默认模型 `gpt-5.5` 的当前服务返回 HTTP 503 `model_not_found`；没有伪造模型成功回答。中断后收到本轮结束事实。Rust 编译与已有两个控制状态测试通过，Pi 完整 `npm run check` 通过。

随后以独立临时 Pi 配置连接本地可控模型接口（脚本，不是真实大模型），使用真实 Pi 工具 `sleep 6; printf 'example'`：Candidate 在工具返回前续跑，返回后在仍流式输出时 Promote，后续输出及 follow-up 使用保留的 Candidate 会话；下一轮 `sleep 6; printf 'different'` 不命中时丢弃候选，Parent 用真实结果继续。Tree 显示旧 Parent／候选经 CleanupCompleted 后进入 Cleaned。此验证证明执行接缝和控制路径，不证明真实大模型效果、预测质量或加速收益。

## 当前等待点与使用范围

第三个参数可指定 Policy YAML，例如 `assets/policy/pi-tool.yaml`：该示例声明字符串枚举 `example`、`true`、`false`，无固定种子地抽取其中两个不同预测。布尔类型也可由 Policy 声明为 `value_type: boolean`，其候选必须是 JSON `true`/`false`，与字符串枚举成员 `"true"`/`"false"` 不同。默认 `assets/policy/policy.yaml` 仍禁用候选（`max_edges: 0`）。编译在终端进入 raw mode 前完成，运行期分别求值 admit/select/predict/launch/match/dispose；Policy 错误回退 Parent，不把未知结果当作接受。

接入报告 ToolPending 后立即执行真实工具；真实结果经 Pi 原有 afterToolCall 处理后报告为 ParentOutcome，立即交回 Parent，不等待 ResolvePending。比较使用 Parent 与 operation 的明确归属。Candidate 在独立目录和独立会话管理器中保存共同消息及配对预测工具结果，以 `agent.continue()` 续跑。控制面只在实际 BranchStarted 后接受 Candidate；缓冲输出只在合法 Promote 后进入主对话，未命中输出不发布。Promote 保留胜出会话并取消旧 Parent 下全部其他 Candidate，含尚在创建的候选。

目前转换标准 system/user/assistant/toolResult 消息与字符串、布尔预测；真实结果携带原始 content，匹配与错误结果是否兼容由 Policy 决定。Candidate 的工具执行由 AScout 的 Branch 生命周期统一控制：未 Promote 前等待，Promote 后放行，Discard 时取消；Pi 不按工具名或命令格式筛选。并行消息中，候选自己的调用使用预测，其余调用等待并复用真实结果，全部配对后才请求模型，不复制工具效果；此路径尚未完成专项端到端验证。非标准上下文问题仅记录、暂缓处理。

Candidate 不再由 Pi 判断工具是否安全；所有未验证 Candidate 工具调用统一等待 AScout 的生命周期处置。当前控制面仍使用目录复制作为最低程度的环境方案，不能把它当作完整 Runtime 隔离；Runtime 能力、Policy 决定和 AScout 的正确性约束仍需接入到正式的执行控制路径。

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

候选接入顺序如下：

1. **NetHack + NLE**：NLE 提供 Gym 风格的 `reset()` / `step(action)` 与结构化观察，实际基础游戏是 NetHack 3.6.6。它不是 DCSS 的接口；接入前必须核实本地构建、可见区域限制、动作结果和状态复制／恢复能力。
2. **DCSS + dcss-ai-wrapper**：DCSS 有独立的 AI wrapper，但当前资料显示它依赖本地或 Docker 中的 DCSS Web 服务。若采用，必须单独核实逐步动作、观察、权限和状态分支能力，不能把它与 NLE 混称。
3. **简化二维迷雾游戏**：只有在真实游戏接口不能提供本次验证所需的动作、观察或分支能力时才制作。简化游戏借鉴真实游戏的迷雾、二维地图、逐步揭示和回合机制，但作为 workload adapter 存在，不改变 Runtime 的一般语义。

真实游戏代码和其运行环境属于外部 workload／接入边界，不直接成为 AScout Runtime 核心。Runtime 的一般契约仍然是建立、操作、分支、静止、恢复、终止和清理，并如实声明能力。对于地图游戏，动作暂存、在 Candidate 分支中执行，或 Promote 后提交，都只是具体 Policy、Runtime 能力和 workload 共同选择的实现策略。

首段验收至少需要证明：

- Parent 使用 Agent 原有权限执行真实游戏动作；Candidate 不得扩大这些权限；
- Candidate 的动作同时满足 Agent 权限、Policy 准入和 Runtime 能力，任一不满足都阻止并记录真实原因；
- Candidate 只能使用当时可见的游戏观察和自己的预测，不能读取隐藏地图或伪造观察结果；
- Parent 的真实动作结果作为权威依据，驱动预测比较及 Promote／Discard；
- 游戏状态、动作、观察、Candidate 启动／阻止、比较、处置和清理均可观测；
- 如果某个游戏接口不能提供独立状态或真实结果核对，则不能声称已完成真实投机验证，应转入简化游戏方案。

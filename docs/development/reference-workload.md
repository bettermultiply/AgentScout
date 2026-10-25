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

第三个参数可指定 Policy YAML，例如 `assets/policy/pi-tool.yaml`：该示例无固定种子地从三种文本值抽取两个不同预测。默认 `assets/policy/policy.yaml` 仍禁用候选（`max_edges: 0`）。编译在终端进入 raw mode 前完成，运行期分别求值 admit/select/predict/launch/match/dispose；Policy 错误回退 Parent，不把未知结果当作接受。

接入报告 ToolPending 后立即执行真实工具；真实结果经 Pi 原有 afterToolCall 处理后报告为 ParentOutcome，立即交回 Parent，不等待 ResolvePending。比较使用 Parent 与 operation 的明确归属。Candidate 在独立目录和独立会话管理器中保存共同消息及配对预测工具结果，以 `agent.continue()` 续跑。控制面只在实际 BranchStarted 后接受 Candidate；缓冲输出只在合法 Promote 后进入主对话，未命中输出不发布。Promote 保留胜出会话并取消旧 Parent 下全部其他 Candidate，含尚在创建的候选。

目前转换标准 system/user/assistant/toolResult 消息与文本预测；真实结果携带原始 content，匹配与错误结果是否兼容由 Policy 决定。等待点工具准入在示例 Policy 中，不按同一消息的工具数量整组拒绝。并行消息中，候选自己的调用使用预测，其余调用等待并复用真实结果，全部配对后才请求模型，不复制工具效果；此路径尚未完成专项端到端验证。非标准上下文问题仅记录、暂缓处理。

未验证 Candidate 的 read/ls/find/grep 和严格字面量 `sleep N; printf 'TEXT'` 可提前执行；其他工具等待 Promote。这里仍是接入侧硬编码的安全能力限制，尚未全部接通 Policy/Effect 能力协商，不能声称这项需求已完成。目录复制拒绝符号链接及特殊文件；这不是完整隔离。

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

嵌套、并行工具专项验证、非标准消息、图像/复杂预测转换、多种结果元数据的一致性、Extent/Rebalance 的持续运行管理和完整依赖/效果提交仍未完成。当前不能推广为任意 coding workload 的正确 Promote。Candidate 共同上下文保存在独立内存 SessionManager 中，不提供进程重启后的恢复保证。

完整多行编辑、Markdown 和工具专门排版、宽字符排版、产品特有交互、断线恢复、资源硬限制与确认式退出清理尚未实现。当前是简单终端原型，不宣称已达到原 Agent 全部交互能力。

环境管理暂不接通完整 Runtime Adapter 是已确认的开发顺序；目录不代表任意 shell 或外部效果已隔离，尤其不隔离扩展自身的代码及网络行为。接入进程退出只确认该进程退出，不证明任意工具后代均已停止或环境资源已清理。

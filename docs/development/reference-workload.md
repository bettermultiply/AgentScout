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

第三个参数可指定 Policy YAML，例如 `assets/policy/pi-tool.yaml`：该示例明确启用一个候选，预测文本 `example`。默认 `assets/policy/policy.yaml` 仍禁用候选（`max_edges: 0`）。编译在终端进入 raw mode 前完成，运行期分别求值 admit/select/predict/launch/match/dispose；Policy 错误回退 Parent，不把未知结果当作接受。

接入报告 ToolPending 后立即执行真实工具；真实结果经 Pi 原有 afterToolCall 处理后报告为 ParentOutcome，再等待 ResolvePending。Candidate 在独立目录和独立会话管理器中保存共同消息及配对预测工具结果，以 `agent.continue()` 续跑。未验证 Candidate 可推理、可生成工具调用，但下一次工具执行先等待 Promote；丢弃会中断等待。控制面只在实际 BranchStarted 后接受 Candidate，已完成的 Candidate 也可保留会话处理下一轮用户输入。缓冲输出只在合法 Promote 后顺序进入主对话，拒绝分支只在观察界面可见。

只支持标准 system/user/assistant/toolResult 消息、单工具调用和单一文本结果；拦截范围为内置 read/ls/find/grep，以及严格字面量形式 `sleep N; printf 'TEXT'`（仅简单字符，不含重定向、展开或外部调用）。目录复制拒绝符号链接及特殊文件。该范围是原型安全限制，不是机制目标的固定策略；完整能力仍需 Runtime／Effect 接入。

## 尚未完成

多候选、嵌套、并发工具组、非标准消息、图像/复杂结果、多种结果元数据的一致性、Extent/Rebalance 的持续运行管理和完整依赖/效果提交仍未完成。当前只实现所述受限路径，不能推广为任意 coding workload 的正确 Promote。Candidate 共同上下文保存在独立内存 SessionManager 中，不提供进程重启后的恢复保证。

完整多行编辑、Markdown 和工具专门排版、宽字符排版、产品特有交互、断线恢复、资源硬限制与确认式退出清理尚未实现。当前是简单终端原型，不宣称已达到原 Agent 全部交互能力。

环境管理暂不接通完整 Runtime Adapter 是已确认的开发顺序；目录不代表任意 shell 或外部效果已隔离，尤其不隔离扩展自身的代码及网络行为。接入进程退出只确认该进程退出，不证明任意工具后代均已停止或环境资源已清理。候选工具只在 Promote 后放行，当前不支持投机期间的业务工具执行。

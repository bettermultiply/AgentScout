# 首要验证 Workload

本文记录当前 AScout 第一阶段的开发与验证范围，不定义全部目标语义，也不声称已经完成完整 Runtime 或 coding workload。

## 当前开发骨架

首个真实接入对象是固定 commit `8a2ebd38549c3521a02db0e5d758a24320e97f94` 的 Pi。Rust 控制面启动并管理一个 Pi 接入连接；Pi 的 `agent-access.ts` 只负责创建 `AgentSession`、执行控制面命令、收集模型输出事实和回传 outcome。

当前闭环包含：

- 两个跨进程 Pi AgentSession：Authority 和 Candidate；
- 两次真实 `deepseek/deepseek-flash` 模型请求；
- 控制面请求并由当前接入 Runtime 以两个不同工作目录承载的两个独立执行环境；
- Rust 控制面调用 Policy 的 `admit`、`select`、`launch`、`predict`、`match`、`dispose`；
- Candidate 报告完成事实，不能直接决定生命周期；
- 协议已经区分 `ParentOutcome` 和 `CandidateCompleted`；
- 但当前 `ParentOutcome` 仍是模型文本占位，尚未来自真实 operation outcome；
- 只有 Policy 的 `dispose` 返回 Promote 时，控制面才更新 Tree 的 authority；
- 旧 Authority 的目录由控制面删除，当前 winner 的目录保留。

## 可观察证据

控制面输出的 Tree 应显示：

```text
authority_branch_id = candidate
authority.status = cleaned
candidate.role = authority
candidate.status = promoted
```

控制面 stderr 应显示 Policy 结果：

```json
{"event":"policy","match":{"verdict":"compatible"},"disposition":{"actions":[{"intent":"Promote"}]}}
```

Pi stderr 应显示真实模型请求，例如：

```json
{"event":"model_request_finished","branch_id":"candidate","role":"candidate","model":"deepseek/deepseek-flash","output_characters":7}
```

这些证据只能证明：控制面拥有 Tree、Policy 由控制面执行、Pi 使用真实模型完成了 Branch 请求；不能证明真实 pending-result boundary 或正确的 Parent-outcome 匹配。

## 当前边界

首轮使用最低程度的 Runtime 承载：当前 Adapter 以不同目录实现独立 Candidate 环境，但目录不是控制面抽象本身。候选暂不开放业务工具、写文件或 shell。尚未实现多 Candidate、Reuse、断线恢复、真实 pending-result 拦截、Parent operation outcome 获取、进程确认式清理和真实仓库 coding workload。当前控制面可以自动启动 Pi 接入，但仍是开发验证入口，不是最终用户交互入口。

外部命令只模拟用户启动控制面和 Pi 接入，不拥有 Tree、Policy 或 Branch 生命周期。旧的 Pi-local `SpeculativeTree`、`BranchRecord`、`ControlState`、一次性根目录驱动和旧 Rust 示例均已删除；废弃原因只记录在开发决策中。

## 后续验证顺序

1. 增加控制面与 Pi 的命令确认和断线处理。
2. 在相同协议下验证 Discard 和 Candidate 失败清理。
3. 接入真实 pending-result boundary，而不是当前由控制面安排的最小双会话场景。
4. 再验证多 Candidate、状态交接和真实 coding workload。

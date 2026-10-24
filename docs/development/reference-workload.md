# 首要验证 Workload

本文记录当前 AScout 第一阶段的开发与验证范围，不定义全部目标语义，也不声称已经完成完整 Runtime 或 coding workload。

## 当前最小闭环

首个真实接入对象是固定 commit `8a2ebd38549c3521a02db0e5d758a24320e97f94` 的 Pi。Rust 控制面启动并管理一个 Pi 接入连接；Pi 的 `agent-access.ts` 只负责创建 `AgentSession`、执行控制面命令、收集模型输出事实和回传 outcome。

当前闭环包含：

- 两个跨进程 Pi AgentSession：Authority 和 Candidate；
- 两次真实 `deepseek/deepseek-flash` 模型请求；
- 控制面创建的两个不同工作目录；
- Rust 控制面调用 Policy 的 `admit`、`select`、`launch`、`predict`、`match`、`dispose`；
- Candidate 报告输出事实，不能直接决定生命周期；
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

这些证据分别证明：控制面拥有 Tree、Policy 由控制面执行、Pi 使用真实模型完成了 Branch 请求。

## 当前边界

首轮不经过完整 Runtime Adapter，Candidate 使用控制面创建的独立目录作为最低程度的文件系统边界。候选暂不开放业务工具、写文件或 shell。尚未实现多 Candidate、Reuse、断线恢复、完整 pending-result 拦截、进程确认式清理和真实仓库 coding workload。

外部命令只模拟用户启动控制面和 Pi 接入，不拥有 Tree、Policy 或 Branch 生命周期。旧的 Pi-local `SpeculativeTree`、`BranchRecord`、`ControlState`、一次性根目录驱动和旧 Rust 示例均已删除；废弃原因只记录在开发决策中。

## 后续验证顺序

1. 增加控制面与 Pi 的命令确认和断线处理。
2. 在相同协议下验证 Discard 和 Candidate 失败清理。
3. 接入真实 pending-result boundary，而不是当前由控制面安排的最小双会话场景。
4. 再验证多 Candidate、状态交接和真实 coding workload。

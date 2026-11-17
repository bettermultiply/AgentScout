# 模型输出记录与回放工具

运行使用 Node 26 直接执行 TypeScript，局部 `package.json` 明确采用 ESM，无构建步骤或运行时依赖。独立严格类型检查包含核心与全部验收脚本：

```sh
npm install --prefix tools/replay
npm run check --prefix tools/replay
npm test --prefix tools/replay
```

仅开发检查需要 TypeScript 和 Node 类型包；版本与仓库已有 WebUI 开发依赖一致，可复用已安装的依赖。运行回放本身不需要安装它们。

记录是系统原生能力。通用运行入口从指定已有 run 直接读取 `session.jsonl`、`events.jsonl` 并在内存准备回复；正常新 run 仍按通常方式保存自己的原生记录。不会额外生成 recording 缓存。

```sh
node tools/replay/export.ts RUN_DIR OUTPUT/recording.json
node tools/replay/serve.ts OUTPUT/recording.json 18080
```

通用交互回放由外部工具启动普通 AScout，不需要先导出：

```sh
node tools/replay/run.ts /path/to/source-run ../pi \
  --runtime /path/to/runtime.json --web 127.0.0.1:8080
```

Policy 必须用 `--policy <name>` 从配置目录选择；源 session 的 `policy.json` 仅用于自身恢复。Runtime 配置及初始环境必须由调用方明确提供，不能从目录布局猜测。回放结束或缺键后 WebUI 保持运行，用户可切到正常配置的 `provider/model` 后继续。

终端命令：`pause` 暂停后续自动输入（在飞轮次继续），`resume` 恢复，`status` 查看进度和错误，`quit` 正常保存并退出两部分。手动输入或切换模型前先 `pause`；工具不检测这些操作。完成时静默停止自动输入，失败报具体原因并停止自动输入；两种情况均继续管理 AScout 和本地服务。AScout 主动退出也关闭工具；SIGINT/SIGTERM 走 AScout 正常 Shutdown。不可捕获的强制杀进程不属于正常退出。

本地服务通过通用 `--models` 追加模型配置，保留 Pi 原配置、认证和扩展；重名 provider 拒绝覆盖。control plane 启动使用配置目录中的 Policy 名称；session 的 `policy.json` 仅供该 session 恢复。`GET /api/input-status` 返回已处理输入序号 `sequence`、可输入 `ready`、当前输入错误 `error`；HTTP 202 仅表示排队。工具等待序号推进且权威轮次结束、无待确认 Promote 后才提交下一输入，不要求所有候选结束。

AScout 保存追加 provider 配置内容，普通恢复不依赖工具临时目录。工具退出后可用普通 `--resume <session-id> --model provider/model`：先校验并恢复保存的运行，再在空闲权威上切换模型。回放服务和自动输入队列不恢复。

只依赖 Node 26。导出只读源文件，在内存中按 transcript 原生 ID、Candidate/Chance 关系和 applied Promote 还原预测批次后的第一条父边（也可能是 custom、摘要或编辑记录）。events 按文件序处理，不按恢复后会重置的 `t_ms` 排序。不解析命名、不比较正文相似度、不按时间猜测；缺引用、批次不完整或关系歧义直接报错。只处理当前 Pi 记录格式。

上下文按原生父链投影：保留最新 compaction 摘要及其保留区间，应用 context_edit，转换 custom_message 和 branch_summary；不把 authority custom 状态当模型消息。失败 assistant 不进入请求；恢复后的同一待决工具结果保留最后一条。外部扩展未持久化的上下文改写不能从这些文件恢复。

`normalize.ts` 供导出与 HTTP 端共同使用。键取最近 assistant 之后的工具结果，其工具名、参数（JSON 对象按键排序）、Pi 文本结果组成材料；若同时存在后续 user/custom 内容也纳入。没有工具结果时取最后 user 文本。材料用 SHA-256，原生 ID、时间、Branch 名不进入键。同键按导出项顺序消费；不能据此证明完整历史相同。

端点只接受 `POST /v1/chat/completions`，成功项在输出任何响应前等待一次 500ms，再以 SSE 返回全部内容。取消期间清理定时器，停止写入连接。缺键或耗尽返回 HTTP 400，不回退真实模型。独立 `serve.ts` 可直接接收源 run，也可供低层检查接收显式导出文件；原生 run 模式默认不写报告，显式 `REPLAY_REPORT` 可指定验收报告，`REPLAY_REQUEST_LOG` 可记录请求；通用入口也支持这两个选项。`missing_keys` 统计缺键／耗尽，`unconsumed_keys` 统计剩余项。端口可省略，自动选择并打印 URL。Pi 配置见 `models.json`，dummy key 仅用于本地服务。

消息边界接受 Pi 的纯工具调用格式：assistant 的 `content:null` 必须伴随非空、通过既有字段校验的 `tool_calls`；其他角色的 null、非法正文和非法工具调用字段仍返回 HTTP 400。此兼容不改变匹配键、消费顺序或取消行为。HTTP 回归见 `verify/messages.test.ts`。

独立 `serve.ts` 的文件输入保持只读：文件名恰为 `recording.json` 时默认报告为同目录 `report.json`；其他文件名在完整路径后追加 `.report.json`，例如 `capture.json.report.json`、`report.json.report.json`。显式 `REPLAY_REPORT` 或默认报告若指向源文件本身（包括符号链接、硬链接），启动时拒绝。

导出项按 session 文件序保存成功和明确 `stopReason=aborted` 的 assistant；成功 reply 保留 text/toolCall 和原 toolCall ID，取消项只保留取消状态及匹配信息。其他 error 或带 errorMessage 的失败项跳过，stderr 分别报告数量。成功与取消项均要求 transcript 原生引用，同键按源顺序消费。取消项命中后不返回回答，等待 AScout 取消 HTTP 请求；30 秒仍未取消则返回 HTTP 504 `replay_cancel_timeout` 并通知自动输入失败。`createReplayServer` 的 `cancelTimeoutMs` 可供验证调整有界超时。报告的 `pending`、`cancelled`、`cancel_timeouts` 分别表示等待、客户端取消、取消超时数量；`hits` 和 `consumed` 包括取消项。

## 验收

```sh
node tools/replay/verify/b1.ts
node tools/replay/verify/native.ts
node --test tools/replay/verify/cancellation.test.ts
node --test tools/replay/verify/report-path.test.ts
node tools/replay/verify/b1-pi.ts EXISTING_B1_PI_RUN
node --test tools/replay/verify/messages.test.ts
```

B1 保留手构造低层规范化、顺序消费、缺失/耗尽和报告检查。B1-PI 支持从已有 B1-PI run 开始，省略参数则使用手构造原生记录，验证真实 Pi provider 对并行工具调用及后续回放请求的解析。所有路径均不调用真实模型。

2026-11-14 格式修复验证：类型检查、15 项 Node 回归、B1、native、B1-PI 通过。套件只提供已有响应，机制正确性仍须另行验证；候选数量与嵌套、活跃祖先丢弃、跨组件取消／超时、最终回答交付及长轨迹环境复现仍有覆盖缺口。请求命中／消费不代表回答已交付，也不证明性能收益。范围及证据见 [回放开发说明](../../docs/development/replay.md)。

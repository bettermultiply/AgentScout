# 模型输出记录与回放工具

记录是系统原生能力。通用运行入口从指定已有 run 直接读取 `session.jsonl`、`events.jsonl` 并在内存准备回复；正常新 run 仍按通常方式保存自己的原生记录。不会额外生成 recording 缓存。

```sh
node tools/replay/export.mjs RUN_DIR OUTPUT/recording.json
node tools/replay/serve.mjs OUTPUT/recording.json 18080
```

通用交互回放由外部工具启动普通 AScout，不需要先导出：

```sh
mkdir /tmp/opencode/replay-run
node tools/replay/run.mjs /path/to/source-run /tmp/opencode/replay-run ../pi \
  --runtime /path/to/runtime.json --web 127.0.0.1:8080
```

Policy 首选源 run 的 `policy.json`，否则显式给出原 YAML；Runtime 配置及初始环境也必须来自原配置或显式参数，不能从目录布局猜测。新 run 与源目录必须分开，Runtime 的 control socket 不得写到源目录。回放结束或缺键后 WebUI 保持运行，用户可切到正常配置的 `provider/model` 后继续。

终端命令：`pause` 暂停后续自动输入（在飞轮次继续），`resume` 恢复，`status` 查看进度和错误，`quit` 正常保存并退出两部分。手动输入或切换模型前先 `pause`；工具不检测这些操作。完成时静默停止自动输入，失败报具体原因并停止自动输入；两种情况均继续管理 AScout 和本地服务。AScout 主动退出也关闭工具；SIGINT/SIGTERM 走 AScout 正常 Shutdown。不可捕获的强制杀进程不属于正常退出。

本地服务通过通用 `--models` 追加模型配置，保留 Pi 原配置、认证和扩展；重名 provider 拒绝覆盖。控制面 `--policy-artifact` 可加载已编译策略，仍将策略正常保存到新 run。`GET /api/input-status` 返回已处理输入序号 `sequence`、可输入 `ready`、当前输入错误 `error`；HTTP 202 仅表示排队。工具等待序号推进且权威轮次结束、无待确认 Promote 后才提交下一输入，不要求所有候选结束。

AScout 保存追加 provider 配置内容，普通恢复不依赖工具临时目录。工具退出后可用普通 `--resume --model provider/model`：先校验并恢复保存的运行，再在空闲权威上切换模型。回放服务和自动输入队列不恢复。

只依赖 Node 26。导出只读源文件，在内存中按 transcript 原生 ID、Candidate/Chance 关系和 applied Promote 还原预测批次后的第一条父边（也可能是 custom、摘要或编辑记录）。events 按文件序处理，不按恢复后会重置的 `t_ms` 排序。不解析命名、不比较正文相似度、不按时间猜测；缺引用、批次不完整或关系歧义直接报错。只处理当前 Pi 记录格式。

上下文按原生父链投影：保留最新 compaction 摘要及其保留区间，应用 context_edit，转换 custom_message 和 branch_summary；不把 authority custom 状态当模型消息。失败 assistant 不进入请求；恢复后的同一待决工具结果保留最后一条。外部扩展未持久化的上下文改写不能从这些文件恢复。

`normalize.mjs` 供导出与 HTTP 端共同使用。键取最近 assistant 之后的工具结果，其工具名、参数（JSON 对象按键排序）、Pi 文本结果组成材料；若同时存在后续 user/custom 内容也纳入。没有工具结果时取最后 user 文本。材料用 SHA-256，原生 ID、时间、Branch 名不进入键。同键按导出项顺序消费；不能据此证明完整历史相同。

端点只接受 `POST /v1/chat/completions`，成功项在输出任何响应前等待一次 500ms，再以 SSE 返回全部内容。取消期间清理定时器，停止写入连接。缺键或耗尽返回 HTTP 400，不回退真实模型。独立 `serve.mjs` 可直接接收源 run，也可供低层检查接收显式导出文件；原生 run 模式默认不写报告，显式 `REPLAY_REPORT` 可指定验收报告，`REPLAY_REQUEST_LOG` 可记录请求；通用入口也支持这两个选项。`missing_keys` 统计缺键／耗尽，`unconsumed_keys` 统计剩余项。端口可省略，自动选择并打印 URL。Pi 配置见 `models.json`，dummy key 仅用于本地服务。

导出项按 session 文件序保存成功和明确 `stopReason=aborted` 的 assistant；成功 reply 保留 text/toolCall 和原 toolCall ID，取消项只保留取消状态及匹配信息。其他 error 或带 errorMessage 的失败项跳过，stderr 分别报告数量。成功与取消项均要求 transcript 原生引用，同键按源顺序消费。取消项命中后不返回回答，等待 AScout 取消 HTTP 请求；30 秒仍未取消则返回 HTTP 504 `replay_cancel_timeout` 并通知自动输入失败。`createReplayServer` 的 `cancelTimeoutMs` 可供验证调整有界超时。报告的 `pending`、`cancelled`、`cancel_timeouts` 分别表示等待、客户端取消、取消超时数量；`hits` 和 `consumed` 包括取消项。

## 验收

```sh
node tools/replay/verify/b1.mjs
node tools/replay/verify/native.mjs
node --test tools/replay/verify/cancellation.test.mjs
node tools/replay/verify/b1-pi.mjs EXISTING_B1_PI_RUN
node tools/replay/verify/b3.mjs EXISTING_B3_RUN
node tools/replay/verify/b3-repeat.mjs EXISTING_B3_RUN
DEEP_ROUNDS=4 node tools/replay/verify/b3-deep.mjs EXISTING_DEEP_RUN
```

B1 保留手构造低层规范化、顺序消费、缺失/耗尽和报告检查。B1-PI 支持从已有 B1-PI run 开始，省略参数则使用手构造原生记录，验证真实 Pi provider 对并行工具调用及后续回放请求的解析。所有路径均不调用真实模型。

B3、重复和深度入口必须提供已有原生 run，不会创建录制运行。已有实验布局要求 run 上一级包含该次运行的 `runtime.json`、`b3-policy.yaml`、`decisions.py`；使用其配置与采样规则，不重新选择预测。输出写全新 `/tmp/opencode/ascout-replay-b3-repeat-*`，不覆盖源 run；B3 回放一次，repeat 五次，deep 三次，深度支持 3/4/10。这些是既有 NLE 场景验收入口，不是任意任务恢复器。

逐次严格核对 Chance 父关系、每 Chance 两 Proposal/Candidate、单 Promote、两个 applied Discard、完整权威 role/text、NLE 观察和报告的 hits/consumed/misses/exhausted/unconsumed。工具仍真实执行，NLE 环境或取消时序改变可使回放失败，不得弱化检查以得到通过。

本轮实测：B1、B1-PI 通过；已有并行恢复记录导出成功，四层记录导出 13 项。真实 NLE 的 B3 和四层整链回放尝试均因工具内容变化后的缺键失败，没有完成严格整链验收。精确证据见 [回放开发说明](../../docs/development/replay.md)。

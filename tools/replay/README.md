# 模型输出记录与回放工具

这些脚本只依赖 Node 26，且不接触真实模型。`normalize.mjs` 是导出端和 HTTP 端共享的规范化实现：工具结果键取最近 assistant 后连续的 tool 消息，工具名、参数（JSON 对象按键排序）和 Pi 风格文本结果组成材料；没有工具结果时取最后一条 user 文本。材料使用 SHA-256。id、时间、Branch 名都不进入键。

```sh
node tools/replay/export.mjs RUN_DIR RUN_DIR/recording.json
node tools/replay/serve.mjs RUN_DIR/recording.json 18080
```

端点只接受 `POST /v1/chat/completions`，强制以 SSE 流式返回。键缺失或同键录制耗尽返回 HTTP 400，并在退出时写 `report.json`。端口不传则自动选择并打印 URL。Pi 的隔离配置可复制 `models.json`，设置 `PI_CODING_AGENT_DIR` 指向该目录；其中 key 是 dummy，不是真实凭据。

`report.json` 的 `missing_keys` 只统计请求到但录制中不存在或已耗尽的键及请求次数；`unconsumed_keys` 统计录制中尚未消费的键及剩余条数。

录制项按 session 文件顺序保存可回放的 assistant；`stopReason=aborted` 或 `stopReason=error`，以及带 `errorMessage` 的 assistant 会跳过，不生成 recording 项，并在 stderr 按类别和总数报告。其余 reply 保留 text/toolCall 块和原 toolCall id。`events.jsonl` 的 transcript 记录用于反查 branch；反查不到时输出 `null` 并告警。

## 验收脚本

以下脚本均使用伪造模型或录制端点，不调用真实模型；脚本结束会等待并清理子进程，再检查相关端口。耗时是本机实测的预计量，首次 `b3` 还包含一次 debug 构建。

```sh
node tools/replay/verify/b1.mjs       # 约 1 秒：规范化、顺序消费、缺失/耗尽、报告和旁支导出
node tools/replay/verify/b1-pi.mjs    # 约 3 秒：真实 Pi provider 解析并行工具调用、回放命中和报告统计
node tools/replay/verify/b3.mjs       # 约 30 秒：NLE 单 Chance 双 Candidate、Promote/Discard、导出和一次回放闭环
node tools/replay/verify/b3-repeat.mjs # 约 2 分钟：同一 recording 的五个全新 run 严格重复回放
DEEP_ROUNDS=3 timeout 600 node tools/replay/verify/b3-deep.mjs   # 三层嵌套 Chance，录制与三次严格回放
DEEP_ROUNDS=4 timeout 600 node tools/replay/verify/b3-deep.mjs   # 四层嵌套 Chance，录制与三次严格回放
```

`b3.mjs` 的录制运行和回放运行分别写入 `/tmp/ascout-replay/b3/run` 与
`/tmp/ascout-replay/b3/run-replay`；`b3-repeat.mjs` 和 `b3-deep.mjs` 使用各自的
`/tmp/opencode` 实验目录，重复回放不会重新录制。它们严格核对 `chance_created` 的 `parent_branch_id`/`parent_chance_id`、每 Chance 两个 Proposal/Candidate、一个 Promote、两个 applied Discard、权威 session 父链、NLE 观察和回放报告。导出修复后录制项为 N=3 的 10 条、N=4 的 13 条（旧记录曾错误显示 14），三次回放均为对应的 `hits/consumed=10`（N=3）或 `13`（N=4）、`misses=0`、`exhausted=0`、`unconsumed=0`。N=3 的父链为 authority → candidate-2/chance-1 → candidate-4/chance-2；N=4 再加 candidate-6/chance-3。后续 Chance 都在祖先 Candidate 已 Promote 后产生，本例没有“祖先 Candidate 被 Discard 后产生其嵌套 Chance”的情况。

# OpenTelemetry 追踪

AScout 的 control plane、Pi Adapter 和 Runtime Adapter 通过 OTLP/HTTP protobuf 发送追踪。没有配置接收端时默认关闭。设置 `OTEL_SDK_DISABLED=true` 可显式关闭三个进程中的追踪。

## 接收与查看

本地可用 Jaeger 验证。以下端口只绑定到回环地址。

```sh
docker run --rm --name ascout-jaeger \
  -p 127.0.0.1:4318:4318 -p 127.0.0.1:16686:16686 \
  jaegertracing/all-in-one:1.76.0
```

在启动 AScout 的终端设置 `OTEL_EXPORTER_OTLP_ENDPOINT=http://127.0.0.1:4318`，SDK 会使用 `/v1/traces`。也可设置 `OTEL_EXPORTER_OTLP_TRACES_ENDPOINT` 为完整的追踪接收 URL。生产环境可将该地址指向 Collector 的 HTTP 接收器，使用 HTTPS 与 `OTEL_EXPORTER_OTLP_HEADERS` 配置认证。单独部署的 Runtime Adapter 需要同样的配置。生产环境按需设置接收端的访问权限和保留期限。

打开 `http://127.0.0.1:16686`，选择 `ascout-control` 服务和 `ascout.task` 操作。已有的 `task_started` Ledger 记录在启用时会带 `trace_id`，可以用 Jaeger 的 `/trace/<trace_id>` 或 `/api/traces/<trace_id>` 直接查询。开发时直接用 Cargo 构建需另运行 `make pi-deps` 安装 Pi 的 npm 依赖。`make` 和 `make install` 会自动安装。

## 读图

- `ascout.task` 从 Prompt 到首次 `task_completed`，包含控制流程和 Pi 工作。`ascout.branch.authority` 和 `ascout.branch.candidate` 显示权威执行与候选执行区间。`local=true` 标明 Pi 本地先启动的候选。
- `ascout.policy.evaluate` 显示 Decision 名称及允许公开的结果类别。`decision=dispose` 的 `result=Promote` 表示 Policy 提议。`ascout.agent.command` 中的 `command=promote`、`result=applied` 表示 Pi 应用了命令。候选区间的 `result=promoted` 标记完整接替发生的时间。
- `ascout.model` 和 `ascout.tool` 复用 Agent 已报告的执行时间。从模型 span 汇总 input、output、cache 和 total token 及 cost。`ascout.policy.provider` 单独记录 Policy 的模型请求，Pi 侧使用同一 SDK 的用量。外部 Rust Policy provider 进程收到 `TRACEPARENT` 和可选的 `TRACESTATE` 环境变量。
- `ascout.runtime.operation` 是控制面的 Runtime 请求。`ascout.runtime.tool_rpc` 与 `ascout.runtime.tool_execute` 分别显示 Pi 工具请求和 Runtime 实际执行。W3C Trace Context 经过内部连接和工作线程传递。
- 暂停恢复目录可能带可选的 `trace-context.json`。找到该字段时，`--resume` 在原 trace 中创建 `ascout.task.resume` 并重新打开执行区间。旧目录在恢复时创建一个新 trace。强制终止进程可能留下仍被子 span 引用、尚未导出的父 span。

追踪只写入配置的接收端。属性只包含明确允许的类别、状态、模型或工具名称、时间及用量。Prompt、工具参数和输出、Policy 输入与原始返回值、路径、认证信息和错误详情留在各自原有边界。Ledger 与 SSE 继续提供恢复和用户可见记录。

每个进程最多排队 1024 个已结束 span，每批至多 128 个。HTTP 请求有 750 ms 超时，单个 SDK 关闭最多等待约 1.5 s。接收端不可用时执行继续，进程关闭时在 stderr 或 `agent.log`、`adapter.log` 汇总失败与丢弃或仍未送达的 span。统计按 SDK 批次结果计数。

## 本地端到端验证

启动 Jaeger 后从仓库根目录运行。脚本仅调用本机的模拟模型，并查询 Jaeger API 验证父子关系和用量总计。

```sh
make frontend pi-deps
cargo build --bins
node scripts/otel-e2e.mjs
```

非默认端口可通过 `ASCOUT_E2E_OTLP` 和 `ASCOUT_E2E_JAEGER` 指定。`PI_ROOT` 默认是 `../pi`。结果逐行输出场景、运行目录及可查询的 trace ID。覆盖无投机、确定性投机、接收端停止与卡住、禁用及无地址、SSE 重放和实际暂停恢复。运行依赖与现有 Filesystem Runtime 端到端场景相同。

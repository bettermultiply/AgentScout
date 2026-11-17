# Agent 接入状态

AScout 当前有一个可构建、可运行的原型：Rust control plane 持有 Tree/Branch 状态，通过 Unix socket 管理 Pi Agent 接入；Runtime Adapter 负责启动 Agent 接入进程，并已提供文件系统 Backend（overlay/btrfs 两种数据面）与进程 Backend。稳定的 Agent-facing 命令语法、C API、完整 Runtime Registry 与能力验证仍未实现。

## 当前可用内容

- `docs/design/` 中已经确认的目标语义；
- 已确认的 Instrumentation、control plane 与 Runtime 责任边界，以及 `docs/development/` 中的开发决策；
- 可构建的 control plane、Pi 接入、Runtime Adapter，以及文件系统/进程 Backend 原型；
- Pi/NLE 的真实投机执行验证；当前验证边界见 [首要验证 Workload](../development/reference-workload.md)。

当前仓库包含可构建代码（`cargo build`、`cargo run --bin ascout-control-plane`）。稳定的对外 Rust API、C API、通用 Agent-facing 命令语法、完整 ASC-Proxy 部署和 Runtime Registry 能力验证尚不存在。

## 计划中的 Agent Surface

Routine Agent-facing control 计划经 Instrumentation 的 ASC-Proxy 进入 Gateway。当前 ASC-Proxy 是 Pi 接入进程内对象，通过 Unix socket 连接控制 Gateway，并通过独立 Unix JSON-lines 连接直接请求 Runtime 执行；不读取 backend 私有工具计划，也不让 control plane 代执行工具。现有 Agent socket、UI HTTP/SSE 与 Runtime ControlSession 通信代码已归入 Gateway；这不代表下列计划接口或通用权限能力已完成。调用方直接传递 Branch 身份，执行端点核对 Branch→Environment 绑定，不补身份。计划覆盖以下操作：

- 查询当前 Branch context；
- 查询 direct children；
- 创建 candidates；
- commit、accept 和 discard；
- 等待 events。

Outcome 由 Interposer 自动发布，不作为独立 Agent command。Resolver delegation 属于 launch 或创建配置，不作为 routine command。参数、返回值、阻塞行为、event 表示和具体命令拼写均未定义。

## 计划中的 Launch 与 Integration

当前支持具备足够拦截点或 Hook 的特定 Agent product。产品相关的内部执行结构、拦截点和 Hook 归 Instrumentation；routine control 计划先进入 ASC-Proxy，再由 Gateway 转交 control plane。当前不承诺完全无修改且无需了解产品内部实现的通用接入。control plane 通过 Runtime Adapter 的 typed contract 接入；原型已实现基础连接与 handshake，Adapter discovery、Registry 验证和稳定对外接口仍未完整实现；Registry 的目标职责与独立验证要求见 [Runtime 设计](../design/runtime.md)及其契约。

launcher、CLI、library、Command + Skill 和 embedded Agent adapter 暂不属于当前范围，后续根据具体产品的可观察和可控制能力单独评估。

支持的 Agent product 必须提供关键执行活动的观察点，以及在真实 operation 开始前暂停或阻止执行的拦截点。Instrumentation 还需要能够识别 operation 的开始、`pending-result boundary` 和完成，并获取 outcome；它负责后续 Decision、control plane 协作以及让调用沿产品原有完成路径结束。

不依赖具体产品的组件责任与控制路径见 [Instrumentation 设计](../design/agent-integration.md)。

## 可用性规则

不得从目标语义或 roadmap item 推断当前支持。只有当前实现及其验证证据存在后，使用文档才会加入对应构建、运行和 API 说明。

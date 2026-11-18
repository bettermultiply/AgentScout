# AScout

AScout（Agent Scout）是面向 Agent 投机执行的通用底座，位于投机策略与执行机制之间：上层只需设计和组合投机执行策略，不必关心具体实现；下层 Runtime 提供投机执行所需的基本能力，而不与某个特定投机算法耦合。AScout 对外抽象为一个 AScoutAgent；每个 AScoutAgent 在 control plane 拥有一棵 Speculative Tree。在 pending-result boundary，parent 可以继续真实 operation，candidate Branches 则基于不同 Proposal 提前执行。Candidate 提交 commit request，持有 resolver capability 的主体至多接受一个 winner；control plane 通过 runtime proxy 触发 Runtime 的节点级转换和 loser cleanup。

## 仓库状态

仓库现在有一个按 control plane ownership 实现的 Agent 管理原型：Rust control plane 拥有 Tree/Branch 状态，通过 Unix socket 管理 Pi Agent 接入；Agent 与 Instrumentation 由 agent supervisor 在可信宿主启动，control plane 只决定用哪个产品并把路径交给它。Runtime 侧已提供文件系统 Backend（overlay 与 btrfs 两种数据面），由其管理环境内的 namespace holder（bubblewrap、cgroup 冻结）；实现范围与已知偏差见 [文件系统 Backend 当前实现状态](docs/fs-backend/current-state.md)。Pi 是首个接入实现，不是 AScout 的架构组成；NLE 是当前验证 workload，不属于 AScout 实现。二者都不应把产品路径、工具或专用语义写进核心契约。它还不是正确完成的投机验证闭环、完整 Runtime、通用 Agent 产品或 coding workload。

目标语义与一级架构已经形成。AScout 按 `Instrumentation -> control plane -> runtime proxy -> Runtime` 组织控制生命周期。工具执行使用独立连接：Pi Product Integration 在已有宿主内独立适配 read/edit/write/bash 参数、schema 与结果格式，一次工具调用对应一次完整 Runtime 操作，文件编辑在目标环境内完成；Runtime 校验 Environment 与请求身份后交给文件系统 holder 或 Firecracker guest agent 原生执行，不加载 Pi SDK，也不逐工具启动 Node 或创建环境。NLE 声明工具共用接收与取消生命周期，游戏语义保留在 backend。control plane 只管理生命周期／Policy 和转交执行访问描述，不代执行工具。runtime proxy 对 control plane 提供稳定 typed contract，并负责目标 Runtime 的能力映射与协议适配。设计文档是语义权威；开发文档记录已接受的实现选择与剩余任务。

## 已确认的实现边界

### 运行 AScout TUI 或浏览器 UI

目标系统通过 Gateway 统一接入 Agent、UI 与 Runtime 的通信，将请求交给相应控制逻辑；实际启动、监视和回收 Agent 及接入进程由 Runtime 负责。Gateway 属于 control plane，不独立部署，由 control plane 统一启动和停止；UI 同样不独立部署，由 control plane 启动和停止。Gateway 与 Runtime 采用网络通信；这些边界不规定必须同进程，也不要求自动打开浏览器，见 [control plane §2.1](docs/design/control-plane.md#21-gateway)。三侧通信入口的源码职责已归入 Gateway，实现现状见 [Gateway 实现核查](docs/development/gateway.md)。当前原型使用 Pi 作为首个接入实现；Pi 接入只运行 AgentSession、收集 Agent 行为事实和执行控制命令，不是用户入口。

先在仓库根目录构建（需要 Make、Rust 与 Node 22.12 或更新版本）：

```sh
make
```

`make` 先准备前端依赖并构建 WebUI，再执行 `cargo build`，`make -j` 同样保证此顺序；任一步失败即停止。前端产物在本地生成并嵌入 Rust 二进制，不入库，具体规则见 [control plane WebUI](docs/development/web-ui.md#生产构建)。

配置文件为 `~/.config/ascout/config.toml`。可在仓库根目录执行以下命令安装[配置样例](config.example.toml)（已有配置时请手动合并）：

```sh
mkdir -p ~/.config/ascout
cp -n config.example.toml ~/.config/ascout/config.toml
```

配置文件不存在时仅使用内置默认值，不会自动安装 Policy。样例与内置默认值一致，仅使用以下键：

```toml
policy_dir = "~/.local/share/ascout/policies"
runs_dir = "~/.local/share/ascout/runs"
default_policy = "policy"
log_path = "~/.local/share/ascout/ascout.log"
```

`log_path` 指定 control plane 进程持有的系统运行日志。格式固定为 UTF-8 逐行 UTC 时间、级别、组件、可选 run 和说明，说明中的换行转为空格；只可改路径，不能配置格式或归属。启动时无法打开日志即拒绝启动；不做日志轮转。界面经 Gateway → UI Projection → Ledger 读取已保存记录，系统报警由 UI Projection 独立转发，不写入 Ledger。

仓库示例请将 `policy_dir` 设置为本仓库 `assets/policy` 的绝对路径；不要把开发策略复制到用户目录。每份 Policy 是含 `policy.yaml` 和 `decisions.py` 的子目录。交互启动会列出并询问 Policy，允许改选；非交互启动使用 `--policy <name>` 或默认 Policy，不等待输入。

从真实终端启动（系统自动创建运行记录目录）：

```sh
ASCOUT_RUNTIME_CONFIG=/path/to/runtime.json \
cargo run --bin ascout-control-plane -- ../pi
```

如果更方便使用浏览器页面，可让 control plane 绑定本机地址：

```sh
ASCOUT_RUNTIME_CONFIG=/path/to/runtime.json \
cargo run --bin ascout-control-plane -- ../pi --web 127.0.0.1:8080
```

浏览器 UI 只绑定 loopback；用本机浏览器打开本机 control plane 输出的地址即可。左侧是权威窗口对话，右侧是可隐藏的投机面板（投机树与分支详情）；候选内容始终只读。TUI 与浏览器 UI 不共享绘制代码，只通过 control plane 提供的 AScoutRunView 和操作请求接口工作。界面的开发代理、构建与嵌入方式见 [control plane WebUI](docs/development/web-ui.md)。

外部回放工具从已有 run 的原生 `session.jsonl`、`events.jsonl` 在内存提供模型回复，并启动普通 AScout 执行 Runtime 与 Policy：

```sh
node tools/replay/run.ts /path/to/source-run ../pi \
  --runtime /path/to/runtime.json \
  --web 127.0.0.1:8080
```

源 `policy.json` 优先；缺失时用 `--policy <name>` 选择原 Policy。用 `--runtime` 明确提供新运行环境。源目录只读；普通 AScout 负责新 run 的记录和恢复。工具终端输入 `pause` 暂停自动输入后，可在 WebUI 手动输入或切换模型；`resume` 继续，`status` 查看消费情况，`quit` 协调正常保存退出。完成或缺键仅停止自动输入，保留系统。详见[回放工具](tools/replay/README.md)及[开发边界](docs/development/replay.md)。

唯一位置参数是 Pi 源码路径。运行记录目录由系统在配置的运行根目录下创建，结束时输出 run id。`ascout-control-plane runs` 列出已有运行。`ascout-control-plane ../pi --resume <run-id>` 按原 id 打开运行：已保存暂停的 task 等待 `/continue`，已结束的 Agent 会话显示原权威对话并等待下一条 prompt 开始新 task。旧 `session-*` 目录名仍是有效的 run id，无需迁移目录或提供 Agent 的 session 路径。Agent 原生 session id 保持独立。agent supervisor 在 Branch 环境外启动 Agent 与 Instrumentation，control plane 只选择产品并传入路径与版本；模型可用 `--model <provider>/<model>` 指定，认证仍用 Pi 自身配置，不写死 provider。Pi 接入通过公开 SDK 报告消息和执行事实，AScout 自己排版，不使用或修改 Pi 主界面。TUI 中输入 `/ascout` 查看当前 Tree，再输入编号查看 Branch；输入 `/events` 查看事件记录；再次输入对应命令或按 Esc 返回主对话。浏览器 UI 提供权威窗口对话与只读的投机树、分支详情。Ctrl-C 请求中断，Ctrl-D 退出，TUI 中 ↑/↓ 滚动。运行记录目录及 `agent.log` 保留用于检查。

已接通工具与模型执行前等待点：Candidate 在独立 session 使用预测工具结果或替代模型续跑。eager 并行放行权威调用；speculative-first 保持原调用等待，由 Policy 依据真实结果或明确的可接受性规则裁决。接受时 Promote 候选 session 并取消旧 Parent，不向旧调用注入候选结果；全部候选不可接受时原调用放行一次。用户仍使用同一个主对话。默认 Policy 禁用候选；可显式启用字符串枚举预测示例：

```sh
ASCOUT_RUNTIME_CONFIG=/path/to/runtime.json \
cargo run --bin ascout-control-plane -- ../pi --policy pi-tool
```

该示例 Policy 从 `example`、`true`、`false` 中无固定种子地抽取两个不同预测。具体随机场景、脚本模型接口及启动方式见 [验证 Workload](docs/development/reference-workload.md)。`/ascout` 查看预测、实际结果、处置和清理，`/events` 查看原始事件；运行记录目录的 `events.jsonl` 保留原始事实。已有脚本模型接口加真实 Pi/bash 的验证，不冒充真实大模型验证，不宣称预测质量或加速收益。

runtime proxy 可以在握手中声明环境工具及其参数结构。control plane 按声明调用 `tool_context` 查询预测上下文，并将 Runtime 执行访问信息交给 Instrumentation；工具由 Instrumentation 经独立执行连接请求 Runtime 执行，control plane 不代执行。Pi 接入从声明动态注册工具，NLE 的名称和动作编号不进入通用接入层。

Parent 已放行并收到真实结果后立即继续，验证并行进行；仅 Promote 时停止旧 Parent。已接入同一等待点的两个 Candidate，至多一个胜出，其余被取消。工具等待点逐项交给 Policy；并行消息续跑需补齐其他工具的真实结果，不重跑这些工具。工具预测支持普通文本、布尔值等 JSON 值的文本形式，以及 Pi 的原生文本／图像内容、显式结果状态和适用的消息详情。工具专有元数据的通用映射、非标准上下文、嵌套、完整效果隔离、backend 能力的独立验证、多行编辑及确认式退出清理仍未完成。详见 [当前实现与验证边界](docs/development/reference-workload.md)。

Policy YAML 编译与受限 Python 求值已可独立运行。本地策略按 `assets/policy/<策略目录>/` 独立保存声明和算法，目前均不随 Git 交付；以下命令及相关测试需要先准备这些本地文件，布局见 [Policy Engine 原型](docs/development/policy-engine.md)。默认 `policy/policy.yaml` 禁用候选：

```sh
cargo run --example policy -- compile assets/policy/policy/policy.yaml /tmp/opencode/policy.json
printf '%s\n' '{"process_state":{"steps_completed":5,"hook_count":5,"phase":"reasoning"}}' | cargo run --example policy -- evaluate /tmp/opencode/policy.json extent -
```

需要 Linux、Python 3、bubblewrap 和 libseccomp。当前接口、支持范围及未完成能力见 [Policy Engine 原型](docs/development/policy-engine.md)；这条路径不等同于完整 Branch 生命周期接入。

- 新实现使用 Rust。具体 crate、模块、进程和部署边界按 Instrumentation、control plane 与 Runtime 的责任和所有权决定（D-024-I）。
- 扩展点的职责语义已确认：Execution、Workspace、Input、Effect 和 Outcome 属于 Runtime；Interposer 属于 Instrumentation，统一负责识别 pending-result boundary、保证真实 operation 只执行一次并自动发布 outcome。
- 默认 Runtime 面向可信 Child Agent，优先高效 sibling isolation；对抗恶意 workload 的 containment 由另行选择的 Runtime 提供。
- Runtime 承载 Agent 动作，并落实声明的环境、资源和效果边界，以及停止和 cleanup；Instrumentation 负责 Session、Agent 执行实体的本地上下文、Branch context、operation 归属和 capability 路由。control plane 管理全局 Tree 与裁决，Runtime 内部的效果协调机制落实外部效果处理。目标语义见 [Runtime 设计](docs/design/runtime.md)，当前文件系统 Backend 与环境进程的接入现状见 [Runtime 开发说明](docs/development/runtime-adapter.md)，实现细节见 [文件系统 Backend 当前实现状态](docs/fs-backend/current-state.md)。

## 文档

### 记录回放验收

回放工具和验收脚本位于 [`tools/replay/`](tools/replay/README.md)。它们只使用伪造模型或已经录制的模型输出，不调用真实模型：

```sh
node tools/replay/verify/b1.ts          # 约 1 秒：导出、规范化、顺序消费和缺失/耗尽报告
node tools/replay/verify/b1-pi.ts       # 约 3 秒：通过现有 Pi provider 验证并行调用与续跑
```

B1 验证记录键规范化及端点的顺序消费；B1-PI 还验证 Pi 对两个并行工具调用的解析和后续请求。套件从已有运行提供响应，机制正确性仍需单独验证；候选数量、嵌套与活跃祖先丢弃、跨组件取消／超时仍有端到端覆盖缺口，见[回放开发说明](docs/development/replay.md)。回放不证明性能收益。

文档以心智模型为唯一根文档，再延伸到各语义权威与实现细节（详见 [文档导航](docs/index.md)）。

- [心智模型](docs/mental-model.md)
- [问题与目标](docs/problems-and-goals.md)
- [设计模型](docs/design-model.md)
- [一级架构](docs/architecture.md)
- [Branch 语义](docs/design/branch-semantics.md)
- [设计决策](docs/design/decisions.md)
- [投机执行底座设计决策](docs/design/speculation-decisions.md)
- [开发决策](docs/development/decisions.md)
- [首要验证 Workload](docs/development/reference-workload.md)
- [Agent 接入状态](docs/usage/agent-guide.md)
- [试一次 AScout 投机执行（NLE）](docs/usage/try-speculation.md)
- [FAQ](docs/usage/faq.md)
- [接入独立可执行的 backend](docs/development/executable-backend.md)
- [投机执行底座抽象设计札记](docs/development/spec-exec-abstraction.md)
- [讨论记录](Discussion.md)

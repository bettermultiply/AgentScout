# Policy Engine 原型

本文记录当前代码，不替代 [Policy Engine 目标语义](../design/control-plane/policy-engine.md)。

## 1. 已实现的路径

`src/policy.rs` 导出的 `PolicyEngine` 可以读取 YAML，检查九类 Decision 的十个函数入口（`admit` 有两个），固定声明的文件内容和各函数的参数，在隔离环境中检查 Python 语法、导入及函数签名，生成 `CompiledPolicy`。编译不调用 Decision 函数体，但会在隔离环境中执行模块顶层代码。

`CompiledPolicy` 可用 serde JSON 序列化。`script(decision)` 返回内含固定源码、该函数参数和执行入口的 Python 脚本。内容摘要覆盖源码、参数、入口、解释器标识和生成脚本使用的 runner；摘要不是来源认证，跨侧接收方仍须验证来源。当前接收方需要同版本的 AScout runner 和相同的 Python `sys.version`。这不是对标准库全部文件或动态库内容一致性的证明。

`PolicyEngine::evaluate` 对已编译结果求值，不再读取作者 YAML 或原始源码。每次调用创建新进程，返回检查后的 JSON 值；Engine 不创建 Branch、不做效果操作，也不应用返回的选择。

```rust
use std::path::Path;
use ascout::policy::{Decision, PolicyEngine};
use serde_json::json;

let engine = PolicyEngine::default();
let policy = engine.compile(Path::new("assets/policy/policy.yaml"))?;
let result = engine.evaluate(&policy, Decision::Extent, &json!({
    "process_state": {"steps_completed": 5, "phase": "reasoning"}
}))?;
assert_eq!(result, "stop");
# Ok::<(), std::io::Error>(())
```

可运行示例（输出目录请自行选择）：

```sh
cargo run --example policy -- compile assets/policy/policy.yaml /tmp/opencode/policy.json
cargo run --example policy -- evaluate /tmp/opencode/policy.json extent assets/policy/extent-input.json
```

`evaluate` 的输入文件参数可以写成 `-`，此时从标准输入读取一份不超过 1 MiB 的 JSON；供 Pi 等接入进程直接传递本次 Decision 数据，不必先落临时文件。

第一条命令输出内容摘要并写入编译产物；第二条命令应输出 `"stop"`。将 YAML 中 `max_steps` 改成 `6` 后重新编译，同一输入会返回 `"continue"`。仅修改原始源码、没有重新编译，不会影响已生成产物。

## 2. 首版 YAML 与调用约定

完整示例见 `assets/policy/policy.yaml` 和 `assets/policy/decisions.py`。顶层必需字段是 `version: 1`、`language: python`、`files`、`fidelity: Best-effort`、`decisions`。可选顶层字段 `runtime` 指定该 Policy 使用的 Runtime 配置路径，路径按控制面当前工作目录解析；控制面优先使用 `ASCOUT_RUNTIME_CONFIG` 覆盖。示例默认 `select` 的 `max_edges: 0`，即不选择任何待选 Edge，保持普通执行；改成 `1` 后可允许一个候选进入投机路径。

每个函数绑定使用 `file`、`function`，可选 `policy_config` 和 `inputs`：

```yaml
extent:
  file: decisions.py
  function: extent
  policy_config:
    max_steps: 5
  inputs: [process_state.active_time]
```

```python
def extent(data, *, policy_config, capabilities):
    return "stop" if data["process_state"]["steps_completed"] >= policy_config["max_steps"] else "continue"
```

- 参数只传给对应函数；省略 `policy_config` 时传空映射。不支持全局参数、继承或覆盖。
- `inputs` 选择该类允许的可选输入；声明后调用方必须提供，未声明的数据被拒绝。
- `capabilities` 当前传 `None`，没有外部计算权限。声明 `capabilities`、`requirements`、`outputs` 等尚未实现的字段会报错，不会被忽略。
- 九类行为都必须显式绑定；不补系统默认函数。入口必须是模块顶层、未装饰的普通同步函数，能按上述形式调用；生成器和异步函数被拒绝。
- YAML 拒绝重复键（包括参数内部）、锚点、别名、tag、指令、merge key、多文档、非字符串键及非有限数值。只按 YAML 1.2 规则识别布尔值，不将 `yes`／`on` 当成布尔值。
- YAML 和全部声明源码分别限 1 MiB，文件最多 128 个，数据嵌套最多 32 层，整数限有符号 64 位。文件采用 UTF-8 文本；二进制资源尚不支持。
- 文件路径是 YAML 目录内规范化的相对路径，不接受绝对路径、`..`、`.`、空路径段、反斜线或 NUL。Python 模块路径必须是 ASCII 标识符，各级包的 `__init__.py` 必须显式列出。不允许覆盖标准库模块。声明文件逐级以目录 fd 和 `O_NOFOLLOW` 打开，拒绝符号链接和非普通文件；YAML 文件本身也不接受符号链接。传入的 YAML 上级目录属于调用方选择的根，不要求其全部祖先目录都不是符号链接。
- 本地 Python 模块按正常 import 和相对 import 规则加载。声明的文本资源可经包的 `pkgutil.get_data` 读取；不支持直接打开 `/policy/...` 或 `importlib.resources`。未声明文件不会回到作者工作区寻找。

## 3. 输入与输出范围

调用方负责准备已经授权、符合任务领域 schema 的事实快照，不能直接传控制面内部对象或未筛选的上下文。原型检查顶层字段、JSON 类型、必要进度事实、机会事实和选择成员资格等，不具备完整的领域 schema 注册与校验；这部分接入责任尚未自动化，不能声称任意嵌套 JSON 都是已授权的事实。

| 入口 | 必需顶层字段 | 可声明的可选字段 | 当前接受的输出 |
|---|---|---|---|
| `admit.opportunity_pass` | `op` | 无 | 布尔值 |
| `admit.feasibility_pass` | `op`、`state`、`contract_context` | 无 | 布尔值 |
| `select` | `frontier_view`、`scheduling_context` | 无 | 输入 Edge ID 的无重复有序子集 |
| `rebalance` | `active_view`、`paused_view`、`scheduling_context` | 无 | 输入 Branch 的 `pause`／`resume`／`preempt` 动作，每 Branch 最多一次 |
| `extent` | `process_state` | `process_state.active_time`、`process_state.resource_counters`、`process_state.result_facts`、`process_state.semantic_facts` | `stop`／`continue` |
| `predict` | `edge_context`、`process_context`、`assumption_set`、`capability_context` | `history` | batch `ProposalSet`，每项有 `payload`，可有 `provenance` |
| `launch` | `edge_context`、`authority_state`、`candidate_view` | `scheduling_context` | 布尔值 |
| `effect` | `effect_descriptor`、`context` | 无 | 只接受无附加参数的 `block`／`deny` 计划 |
| `match` | `proposal`、`evidence`、`match_context` | `outcome` | 四类 verdict，不接受 score |
| `dispose` | `branch`、`resolution`、`context` | 无 | 单个 `Discard`，或仅在 `accepted` 时单个 `Promote`；在 `indeterminate`／`error`／`extent_reached` 时可返回空动作列表 |

`op` 包含 `kind`、`produces_result`、`result_pending`、`has_predictor`；后三者是布尔值。`process_state` 至少包含非负整数 `steps_completed` 和字符串 `phase`。`frontier_view` 每项有 `edge_id`；active／paused 视图每项有 `branch_id`，集合不得重叠。`assumption_set` 是无重复字符串数组。

表中未支持的合法目标能力不会被替换为另一种结果：增量预测、score 规则及阈值检查、其他效果模式、Reuse／Salvage 等交接计划会明确失败。这些是原型缺口，不是修改九类 Decision 的正式值域。本轮已补齐 `Promote` 的意图检查和 Pi 单候选会话接替，真实运行见 [首要验证 Workload](reference-workload.md)。Engine 只检查意图格式及 accepted 前提，不证明 Branch 已被接受；实际会话接替与清理由 Pi 接入调用方落实。`Reuse` 仍保留为后续深度嵌套投机所需的结果／受管状态交接能力。系统应用前的状态有效性、预算、Contract、依赖、Fidelity 和 single-winner 复核仍由状态拥有者完成，目前未接入完整 Speculation Coordinator。

## 4. 执行与失败边界

当前支持 Linux，默认使用 `/usr/bin/python3`、`/usr/bin/bwrap` 和 5 秒 wall time；部署方可通过 `PolicyEngine::new` 指定可信的绝对程序路径与超时。所选解释器须适配当前 `/usr/lib`、`/lib`、`/lib64` 标准库及动态库布局。需要系统 `libseccomp.so.2`；缺少工具、namespace 权限或过滤支持时失败，不回退无隔离执行。

- 编译加载和求值均使用全新 bubblewrap namespace；无外网、宿主工作区、home、凭证或控制面 socket，不挂载 `/proc`。只读挂载解释器、系统库、固定脚本和本次输入；根只读，`/tmp` 与 `/dev/shm` 各限 16 MiB。
- 用户模块导入前加载 seccomp，禁止进程／线程创建、执行其他程序、网络 socket、调试其他进程及若干扩权相关系统调用。没有第三方库加载或联网安装路径。
- 每次调用限制地址空间 256 MiB、CPU 3 秒、单文件 1 MiB、文件描述符 32；禁用 core dump。输入输出分别限 1 MiB，错误诊断最多保留 16 KiB。
- Python 通过独立 fd 3 返回带长度前缀的 JSON；普通 stdout 重定向为诊断。Rust 检查长度、完整退出、JSON、结果结构及相应成员资格。用户代码可以伪造自己的求值结果，但不能借该通道请求系统操作；输出仍须在受信 Rust 侧检查。
- 超时、进程崩溃、Python 异常、输入／输出不合法均显式报错，不转换成 `false`、空候选或“安全默认”。超时或超大输出时终止 bubblewrap 并等待退出，其 PID namespace 随之回收。
- 当前没有异步取消 API、cgroup 总量统计、固定标准库内容摘要、最小动态库文件集或外部能力代理。只读 `/usr/lib` 仍暴露部署方系统库，不适合作为恶意多租户的强化隔离承诺。错误文本可能包含作者主动输出的数据，调用方须限制输入中的敏感内容并控制日志访问。

## 5. 验证记录

2026-10-23 在当前 Linux 主机上完成真实示例验证：九类 Decision 的十个入口均执行 Python；改变 YAML 参数影响结果；原始源码被修改、YAML 被移除后，固定产物仍可求值；重复键、全局参数、别名、缺失 Decision、越界路径、不支持能力和 Fidelity 被拒绝；文件、网络和进程访问探针受限；`print` 不破坏结果管道；无限循环被 CPU 限制终止，长时间 sleep 被 wall time 限制终止；声明的包和文本资源可实际导入及读取。这些不是完整安全审计或性能结论。

实际遇到 bubblewrap 不支持 `--preserve-fds` 的兼容性问题后，新增 `tests/policy.rs`，先复现失败，再改为借标准输出传递结果管道、由可信 bootstrap 分离协议与诊断。测试使用真实 bubblewrap 和 Python，不替换 backend。

# Policy Engine 原型

本文记录当前代码，不替代 [Policy Engine 目标语义](../design/control-plane/policy-engine.md)。

当前首版范围：一个 session 对应一个 AScoutAgent 和一棵树，同一 session 加载的 Policy 固定，多轮 task 使用同一份 Policy，不实现运行期间修改或切换。端侧交付、异步求值及保存／恢复必须保持该 Policy 一致；本轮仅收敛文档，不宣称代码已全部对齐。

长期设计保留 Policy 修改能力；切换时机及活动 Chance 如何使用旧／新 Policy 留到后续支持该能力时确定，不再作为首版待定或阻塞项。已有编译产物、留档与恢复验证不证明运行期间修改已支持。

## 1. 已实现的路径

### 接入侧 predictor 事实与准入顺序修复（2026-11-16）

依据 `docs/design/policy-decisions.md` §2.1，`has_predictor` 是粗筛之前已有的“对该结果可用的预测手段”事实，粗筛本身不做预测。当前接入侧唯一实际交付的预测入口是 `local_policy.decisions.predict`；`CompiledPolicy` 要求十个函数绑定完整，`script(Predict)` 把固定源码和绑定交付端侧，`createAgentAccess.predict` 也从同一个 `decisions.predict` 入口调用。当前没有另一个按操作类别声明 predictor 的注册表或配置。

因此 `hasPredictor(branch)` 只检查该 Branch 已交付的非空 `predict` 脚本，不调用任何 Decision，不依赖 `op` 或 `admit` 的结果；模型和工具 Hook 都先由该事实填充 `op.has_predictor`，再执行既有 `beforeAction`。工具入口同时保留原本查询时已有的 `authority` 事实，传入真正的准入输入。`admit` 仍决定当前具体操作是否值得作为入口；`predict` 仍决定当前输入能否产生 Proposal，允许空集合后走权威路径。可调用入口不承诺非空预测或预测正确性，不能把当前完整 Policy 总有 predict 绑定误解为对所有操作强制投机。未来若需按操作声明更细的预测能力，应先确认设计，不能在此修复中添加配置或工具白名单。

原实现通过 `admit.opportunity_pass` 计算 `has_predictor`，构成循环依赖；模型首次请求和工具首次调用都可复现。修复没有改示例 Policy、预测算法、默认配置、控制面裁决、single-winner 或 commit/discard，没有捕获并转换 Policy 异常。已交付 Policy 的继承、恢复和清理直接决定该事实，无额外缓存。

回归 `tests/predictor-fact.mjs` 使用失败 session 留档 Policy 的完整 files/binding/config 和当前正式 runner 构造端侧脚本，通过真实隔离求值复现两个入口；还检查查询不求值、准入拒绝、空预测、继承/恢复/清理、缺少 predict 入口及异常继续传播。`tests/pi-access.mjs` 的手工 admission fixture 补全 predict 入口，避免把只有 admit/extent 的不完整 fixture 当成具有预测手段。默认留档路径为 `/tmp/opencode/runtime-id-real-e2e-20261116/session/policy.json`，可通过 `ASCOUT_PREDICTOR_POLICY` 指定同一证据的搬迁位置；这两项专用验收脚本依赖保留的失败证据，不是干净克隆自包含测试。

先红日志 `/tmp/opencode/predictor-fact-fix-20261116/red.log`：原实现 0/3，模型与工具均 `Policy evaluation failed`；首次修复后 `green.log` 3/3。最终命令、短 E2E 和已知未通过检查见 [验证 Workload](reference-workload.md) 的同日修复记录。原真实运行失败记录保留，尚未真实模型 E2E 通过。

`src/control_plane/policy/` 导出的 `PolicyEngine` 可以读取 YAML，检查九类 Decision 的十个函数入口（`admit` 有两个），固定声明的文件内容和各函数的参数，在隔离环境中检查 Python 语法、导入及函数签名，生成 `CompiledPolicy`。编译不调用 Decision 函数体，但会在隔离环境中执行模块顶层代码。

`CompiledPolicy` 可用 serde JSON 序列化。`script(decision)` 返回内含固定源码、该函数参数和执行入口的 Python 脚本。内容摘要覆盖源码、参数、入口、解释器标识和生成脚本使用的 runner；摘要不是来源认证，跨侧接收方仍须验证来源。当前接收方需要同版本的 AScout runner 和相同的 Python `sys.version`。这不是对标准库全部文件或动态库内容一致性的证明。

`PolicyEngine::evaluate` 对已编译结果求值，不再读取作者 YAML 或原始源码。每次调用创建新进程，返回检查后的 JSON 值；Engine 不创建 Branch、不做效果操作，也不应用返回的选择。

### 正常运行的 Policy 留档（2026-11-13）

控制面在启动 Adapter 前保存 run 内 `policy.json`，复用 `CompiledPolicy` artifact。它包含同次编译读取的原 YAML 文本、原来源绝对路径、`files` 声明的完整 UTF-8 源码/资源、所有 Decision 绑定和配置、Python 标识、digest，以及 Runtime 引用按启动工作目录解析的路径。不是加载完成后重新读取外部文件；`files` 中的包与相对 import 继续由既有内嵌 source loader 解析。源码在编译期间逐个读取，不承诺外部多文件编辑的事务快照。

保存使用同目录私有临时文件、sync 和不覆盖发布；失败明确返回错误，不启动 Adapter。已有 `policy.json` 的普通启动拒绝覆盖。保存成功但后续基础设施启动失败会留下完整 artifact，不自动删除运行资料。`CompiledPolicy::load` 校验既有 executable digest，Tree 的原生 Policy binding 另外绑定 artifact 字节 SHA-256；YAML/来源元数据不改变原 Decision digest 算法。

普通启动也拒绝含原生 session/events、recovery 或 Agent socket 的已有运行目录。恢复的完整文件摘要校验和反序列化使用同一份字节，不二次读取产物。写入／发布失败不留下半份目标文件；发布后目录同步失败时完整文件可能已经存在，仍报告失败，不启动 Adapter。示例 compile 的输出文件必须尚不存在，重新编译使用新输出路径；仅含 basename 的输出路径仍可使用。

`--resume` 无 Policy 参数时读取 run 内 artifact，不重新编译外部源。有显式 Policy 参数时先编译核对 digest，仍使用留档版本；artifact 存在但损坏或绑定不符直接失败。artifact 缺失时要求显式提供 YAML，原 digest 与 Runtime 路径/内容检查通过才保存补充的 artifact，不回退默认策略。不提供旧 artifact 格式兼容。

Runtime 环境变量仍是显式覆盖入口；恢复未指定时使用保存的 Runtime binding，照旧校验路径、内容和能力。普通启动的 Policy Runtime 相对路径仍按启动 cwd 解析，artifact 中另存解析路径，避免误改为 run 目录或 YAML 目录。Runtime 配置文件及环境仍需保留；本轮不复制完整 Runtime。自包含范围为 YAML 与已声明文件，不包括 Python 可执行文件、标准库、动态库、第三方服务或未声明动态依赖；现有隔离与明确拒绝规则不变。

验证：`cargo check --all-targets`、`cargo build --bins --example policy`、`cargo test --test policy`（1/1）、Policy 模块已有测试（3/3）、`node --test tests/recovery-active.mjs`（4/4）通过，无真实模型请求。手构造 `/tmp/opencode/check-policy-record.py` 验证原 YAML/源码逐字一致、声明包相对导入、源删除后求值、已有 artifact 不覆盖及临时文件清理；删除 `/tmp/opencode/recovery-active-owpWLQ/policy` 后不传 Policy/Runtime 覆盖，真实恢复并再次保存通过；追加验证 artifact 篡改、缺失、显式策略 digest 变化、已有运行保护与源码缺失时不发布 artifact/不启动基础设施均通过，最终证据 `/tmp/opencode/policy-record-o7sd7v9d`。该脚本前两轮在恢复握手完成前请求 Shutdown 超时，最终等待本轮 recovery_loaded 后通过；不据此声称启动期退出竞态已修复。

接续独立验证：`cargo test --lib control_plane` 首次 25/26，一项手构造 `CompiledPolicy` fixture 缺少新必需字段；只补齐三个字段后 26/26，无新增测试或兼容默认值。`cargo test --test policy` 1/1；`node --test tests/recovery-active.mjs tests/recovery-filesystem.mjs` 5/5（真实 Pi/NLE/FS 与本地假模型）。`python3 /tmp/opencode/policy-save-check.py` 在首个隔离进程调用时（作者文件已读取、编译尚未返回）刻意改写原 YAML 与包脚本，保存内容仍逐字等于实际加载版本；删除整个源码目录后求值、十个绑定、来源、相对 Runtime 解析、basename 输出、重复保存不覆盖、发布失败清理、缺少文件和损坏摘要拒绝均通过，证据 `/tmp/opencode/policy-save-check-79vgyts5`。第一次该手工检查漏传现有必需的 `hook_count` 而失败，补齐输入后通过，未修改生产输入校验。

`python3 /tmp/opencode/policy-cli-check.py` 通过真实正常启动→主动保存→删除外部 Policy 目录→不传 Policy 默认恢复；完整文件绑定摘要、已有 run 保护、显式不同 Policy 拒绝、缺失 artifact 明确报错及显式补充、篡改拒绝、进程退出均通过，证据 `/tmp/opencode/policy-cli-check-4zcbwh9l`。此检查只配置不可连接的本地假模型端点，不发送 prompt 或模型请求；不代表完整交互回放已实现。

```rust
use std::path::Path;
use ascout::control_plane::policy::{Decision, PolicyEngine};
use serde_json::json;

let engine = PolicyEngine::default();
let policy = engine.compile(Path::new("assets/policy/policy/policy.yaml"))?;
let result = engine.evaluate(&policy, Decision::Extent, &json!({
    "process_state": {"steps_completed": 5, "hook_count": 5, "phase": "reasoning"}
}))?;
assert_eq!(result, "stop");
# Ok::<(), std::io::Error>(())
```

可运行示例（输出目录请自行选择）：

```sh
cargo run --example policy -- compile assets/policy/policy/policy.yaml /tmp/opencode/policy.json
printf '%s\n' '{"process_state":{"steps_completed":5,"hook_count":5,"phase":"reasoning"}}' | cargo run --example policy -- evaluate /tmp/opencode/policy.json extent -
```

`evaluate` 的输入文件参数可以写成 `-`，此时从标准输入读取一份不超过 1 MiB 的 JSON；供 Pi 等接入进程直接传递本次 Decision 数据，不必先落临时文件。

第一条命令输出内容摘要并写入编译产物；第二条命令应输出 `"stop"`。将 YAML 中 `max_steps` 改成 `6` 后重新编译，同一输入会返回 `"continue"`。仅修改原始源码、没有重新编译，不会影响已生成产物。

## 2. 首版 YAML 与调用约定

本地默认策略见 `assets/policy/policy/policy.yaml` 和 `assets/policy/policy/decisions.py`。顶层必需字段是 `version: 1`、`language: python`、`files`、`fidelity: Best-effort`、`decisions`。可选顶层字段 `runtime` 指定该 Policy 使用的 Runtime 配置路径，路径按控制面当前工作目录解析；控制面优先使用 `ASCOUT_RUNTIME_CONFIG` 覆盖。默认 `select` 的 `max_chances: 0` 不选择任何待选 Chance，保持普通执行。

### 本地策略目录（2026-11-14）

一份策略一个独立目录：`assets/policy/policy/`、`assets/policy/nle/`、`assets/policy/pi-tool/` 各自包含声明 `policy.yaml` 和算法 `decisions.py`。目录组织同时适用于示例和未来正式内置策略，当前三份策略均不成熟，整个 `assets/policy/` 暂不纳入 Git，克隆仓库后需另行准备本地文件，依赖它们的示例及现有测试也需要这些文件。

单个 `decisions.py` 是当前简单策略的组织方式，不限制 Engine 的多文件声明、包与相对导入能力。NLE 的观测预测与验证、Pi 工具示例的准入和抽样分别位于自己的算法文件；默认策略保留配置驱动的抽样分支及不选择 Chance 的行为。YAML 的声明文件仍相对 YAML 目录读取，Runtime 引用仍相对启动 cwd 解析。

现有默认声明的 `predict` 缺少算法要求的 `enum_values`，`match` 缺少 `value_type`；直接调用相应非空结果路径会报错，不能仅把 `max_chances` 改成正数就认为投机配置完整。默认输入样例也尚缺 `extent` 声明要求的 `hook_count`。本次仅整理目录与分离算法，保留声明内容和这些既有限制。

本轮验证：三份声明逐字迁移、三份策略均经真实 Engine 编译，NLE／Pi Runtime 路径仍绑定启动 cwd；301 组手构造输入比较拆分前后适用分支的输出及异常一致。现有候选顺序测试 1/1、Policy 集成测试 1/1、控制面测试 26/26、恢复测试 5/5 通过，`cargo check --all-targets` 与 `cargo build --bins --example policy` 通过。恢复测试显式组合抽样预测与 NLE verifier，使用本地伪模型，无真实模型请求；测试与示例依赖本地未跟踪策略，不构成干净克隆后的自包含交付。

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
| `select` | `frontier_view`、`scheduling_context` | 无 | 输入 Chance ID（字段名 `chance_id`） 的无重复有序子集 |
| `rebalance` | `active_view`、`paused_view`、`scheduling_context` | 无 | 输入 Branch 的 `pause`／`resume`／`preempt` 动作，每 Branch 最多一次 |
| `extent` | `process_state` | `process_state.active_time`、`process_state.resource_counters`、`process_state.result_facts`、`process_state.semantic_facts` | `stop`／`continue` |
| `predict` | `chance_context`、`process_context`、`assumption_set`、`capability_context` | `history` | batch `ProposalSet`，每项有 `payload`，可有 `provenance` |
| `launch` | `chance_context`、`authority_state`、`candidate_view` | `scheduling_context` | 布尔值 |
| `effect` | `effect_descriptor`、`context` | 无 | 只接受无附加参数的 `block`／`deny` 计划 |
| `match` | `proposal`、`evidence`、`match_context` | `outcome` | 四类 verdict，不接受 score |
| `dispose` | `branch`、`resolution`、`context` | 无 | 单个 `Discard`，或仅在 `accepted` 时单个 `Promote`；在 `indeterminate`／`error`／`extent_reached` 时可返回空动作列表 |

`op` 包含 `kind`、`produces_result`、`result_pending`、`has_predictor`；后三者是布尔值。`process_state` 至少包含非负整数 `steps_completed` 和字符串 `phase`。`frontier_view` 每项有 `chance_id`；active／paused 视图每项有 `branch_id`，集合不得重叠。`assumption_set` 是无重复的 `(chance_id, proposal_id)` 引用数组。

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

## Chance id 的当前实现

`admit` 通过时 Tree 创建 Chance 并生成 `chance-<n>` 不透明 id；`select` 的 `frontier_view`、`predict` 的 `assumption_set`、`launch`/`predict` 的 `chance_context` 以及观察事件中的 `chance_id` 都使用这个 id。`branch_created` 携带 Branch 所属 Chance；初始权威 Branch 为 null。控制面不再用 operation id 冒充 Chance id。

`extent` 与 `effect` 由 Agent 接入侧的通用层在真实执行边界按同一编译 Policy 求值，并通过 `branch_suspended` 的原因让控制面记录 Blocked；`rebalance` 由控制面在 Tree 版本变化后求值并下发 pause/resume/preempt。控制面不重复求值 extent/effect，因此没有其它接入形式的实现时，这两类 Decision 不会在控制面侧单独生效。

当前是最小接线：extent 在 Pi 接入的工具执行点按 action 计数求值，停止时 Branch 进入 `blocked/{"extent_reached"}`，由 `resolved_pending` 或取消释放；effect 只处理接入侧对 candidate 外部工具效果求值的 `block`/`deny` 计划，模型推理作为 PURE 步骤不受该 gate 影响，`block` 使 Branch 进入 `blocked/{"effect_gate"}` 直到 Promote、Discard 或清理；rebalance 只按 Policy 返回的 branch_id 执行 pause/resume/preempt，没有独立的资源预算计算、全局成本模型或 effect 插件。三者都通过公共 `status_changed` 事件如实发布 Blocked 与其原因。

嵌套 Chance 已按 Chance 语义接通：权威等待点只要求父 Branch 仍存活（当前权威或候选），父 Branch 不是全局权威不再使 Chance 失效，候选 Branch 上再次 `admit` 的 Chance 与根 Branch 上的 Chance 一样走 Pending、Running、Resolving、Accepted/Discarded。嵌套候选 Branch 的 `AssumptionSet` 携带祖先未确认的 Proposal；祖先 Chance 未 Accepted 前它没有提交资格，match 成功也停留到可提交时再按同一 Promote 流程处置。父 Branch 进入 Discarding/Discarded 时，其上未收敛的 Chance 以 `Discarded(AncestorInvalidated)` 收敛，这是嵌套失效的唯一传播路径；候选 Branch 的外部效果仍受 effect 闸门约束。尚未覆盖的是 Runtime 侧的嵌套执行承载和候选状态重新挂载，当前只保证控制面状态、提交资格与失效传播。

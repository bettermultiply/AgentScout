# Policy Engine 原型

本文记录当前代码，不替代 [Policy Engine 目标语义](../design/control-plane/policy-engine.md)。

当前首版范围：一个 session 对应一个 AScoutAgent 和一棵树，同一 session 加载的 Policy 固定，多轮 task 使用同一份 Policy，不实现运行期间修改或切换。端侧交付、异步求值及保存／恢复必须保持该 Policy 一致；本轮仅收敛文档，不宣称代码已全部对齐。

长期设计保留 Policy 修改能力；切换时机及活动 Chance 如何使用旧／新 Policy 留到后续支持该能力时确定，不再作为首版待定或阻塞项。已有编译产物、留档与恢复验证不证明运行期间修改已支持。

## 1. 已实现的路径

### 纯表达式内存求值与工具错误裁决

编译探针检查入口的 Python AST。`admit.opportunity_pass` 必须生成纯表达式树，否则编译返回 `InvalidInput`。其他 Decision 可在无法纯内存求值时回退到 Python 沙箱。可编译的纯表达式树随 `CompiledPolicy` 保存。`LocalPolicy` 将四个端侧 Decision 对应的树和参数交给 Node，并附带可编译的 `select`、`launch`、`predict` 表达式。Rust 和 Node 在内存中执行常量、容器、局部变量、条件、比较、基础算术、常用容器和字符串方法以及简单推导式。两端保持 Python 的真值和短路规则，并校验输入、输出与 JSON 边界。求值中遇到缺少键或类型错误时，错误直接返回给调用方。

带导入、复杂语法、包入口或能力调用的绑定仍使用 Python worker。`extent` 可在简单条件中保留对模块内辅助函数的调用位置。只有执行到该表达式时才把整个 Decision 交给 Python worker 重算，遵守 Python 的短路规则。其他 Decision 遇到辅助函数调用继续使用 worker。值超出求值预算、整数或对象身份无法精确处理、Unicode 大小写规则依赖 Python 版本等情形也由 worker 求值。元数据省略时沿用 worker，已有 artifact 可继续加载，冻结 fixture 的 digest 不变。编译探针在导入 Policy 模块前检查 `admit.opportunity_pass` 中的模型能力引用，授权校验同时禁止将该 Decision 加入 `policy_model_grant.decisions`。

已交付的工具错误仍以 `ActionFinished.status: failed` 记录，其结果同样交给 `match` 和 `dispose`。缺少权威结果时，控制面丢弃存活候选并按需释放父分支的继续执行等待。离线回归覆盖 `bash exit 1` 的两种预测结果，以及缺少权威结果时的候选回收。

### 端到端投机启动耗时

在真实 control plane、Pi SDK、FilesystemBackend 和 `assets/policy/coding` 上，用本机 SSE 假模型重复触发前台验证命令。每次新进程依次测首个机会、同一进程热运行的机会、空闲超过一秒后的机会。前两次错配让父分支恢复，第三次匹配并晋升。以 Chance 的 `speculation_started_at_ms` 和候选首个模型 `action_started.timestamp_ms` 为主指标。

- Runtime TCP 两端启用 `TCP_NODELAY`。基线的权威 `action_started` 到 Runtime `operation_started` 为约 40 ms，符合长度头和 JSON 帧体分两次发送时 Nagle 算法与延迟 ACK 的表现。单独改 TCP 后，环境请求到就绪的 p50 从 43 ms 降到 2 ms。
- 候选第一步继续正常求值 `extent`。编译探针保留简单条件和短路规则，执行到模块内辅助函数时用隔离 worker 重算。首步模型边界上的 coding `extent` 不需要该辅助函数。带授权的决策仍走原有授权路径。编译时保留沙箱绑定检查，已有 Policy 留档可按原版本继续恢复。
- 每个 AST 编译探针结束即回收。原先探针和实际脚本共用十个位置，探针会把已经检查过的 Runtime Decision worker 挤出缓存。真实脚本的绑定检查与后续调用复用原 worker，首个机会到环境请求的 p50 从 49 ms 降到 18 ms。Node worker 的一秒空闲回收不变。

| Chance start → candidate `action_started`（ms） | 基线 p50 / p95 | 仅 TCP | TCP + extent | 最终 |
| --- | ---: | ---: | ---: | ---: |
| 首个机会 | 134 / 136 | 95 / 96 | 56 / 56 | 24 / 26 |
| 热运行 | 68 / 115 | 31 / 52 | 26 / 34 | 22 / 69 |
| 空闲后 | 104 / 105 | 63 / 64 | 23 / 25 | 23 / 25 |

每组 5 次，按最近秩计算 p95。最终热运行有一次 Runtime 环境操作耗时 47 ms。临时探针中，Pi extension 绑定等待为 0 ms，`session.continue()` 到模型 hook 为 0–1 ms，基线冷／空闲后的 `extent` worker 请求往返约 36 ms。模型动作到假 provider 的 p50 为 2 ms，错配后从权威工具结束到父分支请求到达 provider 的 p50 为 40 ms。进程启动到权威 `branch_ready` 的 p50 从 1,900 ms 降到 1,846 ms，p95 从 2,052 ms 升到 2,648 ms。按 250 ms 采样，进程树的峰值进程数从 46 降到 37，每次运行的峰值 RSS 合计中位数从约 507 MiB 降到 446 MiB，合计包含共享页。Django `run-S90Enb` 的账本用于定位，表内全部为隔离的 coding fixture 实测。

证据及复现脚本位于 `/var/tmp/ascout-startup.euUADp/`，其中 `summary.json` 包含分阶段原始样本，`ab-*` 保留账本、假 provider 到达时间、工具进程启动时间、进程采样和探针日志。可在仓库根目录运行以下命令。`baseline-bin` 包含修改前的一组 release 二进制。

```sh
p=/var/tmp/ascout-startup.euUADp
node "$p/make-bench.mjs" "$p"
cargo build --offline --release --bins
mkdir "$p/replay-before" "$p/replay-after"
BENCH_BIN="$p/baseline-bin/ascout-control-plane" node "$p/bench.mjs" "$p/replay-before"
node "$p/bench.mjs" "$p/replay-after"
node "$p/summarize.mjs" "$p/replay-before" "$p/replay-after" > "$p/replay-summary.json"
```

串行 Rust 测试通过 230 项，Node 离线测试通过 40 项，另有 Filesystem 恢复测试通过。临时 `gate-check.mjs` 使用真实 Pi 与 FilesystemBackend，分别验证首步 `extent` 短路得到 `stop`、调用沙箱辅助函数得到 `stop`，以及 `effect` 阻止候选写入；三项均通过。原始验证命令和日志见同目录的 `verify.sh`、`rust-final-serial.log`、`node-final-serial.log`、`recovery-final.log` 和 `gate-verify.log`。

### Django 原仓离线回放

同一份 `run-S90Enb` 归档 Policy、prompt 和工具参数，在干净 Django 基线、原 Python 3.9 环境、真实 Pi 工具和 FilesystemBackend 上，用回环假模型交错运行优化前／最终 release，各 3 轮。Chance 3/4 合并的 `Chance start → candidate action_started` 中位数为 113 → 30 ms（范围 111–115 → 29–69 ms），到假 provider HTTP 接收为 115.9 → 32.9 ms（113.4–118.0 → 31.1–71.4 ms）。六轮各有两次 Promote，两条目标命令各通过 501 个测试，工作区 diff 与归档一致；真实外部模型请求为 0。

原在线账本的 309/310 ms 未复现：`branch_ready → model action_started` 原为 231–234 ms，离线基线为 41–43 ms、最终为 0 ms（账本毫秒精度）。因此不能用本次 A/B 解释旧运行中该段多出的约 190 ms。小 fixture 的空闲后中位数 104 → 23 ms 则是独立样本，不是 Django 数据。逐次事件、校验、环境和复现脚本见 `/var/tmp/ascout-django-ab.niPBkd/`；复跑：`bash /var/tmp/ascout-django-ab.niPBkd/run-pairs.sh rerun-$(date +%s)`。

### 工具的端侧先行

Pi 只在权威分支没有挂起冲突、当前批次只有一个工具调用时尝试端侧先行。两个 `admit` 判断均须在内存中通过。`select` 须选中该机会，`launch` 须返回 `true`，`predict` 须返回恰好一个 Proposal。端侧只使用其能够完整重建的输入。目前这包括 `frontier_view` 和 `chance_context`。`abstain` 直接执行真实操作。若表达式依赖 `scheduling_context`、`history`、控制面观察、模型能力或其他端侧不可见数据，则沿用控制面路径。

端侧把先行事实随 `speculative_admission_request` 发送。控制面记录 Chance、Proposal 和 Branch，保持 `match`、`dispose` 和 Settlement 流程。候选工具等待独立环境绑定或 Full 晋升后继承父环境。

### Instrumentation predictor 事实与准入顺序修复（2026-11-16）

依据 `docs/design/policy-decisions.md` §2.1，`has_predictor` 是粗筛之前已有的“对该结果可用的预测手段”事实，粗筛本身不做预测。`CompiledPolicy` 要求十个函数绑定完整。常规机会由 control plane 求值 `predict`，符合先行条件的机会在端侧求值。当前没有另一个按操作类别声明 predictor 的注册表或配置。

因此 `Interposer.hasPredictor(branch)` 只读取 control plane 已交付的 `has_predictor` 事实，不调用任何 Decision，不依赖 `op` 或 `admit` 的结果；模型和工具 Hook 都先由该事实填充 `op.has_predictor`，再执行既有 `beforeAction`。工具入口同时保留原本查询时已有的 `authority` 事实，传入真正的准入输入。`admit` 仍决定当前具体操作是否值得作为入口；`predict` 仍决定当前输入能否产生 Proposal，允许空集合后走权威路径。可调用入口不承诺非空预测或预测正确性，不能把当前完整 Policy 总有 predict 绑定误解为对所有操作强制投机。未来若需按操作声明更细的预测能力，应先确认设计，不能在此修复中添加配置或工具白名单。

原实现通过 `admit.opportunity_pass` 计算 `has_predictor`，构成循环依赖；模型首次请求和工具首次调用都可复现。修复没有改示例 Policy、预测算法、默认配置、control plane 裁决、single-winner 或 commit/discard，没有捕获并转换 Policy 异常。已交付 Policy 的继承、恢复和清理直接决定该事实，无额外缓存。

回归 `tests/predictor-fact.mjs` 使用失败 session 留档 Policy 的完整 files/binding/config 和当前正式 runner 构造端侧脚本，通过真实隔离求值复现两个入口；还检查查询不求值、准入拒绝、空预测、继承/恢复/清理、缺少 predict 入口及异常继续传播。`tests/pi-access.mjs` 的手工 admission fixture 补全 predict 入口，避免把只有 admit/extent 的不完整 fixture 当成具有预测手段。默认 fixture 为 `tests/fixtures/predictor-fact-policy.json`，可通过 `ASCOUT_PREDICTOR_POLICY` 指定同一留档的其它位置。

先红日志 `/tmp/opencode/predictor-fact-fix-20261116/red.log`：原实现 0/3，模型与工具均 `Policy evaluation failed`；首次修复后 `green.log` 3/3。最终命令、短 E2E 和已知未通过检查见 [验证 Workload](reference-workload.md) 的同日修复记录。原真实运行失败记录保留，尚未真实模型 E2E 通过。

`src/control_plane/policy/` 导出的 `PolicyEngine` 可以读取 YAML，检查九类 Decision 的十个函数入口（`admit` 有两个），固定声明的文件内容和各函数的参数，在隔离环境中检查 Python 语法及函数入口，生成 `CompiledPolicy`。纯表达式绑定通过 AST 检查。其他绑定的签名检查会在隔离环境中执行模块顶层代码。编译阶段不调用 Decision 函数体。

`CompiledPolicy` 可用 serde JSON 序列化。`script(decision)` 返回内含固定源码、该函数参数和执行入口的 Python 脚本。内容摘要覆盖源码、参数、入口、解释器标识、生成脚本使用的 runner 及声明的 `policy_model_grant`。省略授权字段时保留既有摘要算法。摘要不是来源认证，跨侧接收方仍须验证来源。当前接收方需要同版本的 AScout runner 和相同的 Python `sys.version`。这不是对标准库全部文件或动态库内容一致性的证明。

`PolicyEngine::evaluate` 使用已编译结果。可在内存求值的 Decision 直接返回通过校验的结果。Python 路径最多持有十个按完整编译脚本区分的隔离 worker；同一脚本重复调用只通过有界 IPC 传递输入和结果。求值串行执行，超时、协议错误或进程退出后杀死并 wait，下一次重建；Engine 释放或池满清理时同样回收进程。Engine 不创建 Branch、不做效果操作，也不应用返回的选择。解释器身份探针仍为一次性进程。

Pi `packages/pi-integration/src/local-policy.ts` 使用最多十个 worker 的固定池和串行请求队列，提供 `disposePolicyWorker` 显式释放，并在空闲一秒后释放。每个 worker 只在启动时创建 bwrap namespace。保留 1 MiB 输入和响应上限、默认 5 秒求值超时、启动前及运行中取消检查、stderr 排空，失败后杀死进程并等待 close 后清理。池满或失败当前释放整个本地池。`tests/policy-local-supervision.mjs` 覆盖重复请求计数连续、显式释放后重建、取消、超时及超大输入／输出。

Python 常驻进程只执行可信监督逻辑，每次请求先清理 `/tmp` 和 `/dev/shm` 的遗留文件，再 fork 求值子进程；子进程安装 seccomp 后才加载 Policy，完成后退出，由监督进程 wait 后才发布最终结果；子进程只持有内部管道，无法直接读取宿主的下一请求。`/dev` 挂载只读（单独的 `/dev/shm` 挂载仍按请求清理）。Policy 修改标准库对象不会回写监督进程，不以清理模块作为安全边界。同脚本连续两次请求复用一个 namespace／监督 PID，创建两个求值子进程；求值 PID 不复用。这是每次一次 fork 的隔离代价，不是每次启动 bwrap。标准库和动态库仍只读可见，不宣称禁止全部文件读取。`tests/policy-isolation.mjs` 验证标准库对象和两个可写挂载的数据不跨请求残留；任意自写 artifact runner 不具备该保障，正常入口必须交付宿主编译生成的脚本。

### Policy 模型授权

在 Policy YAML 顶层按需声明：

```yaml
policy_model_grant:
  decisions: [predict, match, extent]
  models: [provider/model]
  max_requests: 2
  timeout_ms: 10000
```

`decisions` 必须非空且互不重复。`models` 必须非空，每个条目由非空的 provider 和 model 组成，以 `/` 分隔，禁止空白字符。`max_requests` 的范围为 `1..=16`，`timeout_ms` 的范围为 `100..=300_000`，额度和超时均按单次求值生效。编译和产物加载检查授权结构，产物加载也验证摘要。

`model_complete` 通过 capability IPC 请求宿主。control plane 通过 `ModelProviderProcess` 启动 `instrumentation/pi/policy-provider.ts`，并将 `ASCOUT_PI_ROOT` 和可选 `--models` 交给桥接进程。Pi 端侧只收到本地 Decision 的授权和声明输入。`evaluatePolicyArtifact` 的第五参数携带授权 Decision、允许模型、请求额度、可选 `timeoutMs` 和宿主完成函数。未授权求值拒绝模型调用。两侧宿主检查当前请求 ID、Decision、模型、额度、大小及超时。两侧使用 Pi 的模型与凭证配置，`piModelComplete` 禁用自动重试。端侧结束求值时取消 provider 请求，Rust 端超时、取消或失败时终止并回收桥接进程及异常 worker。

真实请求的既有证据 `/tmp/ascout-real.log` 仍可查阅。本轮未运行该脚本，剩余真实请求额度为 0。当时的脚本手工构造 runner 且返回 `{ok:true}`，只证明调用链 smoke，不证明正常 Effect schema 或完整 E2E。

`tests/policy_provider.rs` 覆盖正常 YAML 编译、真实隔离 runner、Rust 进程 IPC、Pi provider 桥及 fake Models。`tests/policy-decision-e2e.mjs` 通过真实 control plane、Pi 与 Filesystem Runtime，使用 loopback OpenAI-compatible SSE 验证两侧授权和 Decision 输入。

离线回归通过 `cargo check --all-targets`、`cargo build --bins`、当时的 Rust 测试集 和项目的 Node 离线测试集。Rust 使用 `RUST_TEST_THREADS=1` 串行运行涉及挂载的测试。端到端回归使用本机假模型，其中一笔 `predict` 请求延迟超过 5 秒，以验证声明的超时生效。真实模型请求次数为 0。

### 正常运行的 Policy 留档（2026-11-13）

control plane 在启动 Adapter 前保存 run 内 `policy.json`，复用 `CompiledPolicy` artifact。它包含同次编译读取的原 YAML 文本、原来源绝对路径、`files` 声明的完整 UTF-8 源码/资源、所有 Decision 绑定和配置、Python 标识、digest，以及 Runtime 引用按启动工作目录解析的路径。不是加载完成后重新读取外部文件；`files` 中的包与相对 import 继续由既有内嵌 source loader 解析。源码在编译期间逐个读取，不承诺外部多文件编辑的事务快照。

保存使用同目录私有临时文件、sync 和不覆盖发布；失败明确返回错误，不启动 Adapter。已有 `policy.json` 的普通启动拒绝覆盖。保存成功但后续基础设施启动失败会留下完整 artifact，不自动删除运行资料。`CompiledPolicy::load` 校验既有 executable digest，Tree 的原生 Policy binding 另外绑定 artifact 字节 SHA-256；YAML/来源元数据不改变原 Decision digest 算法。

普通启动也拒绝含原生 session/events、recovery 或 Agent socket 的已有运行目录。恢复的完整文件摘要校验和反序列化使用同一份字节，不二次读取产物。写入／发布失败不留下半份目标文件；发布后目录同步失败时完整文件可能已经存在，仍报告失败，不启动 Adapter。示例 compile 的输出文件必须尚不存在，重新编译使用新输出路径；仅含 basename 的输出路径仍可使用。

`--resume` 无 Policy 参数时读取 run 内 artifact，不重新编译外部源。有显式 Policy 参数时先编译核对 digest，仍使用留档版本；artifact 存在但损坏或绑定不符直接失败。artifact 缺失时要求显式提供 YAML，原 digest 与 Runtime 路径/内容检查通过才保存补充的 artifact，不回退默认策略。现有无 `policy_model_grant` 的产物仍可按原摘要加载。

Runtime 环境变量仍是显式覆盖入口；恢复未指定时使用保存的 Runtime binding，照旧校验路径、内容和能力。普通启动的 Policy Runtime 相对路径仍按启动 cwd 解析，artifact 中另存解析路径，避免误改为 run 目录或 YAML 目录。Runtime 配置文件及环境仍需保留；本轮不复制完整 Runtime。自包含范围为 YAML 与已声明文件，不包括 Python 可执行文件、标准库、动态库、第三方服务或未声明动态依赖；现有隔离与明确拒绝规则不变。

验证：`cargo check --all-targets`、`cargo build --bins --example policy`、当时的 Policy 集成检查（1/1）、Policy 模块已有测试（3/3）、`node --test tests/recovery-active.mjs`（4/4）通过，无真实模型请求。手构造 `/tmp/opencode/check-policy-record.py` 验证原 YAML/源码逐字一致、声明包相对导入、源删除后求值、已有 artifact 不覆盖及临时文件清理；删除 `/tmp/opencode/recovery-active-owpWLQ/policy` 后不传 Policy/Runtime 覆盖，真实恢复并再次保存通过；追加验证 artifact 篡改、缺失、显式策略 digest 变化、已有运行保护与源码缺失时不发布 artifact/不启动基础设施均通过，最终证据 `/tmp/opencode/policy-record-o7sd7v9d`。该脚本前两轮在恢复握手完成前请求 Shutdown 超时，最终等待本轮 recovery_loaded 后通过；不据此声称启动期退出竞态已修复。

接续独立验证：`cargo test --lib control_plane` 首次 25/26，一项手构造 `CompiledPolicy` fixture 缺少新必需字段；只补齐三个字段后 26/26，无新增测试或兼容默认值。当时的 Policy 集成检查 1/1；`node --test tests/recovery-active.mjs tests/recovery-filesystem.mjs` 5/5（真实 Pi/NLE/FS 与本地假模型）。`python3 /tmp/opencode/policy-save-check.py` 在首个隔离进程调用时（作者文件已读取、编译尚未返回）刻意改写原 YAML 与包脚本，保存内容仍逐字等于实际加载版本；删除整个源码目录后求值、十个绑定、来源、相对 Runtime 解析、basename 输出、重复保存不覆盖、发布失败清理、缺少文件和损坏摘要拒绝均通过，证据 `/tmp/opencode/policy-save-check-79vgyts5`。第一次该手工检查漏传现有必需的 `hook_count` 而失败，补齐输入后通过，未修改生产输入校验。

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

本地默认策略见 `assets/policy/policy/policy.yaml` 和 `assets/policy/policy/decisions.py`。顶层必需字段是 `version: 1`、`language: python`、`files`、`fidelity: Best-effort`、`decisions`。可选顶层字段 `runtime` 指定该 Policy 使用的 Runtime 配置路径，路径按 control plane 当前工作目录解析；control plane 优先使用 `ASCOUT_RUNTIME_CONFIG` 覆盖。默认 `select` 的 `max_chances: 0` 不选择任何待选 Chance，保持普通执行。

### 本地策略目录（2026-11-14）

一份策略一个独立目录：`assets/policy/policy/`、`assets/policy/nle/`、`assets/policy/pi-tool/` 各自包含声明 `policy.yaml` 和算法 `decisions.py`。目录组织同时适用于示例和未来正式内置策略，当前三份策略均不成熟，整个 `assets/policy/` 暂不纳入 Git，克隆仓库后需另行准备本地文件，依赖它们的示例及现有测试也需要这些文件。

单个 `decisions.py` 是当前简单策略的组织方式，不限制 Engine 的多文件声明、包与相对导入能力。NLE 的观测预测与验证、Pi 工具示例的准入和抽样分别位于自己的算法文件；默认策略保留配置驱动的抽样分支及不选择 Chance 的行为。YAML 的声明文件仍相对 YAML 目录读取，Runtime 引用仍相对启动 cwd 解析。

现有默认声明的 `predict` 缺少算法要求的 `enum_values`，`match` 缺少 `value_type`；直接调用相应非空结果路径会报错，不能仅把 `max_chances` 改成正数就认为投机配置完整。默认输入样例也尚缺 `extent` 声明要求的 `hook_count`。本次仅整理目录与分离算法，保留声明内容和这些既有限制。

本轮验证：三份声明逐字迁移、三份策略均经真实 Engine 编译，NLE／Pi Runtime 路径仍绑定启动 cwd；301 组手构造输入比较拆分前后适用分支的输出及异常一致。现有候选顺序测试 1/1、Policy 集成测试 1/1、control plane 测试 26/26、恢复测试 5/5 通过，`cargo check --all-targets` 与 `cargo build --bins --example policy` 通过。恢复测试显式组合抽样预测与 NLE verifier，使用本地伪模型，无真实模型请求；测试与示例依赖本地未跟踪策略，不构成干净克隆后的自包含交付。

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
- `capabilities.model_complete(request)` 仅在宿主为本次 Decision 授权时可用。普通启动依据所选 Policy 的 `policy_model_grant` 构造该授权。声明 `requirements`、`outputs` 等尚未实现的 DSL 字段仍会报错。
- 九类行为都必须显式绑定；不补系统默认函数。入口必须是模块顶层、未装饰的普通同步函数，能按上述形式调用；生成器和异步函数被拒绝。
- YAML 拒绝重复键（包括参数内部）、锚点、别名、tag、指令、merge key、多文档、非字符串键及非有限数值。只按 YAML 1.2 规则识别布尔值，不将 `yes`／`on` 当成布尔值。
- YAML 和全部声明源码分别限 1 MiB，文件最多 128 个，数据嵌套最多 32 层，整数限有符号 64 位。文件采用 UTF-8 文本；二进制资源尚不支持。
- 文件路径是 YAML 目录内规范化的相对路径，不接受绝对路径、`..`、`.`、空路径段、反斜线或 NUL。Python 模块路径必须是 ASCII 标识符，各级包的 `__init__.py` 必须显式列出。不允许覆盖标准库模块。声明文件逐级以目录 fd 和 `O_NOFOLLOW` 打开，拒绝符号链接和非普通文件；YAML 文件本身也不接受符号链接。传入的 YAML 上级目录属于调用方选择的根，不要求其全部祖先目录都不是符号链接。
- 本地 Python 模块按正常 import 和相对 import 规则加载。声明的文本资源可经包的 `pkgutil.get_data` 读取；不支持直接打开 `/policy/...` 或 `importlib.resources`。未声明文件不会回到作者工作区寻找。

## 3. 输入与输出范围

调用方负责准备已经授权、符合任务领域 schema 的事实快照，不能直接传 control plane 内部对象或未筛选的上下文。原型检查顶层字段、JSON 类型、必要进度事实、机会事实和选择成员资格等，不具备完整的领域 schema 注册与校验；这部分接入责任尚未自动化，不能声称任意嵌套 JSON 都是已授权的事实。

| 入口 | 必需顶层字段 | 可声明的可选字段 | 当前接受的输出 |
|---|---|---|---|
| `admit.opportunity_pass` | `op` | 无 | 布尔值 |
| `admit.feasibility_pass` | `op`、`state`、`contract_context` | 无 | 布尔值 |
| `select` | `frontier_view`、`scheduling_context` | 无 | 输入 Chance ID（字段名 `chance_id`） 的无重复有序子集 |
| `rebalance` | `active_view`、`paused_view`、`scheduling_context` | 无 | 输入 Branch 的 `pause`／`resume`／`preempt` 动作，每 Branch 最多一次 |
| `extent` | `process_state` | `process_state.hook_count`、`process_state.active_time`、`process_state.resource_counters`、`process_state.result_facts`、`process_state.semantic_facts` | `stop`／`continue` |
| `predict` | `chance_context`、`process_context`、`assumption_set`、`capability_context` | `history` | 显式 `abstain` 或非空 batch。模型推理边界仍支持 `branch_model` |
| `launch` | `chance_context`、`authority_state`、`candidate_view` | `scheduling_context` | 布尔值 |
| `effect` | `effect_descriptor`、`context` | 无 | `allow`、`deny`、`block`、`default`、`stage`、`simulate`，可附 `reason` |
| `match` | `proposal`、`evidence`、`match_context` | `outcome` | 四类 `verdict`，可附数字 `score` 和对象 `details` |
| `dispose` | `branch`、`resolution`、`context` | 无 | `Promote`、`Salvage`（未接受时须加 `Discard`）、`Yield` 或 `Discard`。`indeterminate`／`error`／`extent_reached` 可返回空动作列表 |

`op` 包含 `kind`、`produces_result`、`result_pending`、`has_predictor`；后三者是布尔值。`process_state` 至少包含非负整数 `steps_completed` 和字符串 `phase`。`frontier_view` 每项有 `chance_id`；active／paused 视图每项有 `branch_id`，集合不得重叠。`assumption_set` 是无重复的 `(chance_id, proposal_id)` 引用数组。

增量预测及外部效果协调器尚未接入。`stage`／`simulate` 在效果边界暂停并记录原因。引擎验证处置契约，控制面复核分支生命周期、等待点和 single-winner，Runtime 检查文件冲突后交付。`Salvage` 可以从验证裁决、范围暂停或抢占进入。任务级中断直接取消尚未提交的工作。

## 4. 执行与失败边界

当前支持 Linux，默认使用 `/usr/bin/python3`、`/usr/bin/bwrap` 和 5 秒 wall time；部署方可通过 `PolicyEngine::new` 指定可信的绝对程序路径与超时。所选解释器须适配当前 `/usr/lib`、`/lib`、`/lib64` 标准库及动态库布局。需要系统 `libseccomp.so.2`；缺少工具、namespace 权限或过滤支持时失败，不回退无隔离执行。

- 编译加载和求值均使用全新 bubblewrap namespace；无外网、宿主工作区、home、凭证或 control plane socket，不挂载 `/proc`。只读挂载解释器、系统库、固定脚本和本次输入；根只读，`/tmp` 与 `/dev/shm` 各限 16 MiB。
- 用户模块导入前加载 seccomp，禁止进程／线程创建、执行其他程序、网络 socket、调试其他进程及若干扩权相关系统调用。没有第三方库加载或联网安装路径。
- 每个 worker 限制地址空间 256 MiB、单文件 1 MiB、文件描述符 32；禁用 core dump。不使用跨请求累计的 CPU 限额，逐次请求由宿主 wall deadline 监督。输入输出分别限 1 MiB；Rust 持久 worker 当前丢弃 stderr，本地 worker 排空 stderr。
- Python 通过独立 fd 3 返回带长度前缀的 JSON；普通 stdout 重定向为诊断。Rust 检查长度、完整退出、JSON、结果结构及相应成员资格。用户代码可以伪造自己的求值结果，但不能借该通道请求系统操作；输出仍须在受信 Rust 侧检查。
- 超时、进程崩溃、Python 异常、输入／输出不合法均显式报错，不转换成 `false`、空候选或“安全默认”。超时或超大输出时终止 bubblewrap 并等待退出，其 PID namespace 随之回收。
- 当前没有异步取消 API、cgroup 总量统计、固定标准库内容摘要、最小动态库文件集或外部能力代理。只读 `/usr/lib` 仍暴露部署方系统库，不适合作为恶意多租户的强化隔离承诺。错误文本可能包含作者主动输出的数据，调用方须限制输入中的敏感内容并控制日志访问。

## 5. 验证记录

2026-10-23 在当前 Linux 主机上完成真实示例验证：九类 Decision 的十个入口均执行 Python；改变 YAML 参数影响结果；原始源码被修改、YAML 被移除后，固定产物仍可求值；重复键、全局参数、别名、缺失 Decision、越界路径、不支持能力和 Fidelity 被拒绝；文件、网络和进程访问探针受限；`print` 不破坏结果管道；无限循环被 CPU 限制终止，长时间 sleep 被 wall time 限制终止；声明的包和文本资源可实际导入及读取。这些不是完整安全审计或性能结论。

实际遇到 bubblewrap 不支持 `--preserve-fds` 的兼容性问题后，通过 Policy 编译复现失败，再改为借标准输出传递结果管道、由可信 bootstrap 分离协议与诊断。测试使用真实 bubblewrap 和 Python，不替换 backend。

## Chance id 的当前实现

`select.scheduling_context` 提供只读调度事实。`active_candidate_count` 和 `active_branches` 包含仍参与投机的候选，`blocked_candidate_count` 是其中处于 `Blocked` 状态的数量。`pending_chance_count` 是处于 `Pending` 状态的 Chance 数量。`active_branches` 每项包含 `branch_id`、`chance_id`、`assumption_depth`、`status`、布尔值 `yield_only` 和可为空的 `speculation_kind`、`proposal_provenance`。`frontier_view` 每个机会也包含 `speculation_kind`，例如 `model_inference` 或 `tool_execution`。

`launch.authority_state` 只包含 `branch_id`、`status`、`assumption_depth` 和 `turn_active`，父分支缺失时为 `{}`。`launch.candidate_view` 使用与 `select` 的 `active_branches` 相同的候选摘要。

`admit` 通过时 Tree 创建 Chance 并生成 `chance-<n>` 不透明 id；`select` 的 `frontier_view`、`predict` 的 `assumption_set`、`launch`/`predict` 的 `chance_context` 以及观察事件中的 `chance_id` 都使用这个 id。`branch_created` 携带 Branch 所属 Chance；初始权威 Branch 为 null。control plane 不再用 operation id 冒充 Chance id。

`extent` 与 `effect` 由 Instrumentation 的通用层在真实执行边界按同一编译 Policy 求值，并通过 `branch_suspended` 的原因让 control plane 记录 Blocked；`rebalance` 由 control plane 在 Tree 版本变化后求值并下发 pause/resume/preempt。control plane 不重复求值 extent/effect，因此没有其它接入形式的实现时，这两类 Decision 不会在 control plane 侧单独生效。

`extent` 仅在候选分支执行模型或工具前求值。`steps_completed` 从候选创建时的 0 开始，每次收到完成通知递增。`phase` 表示下一动作类型。只有声明了相应 `inputs` 才提供进入边界的次数 `hook_count`、自创建以来的秒数 `active_time`、分类完成计数 `resource_counters`、最近完成动作摘要 `result_facts` 和下一动作的类型、名称与参数 `semantic_facts`。已准入动作重连沿用原计数。全局晋升释放 `extent` 等待，权威分支跳过 `extent`。停止时分支报告 `extent_reached`，`resolve_pending` 或取消也可结束等待。

`admit` 的 `op` 提供动作标识、参数、结果是否待定，以及预测和权威状态。`admit.feasibility_pass` 还接收 `is_candidate`、`assumption_depth`、`steps_completed`、按工具名统计的成功与失败次数 `tool_counts`、最近最多 16 条已完成动作 `recent_actions`。候选继承父分支的工具统计与最近动作，自己的步数从 0 开始。保存与恢复保留这些值。

`predict` 只在声明 `history` 时接收有界历史摘要，条数由声明中的 `max_entries` 限制。`match` 只在声明 `outcome` 时接收顶层实际结果。`dispose` 的 `candidate_results` 保留已经报告的工具标识、参数和详情。`effect` 仅在候选工具边界求值，隔离能力与 Policy 同时允许时才执行工具。`block` 挂起至放行、丢弃或清理。`rebalance` 按 Policy 返回的分支标识执行 pause、resume、preempt。

嵌套 Chance 使用它所属父 Branch 的真实执行和等待点来验证。内层 Full 可把候选状态合入仍处于投机的父环境，保留胜者 session 及其后代，并让胜者承接原父 Branch 的外层 Chance。Part 只把回执确认的结果交给原父 session，并抛弃内层候选及后代。两者都保留祖先未决 Proposal，且祖先否定仍会丢弃相应的投机延续。文件级结果依赖、暂停顺序和后端限制见 [Runtime 生命周期](runtime-lifecycle.md#full-与-part-接受)。

## 实现差距

每次求值新建 namespace 已被否定，替代机制未选，现有 bubblewrap 路径仍是未替换的原型。
control plane 入口仍内嵌 Pi 启动参数；目标是入口不内嵌产品启动参数，由 UI 提供，通道尚未实现。

## 配置目录与启动检查

control plane 从 `~/.config/ascout/config.toml` 读取 `policy_dir`、`runs_dir`、`default_policy` 与 `log_path`，默认分别为 `~/.local/share/ascout/policies`、`~/.local/share/ascout/runs`、`policy` 与 `~/.local/share/ascout/ascout.log`。仓库开发配置须显式指向本仓库 `assets/policy`。Policy 文件结构保持不变。

入口为 `ascout-control-plane <pi-root> [--policy <name>]`，交互终端启动前允许改选；非交互不等待。新运行使用 tempfile 的操作系统随机目录名（`run-` 前缀），同一名字用作 run id 和 execution identity；在 `runs_dir` 下原子创建，不接受调用方传入运行目录。结束输出 run id；`runs` 列出已有目录名，`--resume <run-id>` 恢复并检查保存的身份。

Policy 不新增 backend 列表。`PolicyEngine::confirm_backends` 读取该 Policy 已引用的 Runtime 配置，以单个 `backend` 或 `backends` 数组中各项的 `id` 对照 Runtime hello 的 `capabilities.backends`；没有引用 Runtime 或配置未声明 backend 时不检查。此检查不改变 Policy 摘要或编译产物结构。失败走既有 Adapter teardown。接口解析仍属于 Runtime，不构成环境保证验证。

调用方迁移：主入口、回放入口和五份本地 fixture 已切换到配置/session 路径，fixture 使用私有 HOME 配置、不写用户主目录。fixture 在启动 Runtime 前同步复制 workspace，消除 stderr 通知后的复制竞态。恢复中的 `launch evaluation: Broken pipe` 根因是缓存 worker 的 `--die-with-parent` 随创建线程退出杀死进程；由 Worker 的 Drop 负责清理后，新增线程退出回归测试通过（原实现先复现失败，日志 `/tmp/ascout-worker-before.log`）。最终 `cargo check --all-targets`、当时的 Rust 测试集 通过；`node --test tests/recovery-active.mjs tests/recovery-filesystem.mjs` 6/6 通过，日志 `/tmp/ascout-recovery-final.log`。回放新 session 必须选 `--policy NAME`，不加载源 session artifact；下游外部脚本仍需按新接口核查。

`predict.chance_context` 携带工具参数，`process_context` 携带 `observation`、`steps_completed`、`authority_active_tools` 和与 `select.scheduling_context.active_branches` 同构的 `active_branches`。策略可用 `history: {mode: transcript_tail, max_entries: 30}` 声明历史投影，或用 `match.outcome: tool_output` 声明真实输出。`rebalance` 的分支视图另提供 `started_at_ms`（毫秒整数）与 `speculation_kind`（`model_inference` 或 `tool_execution`）。`scheduling_context.authority_recent_actions` 复用有界的最近已完成动作摘要，包含 `timestamp_ms`、`tool_name`、`arguments` 和 `status`。`authority_active_tools` 仅列出正在运行的工具及其 `tool_name` 和 `arguments`。阶段识别和是否抢占由 Policy 决定。工具完成会刷新调度事实，即使分支状态没有变化。

Coding Policy 在 `explore` 阶段发现活跃的模型探索分支时返回 `abstain`。`modify_ready` 的改后主动测试与 `verify` 的工具投机各自按自己的条件执行。已完成的 shell 操作需要有明确的写入或修改特征才影响修改阶段判断。只读 Python 检查可以使用 `-c` 或 heredoc。

Coding Policy 每批最多交付 4 项结果，合计最多 8,000 字符，并保留完整的文件或命令输出。`yield_only` 候选停止后，已有的新产出在权威下一次模型推理边界或任务收尾时批量结算，随后回收候选。无新产出时可以直接回收。

### Coding 策略的三阶段循环

读取探索默认用快模型定位代码。只交付扣除权威已请求部分后的只读成果，运行中用 `Yield`，停止或抢占时用 `Salvage` 回收剩余通知。权威完成修改、候选工具失败达到阈值或新机会需要容量时，策略按调度事实回收旧分支。

权威修改后若还没有可用的工作区主验证命令，默认不启动候选。找到命令后，策略可直接启动工具并停在下一次模型推理之前。对验证边界的结果预测使用普通工具输出样式，候选可继续推理和修改，直到再次验证、触发退化止损或达到步数上限。命中的独立小模型失败分支只交付可用成果并保留权威会话。批量通知按完整文件或命令输出计入预算，重复验证命令按签名去重。

# runtime proxy 接入指南

本文面向 Runtime 接入实现作者，说明如何准备能力声明、实现环境操作、参加 Registry 验证、完成注册并处理运行期偏差。runtime proxy 与 Registry 都属于 Runtime：前者呈现接入契约，后者独立校验能力并生成验证证据。正式语义和接入条件见 [runtime proxy 契约设计](../design/runtime-adapter-contract.md)。Agent socket、UI HTTP/SSE 与 Runtime ControlSession 已归入 `src/control_plane/gateway/`；现有通信检查不等于通用认证授权或 Registry 验证已经完成。实现与验证边界见 [Gateway 实现核查](gateway.md)。

## 1. 接入边界

### 当前接入范围

`BackendRegistry` 读取启动 JSON，登记节点可用的 backend 实现与入口，核对接口和能力声明。`BaseRuntime` 根据登记项创建实例，管理生命周期、Environment 关联与操作结果。已有内置单后端、组合路由及文件系统配置继续可用，`backends` 也可以[登记独立可执行程序](executable-backend.md)。当前控制握手提供能力与 backend 身份，Policy Engine 检查所需身份是否出现。跨节点加载、热加载和独立能力验证仍待实现。

### 当前环境实现

目标语义见 [Runtime 设计](../design/runtime.md)。Linux 文件系统 Backend 通过 bubblewrap 创建 mount/user namespace，并在其中把宿主根目录以只读方式挂载，再用 OverlayFS upper/work 层承载写入。当前进入该 namespace 的是环境 holder 及其执行的受管命令，不是 Agent／Instrumentation 宿主；受管命令及其后代继承同一文件系统视图。隔离不是改变 cwd 或复制目录，Agent 工具访问须经已适配的 Runtime 入口。当前实现了 overlay 与 btrfs 两种数据面、逐挂载 overlay、holder 的 cgroup 冻结与 live mount accept；实现范围、已知偏差和需高权限验证的项目见 [文件系统 Backend 当前实现状态](../fs-backend/current-state.md)。

Overlay 是第一项实现工作，不是所有 Runtime 的必需结构。当前 Backend 的 branch
为父 upper 做独立副本（优先 reflink，失败时复制）或 btrfs 快照，子 lower 追加该
副本；父的 upper、lower 和运行中挂载不变。overlay 的 diff 扫描分支自己的 upper
增量。Full `Promote` 将候选相对分叉基线的增量写到目标，Part `Salvage` 先检查所选路径的冲突。
无 live mount 时直接写 upper。有 live mount 时用 bwrap `--json-status-fd` 得到的内层 sandbox pid，校验 mount
namespace，再通过 `/proc/<pid>/root` 打开受管挂载。`openat2` 把重放限制在该挂载内并禁止通过符号链接解析祖先路径。btrfs 只有在当前权限下 `incremental_diff=true` 时才会被选为数据面，
否则降级 overlay 并在能力/日志中说明。

btrfs diff/accept 共用 send dump 增量视图：从 snapshot 声明取得根前缀，按
`rename <source> dest=<target>` 跟踪临时 inode 名称、真实 rename 和删除，普通
变更统一输出 `change`。真实目录 rename 只读取目标子树以补足没有独立 send
命令的未修改子项，不扫描无关 upper。用户 2026-11-10 第四轮验证已覆盖真实 btrfs
Chain/Flatten/mixed 的 fork、真实进程写入、精确 diff、accept 后真实视图与 discard
清理（`incremental_diff=true fast_delete=true`），以及初始 userns 的 direct root
overlay；4 项特权测试全通过、0 failed、0 ignored，脚本退出 0。accept 场景无
live target，不外推运行中目标重放、全部 overlay 元数据或物化能力；范围见
[特权验证](../fs-backend/priv-check.md)。

受管文件效果、主动 Registry 验证和运行时偏差检测仍有实现缺口。`FilesystemBackend` 拥有文件视图、内部工具计划和每个已启动文件系统环境的 namespace holder。它使用 Linux、bubblewrap、user/mount namespace 和 OverlayFS，隔离不可用时启动失败。holder 的冻结优先使用 cgroup v2，回退到进程组信号。终止会回收进程组并扫描目标 mount namespace 的残留后代。回收失败保留句柄，清理须等进程回收成功后才删除挂载数据。物化先排空写入并停止旧 holder，随后按当前 generation 重建 holder。base runtime 通过 agent supervisor 管理 run 唯一的 Agent 与 Instrumentation 宿主，适用于 FS、NLE、Firecracker 和多 Backend 配置。

BaseRuntime 通过 Backend 接口执行环境操作。接收端检查 Environment 可用性，再由普通 Backend 路由分发。文件系统 backend 使用 holder，Firecracker 使用 vsock guest agent，独立程序使用带内容帧的请求与回复。多 Backend 配置可分别设置 `environment_operation` 和 `invoke_tool` 路由。控制协议提供 `tool_context` 查询。通用操作包括 `read_file`、`write_file`、`replace_text`、`list_directory` 和 `exec`。文件字节、替换文本及 shell 输入输出经独立内容帧传送。Pi 适配位于 `instrumentation/pi/tools.ts`。`tool-plans/<environment>.json` 是文件系统后端内部挂载映射。

runtime proxy 中的生命周期状态门槛是所有 Runtime 共同遵守的契约，不是 filesystem
Backend 的专属规则：operate 只接受 Available；branch 接受 Available 或
Suspended（暂停只阻止在父环境内执行新操作，派生读取父状态是允许的）；
promote 和 suspend 只接受 Available；resume 只接受 Suspended；terminate 接受
Available 或 Suspended；cleanup 只接受 Terminated。这样 Suspended 环境不会
通过 runtime proxy 继续接受新的环境内操作，未启用 filesystem 的 NLE/Firecracker 也
保持相同的 Runtime 生命周期语义。filesystem 的额外检查只存在于其 Backend 和
参与 Environment 的 backend 依照配置顺序协作。

runtime proxy 接入工作的对象是 Runtime 环境。基本操作包括 `establish_environment`、`operate_environment`、`suspend_environment`、`resume_environment`、`terminate_environment` 和 `cleanup_environment`。`branch_environment` 只表示可以派生独立子环境。环境接受通过 `accept_environment` 完成，目标为该 Chance 的直接父级环境，支持范围由 Backend 的共享或隔离状态处理能力决定。暂停顺序和文件级 part 回执见 [Runtime 生命周期](runtime-lifecycle.md#full-与-part-接受)。Runtime 可以声明环境工具。control plane 用 `tool_context` 查询预测所需上下文，Instrumentation 直接在独立执行连接调用工具。

先确认以下事实：

- 当前 Runtime 实例使用了哪些配置；
- 哪些环境可以建立、环境内操作、静止、恢复、终止和清理；
- Instrumentation 如何在 Branch 隔离环境外由 Runtime 建立可信宿主，并访问对应环境的控制入口；
- Instrumentation 无权完成的环境级操作由哪一侧执行；
- 进程、workspace、memory、I/O、network、resource 和环境内控制入口分别有什么边界；
- 哪些路径依赖拦截，拦截是否能在影响发生前覆盖全部声明范围；
- 哪些保证当前无法证明，应标记为未验证或不适用。

不要从 Runtime 的理论能力出发编写声明。声明必须反映当前 runtime proxy 实例和当前配置的实际保证。

## 2. 编写能力声明

固定生命周期操作由契约统一规定；`branch_environment` 是可选能力。能力声明描述这些操作作用于不同内容时形成的实际隔离保证，以及共享环境或独立环境下 Branch 之间的非干扰保证。

runtime proxy 提交的是当前实例、当前 Runtime 配置下的 `AdapterDeclaration`，不是理论能力清单。声明包括契约版本、实例标识、配置标识、各类环境内容的 `ContentIsolation`、Branch 非干扰声明和当前部署条件。Registry 解析声明后生成 `CapabilityView`，并附加验证状态；runtime proxy 不填写验证结果。

Branch 非干扰声明按内容类别给出机制、覆盖入口、范围和限制。机制可以描述独立环境、作用前阻止、写入暂存、写时复制或其他实现；机制名称不是固定枚举，Registry 只验证声明的保证是否在覆盖范围内成立。

每类内容按以下隔离属性整理：

| 维度 | 要回答的问题 |
|---|---|
| `visibility` | 其他环境能看到什么？ |
| `modification` | 其他环境能直接修改什么？ |
| `interference` | 其他环境还能通过什么路径产生影响？ |
| `lifetime` | 静止、恢复、终止和清理如何影响它？ |
| `cleanup` | 清理后是否还有残留？ |
| `scope` | 保证覆盖哪些对象、路径或配置？ |
| `limitations` | 哪些例外和路径不在保证范围？ |

这组属性可以随着契约演进扩展。不要用一个 `isolation_level` 掩盖不同内容之间的差异，也不要用自由文本替代可解析的范围和限制。

每项内容可以标记为 `not_applicable`，但必须仍然遵守接口契约并返回明确结果。`not_applicable`、`unsupported`、`unverified` 和 `verification_failed` 含义不同，不能混用。

同一类内容只提交一项有效声明。范围和限制要能被 Registry 解析；自由文本只能补充说明，不能承担能力匹配的唯一依据。配置变化后重新生成声明并重新接入验证。

## 3. 实现环境操作

对每个操作，明确完成条件和失败收敛：

- `establish_environment` 完成后，必须能证明环境已经可用，才能返回完成；
- `operate_environment` 需要返回实际建立、执行或拒绝的结果；
- `accept_environment` 返回已完成的 full、实际提交的 part 范围，或失败。Control plane 根据回执继续胜者或 AuB。
- `branch_environment` 是可选能力：声明支持时，必须返回一个与父环境、与其他 sibling 互不影响的独立环境；父环境保持可用；派生范围如实报告；不支持时必须返回 `Unsupported`，不得用共享别名伪装；
- 共享环境中的 Branch 非干扰必须覆盖所有可能影响权威路径或其他 Branch 的入口；无法可靠识别或阻止的入口不得纳入已验证范围；
- `suspend_environment` 必须阻止新的环境操作，并说明未完成工作；
- `resume_environment` 必须让原环境重新可用，并保持已有状态和隔离边界；
- `terminate_environment` 必须不可恢复地停止 Runtime 管理的执行；
- `cleanup_environment` 必须处理 Runtime 管理的进程、资源、临时状态、句柄和租约；

多个后端调用组成一个操作时，runtime proxy 要定义中间失败如何暴露，以及后续是否必须 cleanup。不要把请求发送、metadata 更新或本地缓存变化当作 Runtime 已完成。

## 4. 注册和运行

目标流程包括 backend 实现登记及独立验证。当前实现范围见 §1。

```text
固定当前 Runtime 配置
    -> Registry 读取配置并登记 backend 实现、入口、接口与能力声明
    -> Registry 解析声明并检查一致性
    -> 通过 base runtime 建立验证环境并执行验证
    -> Registry 发布带验证状态的能力视图
    -> Policy Engine 检查 Policy 是否匹配
```

base runtime 按登记项创建实例并核对实例握手。验证场景与验证结果由 backend registry 管理，runtime proxy 呈现 Runtime 对外的能力视图。

runtime proxy 配置注册后不得在运行期变化。若发现 Runtime 实际行为不符合声明，runtime proxy 必须提供足够证据让 Registry 和 control plane 识别偏差；不能继续返回旧的成功能力。

未验证能力默认不进入普通可用范围。部署方可以主动确认，并把它限制在指定部署范围内使用；runtime proxy 作者必须明确记录这项能力的范围、限制和风险。验证失败的能力不能通过用户确认绕过。

## 5. 运行期偏差

运行期验证是接入验证的兜底，不是替代。发现声明与实际行为不符时，Registry 将能力视图降级到仍然经过验证的范围；control plane 让全部任务使用降级后的范围，协调停止并清理依赖失效能力的投机活动，不依赖的活动可以继续。重新验证通过后才能恢复新的能力范围。

## 6. 实现方向与当前代码

Pi 接入采用宿主单一进程托管全部 Branch session；文件系统 Branch 用常驻 namespace-holder 承载工具命令，不按投机点启动新的 Pi 进程。首个真实 Agent 选择 [Pi](https://github.com/earendil-works/pi)，保留其原有推理循环，通过工具接入、显式会话状态和原生 checkpoint 续跑连接 AScout，本轮不同时适配其他产品。具体会话截取和预测工具结果注入已在 Pi 接入中实现（`checkpointContinuation`、`toolResultInjection`，见 `pi/packages/coding-agent/src/core/pi-adapter.ts`），不把 Pi 的命令级 fork 当作工具等待点分支的现成能力。首要目标是在已有设计约束下跑通真实投机执行，不为更换后端改变上层语义，也不把 Registry 主动验证作为原型启动前置条件；未验证保证仍不得标记为已验证。

进程级实现已落地：`src/runtime/process/` 负责 bwrap 启动、进程组、`--json-status-fd`、cgroup 创建/加入/冻结/解冻/kill，以及内层 pid 与 mount namespace。续跑状态如何交付仍由 Instrumentation 配合，Runtime 负责新承载环境的实际建立、操作及回收；复制、共享或恢复 Session 不自动提供 workspace 隔离或外部效果治理。进程组冻结路径仍未覆盖逃出进程组的后代冻结，跨 Backend 的冻结/快照/解冻编排也待统一，详见 [文件系统 Backend 当前实现状态](../fs-backend/current-state.md)。以下说明当前代码已经存在的 Firecracker 实现。

首轮 Pi 接入允许暂不经过独立 runtime proxy，先在受信接入代码中把候选操作限制为无本地业务副作用的范围，拒绝任意 shell、文件修改及未经授权的外部业务效果；系统管理的独立 session、日志及已授权模型请求另行管理。不以切换 cwd 证明 filesystem 隔离，不宣称该路径已经支持会写入仓库的 coding workload。此为实现范围缩减，不改变正式 Runtime 契约或效果治理语义。

### 当前 Firecracker 实现

当前仓库提供主机侧 `ascout-runtime-adapter` 和 guest 内 `ascout-guest-agent`。主机程序通过 `--config <path>` 读取 JSON；首版配置字段为：

- `control_address`：control plane TCP 地址（例如 `127.0.0.1:8180`）；
- `adapter_instance`：本次 runtime proxy 实例标识；
- `firecracker_path`、`kernel_path`、`rootfs_path`：部署方提供的 Firecracker 资产；
- `guest_access_path`：写入每个 rootfs 副本的 `ascout-guest-agent`；
- `state_directory`：各环境的磁盘副本、API/vsock socket、snapshot 和运行日志目录。

配置责任已确定：runtime proxy 只负责接入所需的配置，环境及其网络配置归 Runtime；实际 IP、端口等分配结果属于运行状态。当前 Firecracker 实现用同一份 JSON 承载接入与 Runtime 配置，不改变这些配置的归属，也不表示当前代码已按该边界重构。独立二进制不是语义要求，具体部署方式与配置文件组织由实现按需选择；正式边界见 [runtime proxy 契约 §1](../design/runtime-adapter-contract.md#1-契约边界)。

`operate_environment` 固定通过 Firecracker vsock 调用 Guest Agent，不自动回退到 SSH。SSH 只用于独立诊断。建立环境时 runtime proxy 复制 rootfs 并注入 Guest Agent，不修改部署方提供的原始镜像。

网络是环境的固有能力。建立环境时 Firecracker Backend 通过免密 sudo 创建独立 network namespace、网桥、TAP、veth 与双层 NAT，Firecracker 在该 namespace 内以降权用户运行，进程与文件保持 runtime proxy 用户归属；guest 使用固定 MAC 与地址，默认路由由 Firecracker Backend 在环境就绪后补齐。cleanup 回收 netns、veth 与 iptables 规则，终止按进程组回收 Firecracker 进程链。分支时 Firecracker Backend 短暂暂停父 VM，创建全量 snapshot 和独立磁盘副本，恢复父 VM，再以独立 rootfs、网络与 vsock 路径恢复子 VM。

真实端到端测试位于 `tests/firecracker_e2e.rs`。测试依赖 KVM、Firecracker、kernel、rootfs、`debugfs`、`cp`、`curl`、OpenSSH、`iproute2`、`iptables`，以及允许免密 sudo 运行 `ip` 与 `iptables` 的 Linux 主机；SSH 诊断测试从环境 netns 内发起。测试使用独立镜像副本；因镜像和 snapshot 占用空间较大，应通过 `--test-threads=1` 串行运行。

### 完整操作与实际限制

Pi Integration 独立适配参数、schema 和结果，一次工具调用对应一次完整 Runtime 操作，不复用 Pi 工具实现。操作集合为 read_file、write_file、replace_text、list_directory、exec；backend 声明工具仍走同一生命周期。完整替换在目标环境内依次验证全部匹配后才写入，exactly_one 默认，all 显式启用。受管理写入按 Environment 内 canonical path 串行；不保证 hardlink 合一、外部 symlink 更换或任意 exec 写入也遵守队列。

同一 Unix／vsock 连接双向使用最多 1 MiB 的 JSON 首行。首行有 `body_length` 时，后接原始字节。每帧使用 4 字节大端长度，最多 64 KiB，最后用长度为零的帧结束。没有 body 的消息只发送首行。独立执行访问描述仍使用 `unix-json-lines`，Control Gateway 的帧格式独立。编辑首行只带每步的旧文本长度、新文本长度和匹配模式，UTF-8 内容在同一 body 内依次发送。目录项在 body 内携带 JSON 业务数据，声明工具的终态 body 携带原始 stdout 和 stderr，并用 stdout 长度分隔。

接收端先校验 Branch／Environment 绑定，再将输入分帧写入匿名临时文件。声明长度与终止帧完整后才准入。请求指纹包含首行内容与实际字节，与分帧方式无关。holder 和 guest 接收完整 body 后才执行，写入／替换使用目标同目录临时文件和 rename。输入错误不会截断原文件。该路径未提供目录 fsync 后的断电保证，也不承诺保留 inode 或 hardlink。原始文件读取按指定范围或图片回退从打开的文件分块发送，接收端落盘保存结果，输出事件同步写入每个操作一个匿名重放文件。慢客户端受 socket 写入背压约束，重放文件让进程输出与客户端读取速度解耦。终态确认释放缓存文件，执行身份仍按既有窗口保留。在途发送持有的文件引用随连接释放。暂存容量取决于部署的临时文件系统，包括 tmpfs 配额和可用磁盘。接收端没有固定文件体积上限或按 Branch 的磁盘配额。

行选择仍只累计选区，最多保留 32 MiB，返回 `actual_range` 和 `has_more`。Pi 默认请求最多 2000 行和 50 KiB，超限只返回完整行，空文件、尾部空行、零 limit 和越界 offset 沿用既有语义。同一次 `read_file` 可按 PNG、JPEG、GIF、BMP、WebP 签名回退为完整二进制。原始字节读取与写入不受行选择预算限制。Proxy 当前用内存中的字节数组发起写入，收齐文件结果后返回连续 Buffer。收齐时可能同时保留帧缓冲和连续 Buffer。图片渲染还会分配 Base64，Pi bash 格式化会累计文本。编辑在目标端缓冲当前和更新后的文本及当前替换对，目录排序保留目录项。上述 API 的内存需求取决于实际内容，传输本身按帧分块。

内置 FS／Firecracker 的单操作取消和 timeout 采用尽力终止操作进程组。脱离进程组的后台后代可能继续存活。独立可执行 backend 由程序按 Environment 与 operation identity 停止工具工作，Runtime 等待终态并保留共享实例。执行端点保留 Environment drain、取消 generation、准入关闭与终态确认。独立程序的分帧、超时及故障回收见 [独立可执行程序接口](executable-backend.md#取消单次工具执行)。

终态沿用 `succeeded`、`failed`、`cancelled` 和 `timed_out`。backend 向接收端保留 `invalid_input`、`not_found`、`permission_denied`、`timed_out`、`io_error`。旧 backend 的未分类错误为 `backend_error`。FS 通过当前 holder 映射绑定 cwd，定向用例使用 `/app`。stock VM 使用已有 `/root`，客户端的 `bound_cwd` 由 Runtime 移除或覆盖。两种 backend 都验证默认和显式 cwd。同环境的别名共享写入锁，独立环境可以并行写入相同 canonical path。

本轮定向验证记录

- `cargo test --test runtime_relative_endpoint` 通过。真实 FS Runtime 写入和读回 33 MiB + 17 字节。测试拆分首行和内容帧，验证提前 EOF 时文件不变，重试与重新分帧不会再次写入，同身份不同字节被拒绝。取消后重试按相同顺序返回原始输出和终态，确认后释放结果。
- `cargo test --test firecracker_e2e pi_sdk_tools_cross_the_real_environment_boundary_without_a_model -- --exact` 通过，7 项过滤。真实 VM 走 Pi → Proxy → Rust 接收端 → guest，验证 33 MiB + 17 字节非 UTF-8 往返、多帧编辑文本、二进制 stdin／stdout／stderr、1 GiB 稀疏文件小范围、图片回退和两个 VM 的独立队列。
- `cargo test --lib runtime::adapter` 11 项，`cargo test --lib guest_agent` 6 项，`cargo test --lib range_regressions` 4 项，`cargo test --lib namespace_holder_runs_concurrent_commands_in_each_branch_mount_namespace` 1 项通过。guest 的 socket pair 测试覆盖长度不足的终止帧和进程组取消。
- `node --no-warnings --experimental-strip-types --import ../pi/packages/coding-agent/src/experimental/source-resolver.ts --test tests/pi-filesystem-tools.mjs tests/pi-asc-proxy.mjs` 11 项通过，覆盖 holder 直接收到截断 body、独立环境和代理取消顺序。相同 Node 参数运行 `tests/pi-access.mjs tests/pi-notification-adapter.mjs tests/pi-recovery-binding.mjs tests/pi-recovery-sources.mjs`，17 项通过。`python3 tests/holder-queue.py` 验证同环境别名确实等待同一把锁。

- `cargo check --all-targets`、`cargo build --bins`、Proxy／工具的 TypeScript 严格检查及相邻 Pi 的 `tsc --noEmit -p ../pi/tsconfig.json` 通过。

### Pi 工作目录与退出状态复核

- `node --no-warnings --experimental-strip-types --import ../pi/packages/coding-agent/src/experimental/source-resolver.ts --test --test-name-pattern='Pi paths|exec streams' tests/pi-filesystem-tools.mjs` 在修复前两项失败。相对写入落在 holder 的只读宿主 session 路径，Pi 原生工具包装器收到退出码 7 时缺少 `isError`。日志 `/tmp/opencode/ascout-pi-integration-red.log`。
- 相同 Node 参数去掉名称筛选，运行 `tests/pi-filesystem-tools.mjs tests/pi-asc-proxy.mjs tests/pi-access.mjs tests/pi-notification-adapter.mjs tests/pi-recovery-binding.mjs tests/pi-recovery-sources.mjs`，29 项通过。holder 覆盖相对和绝对路径、链接后的 `..`、默认 cwd 和显式通用 exec cwd。工具包装器保留退出码、输出和失败标记，超时与取消分别验证。日志 `/tmp/opencode/ascout-pi-integration-green.log`。
- `cargo test --test firecracker_e2e pi_sdk_tools_cross_the_real_environment_boundary_without_a_model -- --exact --nocapture` 通过。默认镜像的工作目录为 `/root`，与宿主目录不同。Pi 相对写入通过通用 `read_file` 在 `/root` 下的绝对路径读回，Pi `pwd` 返回 `/root`，通用 `exec` 显式指定 `/tmp` 仍生效。原有二进制往返及终态检查均通过。日志 `/tmp/opencode/ascout-pi-integration-vm.log`。
- `node --test --test-name-pattern='Pi reports a delivered nonzero' tests/authority-filesystem.mjs` 通过。真实 Pi、control plane 和 FS Runtime 使用本地假模型。退出 7 报告 `action_finished: failed` 并保留输出及退出提示，退出 0 报告 `succeeded`，两次写入各发生一次。日志 `/tmp/opencode/ascout-pi-integration-action.log`，事件目录 `/tmp/opencode/authority-filesystem-OmjGwo`。

## 相关文档

- [runtime proxy 契约设计](../design/runtime-adapter-contract.md)：正式接入契约
- [Runtime 设计](../design/runtime.md#runtime-的组件职责)：Registry 的职责与独立验证边界
- [control plane 设计](../design/control-plane.md)：control plane 使用 Registry 的能力事实
- [Instrumentation 设计](../design/agent-integration.md)：可信宿主内 Instrumentation 的责任
- [效果管理设计](../design/effect-management.md)：Runtime 的环境与效果处理责任

## 外部效果能力差距

当前原型尚未提供 Runtime 内 External Effect Coordinator 与 network backend 协同的通用机制：真实请求本机暂存、模拟能力判断、带来源标记的临时结果、不能模拟时保留进展的 Branch 阻塞、成为权威后的真实请求发送与继续，以及丢弃或前提失效后的取消闭环均待实现。目标语义已确定，见 [效果管理](../design/effect-management.md)；组件归属与不能模拟时的处理不再是未决问题。现有本地 Decision 求值与按操作异步反馈路径保留。

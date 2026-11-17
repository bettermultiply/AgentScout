# Runtime Adapter 接入指南

本文面向 Runtime 接入实现作者，说明如何准备能力声明、实现环境操作、参加 Registry 验证、完成注册并处理运行期偏差。Runtime Adapter 与 Registry 都属于 Runtime：前者呈现接入契约，后者独立校验能力并生成验证证据。正式语义和接入条件见 [Runtime Adapter 契约设计](../design/runtime-adapter-contract.md)。Agent socket、UI HTTP/SSE 与 Runtime ControlSession 已归入 `src/control_plane/gateway/`；现有通信检查不等于通用认证授权或 Registry 验证已经完成。实现与验证边界见 [Gateway 实现核查](gateway.md)。

## 1. 接入边界

### 当前实现目标

目标语义见 [Runtime 设计](../design/runtime.md)。首阶段实现了 Linux 文件系统 Backend：通过 bubblewrap 创建 mount/user namespace，并在其中把宿主根目录以只读方式挂载，再用 OverlayFS upper/work 层承载写入。Agent 启动入口在该 namespace 中执行，因此绝对路径、临时文件和全部后代进程继承同一文件系统视图；不是改变 cwd、复制目录或拦截 Agent API。当前实现了 overlay 与 btrfs 两种数据面、逐挂载 overlay、进程 Backend（cgroup 冻结）与 live mount accept；实现范围、已知偏差和需高权限验证的项目见 [文件系统 Backend 当前实现状态](../fs-backend/current-state.md)。

Overlay 是第一项实现工作，不是所有 Runtime 的必需结构。当前 Backend 的 branch
为父 upper 做独立副本（优先 reflink，失败时复制）或 btrfs 快照，子 lower 追加该
副本；父的 upper、lower 和运行中挂载不变。overlay 的 diff 扫描分支自己的 upper
增量；accept 做三方冲突检测并把增量重放到目标：无 live mount 时直接写 upper，
有 live mount 时用 bwrap `--json-status-fd` 得到的内层 sandbox pid，校验 mount
namespace 后以 `/proc/<pid>/root` dirfd 为根、通过 `openat2(RESOLVE_IN_ROOT)`
安全重放。btrfs 只有在当前权限下 `incremental_diff=true` 时才会被选为数据面，
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

尚待实现：受管文件效果的合法生效路径、主动 Registry 验证和运行时偏差检测。Overlay 变更的 accept/冲突处理已实现：三方冲突检测后，无 live mount 时直接写目标 upper，有 live mount 时经 mount namespace 校验与 `openat2` 安全重放。该 Backend 只负责文件系统，不负责进程终止、网络、内存、资源或 Agent Session；进程启动、冻结、终止和句柄由 `runtime/process` 的 ProcessBackend 负责，FilesystemBackend 只产出 `LaunchFragment` 中的挂载条目。它要求 Linux、可执行 bubblewrap、可创建 user/mount namespace 且宿主允许 OverlayFS；任一条件不满足时启动失败，不回退到未隔离启动。宿主 Pi 由 ProcessBackend 作为 run 唯一进程持有；每个文件系统环境另有一个 namespace-holder 进入该 Branch 的 mount namespace。ProcessBackend 将 holder 放入独立进程组，cgroup v2 可用时用 `cgroup.freeze`，否则回退进程组 SIGSTOP/SIGCONT；终止时先按 cgroup/process group 结束，再扫描目标 mount namespace 结束残留后代。cleanup 会先停止 holder，停止失败时保留进程句柄并禁止删除 upper/work；只有停止成功后才清理 inner 和 filesystem，任一清理失败都返回失败，不报告 cleanup 已收敛。物化轮换 generation 前结束旧 holder，发布后建立新 holder。无 context 的 Backend 方法统一转入 context 路径。establish/branch 的组合失败回滚同样先 terminate filesystem environment，再 cleanup；回滚错误追加到原始 BackendError 的 cleanup_failures。BaseRuntime 配置 filesystem 时，generic operate、tool_context 和 invoke_tool 仍只委托 inner Backend；filesystem 只检查环境状态，并在 holder 启动时提供该 Branch 的 namespace/Overlay 视图，不执行同一业务命令。Pi 宿主进程不进入 Branch namespace，文件工具只通过 Branch 专属 socket 由 holder 代执行。

Adapter 中的生命周期状态门槛是所有 Runtime 共同遵守的契约，不是 filesystem
Backend 的专属规则：operate 只接受 Available；branch 接受 Available 或
Suspended（暂停只阻止在父环境内执行新操作，派生读取父状态是允许的）；
promote 和 suspend 只接受 Available；resume 只接受 Suspended；terminate 接受
Available 或 Suspended；cleanup 只接受 Terminated。这样 Suspended 环境不会
通过 Adapter 继续接受新的环境内操作，未启用 filesystem 的 NLE/Firecracker 也
保持相同的 Runtime 生命周期语义。filesystem 的额外检查只存在于其 Backend 和
启用它的 BaseRuntime 包装层；未配置 filesystem 时这些检查不改变 inner Backend
行为。

Adapter 接入工作的对象是 Runtime 环境。所有合格 Adapter 都必须支持 `establish_environment`、`operate_environment`、`promote_environment`、`suspend_environment`、`resume_environment`、`terminate_environment` 和 `cleanup_environment`；`branch_environment` 是可选能力，只表示派生独立子环境。`promote_environment` 没有环境侧语义时也必须接受并成功返回，不能把 no-op 当作不支持。Runtime 也可以声明需要在真实环境中执行的环境工具；control plane 通过 `tool_context` 和 `invoke_tool` 调用，不解释工具的业务语义。能力声明描述这些操作对不同环境内容形成的实际隔离保证，以及 Branch 之间的非干扰保证。

先确认以下事实：

- 当前 Runtime 实例使用了哪些配置；
- 哪些环境可以建立、环境内操作、静止、恢复、终止和清理；
- Instrumentation 如何在 Branch 隔离环境外由 Runtime 建立可信宿主，并访问对应环境的控制入口；
- Instrumentation 无权完成的环境级操作由哪一侧执行；
- 进程、workspace、memory、I/O、network、resource 和环境内控制入口分别有什么边界；
- 哪些路径依赖拦截，拦截是否能在影响发生前覆盖全部声明范围；
- 哪些保证当前无法证明，应标记为未验证或不适用。

不要从 Runtime 的理论能力出发编写声明。声明必须反映当前 Adapter 实例和当前配置的实际保证。

## 2. 编写能力声明

固定生命周期操作由契约统一规定；`branch_environment` 是可选能力。能力声明描述这些操作作用于不同内容时形成的实际隔离保证，以及共享环境或独立环境下 Branch 之间的非干扰保证。

Adapter 提交的是当前实例、当前 Runtime 配置下的 `AdapterDeclaration`，不是理论能力清单。声明包括契约版本、实例标识、配置标识、各类环境内容的 `ContentIsolation`、Branch 非干扰声明和当前部署条件。Registry 解析声明后生成 `CapabilityView`，并附加验证状态；Adapter 不填写验证结果。

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
- `promote_environment` 完成时表示 Candidate 环境已成为权威环境；共享底层环境的实现可以 no-op，但失败必须如实返回，control plane 不会在失败后切换 Agent authority；
- `branch_environment` 是可选能力：声明支持时，必须返回一个与父环境、与其他 sibling 互不影响的独立环境；父环境保持可用；派生范围如实报告；不支持时必须返回 `Unsupported`，不得用共享别名伪装；
- 共享环境中的 Branch 非干扰必须覆盖所有可能影响权威路径或其他 Branch 的入口；无法可靠识别或阻止的入口不得纳入已验证范围；
- `suspend_environment` 必须阻止新的环境操作，并说明未完成工作；
- `resume_environment` 必须让原环境重新可用，并保持已有状态和隔离边界；
- `terminate_environment` 必须不可恢复地停止 Runtime 管理的执行；
- `cleanup_environment` 必须处理 Runtime 管理的进程、资源、临时状态、句柄和租约；

多个后端调用组成一个操作时，Adapter 要定义中间失败如何暴露，以及后续是否必须 cleanup。不要把请求发送、metadata 更新或本地缓存变化当作 Runtime 已完成。

## 4. 注册和运行

注册流程如下：

```text
固定当前 Runtime 配置
    -> 提交当前实例的能力声明
    -> Registry 解析声明并检查一致性
    -> Registry 建立验证环境并执行验证
    -> Registry 发布带验证状态的能力视图
    -> Policy Engine 检查 Policy 是否匹配
```

Adapter 需要向 Registry 提供建立验证环境、执行环境内操作和观察验证结果所需的接入信息。验证场景和验证结果由 Registry 管理；Adapter 作者不提交自测结果作为接入证明。

Adapter 配置注册后不得在运行期变化。若发现 Runtime 实际行为不符合声明，Adapter 必须提供足够证据让 Registry 和 control plane 识别偏差；不能继续返回旧的成功能力。

未验证能力默认不进入普通可用范围。部署方可以主动确认，并把它限制在指定部署范围内使用；Adapter 作者必须明确记录这项能力的范围、限制和风险。验证失败的能力不能通过用户确认绕过。

## 5. 运行期偏差

运行期验证是接入验证的兜底，不是替代。发现声明与实际行为不符时，Registry 将能力视图降级到仍然经过验证的范围；control plane 让全部任务使用降级后的范围，协调停止并清理依赖失效能力的投机活动，不依赖的活动可以继续。重新验证通过后才能恢复新的能力范围。

## 6. 实现方向与当前代码

Pi 接入采用宿主单一进程托管全部 Branch session；文件系统 Branch 用常驻 namespace-holder 承载工具命令，不按投机点启动新的 Pi 进程。首个真实 Agent 选择 [Pi](https://github.com/earendil-works/pi)，保留其原有推理循环，通过工具接入、显式会话状态和原生 checkpoint 续跑连接 AScout，本轮不同时适配其他产品。具体会话截取和预测工具结果注入已在 Pi 接入中实现（`checkpointContinuation`、`toolResultInjection`，见 `pi/packages/coding-agent/src/core/pi-adapter.ts`），不把 Pi 的命令级 fork 当作工具等待点分支的现成能力。首要目标是在已有设计约束下跑通真实投机执行，不为更换后端改变上层语义，也不把 Registry 主动验证作为原型启动前置条件；未验证保证仍不得标记为已验证。

进程级实现已落地：`src/runtime/process/` 负责 bwrap 启动、进程组、`--json-status-fd`、cgroup 创建/加入/冻结/解冻/kill，以及内层 pid 与 mount namespace。续跑状态如何交付仍由 Instrumentation 配合，Runtime 负责新承载环境的实际建立、操作及回收；复制、共享或恢复 Session 不自动提供 workspace 隔离或外部效果治理。进程 Backend 仍未覆盖逃出进程组的后代冻结，跨 Backend 的冻结/快照/解冻编排也待统一，详见 [文件系统 Backend 当前实现状态](../fs-backend/current-state.md)。以下说明当前代码已经存在的 Firecracker 实现。

首轮 Pi 接入允许暂不经过独立 Runtime Adapter，先在受信接入代码中把候选操作限制为无本地业务副作用的范围，拒绝任意 shell、文件修改及未经授权的外部业务效果；系统管理的独立 session、日志及已授权模型请求另行管理。不以切换 cwd 证明 filesystem 隔离，不宣称该路径已经支持会写入仓库的 coding workload。此为实现范围缩减，不改变正式 Runtime 契约或效果治理语义。

### 当前 Firecracker 实现

当前仓库提供主机侧 `ascout-runtime-adapter` 和 guest 内 `ascout-guest-agent`。主机程序通过 `--config <path>` 读取 JSON；首版配置字段为：

- `control_address`：control plane TCP 地址（例如 `127.0.0.1:8180`）；
- `adapter_instance`：本次 Adapter 实例标识；
- `firecracker_path`、`kernel_path`、`rootfs_path`：部署方提供的 Firecracker 资产；
- `guest_access_path`：写入每个 rootfs 副本的 `ascout-guest-agent`；
- `state_directory`：各环境的磁盘副本、API/vsock socket、snapshot 和运行日志目录。

配置责任已确定：Adapter 只负责接入所需的配置，环境及其网络配置归 Runtime；实际 IP、端口等分配结果属于运行状态。当前 Firecracker 实现用同一份 JSON 承载接入与 Runtime 配置，不改变这些配置的归属，也不表示当前代码已按该边界重构。独立二进制不是语义要求，具体部署方式与配置文件组织由实现按需选择；正式边界见 [Runtime Adapter 契约 §1](../design/runtime-adapter-contract.md#1-契约边界)。

`operate_environment` 固定通过 Firecracker vsock 调用 Guest Agent，不自动回退到 SSH。SSH 只用于独立诊断。建立环境时 Adapter 复制 rootfs 并注入 Guest Agent，不修改部署方提供的原始镜像。

网络是环境的固有能力。建立环境时 Adapter 通过免密 sudo 创建独立 network namespace、网桥、TAP、veth 与双层 NAT，Firecracker 在该 namespace 内以降权用户运行，进程与文件保持 Adapter 用户归属；guest 使用固定 MAC 与地址，默认路由由 Adapter 在环境就绪后补齐。cleanup 回收 netns、veth 与 iptables 规则，终止按进程组回收 Firecracker 进程链。分支时 Adapter 短暂暂停父 VM，创建全量 snapshot 和独立磁盘副本，恢复父 VM，再以独立 rootfs、网络与 vsock 路径恢复子 VM。

真实端到端测试位于 `tests/firecracker_e2e.rs`。测试依赖 KVM、Firecracker、kernel、rootfs、`debugfs`、`cp`、`curl`、OpenSSH、`iproute2`、`iptables`，以及允许免密 sudo 运行 `ip` 与 `iptables` 的 Linux 主机；SSH 诊断测试从环境 netns 内发起。测试使用独立镜像副本；因镜像和 snapshot 占用空间较大，应通过 `--test-threads=1` 串行运行。

## 相关文档

- [Runtime Adapter 契约设计](../design/runtime-adapter-contract.md)：正式接入契约
- [Runtime 设计](../design/runtime.md#adapter-registry)：Registry 的职责与独立验证边界
- [control plane 设计](../design/control-plane.md)：control plane 使用 Registry 的能力事实
- [Instrumentation 设计](../design/agent-integration.md)：可信宿主内 Instrumentation 的责任
- [效果管理设计](../design/effect-management.md)：Runtime 的环境与效果处理责任

## 外部效果能力差距

当前原型尚未提供 Runtime 内 External Effect Coordinator 与 network backend 协同的通用机制：真实请求本机暂存、模拟能力判断、带来源标记的临时结果、不能模拟时保留进展的 Branch 阻塞、成为权威后的真实请求发送与继续，以及丢弃或前提失效后的取消闭环均待实现。目标语义已确定，见 [效果管理](../design/effect-management.md)；组件归属与不能模拟时的处理不再是未决问题。现有本地 Decision 求值与按操作异步反馈路径保留。

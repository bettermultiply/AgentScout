# 测试删除与合并审查

## 无 NLE 的 FS 真实模型短任务通过（2026-11-16）

付费前发现并复现的纯 FS 装配缺口已按授权修复：正常 Adapter 新增单 filesystem 装配，BaseRuntime 的物化写者检查覆盖纯 FS holder。初始三种配置拒绝证据保留于 `/tmp/opencode/fs-real-20261116-preflight`，其中零请求与阻塞状态只描述当时预检。没有用 NLE 或宿主工具绕过。

修复后本地伪模型全链路证据 `/tmp/opencode/fs-local-20261116-MGZZSb`；唯一真实 session `/tmp/opencode/fs-real-20261116-mfUC97`，21.333 秒成功，11 次模型启动，Authority 4 次工具、候选 1 次只读工具，结果、最终回复、隔离、正常物化和 Shutdown 均通过。5 Candidate／5 Discard／0 Promote，不能声称覆盖晋级或晋级后续跑。Pi 估算 $0.0156748（非账单；取消零 usage 不保证不计费）；9 个 response ID、7 个 completed，5 条 aborted 中有 1 条已 completed 且有完整 usage。统计和完整边界见 [参考负载](reference-workload.md)。

离线构建、文件工具 5/5、Runtime 单元测试 141/141 通过。未提交、未派生 subagent，保留已有修改与旧证据。纯 FS 入口用例目前仅在临时 runner 中，尚未纳入仓库持久回归；主 Agent 需审查这两处最小改动及该测试缺口，不再自动付费补验 Promote。

## 重新授权真实调用后的阻塞交接（2026-11-16）

单次真实 session 已执行，证据 `/tmp/opencode/runtime-id-real-success-20261116`，完整统计见 [参考负载](reference-workload.md)。真实模型 2 次均成功，真实 NLE 西移 1 次、compatible／Promote Applied 1 次，两个 Branch 取得不同 Runtime 返回 ID；随后候选模型结果的 continuation hold 未收到释放命令，第二个工具调用挂起。120 秒截止取消，未完成 5 步与最终总结，因此仍不得宣称真实模型多步 E2E 通过。

本次只做构建、运行和离线检查，不再付费重试、不改实现或新增测试；需主 Agent 后续用手工事件／本地模型稳定复现“工具候选 Promote 后模型完成却无 continue”的路径，再修复。旧失败证据与全部工作树修改保留，未提交、未派生 subagent。估算费用 $0.0060204（非服务账单），两 Branch 和自有进程正常清理，相关端口释放。

## Predictor fixture 与文件工具后代等待收口（2026-11-16）

两份 predictor 测试默认改读仓库 `tests/fixtures/predictor-fact-policy.json`，不再读本次临时证据目录。与原失败留档逐字段比较，除 `source` 与 `runtime_config_path` 脱敏外完全相同；未修改 Policy 语义、绑定或原 digest。共享 helper 按 Rust 编译产物的顺序验证 executable digest，E2E 重新编译后检查原 YAML、源码、绑定及 runtime；Python 完整版本属于 digest 输入，不同 Python build 时仅允许这一字段解释摘要变化。fixture 无认证、prompt、模型回复。其余输入为仓库源码、相邻 Pi、现有 NLE venv 与构建后二进制，临时 run 只保存输出。原真实失败证据不动。

文件工具复现日志 `/tmp/opencode/filesystem-before-fix-20261116.log` 保存原第 100 行 `1 !== 0`。根因不是 namespace supervisor 漏等后代，而是临时 bwrap 路径根本没有代码注释和测试所说的串行队列：`early` 和 `after` 同时 spawn，`after` 在 `late` 写入前执行。最小修复是在该路径加每 Branch 工具集合的 promise 队列，前项等 namespace `close` 后才放行下一项；错误吞在队列尾部但仍向原调用者报告，失败写入不会堵死队列。取消/超时仍杀 namespace 入口，PID namespace 清理跨 process group 后代；stdin EPIPE/ECONNRESET 仍记录到 close 后报告，不提前释放队列。未延长 sleep、未改或删原断言。

必须区分常驻 holder：`src/runtime/process/holder.py`、Rust `namespace_holder_runs_concurrent_commands_in_each_branch_mount_namespace` 测试和 reference-workload 已明确规定连接并发、直接 shell 结束可返回、后台工作归环境管理。本次不将临时 namespace 队列套到 holder，也不改变它的后台工作/取消契约。上述 Rust holder 并发测试本轮通过。两条路径的生命周期不同，没有新增全局语义或技术选型。

最终本地结果：

- `node --test tests/predictor-fact.mjs tests/pi-filesystem-tools.mjs`：9/9，`/tmp/opencode/closeout-focused-20261116.log`。
- `node --test tests/pi-*.mjs tests/predictor-fact.mjs`：35/35，`/tmp/opencode/closeout-pi-core-20261116.log`，含取消、超时、失败写入恢复、EPIPE/早关管道和 setsid 后代等待；全部原断言保留。
- `node --test tests/predictor-fact-e2e.mjs`：1/1，`/tmp/opencode/closeout-local-e2e-20261116.log`；run `/tmp/opencode/predictor-fact-e2e-afEJS9`，3 次 loopback 伪模型请求、3 个模型 action、4 次准入、1 个 Candidate、真实西移动作成功、退出码 0。
- `cargo test --offline --lib --bins`：首次 lib 为 169/170，btrfs snapshot-copy 子环境 overlay mount 报 `Device or resource busy`，完整失败证据 `/tmp/opencode/closeout-rust-core-20261116.log`。单项复跑 1/1，`/tmp/opencode/closeout-rust-btrfs-rerun-20261116.log`；不据此抹去首次失败，也未在本轮修改 Rust 挂载逻辑。bin 单独运行通过，日志 `/tmp/opencode/closeout-rust-bins-20261116.log`。
- `node --check agent-access/pi/filesystem-tools.ts`、`git diff --check` 通过。

没有调用真实模型、没有再启动 subagent、没有提交，原有共享修改均保留。**真实模型 E2E 仍未通过，未重跑付费验证。**

## 首次模型请求失败的必要回归（2026-11-16）

本次仅因已发生且可稳定复现的 `has_predictor` 循环依赖新增测试，遵守先红后绿：`tests/predictor-fact.mjs` 以真实失败 session 保存的 Policy 和手工事件复现模型/工具两路，`tests/predictor-fact-e2e.mjs` 以同份 Policy、正常控制面/真实 Runtime/Pi 和本地 SSE 验证首次请求通过。两个脚本的留档依赖、日志、误断言修正及未通过的已有文件工具测试，均记于 [验证记录](reference-workload.md)。没有引入真实模型调用或通用测试框架，没有改动既有文件工具的并行工作；原真实 E2E 仍未通过，保留原失败证据待主审复核。

## 范围与执行约定

本轮只清理有直接证据的重复测试和重复资源启动，不改产品行为、不修复路径、不移动真实模型测试、不提交。开始时已检查 git 状态与根 AGENTS.md；工作区已有大量修改及未跟踪测试，均按现状保留。仓库内未发现其他 AGENTS.md。

审查入口包括 src 内全部 Rust 测试声明、tests 下脚本与 Rust 入口、tools/replay/verify、scripts 下测试检查脚本和 Cargo/package/Makefile 入口。静态清单中 src 有 28 个含测试文件、162 个测试声明；合并后的 tests 有 17 个含测试声明文件、44 个声明；当时 replay 有 4 个测试文件、11 个声明；本次清理后为 3 个测试文件、8 个声明。参数循环会展开更多案例；这不是运行数量，也不是全部测试通过的声明。对候选重复项进一步核对断言、调用方与文档记录；其余按场景清单保留，不声称已逐行证明所有测试都不可再简化。

## 已实施的决定

1. `tests/firecracker_e2e.rs:294`：将独立 establish smoke 测试合并进完整 vsock 生命周期测试，进程存在断言移到 establish 成功后、首次 guest 操作前（第 307 行）。两者原来同用 AdapterHarness::start、同一建立请求与 Available 断言。hello 协议版本、adapter instance 仍由共同 harness 检查；完整测试继续检查 suspend/resume、guest 磁盘内容、terminate、cleanup 和进程/目录回收。减少一次 Adapter 启动、独立 rootfs 副本和 microVM 建立。覆盖断言不减少；失去的是独立 smoke 的测试名和单独失败报告，且其结果现在受较长生命周期案例整体结果影响。
2. `tests/tree-layout.mjs:13`、`:17`：原独立 stem placement 六项案例及全部断言原样移入布局测试，共用现有 Vite server 与 after close，删除原独立文件。保留无邻居、同侧切换、双侧间距、固定角度、距离范围、排序与确定性覆盖；布局原有八项也保留。减少一个 Node 测试文件与一次 Vite loader 创建，失去独立文件运行入口。用户已有 normalizeTuning 修改保持不变。相关开发文档中的测试位置同步更新。

## 候选但未处理

| 候选 | 证据 | 建议及未处理原因 |
| --- | --- | --- |
| `tests/pi-access.mjs:18` 的五种 shutdownAt | 每种都建立协议客户端与伪造 Pi，包含大量共同基础流程 | completed、tool_wait、candidate_gate、admission、EOF 是不同停止时机；合为一次进程将削弱独立退出状态验证。保留，可后续仅抽取共同夹具。 |
| `tests/tree-layout-cache.mjs:4` 与 `tests/tree-layout.mjs:8` | 都创建 Vite server 并加载布局模块 | 可继续合并以减少一次 loader，但 cache 包含 store/global window 与模块缓存状态，需专门检查相互影响；本轮只合并无这类状态的 stem placement。 |
| `tests/firecracker_e2e.rs:564` SSH 与 `:294` vsock 生命周期 | 都启动真实 VM，SSH 还有独立诊断通道 | 不能用 vsock 成功证明 SSH 通路成功。共享 VM 会绑定两种故障定位，保留；未运行任一真实 VM 测试。 |

## 保留不同故障与重要回归

- Firecracker E2E 的重发 establish（359）、运行中 suspend（387）、kill 事件（451）、siblings 内存/磁盘继承（490）、SSH（564）及现存 Codex（599）未删除或移动。Codex 测试未执行。
- Firecracker mock 测试中跨 ID 不阻塞、同 ID 清理复用等待、恢复失败、snapshot 与 resume 同时失败、停止/网络清理失败保留重试资源，是不同故障点；不因共用 harness 删除。
- Filesystem `ops.rs:525/530` 的 overlay 与 btrfs round trip 使用不同机制；`privileged.rs:148/153` 则明确要求真实高权限 btrfs chain/flatten，与普通回退路径不同。`ops.rs:299/372/416/473` 的 replay 注入失败、opaque、目录替换、rollback 失败保留。
- Filesystem authority/materialize/recovery、BaseRuntime、adapter、主入口测试分属不同边界。后台 writer、部分发布重试、receipt、存档损坏、配置资源变化等覆盖不按名字相似合并。工作区新 materialize 和 authority 测试未删。
- Pi history 的 reused call ID、off-path checkpoint、parallel result、nested accepted path、compaction/custom、rollback Git identity 以及 standalone continuation 分别保留；notification 单元交付与主入口命令重传/失败是不同边界。恢复的 none/retry/continue/multiple/filesystem 五个参数不是重复输入。
- transport 的读/写大小限制、contract 序列化、gateway 队列有界、Policy 编译与 proposal 顺序、NLE 初始化与地图历史、priv-check 的伪造 runner/输出检查、replay 的取消超时/断开/源文件保护等均保留。本轮未以测试数量或文件大小作为删除依据。

## 离线验证

- `cargo check --offline --test firecracker_e2e` 通过；只编译检查目标，不启动 VM、Codex 或模型。
- `node --test tests/tree-layout.mjs`：14 passed、0 failed、0 skipped。只使用现有本地依赖与 Vite middleware，不调用模型或外网。
- replay 本次清理后执行 `npm test --prefix tools/replay`：12 passed、0 failed、0 skipped；`npm run check --prefix tools/replay` 通过。仅使用本地手造记录与 loopback 服务，未调用真实模型或访问外网。
- 修改文件的 `git diff --check` 通过。

未执行完整 cargo/node 测试集，也没有声称真实 VM 合并后的端到端运行已验证。

## Authority 文件工具专项审查（2026-11-14）

本轮只修复已复现的大内容传输错误，不提交、不回滚共享修改、不重复完整测试、不调用真实模型。`tests/pi-filesystem-tools.mjs` 通过伪 SDK 调用真实 `filesystemTools`，使用独立 `/tmp/opencode/pi-filesystem-tools-*` 目录与真实 bwrap 私有 PID namespace；将测试 view 挂到宿主 workspace 路径，确认工具只修改 view。

原实现把整个 write/edit 内容 base64 编码后加入 `bash -c` 的单一参数。先新增回归再运行 `node --test tests/pi-filesystem-tools.mjs`：write、edit 均在 `filesystem-tools.ts:37` 抛 `spawn E2BIG`，证据目录 `/tmp/opencode/pi-filesystem-tools-3j6LwD`。修复为原始 Buffer 通过 stdin 管道传入隔离进程，不创建宿主临时内容文件。stdin 错误只记录，仍等待进程 close 后释放串行队列，以保留 namespace supervisor 等待全部后代结束的协议。

回归覆盖大 UTF-8 内容 write/edit/read 往返、宿主文件保持不变、取消与超时的大输入、失败写入后的队列恢复，以及关闭 stdio 并 setsid 的后台后代等待；取消也检查另一个进程组的后代不会继续写入。针对性命令为 `node --test tests/pi-filesystem-tools.mjs`；语法检查为 `node --check agent-access/pi/filesystem-tools.ts` 和 `node --check tests/pi-filesystem-tools.mjs`。

后续修复（2026-11-14）：

- `materialize.py` 目录路径固定改用 `O_PATH|O_NOFOLLOW`，权限用 pinned fd 的 `/proc/self/fd` 路径更新，避免读取目录或临时放宽权限。新增普通用户回归：0300 根目录/嵌套父目录可写入文件而权限不提前变化；0300 目录可设置为 0755；后续 symlink 目录边界失败时保留此前回执，去除边界冲突后重试完成。原始 0300 PermissionError 复现已修复。目录子树删除仍要求目录可读，失败保留回执供重试。
- `filesystem-tools.ts` read operations 现在在对应 Branch 隔离环境调用 Pi 自有 `detectSupportedImageMimeTypeFromFile`，二进制读取仍经 Branch 的 `readFile`。伪 SDK 回归用隔离 view 中的 PNG 与相反内容的宿主同路径验证 MIME 检测、二进制读取和文本路径；`node --test tests/pi-filesystem-tools.mjs` 5/5 通过。直接导入 Pi `read.ts` 仍被缺失的 `@earendil-works/pi-tui/dist/index.js` 阻止，故本回归没有执行 SDK 的 `processImage`/最终内容构造；未更改 Pi 工作区或依赖。
- 验证：新增物化回归先失败后通过；materialize 定向测试 10/10、`cargo check --offline --all-targets`、`cargo build --offline --bins` 通过。Authority/recovery 七项端到端以本地伪模型运行，7/7 通过，未调用真实模型。证据目录为 `/tmp/opencode/authority-filesystem-aSg3ZK`、`recovery-active-{DvwWdc,Leke3s,eBiB8P,xkYuw2,h1UF5T}` 和 `recovery-filesystem-xtNNn3`（均位于 `/tmp/opencode`）；文件工具最后一次证据为 `pi-filesystem-tools-{vkRZut,EjGoOh,wk9ynC}`。

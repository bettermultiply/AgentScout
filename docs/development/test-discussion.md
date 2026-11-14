# 测试删除与合并审查

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

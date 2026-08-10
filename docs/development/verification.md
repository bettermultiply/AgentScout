# 验收状态

ASCout 当前有可构建 Rust workspace skeleton，但没有可用 lifecycle、isolation、security 或真实 backend。旧原型 test matrix 已随源码删除，不能作为新架构的实现证据。

## 当前证据

- 旧实现、tests、examples、header、构建产物和实现专属文档已经不存在；
- 目标语义与 ownership decisions 仍有正式文档；
- 当前 Rust workspace skeleton 已创建；
- `pbw-harness` 当前已有最小 BranchTree/BranchNode 结构与 metadata operation 证据：BranchTree 保存 root 与 BranchId lookup，BranchNode 维护直接 parent、direct children、disposition 和 commit 标记；branch、commit、discard subtree、accept、context 和 children 已有 Harness-side 单元覆盖；
- `pbw-harness` 当前已有 Branch-local Local Controller request/binding 到 central Harness 的 resolved control request 证据：AgentControlIntent 不携带 ASCoutAgent/Branch identity，BranchLocalControlEndpoint 附加 endpoint-bound context，Harness Entry API 将请求交给 Harness Core，Harness Core 通过 ASCoutAgentRegistry 验证 Branch 归属后派发；
- `pbw-cli` / `pbw-local-control` 当前已有最小 HTTP Local Controller 链路证据：临时 CLI 命令 `context`、`children`、`branch`、`commit`、`accept` 和 `discard` 会向约定地址发送 HTTP intent；Local Controller 用自身绑定解析为 resolved request；集成测试覆盖同一 CLI 请求打到不同 controller 返回不同 branch context，以及 branch/children/commit/discard/accept/context 的 Harness-side metadata lifecycle path；
- 当前没有任何 Runtime-backed ASCout lifecycle、isolation、security 或真实 backend mechanism 被标记为已实现；已实现内容限于 Harness-side metadata control path。

## D-021-I Skeleton Gate

一次性 skeleton 只有同时满足以下条件才算完成：

- [x] 当前 workspace crates 都能作为 workspace artifacts 构建；
- [x] formatting 与 workspace tests 通过；
- [x] dependency checks 能拒绝 contract/API/core 的反向依赖；
- [x] `pbw-cli` 作为薄 CLI adapter 构建；不依赖、选择、构造或连接具体 Runtime；
- [x] HTTP Local Controller 最小 server/client 与临时 CLI 命令已有测试证据；该证据只覆盖 Harness-side metadata，不覆盖真实 Runtime lifecycle；
- [x] ASCoutAgent start path 在 Harness Entry/Core 内注册 Harness-side root metadata；Harness Core 到 Runtime Service 的真实连接和 start handshake 尚未实现；
- [x] 所有 Runtime-backed behavior placeholder 都在创建 Runtime state、process、resource、Workspace change 或 external effect 前失败；Harness-side BranchTree metadata operations 属于当前明确实现范围；
- [x] manifest、source path、test、generated artifact 和文档链接均不依赖已删除原型。

2026-07-31 本地验证命令：

```text
cargo fmt --all --check
cargo build --workspace
cargo test --workspace
bash scripts/check_dependency_direction.sh
```

2026-08-01 跟进验证：`pbw-cli` 的 D-021-I skeleton wiring 已移除对 `pbw-runtime-default` / `pbw-runtime-api` 的直接依赖；Harness start path 在 Harness Core 内注册 ASCoutAgent root metadata，以保持 CLI 与具体 Runtime 解耦。同日补充 `pbw-harness` BranchTree/BranchNode 单元覆盖，验证 root lookup、direct parent/child link、grandchild parent 不被误认为 root、parent discard children 后继续 branch、accept committed direct child 后的 non-root/root tree rewrite 与 sibling discard；并补充 resolved control request 单元覆盖，验证 Branch-local endpoint 绑定上下文、Harness 拒绝不属于该 ASCoutAgent tree 的 Branch、以及 Context/Branch/Commit/Discard/Accept 派发路径。

2026-08-01 Local Controller 跟进验证：`crates/pbw-cli/tests/local_controller.rs` 覆盖 Controller determines branch 和 Metadata lifecycle path。测试通过临时 CLI 命令和 HTTP Local Controller 驱动 Harness Entry/Core，只验证 ASCoutAgent/BranchTree metadata，不验证 Runtime lifecycle。

Skeleton gate 不验证 Branch lifecycle、sibling isolation、security、Workspace、Input、Effect、Outcome 或真实 Agent execution。这些能力必须在当前树中同时具有实现和 conformance evidence 后，才能在 roadmap 标记完成。

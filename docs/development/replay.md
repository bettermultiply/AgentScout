# 模型输出记录与回放

## 2026-11-14：assistant null 正文兼容修复

本次仅修复 replay 拒绝 Pi 合法纯工具调用请求的问题。Gateway 首错调查的六次本地捕获中，唯一不符合 `isMessage` 的消息均为 assistant `content:null` 加 `tool_calls`；证据为 `/tmp/opencode/gateway-request-probe.log` 和 `/tmp/opencode/gateway-request-probe-0dDZ1C/requests.json`，原调查见 [Gateway 核查](gateway.md#收尾审查与首错归因)。格式拒绝发生在请求匹配及日志之前。

新增 `tools/replay/verify/messages.test.ts`，通过实际 HTTP 发送 Pi 格式并匹配由原生 toolCall/toolResult 上下文生成的键。修前稳定得到 HTTP 400 `messages must be an array of messages`（`/tmp/opencode/replay-null-fix/red.log`）；修后返回 200 SSE、正确正文及 DONE，恰消费一项。另检查 15 种非法消息仍在匹配之前返回 400。`Message.content` 增加 null；边界仅允许带非空 tool_calls 的 assistant 使用 null，仍逐项执行原字段检查。未扩大既有工具调用校验规则。`normalize.ts` 的文本／调用／回复读取已有 string／Array 判断；`recording.ts` 的 checkpoint 有 Array 判断，用户输入仍明确拒绝非文本，无需修改这些消费者。

类型检查通过，全部 Node 回归 15/15；B1、native、已有源 B1-PI 均通过。日志集中在 `/tmp/opencode/replay-null-fix/{typecheck,green,b1,native,b1-pi}.log`。未修改匹配键、原生读取、同键顺序、500ms 延迟、取消处理或任何源记录。

请求命中或 consumed 只表示回复项已被取用，不代表最终回答已经交付。取消请求时序、剩余项解释及最终回答交付仍须结合实际执行验证；不能以格式修复或局部回归推断完整机制整链通过。格式修复的初始工作区快照、独立 diff 和源文件哈希核对保存在 `/tmp/opencode/replay-null-fix/`；没有真实模型调用。

## 2026-11-14：TypeScript 迁移与维护性检查

`tools/replay` 核心与全部 verify 入口现由 Node 26 直接执行 `.ts`；局部 ESM package、`strict` / `noEmit` / `erasableSyntaxOnly` 检查独立于 WebUI，无运行时库及构建步骤。开发依赖版本沿用仓库已有 TypeScript 5.9 和 Node 26 类型；本机检查复用已安装包。命令见 [工具说明](../../tools/replay/README.md)。

删除了 `loadRecording().inputs` 内无人消费的 `id` 字段和仅包装正文的对象，现在直接返回正文字符串；原生 ID 仍用于读取阶段去重。导出 `branch_id` 被 B1／B1-PI 消费，`session_entry_id` 被源序回归消费，均保留；`messageBody` 由跨仓 Pi notification 回归使用，保留。去掉 requestKey 回归中的无效恒等 map，改为 `structuredClone`；JSON map 使用单参数调用，Promise 初始化直接使用 Node 的 `withResolvers`。B1 使用独立临时目录与自动分配端口，避免共享固定目录与端口。既有取消、UTF-8、报告路径保护及上轮删除闲置策略夹具均保留。

验证：`npm run check --prefix tools/replay` 通过；现有 Node 回归 14/14，加 `tests/pi-notification.mjs` 共 15/15；B1、native、B1-PI 通过。native 产物 `/tmp/opencode/replay-native-KpiYwi`；B1-PI 产物 `/tmp/opencode/ascout-replay-b1-pi-pFLH3Q`，两请求命中、并行调用及后续请求解析、报告计数、四子进程退出和两个动态端口关闭均通过。

`/tmp/opencode/replay-ts-run1.mjs` 在 `/tmp/opencode/replay-ts-run1-ruRFrZ` 对已有 `/tmp/opencode/run-1` 跑两个全新环境：每轮 40 hits/consumed、23 成功、17 客户端取消、2 输入完成，misses/exhausted/pending/unconsumed/cancel_timeouts 均 0；启动 pause 零输入、resume 后完成，完成后保持交互。一轮工具 quit、另一轮 AScout API shutdown，双方退出 0 且 recovery 文件存在；源 session/events/policy 哈希未变，运行目录相关子进程无残留。只使用本地录制输出，未接触用户 8180 服务或调用真实模型。

此验收不扩大既有保证：任意源的工具环境重建、扩展未持久化改写、非文本输入、同键碰撞与取消调度仍受原限制。run-1 两轮不能替代机制的全部端到端验收。普通模型／恢复仍使用原系统实现，本轮没有修改对应 control plane 或 Pi 行为。

收尾增加 Runtime 配置及 input-status 响应结构检查、B1 动态端口后，再跑严格类型检查、B1 和 run-1 两轮均通过；最终端到端证据为 `/tmp/opencode/replay-ts-run1-REI1bp`。正式引用跨本仓与 `../pi` 搜索无旧 replay 入口；工具源码无 `any`、`@ts-ignore` 或 `@ts-nocheck`；`git diff --check` 通过。

最终将 `run.ts` 的读取校验状态、提交 prompt、推进自动输入分为内部函数，保留 await 后的暂停／关闭／失败检查及跳过本轮延迟的行为；`waitingSequence` 仍在请求发出前设置，仅在请求成功后递增输入下标，等待序号推进与 ready 后才解除等待。重新运行 `npm run check --prefix tools/replay`、`npm test --prefix tools/replay`（14/14）和 `git diff --check` 通过；本次拆分未新增测试或重新执行真实环境端到端验证，未调用真实模型。

本文从[control plane 观察接口](observation.md)展开，说明系统外的正确性验证工具，不改变 control plane 主线。

## 目标与边界

通用回放工具的已批准目标是：指定已有运行目录，每次重新读取原生 session/events，用已记录的 assistant 输出替换模型服务，由正常系统按真实用户输入及交互顺序重新执行工具，在正常 WebUI 观察全过程。结束或匹配失败后系统不退出，用户可通过正常模型切换继续输入；真实模型只能由用户之后手动选择。回放不评价 Agent、不证明性能收益。

Replay 套件的职责仅限于读取已有运行的模型响应并按顺序提供给被测系统，不恢复或重演原运行的全部状态。套件本身须保证响应供给正确；借助它编写的测试才检查投机执行、Promote／Discard、隔离环境等机制。在投机逻辑和策略不变的前提下，用户要求回放的执行逻辑保持一致；具体判定范围、依据及环境和时序前提仍待讨论，顺序响应的具体交付规则也尚未确定，未批准全局 FIFO。当前同键队列是实现事实，不代替上述规则的确认，见 [Discussion 第 39 项](../../Discussion.md)。

源目录只读。每次直接读取指定源目录，在内存准备回复队列，不生成额外 recording、导出缓存或源记录副本。正常系统使用独立新 run，照常持久化 session/events、Policy、日志和主动退出恢复资料；不把新 run 自动当作下次回放源。没有层数参数、固定 prompt、候选或步数。

导出与伪造端点放在 [`tools/replay/`](../../tools/replay/README.md)，使用 Node 26，只读取已有 run 内 `events.jsonl` 和 `session.jsonl`。记录是系统原生能力，测试套件不执行录制；低层 B1／B1-PI 保留手构造原生数据验证，B1-PI 也可接收已有样例目录。

### 通用入口（2026-11-14）

```sh
node tools/replay/run.ts /path/to/source-run ../pi \
  --runtime /path/to/runtime.json --web 127.0.0.1:8080
```

新 replay session 必须通过 `--policy NAME` 选择配置目录中的 Policy；源 session 的 `policy.json` 仅用于源 session 自身恢复，不可作为启动参数加载。外部工具要求显式 `--runtime`；配置须使用 `control_address` TCP 地址，不得位于源目录。新 session 的 Runtime 可写路径和初始环境由调用方明确提供，工具不会猜测目录布局或拿退出快照当初始状态。

`tools/replay/recording.ts` 每次启动读取原生 session/events，在内存恢复候选请求上下文。外部本地 HTTP 服务以 OpenAI SSE 返回记录回复，通过通用 `--models` 追加模型配置；普通 Pi 模型、认证和扩展仍可用，重复 provider 拒绝覆盖。运行侧不导入测试工具。按 events 文件顺序、原生 ID 去重，仅将已消费 `message.role=user` 的纯文本输入经 `POST /api/request` 送入当前权威；通知不作为用户输入。不重现墙钟时刻、Interrupt 或未消费队列。

HTTP 202 只确认排队。通用 `GET /api/input-status` 返回本次进程已处理输入序号 `sequence`、`ready` 和当前权威输入失败原因 `error`。外部工具等待序号推进及 `ready=true` 后才提交下一输入；可输入要求权威轮次结束、无排队输入或待确认 Promote、不在恢复／关闭／模型切换中，不要求所有候选结束。此接口是当前状态，序号不跨重启保留，不是每条输入的持久回执。

工具终端 `pause` 暂停后续自动输入，当前轮次继续；`resume` 恢复，`status` 查看消费及错误，`quit` 请求正常保存退出。用户手动输入或切换模型前先暂停工具，工具不检测人工操作。自动输入完成静默停输入；缺键、耗尽或请求失败报告具体原因停输入，不退回真实模型，两者均保留 AScout 及服务管理。任一方主动退出时另一方退出，复用正常 Shutdown/save。浏览器模型 API 为 `POST /api/request` 的 `{"action":"model","model":"provider/model"}`；须等权威空闲，不能在 Promote/Runtime 工作/恢复中改模型。解析或认证失败保留原模型和会话；候选保留自己的模型，新候选继承实际 parent。

普通模型请求失败以 turn_failed 报告并结束本轮，权威会话保留；候选执行失败和 Runtime/协议故障仍走原 Branch 生命周期，不扩大为任意故障恢复。主动退出与 `--resume` 保持正常持久路径。当前回放模型本身未持久化队列：退出后若要接着交互，使用普通 `--resume --model provider/model`，不承诺恢复自动回放进度。

**本轮实测**：`node /tmp/opencode/check-external-replay.mjs`，产物 `/tmp/opencode/external-replay-LUunGA`。已有手造原生记录经外部工具→普通 AScout→真实 bash→工具结果续跑，两条历史输入顺序完成；启动暂停期间零轮次、恢复后正常完成；完成和缺键均保持系统；保留的普通 Pi 本地模型可切换继续；AScout API shutdown 和工具 quit 双向退出都正常保存 recovery.json；源记录 SHA-256 不变且没有 recording 缓存。全部模型请求为本地伪模型。扩展环境仍须与源一致，未持久化的上下文改写可能造成缺键。

本轮相关检查：Rust library 148/148、native/B1/B1-PI 通过。完整 Runtime 留档、初始环境自动重建、非文本输入、长历史同键碰撞和取消时序仍不保证；没有确定性调度。普通保存和 resume 仍归 AScout；自动回放进度不持久化。

## 记录来源

### 独立服务报告路径修复（2026-11-14）

`report-path.test.ts` 使用临时手造录制文件，修复前六项中五项稳定复现源文件被报告覆盖（`capture.json`、`report.json`、显式同源路径及其符号链接／硬链接），`recording.json` 原调用通过。默认规则现为：`recording.json` 写同目录 `report.json`，其他文件名追加 `.report.json`；显式或默认报告与源文件为同一文件时启动拒绝。原生 run 默认仍不写报告。修后六项全部通过，当时合计 14/14 通过，B1、native 也通过，全部使用手造记录和本地服务。规则及命令见 [工具说明](../../tools/replay/README.md)。

### 取消请求与响应延迟修复（2026-11-14）

原生明确 aborted 的 assistant 按源 session 顺序保留为取消项，与成功项共同组成同键队列。匹配取消项后等待 AScout 自行取消 HTTP 请求，不产生回答；默认 30 秒未取消以 HTTP 504 `replay_cancel_timeout` 明确失败并停止自动输入。真正缺键／耗尽仍为 HTTP 400，其他 error 不视为取消。成功项每次请求在响应输出前等待一次 500ms；客户端取消会清理等待定时器和响应监听，不再写连接。

`cancellation.test.ts` 的四项取消／延迟回归修前全部失败，修后通过；端到端又发现跨 HTTP chunk 的 UTF-8 字符被 Buffer 字符串拼接损坏，新增真实分块请求回归先得到 400，改用 `req.setEncoding("utf8")` 后通过。归一化及上下文重建规则保持原有行为。

独立真实 NLE 验证 `/tmp/opencode/run1-cancel-STTMVB`：源 `/tmp/opencode/run-1` 的两条输入 `cd8248f0`、`fdbd1515` 顺序完成，`finished=true`、`sequence=2`、`error=null`；40 requests/hits/consumed，23 成功、17 客户端取消，misses/exhausted/unconsumed/pending/cancel_timeouts 均为 0。原首错 `bc8aef78…` 正常命中；中途新增缺键 `813e77c9…` 的 UTF-8 根因也已消除。使用原 `nle-runtime.json` 的初始配置，仅更换 control_address；保留正常 Pi 环境，模型服务均为本地录制回复。源 session/events/policy SHA-256 前后一致。`result.json`、`report.json`、`requests.jsonl`、原生 run 和日志保留证据，`cleanup.json` 确认退出码 0 和正常 recovery 保存。原 8180 实例未操作。

此次证明该源两输入完成与请求消费正确，不宣称任意源的取消时序或同键碰撞均确定。验证中间产物 `/tmp/opencode/run1-cancel-dxvq5D`、`/tmp/opencode/run1-cancel-CF1SI7` 保留 UTF-8 修复前的失败。B1、B1-PI、native 和 8 项 Node 回归通过；没有付费模型调用。

`events.jsonl` 的 transcript 事实保存 Branch id、`session_entry_id` 与 role；正文从 run 内 `session.jsonl`（权威 session 文件）读取。其余 Policy、操作、状态和清理事实仍由 Ledger 保存。必须先写文件、后写引用；允许文件存在未引用记录，不承诺两边原子提交或从 Ledger 恢复完整 Tree。

当前 Pi 接入由 `instrumentation/pi/session-tree.ts` 维护每个 Branch 的原生持久 SessionManager，Candidate 从待决 checkpoint 复制原生记录及 ID，并由 Instrumentation 汇总最终文件。文件使用 Pi JSONL 格式，记录 id 与 parentId 表达记录的父子关系，不是第二棵控制 Tree。

文件尾部的 `customType=authority` 自定义记录（authority 标记）只是给 Pi 重开文件的读取位置提示，使 resume 的叶子落在权威路径末端。它不进入模型上下文、不作权威裁决；与 Ledger 最后一次 Promote 不一致时以 Ledger 为准。2026-11-16 确认当前不做基于 Ledger 历史查询、活动状态重建的恢复工具；已有保存及正常 resume 保留。同一文件不等于同一个运行 session，winner 保留自己的 session 和在飞进展；历史整理不能据此直接替换运行上下文。

### 已确认的历史语义与迁移缺口（2026-11-13）

最终单一权威 session 是经过验证、用真实结果整理后的执行历史，不要求准确复原候选当时收到的预测输入。保留预测输入节点，将候选在该输入之后的第一条后续记录的 `parentId` 改接对应真实结果 ID；原主／丢弃分支继续保留，不额外强制另存完整投机后续路径。运行期采用 Pi 原生完整持久化、多 session 过渡及 Instrumentation 汇总；源分支停止写入且成功汇总后才删除临时文件，仍活跃的 Promote winner 不能删除。当前已接通上述路径；整体验收缺口见接入实现 §4。

整理后恢复上下文可能不同于原始运行上下文；compaction/custom 必须保存，但引用、摘要、在飞推理与 rollback 外部资源关联仍有未决实现差距，详见 [接入实现](agent-access.md) §4。是否追加真实结果通知由 Policy 决定，默认完全不追加，不是空消息。

**模型回放边界**：正常运行仅生成原生 session/events，不为回放计算或持久化匹配键；键仅在 `tools/replay/` 内存或显式导出产物中生成。导出按 events 文件序建立 transcript 原生 ID 的 Branch 归属，以 candidate_created 的 parent/chance 和 chance_created 的 operation_id 核对候选注入批次；由原生 toolCall ID 验证整批（包括并行 sibling），applied Promote 后将第一后续记录的父边仅在内存中接回预测批次尾部。嵌套候选按创建顺序还原。无正文相似度、命名解析或时间近似匹配，源文件不变。

第一后续记录可以是 custom、compaction、context_edit 或 custom_message，不限 assistant。通知以 details.command_id 关联 applied Promote。还原后按最新 compaction 的保留区间、context_edit replacement、branch_summary/custom_message 构造当前 Pi 模型上下文；失败 assistant 排除，当前待决调用的较早同 toolCallId 结果被后续结果替换。缺原生引用、注入批次不全、未知消息角色或无法唯一确定重挂边时明确失败，不加旧格式回退。未持久化的扩展改写及任意 provider 转换不在保证内。

本轮验证：B1 与 B1-PI 通过，后者从已有 `/tmp/opencode/ascout-replay-b1-pi-hyZBIv` 开始导出、经真实 Pi stream 消费两个输出，hits/consumed=2、misses/exhausted/unconsumed=0；`native.ts` 覆盖自定义首后续、反序并行完成、嵌套、compaction/edit/notification、时钟重置、源文件不变与歧义报错。已有 `/tmp/opencode/recovery-active-BITw2r` 并行恢复样例导出成功，candidate-2 的 4b5fdb14 上下文使用预测批次 02901c7e → 204d72b4。所有模型请求仅到 loopback。

失败 Branch 对有效执行、成功产物和副作用的影响须等效于没有创建过，但审计记录必须存在；旁支不得重新执行或对权威生效。

## 匹配

- 工具调用投机：待决工具调用的名称与参数，加预测内容本身。
- 模型调用投机：最新一条消息的内容，可取内容哈希。
- 不使用 Branch、Proposal 或工具调用的跨运行 id，不比较完整历史。
- 导出沿内存中还原后的原生父关系取得特征，不直接使用最终权威父链。
- 同键对应多条录制时，按录制出现顺序依次返回。缺失或耗尽必须报错，不得退回真实模型。

长分支中两类键都可能碰撞：相同命令与预测可能对应不同历史，相同最新消息也可能对应不同模型请求。并发到达次序和重试还可能改变同键消费顺序；不能把“有匹配”当作历史完全一致的证明。

## 回放风险（实测）

取消请求是否已经发出取决于取消发生时的时序。若录制中取消请求尚未发出，而回放运行发出了该请求，回放端点会返回 HTTP 400；必须以实际请求报告为准，不猜测取消是否发生。

## 当前边界与核实

候选数量与嵌套、活跃祖先 Candidate 被 Discard 后的后代失效、跨组件取消／超时、最终回答交付和可重复演示仍有端到端验证缺口。嵌套失效的单元回归不能替代完整机制验收；套件从已有运行提供响应，本身不评价投机策略或证明机制正确。

原生 SessionManager 已持久化的 compaction/custom 等可随完整索引汇总保留，导出处理其上下文语义；未落盘的变化不能恢复。Ledger 的其他 action／Runtime 事实仍可能保存完整工具参数与结果，不能把 transcript 去重说成 Ledger 完全没有这些正文。

## NLE 核实（2026-11-13）

### 已定位的初始化回归与旧基线失败

`e89533d` 的保存／恢复实现把原先 `seed(core)` 改成了
`seed(core, random_disp, False, random_lgen)`，同时启用 `fix_moon_phase=True`。
独立 lgen RNG 会改变地图生成，不是 core seed 没有生效。当前最小修复保留
`lgen=None`（沿用 core RNG）；不删除保存／恢复、指纹或版本检查，已保存记录仍按其
完整 initial 重建。`tests/nle-initialization.py` 在修复前稳定失败（lgen=11），修复后
通过，覆盖 seed=7 初始位置 `(26,5)`、四步和真实保存状态重建。无需模型调用。

小型对照使用 NetHackScore-v0、seed=7、action_delay_seconds=0，指向同一个
Python 和仓库 bridge。实际路径为 NleBackend::new 启动
配置中的 bridge，establish 下发 seed，bridge 的 new_world 调用 NLE seed 后 reset。
本机 NLE=1.3.0、Gymnasium=1.2.0，库文件时间早于三个源 run，但源 run 没有原生版本
记录，不能据此声称已证明原二进制身份。成功工具结果没有添加墙钟或回放键。

小型对照 `/tmp/opencode/nle-probe.py` 与 `nle-probe.log` 直接用真实 NLE 执行已有
单层／四层／十层动作，分别改变 disp=11/22、lgen=None/7/99、fix_moon_phase=false/true。
只有 lgen=None 的每一步 after 与三个源 run 全字段一致；改变 disp 未改变这些轨迹。
这不是任意长轨迹确定性保证。原生 before 的差异明确在 message：单层为
`Watch out!  Bad things can happen on Friday the 13th.`，四层和十层为
`Be careful!  New moon tonight.`；当前固定日期效果为普通欢迎消息。关闭固定日期效果
在当前系统日期 2026-11-13 只能匹配单层 before，四层仍不匹配。日期差异尚未修复；
不伪造观察、不篡改源 run，不擅自引入进程时钟替换技术。

初始化修复的小回归证据为 `/tmp/opencode/nle-initialization-before.log`、`/tmp/opencode/nle-initialization-after.log`；`/tmp/opencode/current-disp-probe.json` 记录 disp=11/22/99 对动作 [1] 和 [1,2,3,4] 共六次短轨迹的全部 before/after 一致。这不构成任意长轨迹或全游戏保证。

Policy 随机子集仍按配置顺序输出的回归见 `tests/policy-proposal-order.py`。该局部回归不代表完整机制验收。

实际路径：`src/runtime/backends/nle.rs:69-108` 启动配置的 Python 与 `examples/nle-runtime-bridge.py`，通过 stdin/stdout JSONL 逐请求调用，Mutex 串行化请求（114-138）。`invoke_tool` 只把 bridge 的 result/evidence 包装回传（270-299）。Runtime 配置可由 `ASCOUT_RUNTIME_CONFIG` 覆盖，或由启动时加载的 Policy 的 runtime 路径选择（`src/bin/ascout-control-plane.rs:292-296`）；不是每个 Prompt 的参数。此为已有实现路径记录；当前首版同一 session 的 Policy 固定，不实现修改或切换，保存／恢复须保持一致，范围见 [Policy 开发说明](policy-engine.md)。

`NleConfig.seed` 默认 None，establish 才下发；当前 bridge 显式保存 core、随机 disp、reseed=False、lgen=None，并固定日期效果。NLE `nle/env/base.py:480-513` 说明 None 的 disp 随机选取，lgen=None 不启用独立地图生成 RNG。未配置 seed 时 core 也随机。当前配置不能单独指定 disp、reseed、lgen 或日期策略；固定一个 core seed 不代表整个游戏的完整复现保证。

逐项检查结果构造：bridge:47-93 只输出 chars、blstats、message，并从这些观测渲染 position/hp/depth/turn/hunger/map；invoke:237-250 返回 result 及 before/after/action/is_error。成功工具内容不加入墙钟、随机 id、绝对路径、PID 或耗时；turn 是游戏回合，不是墙钟。延迟仅 sleep。传输 id 是请求计数（Rust:119-121），不进入 result；错误文字可能包含 Guidebook 路径（bridge:254），不能推广成功结果结论到错误路径。Pi 的消息 timestamp、Ledger 时刻与模型耗时另行生成，不属于工具内容，不纳入内容键。

实验脚本和原始输出仅在 `/tmp/ascout-nle-repro/`。实际命令：

```sh
/home/betmul/BETMUL/AgentScout-suit/nle-313/bin/python /tmp/ascout-nle-repro/run.py
```

它直接启动项目原 bridge，以 establish + 12 次 `invoke(nle_action)` 最接近 Runtime 的真实工具入口；动作 `[1,2,3,4,5,6,7,8,19,1,2,19]`，未经过模型、未经过 Rust transport。实际输出：

```text
current same core byte_equal= True bytes= 49961 49961
current changed core byte_equal= False bytes= 49961 51146
 first different line= 1
explicit core+disp same byte_equal= True bytes= 49961 49961
explicit core+disp changed byte_equal= False bytes= 49961 51146
 first different line= 1
```

每轮 13 条响应均 ok。后两项使用临时 bridge 副本，只把 seed 调用改成显式 core=disp=seed、reseed=False；不是仓库修复。当前序列通过不证明 disp 不影响其他轨迹，也不证明完整游戏确定性。

Candidate 不取得 NLE 快照、不 fork 进程、不重放动作。control plane 为每个 Branch 分配独占的 Environment；NLE Backend 的内部世界实现不构成 control plane 的 Environment 共享契约。候选复制 Agent checkpoint 并注入预测工具结果（main.ts:247-278），推进推理；真实工具在接受前 gate，tools.execute 还有角色检查（main.ts:86-90）。所有 Branch 实际共享一个 NLE 进程和 RNG，没有独立复制的 RNG 状态；bridge 的 branch 操作本身也只是 World 别名（187-194），不是实际隔离能力。

结论：仅替换模型输出在限定动作轨迹上可行，但不能无条件保证旧运行逐字复现。需固定完整 RNG 配置、NLE/NetHack 版本、日期策略、初始状态与动作顺序，并防止候选提前真实动作和重试重复动作。bridge 的运行恢复保存已包含 initial 种子、kwargs、版本和配置，但旧 run 没有这些原生保存资料；不能向 session/events 补写测试专用元数据。异步调度不因模型输出固定就自动稳定。

## Pi 端点核实

追加 provider 配置由普通 AScout 复制为 run 的 `models.json`，并将配置内容保存到 Agent recovery 记录；恢复从已校验的记录重建，不能依赖外部工具的临时路径。恢复时显式提供不同的 `--models` 内容会被拒绝。`--resume --model provider/model` 先按保存的模型完成原有恢复校验，再在空闲权威上走普通模型切换；thinking、工具、Pi 配置与扩展绑定仍校验，回放队列不保存。

2026-11-14 恢复补验：修复前 `/tmp/opencode/resume-replay-Jp4eQ9/resume.log` 实际报 `Recovery model, thinking level or tool configuration changed`。修复后 `/tmp/opencode/resume-replay-Dy8cjl` 完成外部工具 quit 保存退出→普通 `--resume --model local/fake`→recovery_loaded 无错误→model_changed→新输入 turn_completed→正常退出。脚本 `/tmp/opencode/resume-replay-check.mjs`，全部为本地伪模型。`/tmp/opencode/check-replay-stop.mjs` 在第三条历史输入仍待提交时制造第二轮缺键，等待 1.2 秒后输入序号仍为 2；证据 `/tmp/opencode/external-replay-759N0k`，退出后按运行路径及回放脚本检查进程无匹配。后续并行工作移除了旧 Policy YAML，扩大复跑因此受夹具路径缺失阻塞；不将该复跑报告为通过。

Pi `ModelRuntime` 支持通用追加 provider；外部工具以临时独立 models 文件提供本地端点，不改写用户的 models/auth 文件。普通启动的 `main.ts` 仍接受 `--model`、`--models` 与 `--thinking`；重名 provider 报错而不覆盖。手动切换继续使用普通 `provider/model`。

最小配置（端口替换为本地端点；apiKey 是 dummy，不是真实凭据）：

```json
{"providers":{"replay":{"baseUrl":"http://127.0.0.1:18080/v1","api":"openai-completions","apiKey":"dummy","models":[{"id":"fixture","reasoning":false,"input":["text"],"contextWindow":8192,"maxTokens":256}]}}}
```

选择 `openai-completions`：POST /v1/chat/completions；Pi 强制请求 `stream:true`（pi/packages/ai/src/api/openai-completions.ts:802-810）。最小成功响应为 HTTP 200、Content-Type text/event-stream：一条带 choices[0].delta.role/content 的 data JSON，随后一条 delta={}、finish_reason=stop，再 `data: [DONE]`。工具回复用 delta.tool_calls=[{index:0,id,type:"function",function:{name,arguments:"JSON 字符串"}}]，终态 finish_reason=tool_calls。chunk 携带 id/object/created/model/choices；实验未提供 usage 也能解析。实际服务还需支持参数增量与多个工具调用；不是普通非流式 JSON 响应。

最小实验 `/tmp/ascout-endpoint/probe.mts` 使用 ModelRuntime 读取临时 models.json、resolveCliModel 选择模型，再调用 Pi 的真实 provider stream，dummy key 只发给 loopback。直接 node 初次运行因缺 pi-ai/dist 失败；使用项目原有 source-resolver 后成功，无需修改或构建 Pi：

```sh
node --import /home/betmul/BETMUL/AgentScout-suit/pi/packages/coding-agent/src/experimental/source-resolver.ts /tmp/ascout-endpoint/probe.mts
```

```text
{"request":1,"stopReason":"stop","content":[{"type":"text","text":"fixed reply"}]}
{"request":2,"stopReason":"toolUse","content":[{"type":"toolCall","id":"call_fixed","name":"nle_action","arguments":{"action":1}}]}
requests=2
```

每个 case 一次请求，不是完整 Agent 工具执行闭环。`request-1.json`、`request-2.json` 和 `reply-*.json` 保存实际请求与解析结果，不存认证 header。第二次实际收到的 messages 是 user("move")、assistant(tool_calls: nle_action, arguments="{\"action\":2}")、tool(content="prediction", tool_call_id="previous")；足以按调用关联读取最新内容与工具命令，不应把“前一条 assistant”机械解释为数组倒数第二项，多工具需按 tool_call_id 关联但不把 id 放入跨运行键。

消息不是原样 JSON：Pi provider convertMessages:1184-1220 会解析 transcript、转换消息及工具 id；1350-1370 序列化参数；1400-1430 将工具文本块以换行拼接，空结果变成占位文字，可能加入图片兼容内容。transform-messages.ts 还会处理跨模型 thinking、失败消息与工具配对。导出必须与所选协议的内容转换一致，不能直接哈希整条 session 记录或整个请求。SDK:451-455 允许扩展改写上下文，agent-session.ts:3034-3047 允许自动 compaction/overflow recovery；不保证原始消息不压缩或不截断。当前 system-prompt.ts:182 置入 cwd，但该文件未发现动态日期注入；自定义提示与扩展仍可能加入日期。消息 timestamp 不进入本次请求 messages（实测）；HTTP id、session header、耗时等不纳入内容键。cwd/系统提示不属于已决定的最新内容键，但会影响模型语义，回放只固定刺激，不证明这些环境一致。

重试：provider 内置 SDK maxRetries=0，但外层 retryProviderRequest 可配置重试（openai-completions.ts:376-389；utils/provider-retry.ts:25-36,118-139），408/409/429/5xx、连接异常可再次到达端点；默认 helper 预算为 0，SDK 从 settings 传入配置（sdk.ts:347-357）。Agent 层默认 retry.enabled=true、maxRetries=3（settings-defaults.ts:18-25），按 isRetryableAssistantError 的文本分类处理服务过载、服务器/网络错误（utils/retry.ts:37-80,270-275）；不是所有非 200 都重试。空流缺 finish_reason 明确报错（openai-completions.ts:689），是否 Agent 重试取决于错误文本；带 stop 的空内容也不能当作缺失匹配成功。建议回放专用 settings 禁用 retry、provider retry 与 compaction，并使缺失匹配返回不重试的 4xx；否则同键可能误消费下一条录制。此建议尚非 control plane 实现变更。

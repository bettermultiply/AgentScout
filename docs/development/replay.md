# 模型输出记录与回放

本文从[控制面观察接口](observation.md)展开，说明系统外的正确性验证工具，不改变控制面主线。

## 目标与边界

降低 NLE 深度投机验证的模型推理成本。回放只用录制的 assistant 输出替换模型服务，检验系统的投机、Promote、Discard 与 single-winner 反应；不评价 Agent、不证明性能收益。NLE 工具真实重新执行，不以录制的工具结果代替执行。

导出与伪造端点放在 [`tools/replay/`](../../tools/replay/README.md)，使用 Node 26，只读取已有 run 内 `events.jsonl` 和 `session.jsonl`。记录是系统原生能力，测试套件不执行录制：B3、repeat、deep 均须提供已有 run，先导出再执行一次、五次或三次回放；低层 B1／B1-PI 保留手构造原生数据验证，B1-PI 也可接收已有样例目录。

## 记录来源

`events.jsonl` 的 transcript 事实保存 Branch id、`session_entry_id` 与 role；正文从 run 内 `session.jsonl`（权威 session 文件）读取。其余 Policy、操作、状态和清理事实仍由 Ledger 保存。必须先写文件、后写引用；允许文件存在未引用记录，不承诺两边原子提交或从 Ledger 恢复完整 Tree。

当前 Pi 接入由 `agent-access/pi/session-tree.ts` 维护每个 Branch 的原生持久 SessionManager，Candidate 从待决 checkpoint 复制原生记录及 ID，并由接入侧汇总最终文件。文件使用 Pi JSONL 格式，记录 id 与 parentId 表达记录的父子关系，不是第二棵控制 Tree。

文件尾部的 `customType=authority` 自定义记录（authority 标记）只是给 Pi 重开文件的读取位置提示，使 resume 的叶子落在权威路径末端。它不进入模型上下文、不作权威裁决；与 Ledger 最后一次 Promote 不一致时以 Ledger 为准。恢复工具尚未实现。同一文件不等于同一个运行 session，winner 保留自己的 session 和在飞进展；历史整理不能据此直接替换运行上下文。

### 已确认的历史语义与迁移缺口（2026-11-13）

最终单一权威 session 是经过验证、用真实结果整理后的执行历史，不要求准确复原候选当时收到的预测输入。保留预测输入节点，将候选在该输入之后的第一条后续记录的 `parentId` 改接对应真实结果 ID；原主／丢弃分支继续保留，不额外强制另存完整投机后续路径。运行期采用 Pi 原生完整持久化、多 session 过渡及接入侧汇总；源分支停止写入且成功汇总后才删除临时文件，仍活跃的 Promote winner 不能删除。当前已接通上述路径；整体验收缺口见接入实现 §4。

整理后恢复上下文可能不同于原始运行上下文；compaction/custom 必须保存，但引用、摘要、在飞推理与 rollback 外部资源关联仍有未决实现差距，详见 [接入实现](agent-access.md) §4。是否追加真实结果通知由 Policy 决定，默认完全不追加，不是空消息。

**模型回放边界**：正常运行仅生成原生 session/events，不为回放计算或持久化匹配键，不依赖 `tools/replay`；键只在系统外导出产物中生成。导出按 events 文件序建立 transcript 原生 ID 的 Branch 归属，以 candidate_created 的 parent/chance 和 chance_created 的 operation_id 核对候选注入批次；由原生 toolCall ID 验证整批（包括并行 sibling），applied Promote 后将第一后续记录的父边仅在内存中接回预测批次尾部。嵌套候选按创建顺序还原。无正文相似度、命名解析或时间近似匹配，源文件不变。

第一后续记录可以是 custom、compaction、context_edit 或 custom_message，不限 assistant。通知以 details.command_id 关联 applied Promote。还原后按最新 compaction 的保留区间、context_edit replacement、branch_summary/custom_message 构造当前 Pi 模型上下文；失败 assistant 排除，当前待决调用的较早同 toolCallId 结果被后续结果替换。缺原生引用、注入批次不全、未知消息角色或无法唯一确定重挂边时明确失败，不加旧格式回退。未持久化的扩展改写及任意 provider 转换不在保证内。

本轮验证：B1 与 B1-PI 通过，后者从已有 `/tmp/opencode/ascout-replay-b1-pi-hyZBIv` 开始导出、经真实 Pi stream 消费两个输出，hits/consumed=2、misses/exhausted/unconsumed=0；`native.mjs` 覆盖自定义首后续、反序并行完成、嵌套、compaction/edit/notification、时钟重置、源文件不变与歧义报错。已有 `/tmp/opencode/recovery-active-BITw2r` 并行恢复样例导出成功，candidate-2 的 4b5fdb14 上下文使用预测批次 02901c7e → 204d72b4；四层 `/tmp/ascout-replay/b3-deep-4/deep-recording` 导出 13 项，十层记录导出 31 项。B3 真实工具回放尝试 3 hit／1 miss，四层也因缺键失败，不能宣称严格整链通过。B3 新运行 NLE 初始地图／位置与源记录不同，严格保留 400 及所有结构、正文、观测和消费断言；不重新录制或弱化匹配。日志 `/tmp/opencode/native-b3.log`、`/tmp/opencode/native-deep4.log`。所有模型请求仅到 loopback。

失败 Branch 对有效执行、成功产物和副作用的影响须等效于没有创建过，但审计记录必须存在；旁支不得重新执行或对权威生效。

## 匹配

- 工具调用投机：待决工具调用的名称与参数，加预测内容本身。
- 模型调用投机：最新一条消息的内容，可取内容哈希。
- 不使用 Branch、Proposal 或工具调用的跨运行 id，不比较完整历史。
- 导出沿内存中还原后的原生父关系取得特征，不直接使用最终权威父链。
- 同键对应多条录制时，按录制出现顺序依次返回。缺失或耗尽必须报错，不得退回真实模型。

长分支中两类键都可能碰撞：相同命令与预测可能对应不同历史，相同最新消息也可能对应不同模型请求。并发到达次序和重试还可能改变同键消费顺序；不能把“有匹配”当作历史完全一致的证明。

## 回放风险（实测）

取消请求是否已经发出取决于取消发生时的时序。若录制中取消请求尚未发出，而回放运行发出了该请求，回放端点会返回 HTTP 400；B3 与深度回放的三次全新 run 必须以实际请求报告为准，不猜测取消是否发生。

## 当前边界与核实

### 深度回放结构（2026-11-12）

`b3-deep.mjs` 从 `chance_created` observation 的 `parent_branch_id` 与
`parent_chance_id` 识别嵌套 Chance，而不是从 `candidate_created` 推断。N=3 的结构为：
`chance-1` 的 parent 为 `authority`，`chance-2` 的 parent 为 `candidate-2/chance-1`，
`chance-3` 的 parent 为 `candidate-4/chance-2`；N=4 再增加
`chance-4` 的 parent 为 `candidate-6/chance-3`。每个 Chance 有 2 个 Candidate、1 个
Promote 和 2 个 applied Discard；每个后续 Chance 都在其祖先 Candidate 已 Promote 后产生。
因此本次没有“祖先 Candidate 被 Discard 后其嵌套 Chance”的案例。导出修复后录制为 10
条（N=3 为 10，N=4 为 13；旧记录曾显示 14），三次回放均应为 `unconsumed=0`，并核对 hits/misses/exhausted、authority chain、NLE
及事件结构。

原生 SessionManager 已持久化的 compaction/custom 等可随完整索引汇总保留，导出处理其上下文语义；未落盘的变化不能恢复。Ledger 的其他 action／Runtime 事实仍可能保存完整工具参数与结果，不能把 transcript 去重说成 Ledger 完全没有这些正文。

端点、导出和一次 B3 闭环已由上述脚本验证；重复回放脚本逐次比较 Chance/Proposal/Candidate 数量、Promote、applied Discard、权威父链 role/text、真实 NLE 观察及命中/缺失/耗尽报告。它仍不证明模型推理质量或性能收益。

## NLE 核实（2026-11-12）

实际路径：`src/runtime/backends/nle.rs:69-108` 启动配置的 Python 与 `examples/nle-runtime-bridge.py`，通过 stdin/stdout JSONL 逐请求调用，Mutex 串行化请求（114-138）。`invoke_tool` 只把 bridge 的 result/evidence 包装回传（270-299）。Runtime 配置可由 `ASCOUT_RUNTIME_CONFIG` 覆盖，或由 task 绑定 Policy 的 runtime 路径选择（`src/bin/ascout-control-plane.rs:292-296`）；不是每个 Prompt 的参数。

`NleConfig.seed` 默认 None（nle.rs:30-37），establish 才下发（200-205）；bridge:178-184 调用 `environment.unwrapped.seed(int(seed))` 后 reset。已安装 NLE 的 `nle/env/base.py:480-513` 表明这只设置 core，disp=None 会随机选取，reseed 默认 False；未配置 seed 时 core 也不固定。当前配置不能单独指定 disp、reseed 或 lgen。因此不能把配置一个 seed 当作完整 RNG 可复现保证。

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

Candidate 不取得 NLE 快照、不 fork 进程、不重放动作。Runtime 声明 shared_environment=true、branch_environment=false（nle.rs:189-190），Controller 将操作映射到同一个 authority 环境（coordination/controller.rs:14-18）。候选复制 Agent checkpoint 并注入预测工具结果（main.ts:247-278），推进推理；真实工具在接受前 gate，tools.execute 还有角色检查（main.ts:86-90）。所有 Branch 实际共享一个 NLE 进程和 RNG，没有独立复制的 RNG 状态；bridge 的 branch 操作本身也只是 World 别名（187-194），不是实际隔离能力。

结论：仅替换模型输出在限定动作轨迹上可行，但当前不能无条件保证整个运行逐字复现。需要固定 core、disp、禁止 reseed，固定 NLE/NetHack/环境配置、工具内容、初始状态与动作顺序，并防止候选提前真实动作和重试重复动作。NLE 构造默认 `fix_moon_phase=False`（`nle/env/base.py:195-202`）；本轮未追踪其跨日期／初始化时间影响，因此不能声称时间因素已全部排除。建议后续允许显式配置 disp/reseed（必要时 lgen），保存实际种子及环境版本，并扩大轨迹核实；当前仅记录上述实现缺口，不修改实现。异步调度不因模型输出固定就自动稳定。

## Pi 端点核实

`main.ts:27-41` 只接收 `--model` 与 `--thinking`，通过 `ModelRuntime.create()` 和 `resolveCliModel` 解析；没有自有 baseUrl CLI 参数。Pi `core/model-runtime.ts:218-223` 默认从 getAgentDir()/models.json 读取；`packages/coding-agent/src/config.ts:604-610,630-636` 默认变量为 `PI_CODING_AGENT_DIR`（APP_NAME 改名会改变变量前缀）。指定独立 agent 目录即可隔离真实凭据，不需要改 Pi。精确使用 `--model replay/fixture`，不要使用可能模糊匹配的短名称；resolver:450-505 使用全模型列表、解析 provider/model。baseUrl 与 key 写 models.json，不依赖未核实的 URL 环境变量。

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

重试：provider 内置 SDK maxRetries=0，但外层 retryProviderRequest 可配置重试（openai-completions.ts:376-389；utils/provider-retry.ts:25-36,118-139），408/409/429/5xx、连接异常可再次到达端点；默认 helper 预算为 0，SDK 从 settings 传入配置（sdk.ts:347-357）。Agent 层默认 retry.enabled=true、maxRetries=3（settings-defaults.ts:18-25），按 isRetryableAssistantError 的文本分类处理服务过载、服务器/网络错误（utils/retry.ts:37-80,270-275）；不是所有非 200 都重试。空流缺 finish_reason 明确报错（openai-completions.ts:689），是否 Agent 重试取决于错误文本；带 stop 的空内容也不能当作缺失匹配成功。建议回放专用 settings 禁用 retry、provider retry 与 compaction，并使缺失匹配返回不重试的 4xx；否则同键可能误消费下一条录制。此建议尚非控制面实现变更。

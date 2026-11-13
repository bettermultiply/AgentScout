# 模型输出记录与回放

本文从[控制面观察接口](observation.md)展开，说明系统外的正确性验证工具，不改变控制面主线。

## 目标与边界

通用回放工具的已批准目标是：指定已有运行目录，每次重新读取原生 session/events，用已记录的 assistant 输出替换模型服务，由正常系统按真实用户输入及交互顺序重新执行工具，在正常 WebUI 观察全过程。结束或匹配失败后系统不退出，用户可通过正常模型切换继续输入；真实模型只能由用户之后手动选择。回放不评价 Agent、不证明性能收益。

源目录只读。每次直接读取指定源目录，在内存准备回复队列，不生成额外 recording、导出缓存或源记录副本。正常系统使用独立新 run，照常持久化 session/events、Policy、日志和主动退出恢复资料；不把新 run 自动当作下次回放源。没有层数参数、固定 prompt、候选或步数；B3 只是场景验收工具。

导出与伪造端点放在 [`tools/replay/`](../../tools/replay/README.md)，使用 Node 26，只读取已有 run 内 `events.jsonl` 和 `session.jsonl`。记录是系统原生能力，测试套件不执行录制：B3、repeat、deep 均须提供已有 run，先导出再执行一次、五次或三次回放；低层 B1／B1-PI 保留手构造原生数据验证，B1-PI 也可接收已有样例目录。

### 通用入口（2026-11-13）

```sh
mkdir /tmp/opencode/replay-run
target/debug/ascout-control-plane /tmp/opencode/replay-run ../pi \
  --replay /path/to/source-run --runtime /path/to/runtime.json \
  --web 127.0.0.1:8080
```

源 run 的 `policy.json` 优先；不存在时必须显式提供第三个位置参数 `original-policy.yaml`，不得回退默认策略。若两者都提供，核对编译 digest。Runtime 选择沿用正常系统：`--runtime`、`ASCOUT_RUNTIME_CONFIG`、Policy 的原 runtime 引用；缺失直接报错。配置须使用新 control_socket，不得位于源目录。新 run 不得位于源目录内。Runtime 的其他可写路径和初始环境由调用方明确提供，系统不会猜测测试目录上一级布局或拿退出快照当初始状态。

`agent-access/replay/` 是运行侧与导出工具共同使用的无依赖模块，运行侧不导入 tools 测试代码。每次启动读取当前原生 session/events，恢复候选实际模型上下文，在进程内注册本地 `replay/recorded` provider。原生记录不增加回放匹配键。按 events 文件顺序、原生 ID 去重，仅将已消费 `message.role=user` 的纯文本输入送入当前权威；通知不作为用户输入。不重现输入被接收的墙钟时刻、Interrupt 或未消费队列；每轮等待权威空闲且控制工作/命令收齐再送下一条。

自动输入完成或任意匹配缺失/耗尽时保留系统，不退回真实模型；错误与消费数量可从 events、API notice 检查。用户手动输入或请求模型切换会停止剩余自动输入。浏览器权威窗口可输入 `provider/model` 切换，API 为 `POST /api/request` 的 `{"action":"model","model":"provider/model"}`；须等权威空闲，不能在 Promote/Runtime 工作/恢复中改模型。解析或认证失败只报告错误，保留原模型和会话；候选保留自己的模型，新候选继承实际 parent。真实模型仅由用户主动选择，认证仍使用正常 Pi 配置。

普通模型请求失败以 turn_failed 报告并结束本轮，权威会话保留；候选执行失败和 Runtime/协议故障仍走原 Branch 生命周期，不扩大为任意故障恢复。主动退出与 `--resume` 保持正常持久路径。当前回放模型本身未持久化队列：退出后若要接着交互，使用普通 `--resume --model provider/model`，不承诺恢复自动回放进度。

**本轮实测**：手造原生多用户输入→真实 bash→工具结果续跑，3/3 回复；结束与缺键两种情况均保持 WebUI/API，未知模型切换失败后仍能切到本地 HTTP 伪模型并继续；正常新 run 保存、原 Policy artifact 优先、普通重启恢复后继续输入通过。源 session/events 的 SHA-256 与 mtime 未变，无额外 recording 缓存。命令 `node /tmp/opencode/check-general-replay.mjs`，最终复跑资料 `/tmp/opencode/general-replay-keCEWl/result.json`。NLE 通用入口单层 4/4、四层 13/13 回复、1/4 Chance 与 1/4 Promote，零剩余/失败，最终复跑资料 `/tmp/opencode/general-nle-AyrZyk/report.json`；首次使用全局 Pi 扩展时上下文改变导致缺键，独立空 `PI_CODING_AGENT_DIR` 后通过；扩展环境也必须与源一致。

相关检查：Rust library 148/148、Pi 接入/历史/通知/恢复 25/25、主动恢复 4/4、native/B1/B1-PI 通过；前端构建通过（保留 >500KB bundle 警告）。未调用真实模型。完整 Runtime 留档、初始环境自动重建、非文本输入、长历史同键碰撞和取消时序仍不保证。

## 记录来源

`events.jsonl` 的 transcript 事实保存 Branch id、`session_entry_id` 与 role；正文从 run 内 `session.jsonl`（权威 session 文件）读取。其余 Policy、操作、状态和清理事实仍由 Ledger 保存。必须先写文件、后写引用；允许文件存在未引用记录，不承诺两边原子提交或从 Ledger 恢复完整 Tree。

当前 Pi 接入由 `agent-access/pi/session-tree.ts` 维护每个 Branch 的原生持久 SessionManager，Candidate 从待决 checkpoint 复制原生记录及 ID，并由接入侧汇总最终文件。文件使用 Pi JSONL 格式，记录 id 与 parentId 表达记录的父子关系，不是第二棵控制 Tree。

文件尾部的 `customType=authority` 自定义记录（authority 标记）只是给 Pi 重开文件的读取位置提示，使 resume 的叶子落在权威路径末端。它不进入模型上下文、不作权威裁决；与 Ledger 最后一次 Promote 不一致时以 Ledger 为准。恢复工具尚未实现。同一文件不等于同一个运行 session，winner 保留自己的 session 和在飞进展；历史整理不能据此直接替换运行上下文。

### 已确认的历史语义与迁移缺口（2026-11-13）

最终单一权威 session 是经过验证、用真实结果整理后的执行历史，不要求准确复原候选当时收到的预测输入。保留预测输入节点，将候选在该输入之后的第一条后续记录的 `parentId` 改接对应真实结果 ID；原主／丢弃分支继续保留，不额外强制另存完整投机后续路径。运行期采用 Pi 原生完整持久化、多 session 过渡及接入侧汇总；源分支停止写入且成功汇总后才删除临时文件，仍活跃的 Promote winner 不能删除。当前已接通上述路径；整体验收缺口见接入实现 §4。

整理后恢复上下文可能不同于原始运行上下文；compaction/custom 必须保存，但引用、摘要、在飞推理与 rollback 外部资源关联仍有未决实现差距，详见 [接入实现](agent-access.md) §4。是否追加真实结果通知由 Policy 决定，默认完全不追加，不是空消息。

**模型回放边界**：正常运行仅生成原生 session/events，不为回放计算或持久化匹配键，不依赖 `tools/replay`；键仅在回放内存或显式导出产物中生成。导出按 events 文件序建立 transcript 原生 ID 的 Branch 归属，以 candidate_created 的 parent/chance 和 chance_created 的 operation_id 核对候选注入批次；由原生 toolCall ID 验证整批（包括并行 sibling），applied Promote 后将第一后续记录的父边仅在内存中接回预测批次尾部。嵌套候选按创建顺序还原。无正文相似度、命名解析或时间近似匹配，源文件不变。

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

## NLE 核实（2026-11-13）

### 已定位的初始化回归与旧基线失败

`e89533d` 的保存／恢复实现把原先 `seed(core)` 改成了
`seed(core, random_disp, False, random_lgen)`，同时启用 `fix_moon_phase=True`。
独立 lgen RNG 会改变地图生成，不是 core seed 没有生效。当前最小修复保留
`lgen=None`（沿用 core RNG）；不删除保存／恢复、指纹或版本检查，已保存记录仍按其
完整 initial 重建。`tests/nle-initialization.py` 在修复前稳定失败（lgen=11），修复后
通过，覆盖 seed=7 初始位置 `(26,5)`、四步和真实保存状态重建。无需模型调用。

源配置和重跑配置均为 NetHackScore-v0、seed=7、action_delay_seconds=0，指向同一个
Python 和仓库 bridge；套件只替换 control_socket。实际路径为 NleBackend::new 启动
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

修复后单层三次独立尝试均 4 hits/4 consumed、四层三次均 13 hits/13 consumed，
misses/exhausted/unconsumed 均 0，结构与动作结果恢复，但全部严格验收仍失败。
四层另有权威历史差异：源文件链中是预测 `moved`，当前整理后的链中是真实工具结果；
这符合当前已确认语义，不通过改回运行语义或弱化 audit 绕过。脚本遇到首个严格差异即
退出，所以一次 `b3-deep.mjs` 的三轮内部循环未完成；三次独立调用不等于三轮验收通过。
这些旧基线的严格比较仍失败；不兼容旧日期或旧权威历史语义。当前语义的重复验收另用下述已存在原生 run，不把新结果冒充旧基线通过。

原始证据：`/tmp/opencode/nle-initialization-before.log`、`nle-initialization-after.log`、
`nle-fixed-b3.log`、`nle-fixed-b3-three.log`、`nle-b3-final-3.log`、
`nle-fixed-deep4.log`、`nle-deep4-final-2.log`、`nle-deep4-final-3.log`。
各日志的 artifacts 目录保留原生 session/events、requests、report 及 cleanup.json；
均确认子进程及端口清理。未重新录制，未修改源记录，模型端点仅 loopback。

### 当前语义重复验收（2026-11-13）

基线来自前轮已经真实完成、但与旧源比较失败的原生回放记录：单层
`/tmp/opencode/ascout-replay-b3-repeat-bIAF7f/run-1`，四层
`/tmp/opencode/ascout-replay-b3-repeat-K0uZLH/run-1`。不是新调用模型录制，旧源不变。
为适配脚本既定的父目录配置位置，使用 `/tmp/opencode/prepare-current-replay-bundles.py`
复制到 `/tmp/opencode/current-replay-inputs-0k1c9rtd/{single,deep4}/source`；普通文件逐字节核对，
不复制已失效的 Unix socket。`provenance.json` 保存来源和 SHA-256，原始 policy/runtime/decisions
也原样保留在 source 中，父目录提供同次配置副本。每轮仅重定位 runtime 路径和 control_socket。

本轮复现并最小修复两项：
- 单层同次 Policy 用 SystemRandom.sample 排列完整候选集，导致 moved/blocked 的 Candidate 身份交换。
  Policy 仍随机选择子集，但按配置顺序排列已选结果。先失败后通过的回归为
  `tests/policy-proposal-order.py`；仅 single 父目录 decisions.py 同步此修复，
  `policy-fix.json` 保存前后哈希，source 不变。四层原配置已使用 candidates[:count]，未改。
- 并行不同键的完成顺序可以交换；验收脚本原来比较导出整个数组，错误地要求跨键总顺序相同。
  现在严格比较键集合及每个键的完整 branch/reply 队列，保留同键顺序、数量、分支和全部回复断言；
  不改导出器或端点顺序。`export-content.test.mjs` 先失败后通过，含同键反序／增删／正文变化拒绝。
  初次失败证据为 `/tmp/opencode/current-single-5.log`（OTCm8H）和修正比较后
  `/tmp/opencode/ascout-replay-b3-repeat-9WDxkS`（Policy 身份交换，4/4 命中）。

精确验收命令（仓库根目录）：

```sh
node tools/replay/verify/b3-repeat.mjs /tmp/opencode/current-replay-inputs-0k1c9rtd/single/source
DEEP_ROUNDS=4 node tools/replay/verify/b3-deep.mjs /tmp/opencode/current-replay-inputs-0k1c9rtd/deep4/source
node --test tools/replay/verify/export-content.test.mjs
python3 tests/policy-proposal-order.py
/home/betmul/BETMUL/AgentScout-suit/nle-313/bin/python tests/nle-initialization.py
/home/betmul/BETMUL/AgentScout-suit/nle-313/bin/python /tmp/opencode/current-disp-probe.py
node tools/replay/verify/native.mjs
node tools/replay/verify/b1.mjs
```

单层 5/5 完整通过，每轮 Chance=1、Proposal/Candidate=2、Promote=1、applied Discard=2、
NLE action=1、hits/consumed=4；四层 3/3 完整通过，每轮 4、8、4、8、4、13。
所有轮次 misses/exhausted/unconsumed=0、missing_keys/unconsumed_keys 为空。
Chance 父结构、Proposal/Candidate/处置事实、权威父链 role/text、真实 NLE before/after、
导出键及完整回复队列全部严格比较通过；不是时间戳或全部原生日志字节相同的承诺。
`repeat-report.json` 和 `cleanup.json` 分别在
`/tmp/opencode/ascout-replay-b3-repeat-tsCtg4`、`/tmp/opencode/ascout-replay-b3-repeat-OhpcwY`，
进程和端口清理均通过；日志为 `/tmp/opencode/current-single-5-final.log`、`current-deep4-3-final.log`。

上述小回归、native、B1 均通过。`current-disp-probe.json` 记录 disp=11/22/99 对已有
单层动作 [1] 和四层 [1,2,3,4] 共六次真实 NLE 轨迹的全部 before/after 严格一致；
不能据此声称任意长轨迹或全游戏不受 disp 影响。全程无真实模型请求，只有 loopback 回放。
剩余范围是长轨迹、自然重复键及并发／重试／取消时序、活跃祖先被丢弃的嵌套端到端覆盖，
不以本轮短轨迹通过宣称这些已验收。

实际路径：`src/runtime/backends/nle.rs:69-108` 启动配置的 Python 与 `examples/nle-runtime-bridge.py`，通过 stdin/stdout JSONL 逐请求调用，Mutex 串行化请求（114-138）。`invoke_tool` 只把 bridge 的 result/evidence 包装回传（270-299）。Runtime 配置可由 `ASCOUT_RUNTIME_CONFIG` 覆盖，或由 task 绑定 Policy 的 runtime 路径选择（`src/bin/ascout-control-plane.rs:292-296`）；不是每个 Prompt 的参数。

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

Candidate 不取得 NLE 快照、不 fork 进程、不重放动作。Runtime 声明 shared_environment=true、branch_environment=false（nle.rs:189-190），Controller 将操作映射到同一个 authority 环境（coordination/controller.rs:14-18）。候选复制 Agent checkpoint 并注入预测工具结果（main.ts:247-278），推进推理；真实工具在接受前 gate，tools.execute 还有角色检查（main.ts:86-90）。所有 Branch 实际共享一个 NLE 进程和 RNG，没有独立复制的 RNG 状态；bridge 的 branch 操作本身也只是 World 别名（187-194），不是实际隔离能力。

结论：仅替换模型输出在限定动作轨迹上可行，但不能无条件保证旧运行逐字复现。需固定完整 RNG 配置、NLE/NetHack 版本、日期策略、初始状态与动作顺序，并防止候选提前真实动作和重试重复动作。bridge 的运行恢复保存已包含 initial 种子、kwargs、版本和配置，但旧 run 没有这些原生保存资料；不能向 session/events 补写测试专用元数据。旧日期与旧权威历史失败仍保留，当前语义的短轨迹重复验收如上。异步调度不因模型输出固定就自动稳定。

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

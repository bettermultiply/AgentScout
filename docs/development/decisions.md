# 开发决策

## 2026-11-13：前端构建产物不入库

废弃“构建产物提交到 `webui/dist`、`cargo build` 不依赖 Node”的交付方式：用户决定不提交构建产物，改为在本地生成后嵌入 Rust 二进制；代价是首次克隆及前端源码变化后必须先安装前端依赖并构建，再运行 `cargo build`，缺少嵌入文件会编译失败。当前不实施自动前端构建。

## 2026-11-13：直接构造连接原型

用户批准第二种方案：利用生成轮廓时的位置映射选择实际可见父侧端点，沿子侧选有限终点，按端点和切线直接构造 cubic。替换根区的映射必须指向新连接，不能用隐藏普通边界。保留锚点、中心线、宽度、方向和叶茎 placement；局部外形允许改变，视觉验收由用户完成。未增加参数，未实施 task/tuning 缓存分离。

废弃布局热路径的两侧首交点、全父环求交、交点后弧长距离及单位置父枝专用 roundedBase 规则：这些规则对短粗枝与相切退化易返回 null，且求交和弧长反演昂贵。以下旧验收数字不作为直接构造轮廓的验收证据。

2026-11-14 用户已检查并接受当前画面，批准小范围清理，以代码和文档整洁降低心智负担。核实生产仅调用 `connectSides` 后，删除 `smoothConnection.ts`、只验证旧算法的 `tests/tree-smooth-connection.mjs`，以及无生产消费的求交、弧长积分、反演和辅助函数。删除 `Join`（包括曾以 `exitArc=0`、`parentArc=位置` 兼容的诊断字段）、布局结果的叶茎 `connection`、子枝 `connection.path/joins`；删除未读取的 `attach.fork` 和重复 bounds 遍历。实际绘图 `d`、控制点和所有保留位置数据在七案例逐值一致，现有布局／placement 测试和构建通过。未重做轮廓生成、缓存或构造规则，未增加公共命名或参数；视觉接受不等于穷举极端参数。后续增量布局、输入分离和有界缓存仍未完成。

旧阶段证据归档：stage5 六树21枝78叶茎，56连接22回退，完成位置和189个不变量通过；其V7以每cubic 200段检查407771100对非相邻线段，横穿／接触／重合为0，非解析证明，材料 `/tmp/stage5/`。stage5b保留48段种子与公式，仅包围盒排除／缓存，23项测试5956.58ms，大采样4457.66ms，六树原生值一致；独立检查仍发现旧新共同漏检及非最早交点，材料 `/tmp/stage5b/`。第四阶段默认角nested/fourChildren、lead=0/.5共22/22连接；固定30°/90°补充44/44，不能替代整体验收，材料 `/tmp/stage4/final/`。这些旧外形数据不再作为当前直接构造验收依据。

## 2026-11-13：首交点连接、范围夹角与查询串默认值同步（连接规则已废弃）

当前连接以子枝两侧外沿各自的首交点为依据，父侧和子侧各取独立单侧距离，默认均为 0.15W、范围 0–2、步长 0.01；三次曲线控制柄分别为有效距离的 2/3。保留 React Flow + SVG，纯几何模块位于 `webui/src/geometry/`。本次只修改参数默认值与文档，没有修改算法，也没有修复第二阶段 V3、V7、V11 未通过项。

废弃记录：

- 父枝总 1.3W 弧长窗口被“各首交点沿父轮廓远离另一交点取单侧距离”取代，原因是父窗口端点与子侧出口不匹配会形成绕钩。
- `minChildAngle` 与将超限请求角钳至 90° 的规则被合法范围参数取代：`childAngleMin/Max` 各范围 5–90°，默认 30/90；`childAngle`／`childAngleRandom` 的固定值／范围随机双模式被范围参数取代。范围以 `${id}/childAngle` 稳定采样，选侧检查初始绝对方向是否超过 90°。
- 子枝起点位于父外沿的规则被父中心线起点取代，原因是外沿起点可能让一侧无交点。Chance 圆心仍在按实际首段方向选侧的父外沿，不能把它与子枝起点混同。
- 向父内重叠 `overlap`、`rootCap` 及 `parentCurveCount` 等窗口式连接结构被首交点闭合轮廓与单位置父枝的 `roundedBase` 取代；空枝只画芽。
- 默认 K=20、τ=60000ms、转角 3°、最大倾角 60°、根宽 45、宽度指数 0.5、Chance 半径比例 0.5 被用户完整查询串的真实解析对象取代，分别为 60、10000ms、18°、72°、9、0.1、0.05。其余字段逐一沿用解析值，键、范围与算法不变；`tau` 直接使用毫秒，面板 `scale=1000` 不表示 URL 换算。

第二阶段报告实际为 42 配置、231 次分叉，18 次成功连接和 213 次 null；V3 外部根点 28，V7 漏盖 8 点及额外材料 262 点，V11 的 null 图缺标记。成功连接数不能代替任意交点数量；短粗仅为待验证归因，不把文档同步或默认变化表述成算法验收通过。当前实现与缺口见 `tree-connection-repair.md`。

## 2026-11-13：本轮明确批准保留 dispose.context.authority_outcome

用户在本轮明确回复“同意”，批准保留 `dispose.context.authority_outcome`，用于向 Policy 提供对应 Chance／operation 的真实结果。该字段不自动给模型追加消息；是否追加真实结果通知仍由 Policy 通过可选 `notification` 决定，默认完全不追加。Match evidence 与 `context.match` 保持不变。

早先文档把该字段写成“用户同意”时，缺乏可独立核实的批准依据；本条记录的是本轮明确批准，不追认早先已确认。同步纠正接入实现文档的批准溯源，清除 Discussion 中对应待确认项。本轮仅文档收尾，不修改实现、不运行测试、不委派；只提交本次可隔离的文档差异，Discussion 被忽略则不强制纳入提交。

## 2026-11-13：Pi 原生分支持久化实施进展

按 6b99d9b 接通独立原生持久 SessionManager，复制 native entries 和 ID；不用消息投影重建 checkpoint，也不在活跃 manager 调用会修改 receiver 的 createBranchedSession。汇总使用临时文件及 rename，校验父引用和循环，只有停止写入且成功汇总的源文件才清理。运行 manager 保留预测上下文；汇总父链单独按真实结果整理。旧 message_end 向共享文件生成新 ID 的方式废弃。

废弃记录（2026-11-13 用户批准清理）：曾在调用前计算 requestKey，以 assistant 原生 ID 写入 authority custom 的 data.requests，并写入恢复快照；导出优先读取这些键。此方案使正常运行依赖系统外回放工具，并为测试额外记录专用数据，现已移除。临时拒绝含 authority 标记文件的限制也已移除，改为按原生 transcript ID、Candidate/Chance 和 applied Promote 在导出内存中还原父关系。测试套件先从已有运行导出，不先录制；不为测试新增运行期元数据、不兼容旧格式、不通过正文或时间猜测。没有新增 Policy 类别，默认不追加真实结果通知。

同仓库 rollback 验证保留 targetId、ref、repoRoot 与 Git object，可读 blob 且启动 GC 不删引用；不等于跨仓库迁移、执行 restore 或完整任务继续。摘要跨越预测输入、任意 custom 外部引用和机制状态交接仍需收敛，不自行改写摘要或选择新的 Backend。

## 2026-11-13：真实结果整理历史与正确状态交接

用户确认最终权威 session 的含义改变为经过验证、用真实结果整理后的执行历史，不再要求准确复原候选当时收到的预测输入。保留预测输入节点，将其后第一条后续记录的 `parentId` 改接对应真实结果 ID；原主／丢弃分支保留，single-winner 与单一最终 session 不变。不采纳额外强制另存完整投机后续路径的建议。

废弃“历史消息父关系永不改写”“assistant 父记录必然等于原请求输入”“整理后恢复上下文必然与原始运行上下文相同”的旧承诺。Promote 保留 winner 执行进展，但不能从中推导历史整理可直接替换运行中上下文。compaction/custom 必须保存；整理中的引用／摘要、在飞推理和 rollback 外部资源关联仍是实现差距，不擅定重启、强制取消或替换运行上下文。

运行分支采用 Pi 原生完整持久化、多 session 过渡、接入侧汇总。源分支停止写入且成功汇总后才能删临时文件，活跃 Promote winner 不能删。这是已定方向，不是已完成迁移；当前消息结束事件向共享树文件追加的实现尚未满足它。

正确状态交接是机制保证；FS 共同快照差异合入 winner 仅为讨论方向，不写成已实现 Backend，也不授权通用状态 merge。是否追加真实结果通知由 Policy 决定，默认完全不追加，不是空消息，不强制新增 Policy 名称或类别。历史重挂后的模型回放不能直接假设父链是原请求上下文；当前原生记录还原实现及验证边界见 replay.md。

本轮只更新设计、开发和 Discussion 文档；目标见 `../design/agent-integration.md` §3，实现差距见 `agent-access.md` §4 和 `replay.md`。不编码、不跑测试、不调用真实模型。

## 2026-11-13：讨论清单整理与旧方案归档

用户确认：Discussion 只保留未决问题、理解问题所需的已确认前提和简短文档入口。目标语义、实现状态与验证证据分别下沉到对应主题；已确定但未完成的能力列为实现或验证缺口，不重新当作未决设计。清单汇总不等于授权同时实施各项。

本次从 Discussion 清除的旧方案补记如下，均为历史，不是当前设计：

- 2026-11-10 第一版树形将长度解释为事件顺序，以圆形、六边形、方形等区分事件，同类信息可合并；第二版以当前权威路径居中、按子树重量分配左右，子枝放在父枝下方，连续同名工具或模型调用合并。上述顺序布局、权威驱动几何和调用合并后来被真实调用时间、稳定 Branch 几何及每次调用独立绘制取代。相关旧提交 `16ed127` 的 dist 曾包含当时未提交的并行类型迁移，不能视为独立源码与产物一致的验收依据。
- 2026-11-11 连续对话早期方案曾把历史权威 Branch 窗口改名并取消选择；随后用户澄清为新增独立权威窗口，保留各 Branch 的只读窗口。旧“模型调用是小树枝”措辞改为模型调用叶茎、工具调用叶片。当前行为见 web-ui.md。
- 2026-11-12 独立 SVG 原型 `/tmp/opencode/tree-prototype-v3/` 被放弃；后续以仓库 WebUI 为准。早期默认 K=150、τ=24 秒以及屏幕像素补偿已被后续调参和统一树内坐标替代；当前参数及共享几何见 tree-parameters.md。旧样例的不同轮次重叠数、尺寸与缩放结果只证明当轮状态，不构成当前验收标准。
- Pi 原生 PTY／终端模拟器嵌入方案已被 2026-11-03 UI 责任边界修正取代：通过公开 SDK 取得 Pi 信息，由 AScout UI 展示，不修改 Pi 仓库或嵌入产品终端画面。旧队列中的“接通完整 InteractiveMode／原生终端传输”不再作为既定实现任务，实际交互需求另行确认。
- 原 Discussion 的“src 尚未归组”不再符合当前 `src/lib.rs`：它已只公开 control_plane、protocol、runtime。仍需审查的是公开接口和依赖方向，不据文件迁移推断架构全部完成。
- 原 Discussion 关于物化 journal／备份恢复的安全提案不作为现行要求；后续物化决定已明确暂不保存被覆盖宿主版本，冲突策略仍待定，以 materialize-proposal.md 为准。控制面判断压平的早期提议亦未获定案，须先核对 Runtime 自主整理边界。

已下沉的 NLE 闭环、退出、Chance 状态与 session 保存记录不再复制到 Discussion。其历史证据与能力限制继续分别由 reference-workload.md、policy-engine.md、replay.md 等保存。本次没有重新运行这些验收，也不将历史的“不提交”说明提升为永久协作规则。

## 2026-11-13：整个投机面板拖动调整宽度

用户确认在主面板与整个投机面板之间提供可拖动分隔线，调整整个投机面板，不是临时调参面板。沿用现有 React/CSS，不引入布局依赖，不修改树绘制公式或控制语义。

初始宽度沿用 `clamp(410px, 34vw, 620px)`。桌面分隔线宽 8px，投机面板至少 410px，主面板至少保留 480px；1020px 及以下沿用 `min(94vw, 560px)` 抽屉并隐藏分隔线。用户请求宽度只保存在页面内存中，关闭再打开保留，刷新恢复默认；窗口缩小时限制实际宽度，恢复大窗口后恢复请求宽度。

分隔线支持聚焦、左右键、Shift 加速和 Home/End，提供 separator 无障碍属性。指针释放、取消、捕获丢失、关闭面板、容器尺寸变化及卸载均结束拖动。实现与既有无模型浏览器验收见 `web-ui.md`。本轮保留全部混合未提交改动，App、CSS、开发文档与 dist 含既有工作，不能安全隔离提交，不整体暂存或提交。

## 2026-11-13：连接过渡起点、出根侧与亮缝修正

用户批准子侧过渡从“子枝两侧外沿各自露出父枝可见轮廓的位置”起算 0.15W，并明确授权修正错误的根锚点位置；父侧参数定义、初角、宽度、中心线时间长度与叶茎叶片规则不变。据此废弃以下旧规则：

- 旧“从子枝原始根点沿初始方向投影 0.15W”：端点会落在父枝填充内部，可见出口出现折角。现改为露出父枝可见填充的位置沿该侧外沿再前进 0.15W。
- 旧“父侧边缘取 `trunkSide` 按中心线与宽度计算的边”：空枝、单位置父枝和父枝自身连接之后的实际外沿与它不一致。现改为父枝实际可见闭合轮廓上的弧长窗口。
- 旧“连接控制柄取沿切向投影或 0.15W/3”：短到接近零，切向正确也会在根部横穿子枝外沿，填充不覆盖子枝中心线，视觉上像从另一侧出根。现改为弦长 1/3。
- 旧“父枝与子枝填充共享同一条边”：抗锯齿在共享边留下亮缝。现在连接填充向父枝内部多伸 0.08×根宽，由上层父枝遮住。
- 旧“空枝芽也外加父侧连接”：芽根宽为零、两侧在根部与父外沿相切，外加连接反而制造肩部。现在空枝只画芽，不再生成连接。
- 旧“单点父枝照常截父外沿”：没有纵向外沿可截，现改用绕分叉点的圆根；父枝只有一个位置时其子枝初角可超过最大倾角，现按子枝实际第一段方向纠正根锚点所在侧，分叉距离和宽度不变。

实现、浏览器填充并集证据与仍存缺口见 `tree-connection-repair.md`。本轮仍不提交。

## 2026-11-13：按用户批准撤回分叉回中与 Proposal 验证事件

用户报告最新改动破坏分叉衔接，确认仅撤回最近一轮。废弃本轮世界中轴分叉概率、世界左右与范围初角向内调整，撤回 `ControlResult::Validation`、`ProposalValidated`、验证 work id 经 Promote Applied 到 Chance Accepted 的关联、Chance 准入源时间字段、父枝验证点截止及端点/数据缺口标记和对应测试。恢复上一轮的本枝局部概率、范围稳定采样、Chance 创建记录时间定位及完整父枝绘制；真实验证时间仍是未实现能力，不猜测替代时间。

保留此前共用轮廓法线的叶茎锚点、同时间零长采样及单位置截面、逐点累计 bounds 的栈溢出修复、全部调参、共享几何和 NLE 等混合工作。本轮先备份文件及 diff 后做精确反向补丁，不整体恢复文件、不删除 tree 目录、不一揽子提交。

## 2026-11-12：轮廓锚点修复与父枝验证截止的观测阻碍

废弃叶茎锚点使用调用点前一段航向、而枝干轮廓使用相邻采样中心差法线的两套截面计算；两者必须复用同一法线与宽度。同时间调用不添加零长采样；单位置枝保留真实宽度横截面，不添加纵向尾段。

用户取消父枝“短突出”与“忽略时间”的方案。目标改为子枝被采纳后，父枝按原时间映射最多绘制到对应 Proposal 的真实验证点，终点用 SVG 短横线及悬停说明，不混淆 Chance 圆。实现暂受阻：公共事件没有 Proposal 验证时刻，不能以 Promote、Accepted 或 Chance 创建时刻代替。

分叉应表示投机开始，但当前几何取 `chance_created.timestamp_ms`，该值由 Ledger 追加时生成，不是 `admit` 实际发生时刻；候选 `branch_created.speculation_started_at_ms` 才是准入请求的接入侧投机起点。Chance 无候选时公共字段缺少这一源侧时间；本轮不擅自改变已确认语义或扩大事件协议。详见 Discussion.md 与 web-ui.md 的待补观察说明。

## 2026-11-12：第十一轮叶茎、封顶、障碍与距离制弯曲

本轮在第十轮基础上修正五处：

- 叶茎长度删去 15 单位的固定底座，改为 `9 × log1p(秒)`，下限 4、上限 34 CSS px。耗时越短叶茎越短，不存在“所有叶茎都从 15px 起跳”的常数。
- 叶茎颜色只跟随所属 Branch，删除“权威祖先在分叉点之后的叶茎改按候选变暗”的第二套规则。旧规则会让白干上长出粉茎，但树上并没有对应的粉色 Branch。全树现在每个 Branch 的叶茎只有一种颜色。
- 子枝整条轮廓按父枝在分叉点的宽度等比缩小，使起点不超过父宽的 0.9 倍。这条上限只在单子枝或子枝占据几乎全部剩余工作时生效；第十轮的 90 → 9.6 落差不再出现。
- 主干作为子枝的障碍。子枝起点若在某个祖先树干轮廓内，视为从该树干长出，不判为穿越；起点在轮廓外、路径却进入祖先树干时，换成镜像角度重排；两侧都撞时保留原来的确定方向。取代“离开种子轴就向外”的单一启发式。
- 弯曲从“占整枝长度的比例 + 两遍弧长重采样”改为“已走距离的函数”：180–340 单位的指数回正，叠加约 420 和 210 单位波长的两根低频正弦。追加更晚的事件只延长枝尾，已画出的前缀不变；实测删掉 `child-b` 最后两次调用后，共享前缀的最大横向偏差约 0.015 布局单位。

- 主干宽度上限不再固定 90，改为整棵树逻辑跨度的 2.4%，夹在 4–90。旧的固定上限会让一次只有几次调用、十几秒的短运行得到一根宽度约等于树高三分之一的主干；长运行的宽度几乎不变。分叉处由“子枝整棵子树在分叉点一次性计入父宽”造成的矩形台阶，用两遍 3 点平滑改成接近分叉时逐渐张开、离开后收窄的过渡。
- 候选 Branch 上的成功叶改用该枝的玫红（`--leaf-candidate`：深色 #F0A2D2、浅色 #A83A7E），失败、取消、超时仍保留红、空心灰、空心琥珀。否则粉紫枝上全是绿叶，两个通道读起来互相矛盾。未采纳枝的枯叶覆盖仍然优先。
- Pi 适配器在 `start_branch` 收到 `role=authority` 时把该 Branch 采纳为权威。复用同一个 run 目录时，`session.jsonl` 仍带着上一次运行写入的 authority marker；旧代码只在这个 marker 存在时设权威，控制面新创建的 `authority` 却不在里面，于是第一个 prompt 被 “Only the authority accepts user input” 拒绝。控制面显式启动的权威 Branch 应当覆盖旧 marker。

本轮实测：9 个 Branch、66 根叶茎、66 片叶；叶茎 4.0–9.7 CSS px，30 分钟那次 34.4；叶片 14.7；叶茎锚点 66/66 落在轮廓半宽处；祖先主干穿越 0 处。fixture 里没有子枝触及 0.9 上限，该项只做了代码与类型检查。

## 2026-11-12：第十轮树形态以时间与后续调用为准

本轮废弃第九轮及以前按常数近似树形的做法：

- 子枝长度不再取 `max(min(自身自然长度, 主干高度 × 0.48), 模型调用数 × 125 + 120)`，不再给主干加 100 单位尾巴，不再为了塞下装饰做 `contentScale` 压缩。长度改为该 Branch 从创建到最后一个事件（丢弃/清理事件优先）的逻辑距离加 12 单位短尾，与调用和 Chance 共用 `150 * log1p(age/24000)`。
- 取消调用之间 82 单位的强制间距。调用和 Chance 的位置只由逻辑时间决定；短间隔挤在一起是事实，不再用视觉间隔改写时间。
- 废弃 `trunkSegments[].width` 这套“算了但没画”的死代码，以及根枝 90、子枝父宽 × 0.55 的直线收缩。宽度改为全局函数：该点之后（含子枝子树）的调用数经指数 0.72 映射到 2.5–90 单位；分叉处父宽一定不小于子枝，单子枝时连续。
- 废弃子枝中心线外推 `(父半宽 + 子半宽 − 6)` 和根端圆点。子枝中心线从父枝中心线的 Chance 点开始，父枝绘制在上层覆盖子枝平口；不再有外推的弯根，枝尖不再出现圆点。根枝起点圆头保留。
- 废弃固定六角度集合和全部落在 48–55° 的映射。初始角改为稳定 hash 在 26–56° 连续取值，靠近种子轴的兄弟枝左右随机游走并在单侧多两个时换边，同侧近邻至少相差 15°。分叉点离开种子轴超过 10 单位的子枝固定向外生长，避免孙枝横穿主干。
- 废弃叶片的 `+ 6*unit` 悬空锚点、把叶片转到主干切线、`clearBlade` 上下挪位避让、固定 18×7.5 叶片和固定 24 单位叶茎。模型调用是一根叶茎：从真实轮廓半宽处长出，长度取模型耗时；工具叶片沿叶茎方向排开，大小取工具耗时；叶片只按角度散开，不再移动事实锚点。
- 废弃 `finished`、`discarded`、`cleaned` 合并成同一个灰，以及从未被节点输出的 `tree-trunk-abandoned` / `tree-twig-abandoned` 样式。枝的显示状态改为 `authority`、`authority-finished`、`candidate`、`aborted` 四种；当前权威白色，历史权威（含已结束的旧主干）为按 72% 与背景混合的较淡白色（不用透明度，避免与下层子枝混色），未采纳灰色 40% 且叶片统一变枯。
- 废弃 `.tone-success` 等叶片状态色只做明暗滤镜、并覆盖所属枝颜色的做法。调用状态改为独立通道：成功实心绿、失败实心红、取消空心灰、超时空心琥珀；枝的透明度包住叶片，未采纳枝上的叶子不会再保持鲜绿。
- 删除多余的 Chance 菱形分支：所有 Chance 都是统一圆点，空 Branch 由自己的节点画芽。

## 2026-11-12：第九轮树几何修正（工作区实现）

本轮保留现有 NLE/map 与 fixture 事实，不调用真实模型、不新增仓库测试、不提交。Chance 的 SVG 元素改为统一直径的实心圆；只有已有的空 Branch 才显示实心菱形芽，缺少对应 Branch 的 Chance 仍显示为 Chance 圆。根接口改为按父干局部轮廓向内插入，主干端点补圆头并由父节点层级保护父干轮廓。`c9` 没有现有子 Branch，`child-e` 没有现有 Chance/孙枝事实，不能凭空补事件，最终 PNG 验证须明确写 `unproved`。

本轮废弃的固定外移 Chance、Chance 菱形、无端点圆头和仅靠 DOM 声明尺寸的做法只保留在历史记录中，不作为第九轮证据。

## 2026-11-12：第八轮投机树独立角度与同组墨迹

废弃第七轮全部子枝强制55°、子节点覆盖父节点、法线分离Chance连线与屏幕overlay、空心环芽、半圆枝尖以及叶片反缩放。稳定hash独立确定初始30–55°、s0 0.25–0.5、末端0.4–0.7初始角且至少12°；中心线累计真实弧长反馈，约三分之一后半段轻微回摆，远外枝回正更强。父干前置，子根宽硬取父处0.55，连接轮廓在父缘外偏并位于父干下；树尖连续锥形，不画根盘。

Chance、芽、叶茎、叶片进入同一随React Flow变换的SVG组，尺寸以初始fit作为CSS参考，此后随真实zoom一起变换（不在每次zoom时反缩放）。近邻Chance沿父干±7px视觉错位、无连线，事实时间不变。空芽28×4.5px实心菱形；叶片18×7.5px，叶轴跟随叶茎末端，1px本色暗15%中脉不越端。图例同步三类枝色；暗色authority #F4F8FC、finished #7F8B99。fit按墨迹测量居中，上下目标各50px。

仅临时fixture调整已有调用时钟：root七个Chance目标8/20/31/32/50/68/85%，c3普通child-c、c4 empty，保留孙枝，不增加Branch/叶。第八轮脚本与PNG写/tmp，forks与nearby使用不同真实zoom/聚焦；删除旧轮次验收字段。SVG几何与PNG证据分开，未证明的指定项说明原因，accepted保留false。不commit/push/stash，不调用真实模型，不新增仓库测试；保护既有NLE/map工作。

最终临时验收：root Chance约7.93/19.94/30.95/31.95/49.96/67.97/84.99%；圆盘零相交，同枝叶茎锚点最小28.018px。PNG上下墨迹边距约49.55/54px；forks真实zoom2.609、nearby3.757，forks加高至2200 CSS px以完整容纳c2/c3及各自前300物理px。暗色权威/已结束对比约3.25，两者对暗底16.10/4.95。第八轮末项修复后，empty bud沿主干边缘绘制28×4.5 CSS px实心菱形，capture从实际屏幕端点量得28.0 CSS px；叶簇避让改为按实际叶茎切线检查并保持同枝锚点间距，暗/亮两套SVG三点检测均为0片压干，最小叶茎间距约28.12 CSS px。PNG辅助检查也未检出叶片颜色与主干的重叠样本；芽的PNG量测受抗锯齿和端点覆盖影响约25.5 CSS px，故仍保留为辅助证据，不宣称全部完成。

## 2026-11-12：第七轮投机树实现与未通过验收

当前实现采用父侧缘出枝、按中心线累计弧长反馈的指数回正、父宽0.55递乘与连续锥形轮廓，Chance改为固定10 CSS px青色实心点；近邻保留事实锚点并沿局部法线偏开视觉点、加连接线。空Branch绘制28 CSS px芽与空心环实心核，模型调用各保留叶茎，每茎至多三片16 CSS px叶片。只用mock事件，不调用模型，不新增仓库测试，不commit/push/stash。

本轮增量把子枝根部中心外移到父宽与子根宽之和的一半，并继续增加半子宽与法向偏移，使两个闭合轮廓在父缘外相切；React Flow 节点按树深递增 z-index，使子枝首段在共享 Chance 处可见。稳定 hash 仍保留枝间差异，但产品角度映射到48–55°以抵消轮廓与栅格化损失。最终 PNG 复测（暗/亮相同几何）child-c 为 `-39.91°/-39.91°`，child-e 为 `37.68°/37.68°`，grandchild 为 `-42.61°/-42.61°`，late-child 为 `36.94°/36.94°`；暗色 authority/finished 对比 `4.41`，浅色保持 `2.53`，叶片压干为 `0`，末端可测宽度为 `0.195 CSS px`。Chance 改为实心菱形，empty 改为贴干实心菱形；截图保留 c3/child-c、c4/empty、grandchild。

废弃第六轮固定36布局单位衰减、以子枝宽度限制Chance直径、空Branch DOM高度350、以矩形或CSS声明替代PNG结果以及capture直接写viewport transform的做法。旧第六轮“自动检查全部通过”的表述不构成第七轮验收证据。

验收材料放在/tmp/opencode，正式PNG为/tmp/ascout-tree7-{rich,forks,chances-nearby,authority}-{dark,light}.png。当前accepted=false：六个同父方向在±30–55°和两两间隔15°限制下无法同时容纳；屏幕空间不足以使36次独立调用均相隔40 CSS px。嵌套宽度下限与比例、空芽晚到Chance连接、叶片/叶茎完整碰撞避让仍有缺口。合成PNG无法独立分离证明的角度、曲率、尺寸和碰撞字段必须写null并列入remainingGaps，不以DOM替代、不声称全部完成。不得再以“全图有重叠”为由无条件拒测所有枝：2026-11-12脚本已按法向PNG连续色截面实际取边界中点，child-a/b/c/e/late-child有效截面104/165/64/30/62，角度、宽度和可测分段长度已记录于web-ui.md及tree7-validation.json。整树包围盒明确包括叶片；Chance填充与描边外径分开报告；forks前300px记录实际PNG存在及canvas边距，但c3空芽不足300px仍失败。rich最小叶茎锚点距离21.528637 CSS px明确标为DOM辅助证据，未混作PNG结果。

第七轮其他未完成项：真实弧长参数化、父杆外80物理px可见角验证、统一连接圆角、父末端切线规则、无模型工具入口统一、追加曲线前缀稳定性。当前真实forks缩放约2.24倍，DOM八个Chance圆盘无相交且PNG均检出青色内填充；这不等于PNG已证明全部枝线几何或Chance对父杆对比度。

## 2026-11-12：第六轮投机树可见性与形态修正

第五轮的实现和验收口径废弃：不再把透明 HTML 叶片点击层当作叶片像素证据，也不再以 DOM 节点高度代替空 Branch 的视觉证明。当前实现使用闭合实心树干、Chance 实心点加反色描边、子枝根部不重复绘制 head；SVG 叶片为细长柳叶形并使用叶片本色中脉；空 Branch 具有专用 `aria-label`/`title`，其晚到子树仍保留逻辑时间锚点。

第六轮的临时 headless 断言、截图和 PNG 像素统计只放在 `/tmp/opencode`，不新增仓库测试、不调用真实模型、不提交临时材料。forks 截图必须通过 React Flow 的真实放大控制并检查 Chance 与 Branch 端点包围盒；若空 Branch 的小芽盒和子树锚点无法同时满足，validation 的 `remainingGaps` 必须明确记录，不能用 CSS/DOM 声明替代 PNG 证明。

## 2026-11-12：第五轮验收纠偏

废弃用最小事件槽间距移动 Chance 的做法；调用与 Chance 直接共享事件 timestamp 到逻辑距离映射，叶簇避让不改变时间锚点。废弃 capture 脚本强制 DOM disabled 的做法，fixture 保持 SSE 连接并发布 ready，按钮截图通过真实输入/清空触发组件状态。废弃把宽度误报为 treeAspect、把任意高对比像素当作三类树干证据的验证；现在合并全部 Branch 屏幕矩形，并在对应树干的已知位置读取最终 PNG 像素。旧第四轮验收数值不代表第五轮通过。

第五轮验收进展写在 web-ui.md 与临时 validation 的 remainingGaps；不提交 Git、不调用真实模型、不新增仓库测试。旧的角度硬截横偏会使起始切线偏离分配角，现以保留起始切线并单调回正到 20° 的曲线替换；工具叶片使用随 viewport zoom 补偿的 SVG 尺寸和共点叶柄。

## 2026-11-12：第五轮投机树像素验收（工作区实现）

第五轮把分叉方向限制为初始同号的 5°–65° 候选，并让路径沿父干向下回正；主干继续使用一条闭合实心轮廓，空 Branch 最短 28px。Chance 的可见半径取子枝根宽与父点宽的较小值并保留 3px 直径下限，命中层仍透明。模型叶茎与工具叶片保持独立，叶片为闭合柳叶 path 并增加 1px 中脉；禁用发送按钮使用灰色约 0.45 opacity 和 not-allowed 光标。

第五轮 rich fixture 的 root Chance 分布改为 15%、30%、31%、52%、70%、86% 的逻辑时间目标，并保留 30 分钟调用、空 Branch 子树和 60+ 根调用。所有 PNG、截图和断言脚本继续只放在 `/tmp/opencode`，不调用真实模型、不新增仓库测试、不在本轮提交。

## 2026-11-12：第四轮投机树最终渲染规则

本轮废弃第三轮中以多个 stroked segment 叠加绘制树干、以 CSS 变量而非最终合成像素判断对比度、以及用非闭合叶片视觉表示调用的旧实现。当前目标语义是一条单一闭合实心变宽轮廓 path，宽度按剩余调用比例全程渐细，模型叶茎与工具叶片各自独立绘制；工具叶片必须是闭合杏仁/柳叶形，Chance 为实心点加描边。旧实现仅保留在下方历史记录，不得作为本轮验收证据。

本轮临时验证最新实际生成 6 条闭合树干 path、5 个可见 Chance、59 个 SVG 叶片；同一 model twig 内的叶片 overlap 为 false，HTML LeafButton 叶片视觉透明。Playwright 临时隐藏非 root Branch 后生成 root-only PNG，root 中心线法线扫描输出 `10/9/7/5px`，root/tail 达到要求；布局样本也保持单调。Chance coverage 自动记录 root c1/c2/c3 为 front、c4 为 middle、c5 为 back。旧的 CSS-only 和跨 Branch 宽父矩形采样结果不得作为验收证据。

## 2026-11-12：第三轮投机树最终几何规则（历史）

本轮取代下文 11 月 11 日关于“稳定代价交换左右通道”和旧压缩实现的临时规则：兄弟方向不再通过 cost 交换，创建顺序固定；确定性候选角度扫描保证同一父枝在 40px 内的兄弟分叉至少相差约 15°。主干使用 1 秒逻辑时间格与 Branch 自有年龄的 `log1p` 距离，调用与 Chance 共用追加式最小 28px 槽位；模型/工具尺寸也使用对数映射。树干后缀调用数非线性归一化到 1.5~9px，保留至少 1.5px 的空尾巴。Chance 圆的直径等于子枝根宽度，透明命中区显示既有 Chance/Proposal 信息，但子枝根头本身不重复绘制。旧实现仍保留在本文件仅作为历史记录，不再是当前语义。

验证结果：`npm run typecheck`、`npm run build`、`cargo build` 和 `git diff --check` 均通过；`/tmp/opencode/assert-tree3.mjs` 的 fixture 数值断言全部通过。针对 rich fixture 的 129 个事件实测 `foldEvents → layoutTree` 约 9.1ms，生成 473 个采样点、468 个树干段；方向 hash 使用 `task/branch/tick`，14px 仅是渲染细分。root 的 72 calls 首段宽 9px，8-call child 首段约 2.944px，28-call child 首段约 4.641px，均向 1.5px 尾部收窄。57 个工具叶片的 DOM rectangle 无重叠；Chance 圆心误差约 0.0052px；light/dark 有效前景-背景对比度实测分别约 16.30:1 / 14.51:1，composer textarea 宽度 1016px。未添加仓库测试，未调用真实模型。

## 2026-11-11：投机树按逻辑时间生成自身路径

废弃 lane、固定横向轨道、子树重量排版、`FORK_DROP`、`FORK_ROOT_HEIGHT`、`STUB_FORK`、人为根部弯曲和父子独立连接线。WebUI 只根据 Branch 自身事件生成路径：Branch 起止使用已有 `timestamp_ms`（未结束到任务逻辑时间），模型/工具使用各自的开始、结束和持续时间字段；缺少逻辑时间时仅隔离地按事件顺序降级并记录，不以浏览器时间或 sequence 冒充时间。模型叶茎左右交替，工具均在该模型叶茎末端成簇。

父子接点必须来自 chance 的真实父事件。当前观察 `ChanceState` 没有保留 `chance_created.timestamp_ms`，而 `branch_created` 有 `timestamp_ms`，所以本轮临时使用子 Branch 创建事件作为明确接点，并在 WebUI 文档记录数据缺口；不修改后端协议、不使用父 Branch 最后一个 sequence 猜测。树干宽度由当前 Branch 与全部后代在该逻辑时刻之后的调用后缀数连续 clamp 映射，Promote 不能改变几何。方向由横向偏移连续改变的概率函数和稳定 hash 决定。

本文记录已接受的实现架构与验证决策。目标语义归 `docs/design/` 所有。

## 2026-11-11：投机树接枝与复叶语义修正

废弃虚线分叉、候选中空描边，以及“模型小枝条上沿茎排列工具叶子”的表示。Branch 以初始权威根轴为中心分配绝对左右通道，嵌套链复用已空通道，不累加横向偏移；分叉由子 Branch 的 SVG 直接绘制连续实线并接入枝干。模型调用是一根叶茎，工具调用只在末端形成叶片簇；每次模型调用独立，仅同一模型调用内的连续同名同状态工具可合并。权威变化仍然只改变样式，不参与几何计算。枝头缩小为 6px 状态标记，保留透明点击范围。

## 2026-11-11：第二轮树形几何与 headless 验证

- 方向概率使用当前树点相对初始权威中轴的全局 x 坐标；稳定随机值同时决定转向方向和相对上一段方向的转角幅度。Branch 局部偏移和递归 seed 不再作为方向依据。
- 子 Branch 的首个树干点直接位于父 Branch 的 Chance 分叉点；不再绘制侧向连接曲线、额外根部偏移或虚线。子 Branch 从该点以自己的稳定初始角度继续向下生长。
- 同一父 Branch 的子枝按 Chance 创建时已知的时间/sequence 排序；左右方向只在布局创建时按稳定代价交换，Promote、Discard 和后续事件不会重排已存在的前缀。
- 空 Branch 保留最短树干并可继续承载子 Branch。父干粗细在 Chance 位置之后计入所有后代的调用总数，而不是用父 Branch 的 sequence 近似判断。
- 长逻辑时间使用单调有上限的压缩映射；本轮丰富 fixture 包含 3 层 Branch、3 个同父 Chance、已接受 Branch、空 Branch 子树、30 分钟调用和几十次权威调用。headless Chromium 测得 5 个 Branch 的节点包围盒约 `193px × 920px`；深色和浅色主题截图均已生成。

废弃本轮之前的“子 Branch 侧向连接曲线”实现；它虽然恢复了视觉上的侧向生长，但根部不再是 Chance 点的直接分叉。

## 2026-11-10：嵌套 Chance 接通与提交资格

废弃“Chance 的权威等待点要求父 Branch 是当前权威”的实现限制。Chance 的权威值来自父 Branch 自己的真实执行，只要父 Branch 仍存活（权威或候选），等待点就成立；候选 Branch 上再次 `admit` 的嵌套 Chance 与根 Branch 上的 Chance 使用同一套 `ChanceStage`、验证和失效规则。嵌套候选 Branch 的 `AssumptionSet` 含祖先未确认的 Proposal，祖先 Chance 未 Accepted 前没有提交资格；match 成功但尚不可提交时保留在 Resolving，待祖先确认后再按同一 Promote 流程处置。父 Branch 进入 Discarding/Discarded 时，其上未收敛的 Chance 以 `Discarded(AncestorInvalidated)` 收敛，这是嵌套失效的唯一传播路径。候选 Branch 的外部效果仍受 effect 闸门约束。

## 2026-11-10：Agent 接入侧通用层与 Pi 适配分离

Agent 接入侧的通用逻辑迁到 `agent-access/core/`：本地 Policy 求值调用、`extent` 计数与求值、`effect` 闸门、`branch_suspended` / `branch_resumed` 上报和等待控制面处置的 hold。`agent-access/pi/main.ts` 只做 Pi 执行点接入并调用通用层，行为不变；产品专有的消息、checkpoint、session 和 SDK 细节不进入通用层。`extent` 与 `effect` 由通用接入层求值，Product Integration 不求值；控制面代码不新增求值。接口、覆盖范围与限制见 `docs/development/agent-access.md`。

## 2026-11-10：Chance 状态与 BranchStatus 合并，废弃 BranchPhase 等旧实现

Chance 现在是 Tree 中 `admit` 通过时创建、拥有不透明 id 并持有 Proposal、关联 Branch、权威值状态与验证依据的记录；Branch 只保存 `chance_id`。`select` 的输入是全部 Pending Chance 的 `frontier_view`（字段 `chance_id`），不再只处理单个机会；`predict` 不再接收或猜测 NLE 专有的 `action` 字段。`AssumptionSet` 改为 `(chance_id, proposal_id)` 引用。

由此废弃以下旧实现与表述，正式文档不再保留：

- `BranchPhase` 枚举及 Branch 执行状态上的 `phase` 字段、phase 相关观察/展示。控制面实际推进到哪一步不再由 Branch 上的阶段表示；Decision 历史由 Ledger 和观察事件记录。
- `BranchStatus::AwaitingValidation`。已跑完、等待裁决的状态由 `BranchStatus::Finished` 表示。
- “`launch=false` 等于不创建候选”。现在 `launch` 只决定是否暂不取得权威值，Proposal 与 candidate Branch 仍按 `predict` 创建。
- “`select` 只收单机会”。现在 `select` 接收全部 Pending Chance 的有序子集；未被选中的 Chance 保持待选，并在权威值返回后按 `NotSelected` 收敛。
- Proposal 内嵌在 `BranchRecord` 中。Proposal 现在属于 `ChanceRecord`，Branch 只保存不可变候选执行与 `chance_id`。
- 验证依据借存于 `StartCandidate` 的 pending operation。权威结果与 Runtime 证据现在挂在 Chance 上，`PendingOperation` 只保存操作身份和状态。

## 2026-11-10：模型用量不得静默丢弃，`action_id` 可为 null

真实 NLE 运行中模型已被调用，但控制面没有收到任何 `model_usage`。根因是 Pi 在 `message_end` 之前先用 `stream.result()` 结算模型流并执行 `beforeModelInference` 的完成回调；该回调把 action 从配对队列中移除，随后 `message_end` 找不到配对项，旧实现直接丢弃 `Usage`。修复后完成回调只结束 action 生命周期，`message_start` 标记对应的 action，`message_end` 才取走并发送 `Usage`；只有没有产生消息的请求才在完成回调中移除。

此前要求 `Usage.action_id` 必填字符串。现在接入侧确实无法把一次模型调用与已上报的 action 配对时（例如调用没有经过 Hook），仍然发送 `Usage`，`action_id` 为 null；公共事件 `model_usage.action_id` 也允许 null，不伪造身份，也不静默丢弃。控制面按 Branch 保存每次模型用量，不再用 action id 作为唯一键，因此同一 Branch 的多次未配对调用都能保留并计入结算成本。

## 2026-11-10：对话观察移除 unverified 字段

废弃 `GET /api/branches/{branch_id}/conversation` 响应与 `conversation_updated` 事件中的 `unverified` 字段，以及“候选内容必须明确标记为未验证”的旧表述。候选是否属于投机由 Branch 的角色（Authority/Candidate）与状态完整表达；对话接口改为返回该 Branch 当前的 `role` 与 `status`，前端据此显示投机身份。旧字段名仅保留在本记录。Agent Adapter 能力验证语境中的“未验证”是另一件事，含义不变。

## 2026-11-07：首版通用文件系统 Backend

文件系统隔离实现为独立 Backend，仅拥有 Overlay upper/work 生命周期和 Agent
启动时的文件系统视图；不拥有网络、内存、资源或 Agent Session。BaseRuntime
在该 Backend 提供的 bubblewrap 命令中启动 Agent，mount/user namespace 由
启动入口继承到全部后代进程。Linux namespace、bubblewrap 或 OverlayFS 不可用
时必须拒绝启动，不得回退成普通 cwd 启动。Branch 使用父 upper 的只读 lower
视图，promote 为环境侧 no-op，discard/cleanup 删除 child upper/work；首版不
自动合并文件变更，也不把文件写入自动提升为权威效果。

启用 `filesystem` 配置时，即使没有填写 `base_runtime`，装配器也自动创建
`BaseRuntime`，避免 Backend 建立成功但 Agent 启动绕过隔离。BaseRuntime 负责
Agent 进程组和停止，文件系统 Backend 只负责 namespace/Overlay 视图；进程
主动创建新 session 后不在进程组停止保证内。

cleanup 的固定顺序是先停止 BaseRuntime 管理的 Agent 进程组，再清理 inner
和 filesystem；停止失败会保留 AgentProcess 句柄，禁止删除 upper/work。即使
inner terminate 失败，terminate 仍先尝试停止 Agent，避免其继续污染 Overlay，
并合并报告所有失败。BaseRuntime 的无 context 方法统一委托对应 context 方法，
防止绕过 filesystem。

组合 Runtime 在 establish/branch 的 inner Backend 中途失败时，不能直接调用
filesystem cleanup（其合法前置状态是 Terminated）。现在回滚顺序固定为
filesystem terminate -> filesystem cleanup；原始错误的 completed/remaining 保留，
terminate 或 cleanup 失败追加到 cleanup_failures。若回滚仍失败，环境 metadata
保留以便后续收敛，不伪造已清理。

BaseRuntime 组合 filesystem 后，generic operate、tool_context 和 invoke_tool
仍由 inner Backend 执行。filesystem 不再执行 generic operate，也不返回自己的
命令输出；它只在这些调用前检查环境是否为 Available，并在 Agent 启动时提供
可由后代进程继承的 bubblewrap namespace/Overlay 视图。这样 NLE/Firecracker
等已有业务 Backend 不会被静默绕过，且同一命令不会因组合而执行两次。

本轮范围审查确认 Adapter 的生命周期门槛是 Runtime Adapter 契约的通用语义，
不是为 filesystem 临时增加的限制：operate/promote/suspend 需要 Available，
branch 允许 Available 或 Suspended（暂停只阻止父环境内的新操作，派生读取
父状态仍是合法操作），resume 需要 Suspended，terminate 允许 Available 或
Suspended，cleanup 需要 Terminated。保留这些 Adapter 门槛不会改变未启用
filesystem 的 NLE/Firecracker 行为；filesystem 专属的状态检查仍只在
FilesystemBackend 和配置了 filesystem 的 BaseRuntime 中执行。

## 2026-11-06：通用 Agent 事实协议

废弃 `ToolPending`、`ParentOutcome`、`CandidateCompleted` 和含义混合的 `CleanupCompleted` Agent 消息，以及 `Hello` Agent 握手命名。分别使用 `ActionStarted`（等待边界由 result_pending 表达）、`ActionFinished`（带 action kind 与终态）、`AgentSessionReleased` / `BranchReleased`、`AgentConnected`；Runtime 握手不受此命名变更影响。Transcript 不再由 Adapter 声明 Authority/Candidate 角色。线协议与 Pi 接入同步切换，不保留旧消息兼容分支。

`CommandResult` 扩充 Accepted/Started，只有终态消费 pending command；Applied 仍只表示 Agent Adapter 已落实。Runtime Adapter 发出操作开始/完成事件，保留已有 OperationResult 请求配对；控制面记录实际 Runtime 事件。环境建立失败由 Runtime 结果驱动 Candidate 丢弃，不再构造 Agent BranchFailed。目标事实类型的完整定义见 `docs/design/agent-integration.md`；原型接入覆盖范围见 `reference-workload.md`。

普通 `ActionStarted` 只登记 action 事实。只有接入侧本地 Decision 明确产生 `SpeculativeAdmissionRequest` 时，控制面才会登记投机操作并调度 Policy 预测。Pi 工具 Hook 已执行下发 artifact 中的 `admit`，普通工具不自动准入，Policy 允许时才产生投机准入请求；validator/evidence-only 路径与完整全局 rebalance 仍未实现，代码必须保留准确的未启动状态。

本原型现已把 Proposal 独立保存在 `SpeculativeTree.proposals` 中；Branch 只保存 `proposal_id` 和 `assumption_set`。当前实现为每个候选建立一个直接 Proposal，AssumptionSet 仍按 Proposal ID 表达，但尚未实现任意多 Proposal 依赖的传播、确认和重挂载。控制面下发编译后的本地 Decision artifact，Pi 在真实工具 Hook 中求值；`extent` 由接入侧按 Branch 隔离的 Hook 次数求值，Hook 次数不是 `steps_completed` 的完成步骤数。`launch=false` 会保留 Proposal 并记录 validator/evidence-only unsupported，不启动 Authority Invoke。控制面 `BranchPhase` 只记录控制面实际推进的阶段：显式准入请求进入 `select`，随后按实际路径更新 `predict`、`launch`、`effect`、`match`、`dispose`；本地 `admit` 不单独占用控制面阶段。完整全局 `rebalance` 仍未实现；本地 extent stop 已可在 Hook 边界暂停，并支持取消、discard、cleanup 和合法恢复。

## 2026-11-04：Runtime 整体边界与首阶段范围

已收敛：AScout 自身支持一套完整 Runtime，承载 Agent 一步动作，使其正常、正确运行，并把对真实世界或 Authority 的影响限制在容忍范围内。Runtime Adapter 与 Effect Coordinator 是这个整体中的接入和效果管理职责，不是平级的独立运行系统。Agent Session 内部管理仍归接入侧，Runtime 负责动作承载和外部效果边界；状态可以复制、共享、写时复制、重建或恢复，优先低成本方式。

能力可插拔、组合或缺失，但必要保证不能缺失。第一阶段先实现覆盖 Agent 及其派生进程文件操作的 Overlay 文件系统，不要求 VM；Overlay 本身不是 Runtime 的固定必需项。后续才展开 Runtime 与 Policy 的协调、协商和运行时检测。该决定最初只更新文档；文件系统 Backend 其后已实现 overlay/btrfs 数据面、逐挂载 overlay、live mount accept 与进程 Backend，当前覆盖范围和偏差见 [文件系统 Backend 当前实现状态](../fs-backend/current-state.md)。

**废弃记录**：不再采用“Runtime 仅是 AScout 不拥有的外部后端”“Runtime Adapter 是独立一级侧面”“外部效果执行只属于控制面而不属于 Runtime”的架构定义。此前四侧路径中的 Adapter 现在作为 Runtime 的接入边界；效果协调归入 Runtime 整体责任。这里改变的是职责定义，不预定 crate、进程、transport 或部署方式。既有 Branch、状态可见性、commit/discard 和 single-winner 规则保持不变。

目标说明见 `docs/design/runtime.md`；首阶段与未实现能力见 `docs/development/runtime-adapter.md`。本节取代本文其他历史记录中关于上述架构边界的表述。

## 2026-11-02：Candidate 最终失败不再等同于立即丢弃

问题：Candidate 已进入 `Promoting` 时，模型返回 429，Pi 立即报告 `branch_failed`；控制面把它当作普通 Candidate 失败并丢弃，而 Pi 已先切换 authority，造成两侧权威分叉。

现在的行为：

- Promote 仍是一步决策；Candidate 的模型续跑使用 `AgentSession.continue()`，保留 Pi 的自动重试、压缩和 settle 流程。
- Pi 只在重试耗尽等最终失败后报告 `branch_failed`，并记录失败/取消状态；此后的迟到 Promote 被拒绝，不切换 authority。
- 控制面在 `Running` 收到失败时丢弃 Candidate；在 `Promoting` 时暂存失败，等 Promote 结果；Promote 已应用后失败属于权威失败，显式结束 run，而不是当 Candidate 丢弃。
- Runtime 增加固定生命周期操作 `promote_environment`，在 Agent Promote 前调用；共享环境实现可以 no-op。Runtime promote 失败则丢弃 Candidate，不切换 Agent authority。

验证：新增 Rust 状态机回归测试和 TypeScript `AgentSession.continue()`／PiAdapter 晚到 Promote 测试；`cargo check`、`cargo test --lib --bins`、`npm run check` 通过。未运行真实 NLE。

## 2026-11-02：Pi 模型参数直接传入接入入口

AScout 启动的是 Pi 的 `agent-access` 接入入口，不是 Pi 交互式 CLI。此前该入口没有解析模型参数，只能回退到 Pi 配置中的默认模型；用户配置里的默认模型过期时，会错误地落到不可用模型。

现在 `agent-access` 接受 `--model <provider>/<model>` 和 `--thinking <level>`，通过 Pi 的 `ModelRuntime` 与 `resolveCliModel` 解析后传给 `createAgentSession`。控制面把这两个参数原样转发给接入进程；Authority 使用显式模型，Candidate 继续继承 Parent 的模型与 thinking level。认证仍使用 Pi 默认配置，不再需要复制独立的 `PI_CODING_AGENT_DIR`。

同时把 NLE 演示依赖改为可复现的相对布局：`requirements-nle.txt` 固定 `nle==1.3.0` 与 `gymnasium==1.2.0`，`nle-runtime.json` 使用 `../.venv-nle/bin/python` 和 `examples/nle-runtime-bridge.py`。Runtime 配置由 Policy 的 `runtime` 字段选择，控制面只在需要覆盖时读取 `ASCOUT_RUNTIME_CONFIG`。

验证：`npm run check`、`cargo build`、`cargo test --lib --bins` 通过；使用 `--model deepseek/deepseek-flash --thinking low` 和默认 Pi 认证运行 NLE，12 次动作中 11 次 Promote、1 次 Discard、0 次 `branch_failed`，模型调用均为 `deepseek-flash`。

## 2026-11-02：NLE 游戏内容留在 Runtime

- `nle_action` 的 `result`（位置、生命、消息、地图摘要）和 `evidence` 的构造移到 `examples/nle-runtime-bridge.py`；`NleBackend` 只转发，不解析 `blstats`、`chars`、`message`。
- 控制面只解析通用的 `{result, evidence}` 外层；Pi 只把 `result` 文本交给 Agent；只有 Policy 读取结构化 evidence 做游戏语义判断。
- 完整 NetHack 3.6 Guidebook 来自 NLE 1.3.0 源码包，放在 `examples/nle/Guidebook.txt`；Runtime 通过 `nethack_guide` 工具按章节提供，不再把手工摘要当作完整规则。

## 手动试用后的展示修正

历史方案：曾讨论以独立 PTY 与终端模拟器嵌入 Pi 原生渲染，但该方案已被放弃，不是当前目标。当前只通过 Pi 公开 SDK 获取 AgentSession 的消息、思考、工具调用、结果和事件，由 AScout UI 自行排版；不修改 Pi 源码、不调用 Pi 内部展示组件，也不捕获 Pi 原生终端画面。

当前限制：不接管 Pi 完整 InteractiveMode、原生编辑器或命令菜单；输入使用 AScout 的 Prompt/Abort 路径。图片和其他尚未映射的消息内容会明确标记为未展示，不伪造为普通文本。

验证：零模型调用的真实 PTY 检查覆盖原生 Markdown、中文、工具结果、分支内容分隔、窗口 resize、socket 断开退出；控制面检查覆盖 Tree、非法编号、Branch 查看、Tab 运行事实、Esc 返回及 Ctrl-D。Pi 完整 `PATH=/usr/bin:$PATH npm run check` 通过。Promote 后连续流式展示未做真实模型验收。清理确认后释放 PTY 文件描述符，只保留模拟器的最后画面；未变化画面不重复刷屏。

- Runtime Adapter 部署形式暂不变更；先修正 `/ascout` 无效编号、Branch 上下文和工具展示。
- 展示代码放在 `src/bin/ascout-control-plane/view.rs`，只格式化已有事实，不改变 Branch 生命周期或输入路由。
- 分支详情使用结构化文本，分别显示运行事实、创建时继承的展示上下文、分支自身输出；候选创建后不再跟随 Parent 的后续输出。
- Pi 接入侧不再把工具事件拼接成 Output 文本；控制面根据已有 Transcript 显示工具调用和返回。Output 只承载实时模型正文和明确错误，Ledger 的原始事实记录保持不变。
- NLE 任务知识放在外部示例 `examples/nle-task.md`，由用户提供给 Agent；动作表基于实际 `NetHackScore-v0` 环境核对，不把游戏规则写入核心。
- 当前只做基础配色与内容层次，完整 Pi 风格 Markdown、工具折叠、多行输入及宽字符排版仍未完成。

## 2026-11-02：Runtime 启动 Agent 接入侧，运行目录与 Ledger 归位

已确认三项归属，并按此修改了控制面：

- **Agent 接入进程由 Runtime 启动**。控制面（用户）决定用哪个 Agent 产品、哪个版本，把命令、参数、工作目录和日志路径作为 `operate_environment` 的启动参数交给 Adapter；Adapter 在环境内启动它，并在自己的进程退出时回收。控制面不再 `Command::new("node")`，也不再持有 Agent 进程句柄。
- **运行目录由调用方创建**。控制面原本自建临时目录，既放日志又当工作目录；这属于实验脚手架。现在第一个命令行参数就是调用方创建好的运行目录，控制面只使用它，不再删除或回收。
- **事件记录归 Ledger**。新增 `src/control_plane/ledger.rs`，控制面通过 `Ledger::open` / `record` 追加事实，不再自己 `writeln!` 到文件。Ledger 只记录，不参与决策。

附带约束：当时 `ASCOUT_RUNTIME_CONFIG` 是必需的，因为环境和 Agent 都由 Runtime 建立；后来 Policy 的可选 `runtime` 字段可以提供同一路径，环境变量只用于覆盖。

Agent 接入侧仍然是唯一控制 Agent 会话的地方：创建会话、克隆上下文、发送 prompt、中断、Promote/Discard、清理都在接入侧执行，控制面只发命令并接收事实。

## 2026-11-02：控制面不再持有环境状态，结果判定交给 Policy

已确认三项职责归属，并按此修改了 NLE 测试路径：

- **控制面只发命令，不保存环境状态**。原先控制面里有一个包装结构，自己记着“哪个环境是权威”、每个环境的坐标和最后观测，并用坐标差算出 `moved`/`blocked`。这些职责归 Runtime Adapter：控制面按 Branch 名字发操作，Adapter 负责对应到环境；需要观测时向 Runtime 要一次 `observe`。控制面现在只持有 Adapter 进程句柄和协议会话，没有专门的包装名词。
- **观测缓存只有一份，在环境自己那里**。`NleBackend` 原先自己存一份最后观测，绕过了 bridge 已经实现的 `observe`；控制面又存了第二份。两处都删了，`observe` 直接转发给 bridge。
- **真实结果的判定属于 Policy**。Runtime 只交回动作前后的两份公开观测；`predict` 从观测产生预测，`match_proposal` 从 `blstats` 读出实际是 `moved` 还是 `blocked` 再与预测比较。控制面不再出现坐标、格子或移动这类词。

工具通道现在由 Runtime 生成 `result` 文本（可读摘要）和 `evidence` 结构化事实；控制面只转发，不解释游戏字段。早期 `{"is_error", "observation"}` 的结果形态不再使用。

## 2026-11-01：一次 run 只提供一个工作目录，不再按 Branch 复制目录

已确认：控制面不再为每个 Branch 创建和复制工作目录，一次 run 只有一个工作目录，所有 Branch 共用。Branch 是同一执行状态的别名，不各自持有目录。

原因：Pi 把会话工作目录的绝对路径写进系统提示，而系统提示位于对话最前面。每个 Branch 一个目录时，该路径每开一个候选就变化一次，导致整段对话的前缀缓存在每次新建候选时全部失效。48 个动作的实测中 87% 的花费来自这类整段重读。

影响与边界：

- Candidate 在当前实现中不执行工具（工具调用暂存到 Promote 才放行），共享目录不扩大它的实际能力。
- 目录级隔离属于 Runtime 职责，不能靠命名不同的 Branch 目录冒充。需要并行执行工具的候选时，由 Runtime 提供隔离。
- 已废弃：按 Branch 复制工作目录（`copy_workspace`）、清理时删除 Branch 目录、`BranchRecord.workspace` 字段。控制面在收到 `CleanupCompleted` 时只更新 Branch 状态。

## 2026-11-01：Branch 非干扰、环境形态与产品接入解耦

已确认以下边界：

- **Branch 的固定要求是非干扰**：一个 Branch 不得影响权威路径，也不得影响其他 Branch。独立执行环境只是实现非干扰的一种方式，不是 Branch 的定义，也不是所有 Runtime 的前提。
- **`branch_environment` 只表示派生独立子环境**。不能派生独立环境的后端不得用共享别名伪装成功；该操作是可选能力，不是所有合格 Adapter 都必须具备的固定操作。
- **共享环境 + 作用前阻止**是另一类非干扰实现。NLE 采用这种方式，但它属于 NLE 专用 Runtime／测试 workload 的描述，不属于 AScout 核心契约。AScout 只要求 Adapter 声明并兑现所选非干扰机制的范围与限制。
- **NLE 只是测试用例，不属于 AScout 实现**。NLE 的 bridge、策略和运行环境作为外部 workload／适配保留，不进入核心 crate 的能力边界；核心契约不得出现 NLE 专用语义。
- **AScout 不与 Pi 耦合**。Pi 是首个接入实现，不是架构组成；设计文档不得把 Pi 写成 AScout 的前提，控制面也不应把 Pi 的路径、工具或会话结构固化进核心协议。

当前实现差距：`src/runtime/backends/nle.rs` 仍在核心 crate 中。控制面已不再对 NLE 调用 `branch_environment`，也不再识别 `nle_action`；它按 Runtime 声明的共享环境和环境工具调用 `tool_context`／`invoke_tool`。Pi 接入的启动已按 2026-11-02 决策移入 Runtime Adapter。

## 2026-10-31：NLE 投机改为动作边界暂存，废弃进程级 fork 分支

已确认环境不可修改：不为适配 AScout 在 NLE 或任何外部 workload 侧增加克隆、分支或回放能力。先前在 `examples/nle-runtime-bridge.py` 中用 `os.fork()` 复制环境进程、用 socketpair 建私有通道、失败后反向移动回退的方案废弃。废弃原因：

- fork 让 workload 脚本承担了分支语义，而隔离策略本应属于 Runtime；
- 它只能从根环境分支，从已 Promote 的候选再分支会得到没有游戏对象的空壳；
- 子进程异常退出时会掉回父进程主循环，产生重复响应；
- 单世界下反向移动仍消耗回合，怪物会行动，“可逆”只是位置可逆。

CRIU 一类进程快照同样不采用：需要特权、镜像 I/O 和 FD 重绑，每步一条分支不划算。

现行实现：环境单实例；`branch` 只是同一世界的别名，控制面不再使用它；Candidate 只在预测结果下推演一步，工具调用暂存到 Promote；动作前固定延迟（`action_delay_seconds`，当前配置 1 秒）；预测由固定程序从公开观测枚举，模型不负责预测。旧内容不再出现在正式文档。这里的 `branch` 是 NLE 专用测试适配的逻辑视图，不是 AScout 的 `branch_environment`；边界见 2026-11-01 条。

## 2026-10-25：随机双 Candidate 与 Parent 不等待裁决

用户确认 Workload 与 Policy 分离：Parent 真值随机取 example/true/false，Policy 用非固定随机抽取两个不同预测；候选执行 Predicted 命令，Parent 默认继续 spec fail。Parent 结果交付不等待比较，只有 Promote 才取消旧 Parent。等待点按 Parent 和 operation 归属，不因一条消息有多个工具而整组拒绝；其他工具结果需真实配对，不允许重复其效果。Promote 后取消旧 Parent 下其余候选，保持 single-winner。工具准入不写入 Pi；由 Policy、AScout 和 Runtime 的能力边界共同决定。

废弃先前 afterToolCall 等待 ResolvePending、单候选容量、固定 example 预测及以单工具数量作为准入条件的实现。等待点准入和结果匹配在示例 Policy；Pi 不按工具名或命令格式筛选 Candidate 工具。当前接入把布尔预测和字符串枚举预测转换为 Agent 工具所需的文本表示，复杂结果转换仍有实现差距，见 reference-workload.md。非标准上下文仅记录，暂缓处理。

随机场景位于独立脚本模型接口，不是控制面业务逻辑，也不是大模型成功证据。已有脚本命中实测确认两个候选先执行、Parent 真值返回、单一 Promote 和 loser/Parent 清理；既有在飞输出验证与本次随机场景验证应分开描述。

## 2026-10-25：TUI 与实际投机过程观察优先

先实现稳定的 AScoutAgent TUI，由 AScout 接收 Agent 输出和运行事件并呈现，输入与中断经对应 Agent 接入实现传递；产品差异留在接入实现，不依赖 Pi 原有主界面。增加当前 AScoutAgent 整棵 Tree 的只读观察，可选择具体 Branch 深入查看。先使用简单环境，不把接通完整 Runtime Adapter 环境管理作为前置条件；不据此声称已实现完整环境隔离。实际启动、执行、预测比较、Policy 处置与清理必须有事实依据，不能用两个独立模型请求或发送命令代替完成证据。当前已完成自有 TUI 与受限单工具真实等待点续跑，具体可用范围以 reference-workload.md 为准。

用户已批准 `/ascout` 为 AScout TUI 内的整棵 Tree 观察操作，选择 Branch 只改变观察对象，不切换主对话或触发生命周期操作；命令由 AScout 处理，不作为提示词发送给 Agent。该名称与含义记录于术语表。

废弃助手提出的“复用 Pi 主界面”的实现方向；目标是整合原 Agent 的体验，而非把 AScout 用户端绑定到 Pi TUI。首版仍接入 Pi，不同时实现其他 Agent 的适配。

首步实现：现有 `ascout-control-plane` 开发入口改为 AScout 自有终端界面，保持 socket 接入、控制面 Tree 和连续 Pi AgentSession，加入流式输出与只读 Tree/Branch 观察。废弃该入口原先在 Branch 启动时直接发起无共同等待点的候选、把模型最终文本作为 ParentOutcome 的演示路径；接入侧不再产生该占位结果。退出保留运行目录和诊断文件，不声称已确认所有工具后代及环境清理。

本轮连接真实投机：ToolPending 报告后让真实工具继续一次，复用既定 ParentOutcome 名称表示真实工具结果，不另引入 ToolOutcome。Candidate 使用独立目录与独立内存 SessionManager，保存共同上下文与预测 toolResult 后 continue，不以新用户提示词伪造续跑。控制面求值 admit/select/predict/launch/match/dispose，在合法 Promote 后发布缓冲输出并保持活跃 Candidate，不命中回退 Parent。未验证 Candidate 的工具调用由 AScout 的 Branch 处置控制：Pi 只等待控制命令，不按工具名或命令格式判断安全性；Runtime 能力与 Policy 决定仍需由控制面接入。默认示例不自动开启投机，新增可显式选择的 pi-tool.yaml。实测使用本地脚本模型接口加真实 Pi/bash，不能作为真实大模型效果证据。

废弃 D-031-I 中“用户不可直接观察候选”的限制；保留主对话只属于权威路径、候选不建立用户对话、用户不直接裁决 Branch 的边界。只读观察明确标记候选内容未验证，不向其他 Branch 反馈这些内容。

## D-029-I：废弃单 Edge 即时启动与固定 frontier 策略

2026-09-04 决定：废弃由 `admit` 精判直接决定投机是否立即启动的七槽决策面，现由 `admit` 决定机会是否进入待选集合，独立的 `select` 在同一 task / AScoutAgent 的全部待选 Edge 间选择当前启动项。也废弃将 best-first、固定的 `P(存活) × 期望延迟节省 / 资源成本` 排序、浅层分叉/深层单链或强制 `survival` 字段写成 AScout 机制规则的表述；这些机制作为 Policy 可选策略继续受支持。当前语义见 `docs/design/policy-decisions.md`、`docs/design/control-plane.md` 与 `docs/design/decisions.md` D-043。

## D-030-I：废弃 Scheduler 固定暂停与抢占策略

2026-09-04 决定：废弃“所有投机工作固定低优先级”以及 Scheduler 根据内置价值排序自行暂停或抢占投机工作的表述。Scheduler 继续拥有运行队列、资源和硬约束，但暂停、恢复和价值驱动的抢占由独立 `rebalance` Policy Decision 选择；抢占后的处置由 `dispose` 决定。当前语义见 `docs/design/policy-decisions.md` §2.9 与 `docs/design/decisions.md` D-044。

## D-033-I：Speculation Coordinator 吸收 Scheduler 的控制面职责

2026-09-04 决定：在控制面组件模型中，不再将 Scheduler 作为独立组件。其待选机会、运行调整、预算和硬约束执行职责并入 Speculation Coordinator；`select` 与 `rebalance` 仍由 Policy Decision 决定，Speculation Coordinator 在统一的全局投机视图上负责激活、复核和执行这些意图。D-030-I 中关于 Scheduler 的描述保留为历史决策记录，当前组件结构以本条和 `docs/design/control-plane.md` 为准。

## D-031-I：废弃端用户直接参与投机 Branch 裁决

2026-09-04 决定：废弃通过专门投机 UI 让端用户直接观察、选择或裁决内部 Branch 的表述。端用户只通过 AScoutAgent 视图操作当前有效的权威执行主线，并只能看到该路径的内容、状态与交互；人作为 Authority 提供输入或中断时，也通过该主线进入系统。Branch 接受、丢弃、重新挂载与清理在内部完成，不要求用户选择或重绑定 Branch。当前语义见 `docs/design/agent-integration.md`、`docs/design/branch-semantics.md` 与 `docs/design/decisions.md` D-045。

## D-032-I：前提变更暂不兼容或修复 Branch

2026-09-04 决定：废弃在用户操作改变 Proposal 前提后继续保留、挂起、兼容判断、修复或重新对齐受影响 Branch 的候选方向。现阶段所有依赖已变更前提的 Branch 及其 subtree 一律剪枝；未依赖该前提的 Branch 不受无关输入影响。当前语义见 `docs/design/branch-semantics.md` §6、`docs/design/policy-decisions.md` §4 与 `docs/design/decisions.md` D-046。

## D-034-I：抽象札记收敛为论文证据

2026-09-14 决定：`spec-exec-abstraction.md` 只保留论文覆盖、实验结果、负结果、效果处理现状、shadow/replay 方法学、接入与隔离事实、嵌套投机事实及待验证问题。已经在 `docs/design/` 正式呈现的生命周期、Policy Decision、Effect、架构、组件职责、契约和指标方案从该文删除；该文不再作为目标语义或实现结构的来源。

## D-015-I：首个真实 Workload 使用 Delayed-Choice Coding

首个真实端到端 workload 通过 Product Integration 的观察和拦截接缝接入 Agent，使用真实仓库。不要求 Agent 主动调用 AScout，也不开放显式生命周期协作。具体产品和接入方式尚未选择。Sequential 与 speculative run 使用相同 Agent、任务、outcome 和预算。

**废弃记录（2026-09-17）**：原先使用 `ascout codex`、Command + Skill，由 Agent 主动调用命令并先验证显式路径的路线错误，已废弃。确定性测试 Agent 专用于 conformance tests 的初期测试安排也不再采用，初期测试范围见 D-035-I。

Workload contract 与指标见 [reference-workload.md](reference-workload.md)。

## D-035-I：控制面最小闭环与初期实现约束

2026-09-17 确定：先实现控制面最小闭环。保持代码简洁、干净，避免为尚未需要的扩展增加抽象和占位实现；能以简短清晰代码解决的问题不引入冗长框架。

2026-10-23 更新测试约束：原型阶段默认不编写测试。只有实际遇到可复现、且今后很可能再次出现的问题时，才针对该问题编写测试；先让测试稳定复现问题，再修复代码直到测试通过。已有测试不因此自动删除，真实执行验证也不等同于必须新增自动化测试。

**废弃记录（2026-10-23）**：不再采用“设计初期先定少量端到端测试、须经用户同意并在业务逻辑实现前定稿”的普遍门槛。早先等待 test 通过／失败的两个 Proposal、确定性 FollowUp 及两个控制面场景的首批测试安排不再作为实现前提。

早期 `tests/test_outcome.rs` 及其确定性替代实现已经删除，不再作为当前正确性证据。当前 Runtime Adapter 端到端测试直接启动真实 Firecracker，验证 microVM 建立、guest 内操作、物理暂停与恢复、终止与清理、请求重试去重、在飞操作打断、意外停止事件，以及 snapshot/restore 分支和 sibling 隔离；SSH 诊断同样使用真实 virtio-net 与 guest sshd。当前已实现 task 身份校验、Proposal 独立存储和待选集合；仍未实现全局唯一 Proposal identity、九类 Decision 的完整接通、fallback、嵌套树、效果处理和主动环境验证。当前 Rust 接口仍是实验接口，不是稳定对外契约。

Adapter Registry 的主动环境验证延后实现，初期先检查接入接口与契约的对应关系。这是原型限制，不改变目标契约：接口检查不构成隔离保证的实际验证，相关能力不得标记为已验证。尚未实现的差距是 Registry 自主建立环境、执行验证场景并形成验证证据。

## D-047-I：重做由控制面拥有 Agent 状态的 Pi 最小接入

2026-10-24 确定：此前位于 Pi `packages/coding-agent/src/core/ascout.ts` 的 `SpeculativeTree`、`BranchRecord`、`ControlState` 和根目录一次性 `ascout.ts` 驱动不符合责任边界，统一作为废弃实验材料，不在其上继续扩展。它们把 AScout 的 Tree、Branch 生命周期、Policy 落实和用户入口编排放进 Agent 接入侧，不能作为正式系统或端到端完成度证据。

后续实现由 AScout 控制面拥有每个 Task 的 Speculative Tree 和 Branch 状态，控制面创建、登记、推进、接受、丢弃和清理 Agent 执行；Pi 只通过最小 Product Integration／Interposer 接入，报告 Agent 行为事实和 outcome，执行控制面返回的命令，不维护 AScout Tree、Policy 状态或生命周期裁决。控制面与 Pi 之间的通信必须传递受契约约束的事实、命令、结果和事件，不能以 Pi 的本地对象作为控制面状态。

第一版不等待完整 Runtime Adapter：候选 Branch 使用 AScout 运行管理建立的独立目录作为最低程度的文件系统隔离，目录创建、可见范围、清理和 Branch 归属由控制面／运行管理落实。仅修改 Pi 的 cwd 或在 Pi 内创建目录不构成已实现的隔离。

根目录一次性启动程序不得作为系统入口。测试可以是 AScout 内部测试，或从外部模拟用户行为的端到端脚本；测试脚本不能承载正式 Tree、Policy 或 Agent 管理职责，也不能把固定 `choice` 场景包装成通用系统能力。当前旧原型的真实模型运行只保留为历史实验记录。

首个实现步骤采用已确认的最小名称 `AgentMessage`、`AgentCommand`、`SpeculativeTree`、`BranchRecord`、`ControlState`、`BranchRole`、`BranchStatus` 和 `AgentController`。它们位于 AScout 控制面代码中：控制面创建 Branch 工作目录、处理 Pi 报告的启动和 outcome，并生成 Promote、Discard、Cleanup 命令。内部 Rust 测试验证控制面状态转移和目录隔离；随后已通过 `ascout-control-plane` 与 Pi `agent-access.ts` 的 Unix socket 连接完成真实跨进程验证。

已完成的下一步（2026-10-24）：旧 Pi-local 实现和一次性驱动已直接删除。Rust 新增 `ascout-control-plane` 二进制，Pi 新增最小 `agent-access.ts` 接入；二者通过长度前缀 JSON Unix socket 通信。控制面负责调用 Policy 的 `admit`、`select`、`launch`、`predict`、`match` 和 `dispose`，并且只有 `dispose` 的 Promote 意图才会触发 Tree Promote。真实运行已验证两个跨进程 Pi AgentSession、真实 `deepseek/deepseek-flash` 模型请求、Candidate 独立目录、控制面 Tree 更新和旧 Authority 清理。

该实现随后被发现仍有三项边界错误，不能作为正确闭环继续扩展：Pi 接入由用户手工启动而非由控制面启动；控制面把目录直接当作独立执行环境；控制面用 Candidate output 而不是 Parent 的真实 outcome 作为 `match` evidence。后续实现必须由 AScout 用户入口向控制面提交请求，由控制面启动 Pi 和 Agent 接入；由 Runtime Adapter 创建并报告抽象的独立 Candidate 环境，当前目录只作为该 Adapter 的实现细节；并由 Parent outcome 与 Candidate hypothesis 完成 Policy 匹配。

## D-036-I：Policy DSL 首版编译为 Python 脚本

2026-09-17 确定：PolicyDefinition 只通过 Policy DSL 定义；九类 Decision 都需要根据各自的输入作出规定输出，每个位置必须具有明确行为。固定取值和常用规则只是决策逻辑的快捷表达，例如 Eager 编译为恒定返回 `true` 的 `launch` 决策。

首版 Policy Engine 将 DSL 编译为可交付的 Python 脚本，并通过运行脚本对已编译的 Policy Decision 求值、作出本次决定。控制面可以直接调用 Policy Engine，也可以按需查询编译结果并将脚本交付给 Agent 接入侧，由接入侧以相同语义求值；Policy Engine 不负责落实决定、选择运行位置或传输。脚本是内部编译产物，不是另一种用户入口，也不能包含控制面内存对象或仅在编译进程内有效的引用。

2026-10-23 确定编写形式：YAML 描述 Policy 的结构、各 Decision 的行为和配置，并通过文件路径引用独立的算法源码。首版算法语言仅支持 Python，九类 Decision 暂时分别显式指定；不能通过省略某类 Decision 静默补齐行为。YAML 和被引用源码共同组成一个 PolicyDefinition，不增加独立算法加载入口，也不将自定义策略限制为内置算法菜单。源码语言与编译产物是独立选择，具体库范围和调用方式仍待确认。

2026-10-23 确定参数作用域：用户可在 YAML 中为各 Python 函数分别指定算法参数，传给对应函数；首版不支持全局共享参数，不设全局／函数配置的继承、合并或覆盖规则。参数与系统提供的运行时数据、授权能力分开。具体字段布局和调用约定仍待确认。

系统按每类 Decision 的约定提供数据，算法只在规定的数据及授权能力范围内计算，返回值经检查后由状态拥有者落实。源码路径在编译时解析并固定所用代码、明确依赖；运行期使用编译结果，跨侧交付不能只传作者机器上的原始路径。这里确定的是职责和交付边界，尚未确定具体 YAML 字段、函数入口或依赖打包方式。YAML、参数限制和静态检查都不能替代真实执行权限边界。

这一选择用于优先验证 Policy DSL、九类 Decision 的输入输出契约和跨侧运行路径，不把 Python 固化为长期执行载体。后续更换执行形式仍须保持已选语言语义和 Decision 接口，不能假定源码自动具备跨后端可移植性。

2026-10-23 用户授权开始实际实现，并确认 `policy`、`PolicyEngine`、`CompiledPolicy` 和 `Decision` 的代码命名。现有实现已打通 YAML 检查、独立源码固定、各函数参数传递、可序列化产物与按 Decision 导出 Python 脚本、隔离加载检查、实际求值、基本输入输出检查和失败返回。部署使用 bubblewrap namespace、用户 import 前 seccomp、rlimit、限量临时空间及受信调用侧超时监督，不使用普通 Python 子进程冒充隔离。实际遇到 bubblewrap 不支持 `--preserve-fds` 后，先新增真实环境的回归测试复现，再修复管道启动方式。

尚未实现的内容包括完整领域 schema 和 Contract／Fidelity 检查、外部模型等能力调用、第三方库环境固定、标准库内容身份、增量预测、score 规则、多数效果与处置计划检查、异步取消、cgroup 总量统计、生产级交付认证以及状态拥有组件和端侧真实接入。未知 YAML 字段和不支持的结果会被拒绝，不静默降级或伪造成功。具体 API、运行方式、数据清单、验证与缺口见 [Policy Engine 原型](policy-engine.md)。

开始实现不表示其全部候选能力已完成。代码中实际采用的原型接口以开发文档为准，不把尚未实现的草案能力写成可用能力。

## D-016-I：默认 Runtime 优先高效隔离

默认 Runtime 面向可信 Child Agent 与工具，优先低开销环境隔离，不提供 adversarial containment。它必须强制维护声明的环境、资源和效果边界，以及停止和 cleanup。Agent Session、执行实体的本地上下文、operation 归属和 capability 路由由 Agent 接入侧管理。

**废弃记录（2026-11-04）**：此前曾以“Runtime 只提供环境能力”为由，把外部效果实际处理移到控制面。此边界已被本文顶部的 Runtime 整体边界取代；Session 仍由接入侧管理，环境与效果执行归 Runtime，Tree 与全局裁决归控制面。

需要 adversarial containment 时，用户选择另行适配的 sandbox/hardened Runtime。AScout 不协商 security guarantee；当前首个实现为文件系统 Backend（overlay/btrfs）与进程 Backend，见 [文件系统 Backend 当前实现状态](../fs-backend/current-state.md)。

## D-017-I：新 Rust 实现不继承旧代码

新实现继续使用 Rust。旧 `src/**`、Cargo manifests、tests、examples、C header、构建产物和实验文件已经删除，未读取、迁移或复用。新实现只能依据已接受的设计与开发决策建立。

## D-024-I：实现组织服从 AScout 可控边界

当前实现组织遵循 Agent 接入侧、控制面与 Runtime 的责任边界。Runtime Adapter 是 Runtime 的接入边界，Effect Coordinator 属于 Runtime 的效果协调职责；这些职责不直接推出 crate、进程或部署数量。

Execution、Workspace、Input、Effect、Outcome 与 Interposer 的职责语义不绑定具体 crate、模块、进程或服务边界。

后续 crate、模块、进程和服务边界必须依据责任、依赖方向、权限、替换频率与部署需求决定，不能由“一侧一模块”或“一组件一 crate”直接推出。具体 Adapter transport、discovery、连接生命周期和 Runtime backend 属于 Adapter/部署实现选择。

**废弃记录**：此前先要求“实现组织服从四侧一级架构”，随后又把 Runtime 定义为 AScout 不拥有的外部后端、把 Effect Coordinator 放在控制面。两种解释均已由 2026-11-04 的 Runtime 整体边界取代；不再以“四侧”推导源码或文档拓扑。

## D-025-I：项目名称统一为 AScout

2026-08 决定：项目现行名称统一为 **AScout**（Agent Scout），相关现行复合名称使用 `AScoutAgent`。正式设计、架构、使用和开发文档不再使用 `ASCout` 作为现行名称；本条仅保留旧拼写用于历史追溯。该名称变更不改变既有生命周期、Policy、Runtime 或接口语义。
## 2026-11-02：独立 Agent 接入核心的第一步

已确认并开始落实四层边界：控制面继续拥有 Speculative Tree、全局 Policy 和 Branch 生命周期；TypeScript 接入包承载 Agent 无关的协议类型、命令传输、命令去重、能力声明和局部执行 Policy；具体 Agent 通过 Adapter 提供会话与工具切点能力；Runtime 继续负责任务环境和工具效果约束。

第一步已加入 `pi/packages/ascout-agent-access`。公共协议只使用 `branch_id`、`operation_id`、`tool_call_id` 和不透明的 `CheckpointHandle`，不暴露 Pi transcript 或 session 类型。`CheckpointHandle` 只能由产生它的 Adapter 解释，公共层只保存和转发。

## 2026-11-02：Pi Adapter 与 Runtime 工具声明

已完成第二步：

- `AgentAccessCore` 真正驱动 Agent 连接和命令分发，并缓存重复命令的结果；同一 `command_id` 携带不同命令时返回 `rejected` 命令结果，而不是把协议冲突误报成 Branch 失败。
- `agent-access.ts` 只保留入口连接与生命周期，Pi 的 session、transcript、checkpoint、工具拦截和候选续跑全部移入 `PiAdapter`。
- Runtime Adapter 在 `Hello` 中声明 `RuntimeCapabilities`：可执行的 `RuntimeTool` 列表、是否共享底层环境、是否支持独立 `branch_environment`。
- 控制面不再识别 `nle_action`。它按 Runtime 声明的工具名调用 `tool_context` 和 `invoke_tool`，用 Runtime 返回的 `result` 更新 Agent 工具通道，用 `evidence` 交给 Policy 匹配。
- NLE Runtime 声明 `shared_environment: true`、`branch_environment: false` 和 `nle_action` 工具；Candidate 不再创建虚假的 `branch_environment` 视图，环境动作只发生在权威环境。
- Pi Adapter 从 `ASCOUT_RUNTIME_TOOLS` 读取 Runtime 工具描述并动态注册；它不知道 NetHack、动作编号或 NLE 语义。
- 控制面在 ToolPending 后立即发送一次 `resolve_pending`；候选处置阶段只决定 Candidate 生命周期，不再重复发送同一 `command_id` 的 resolve 命令。

验证：`npm run check`、`cargo check`、`cargo test --lib --bins` 通过。本地脚本模型加真实 Pi 工具完成一次 Promote 和旧 Parent 清理，并在 Promote 后继续下一轮。真实模型 DeepSeek V4.1 Flash 加真实 NLE 完成 10 次动作：9 次 Promote、1 次 Discard、10 次 Discard／Cleanup 命令 applied、0 次 `branch_failed`，总花费约 $0.0060；候选 Promote 后继续执行，旧 Parent 被清理。

## 2026-11-03：Ledger 接管后的归档失败所有权

废弃此前“不新增 Ledger 失败语义、错误直接返回调用方”的表述：它只说明了错误如何传出，没有说明已经移出活动 Tree 的 Branch 由谁保存。按值移交后若追加失败，Branch 会随调用返回被释放，历史在任何组件都不负责的情况下消失。

现行规则：Branch 只有在 `Cleaned`、非当前 Authority、无未完成控制命令且无子 Branch 引用时才从活动 Tree 移交给 Ledger。Ledger 接管后，写入成功才按约定释放记录；写入失败时继续持有该 Branch，并由控制面报告这次运行的历史不完整。写入成功仍不包含强制 `flush`，失败后也不对已写入的部分内容作回滚或去重承诺，不引入自动重试或恢复队列，不把 Branch 放回活动 Tree。

实现：`Ledger` 保留未写入的 Branch，`archive_branch` 失败时不再丢弃记录；控制面退出时列出未能归档的 Branch 并返回错误。

### 2026-11-05：控制状态归树与消息触发

用户确认：task 创建时建立树，具体投机状态归树；Controller 不持有 task 状态，可协调多棵树。Policy 为决策函数，树的方法负责合法转换和后续操作要求。消息／定时器触发同一 task 顺序处理，耗时操作异步返回。Gateway 的统一范围暂未决定。

废弃实现：AgentController 持有 ControlState 和编号，及主循环每 100 毫秒扫描、同步调用 Policy／Runtime 的组织方式。此前 start_candidates、advance_candidates 与同步回调包装由树的事件处理和无状态协调替代。未经用户同意不提交。
# 2026-11-09：物化责任边界与 Firecracker 清理

控制面只报告操作间隙，Runtime 自行决定内部整理；控制面不持有物化专用语义。
撤回 materialize 提案中控制面触发 `FilesystemMaterialize`、新增物化状态/确认
字段、由控制面选择物化冲突策略的建议。本轮只同步文档，不实现新接口；宿主锁、
冲突处理及环境保留仍待后续明确。

Firecracker 停止未确认不标记 Terminated、不丢 Child、不拆资源；清理失败保留环境
和网络所有权供重试，全部清理成功才释放。启动失败回滚适用同一规则。生命周期
操作暂串行化，环境锁覆盖同环境操作，避免清理、重建与旧状态更新竞态。

# 2026-11-09：物化可选、宿主写回要求与后台回收

用户确认以下实现边界：

- 物化是可选能力，不在关键路径上。运行阶段物化后重置 authority 的基准；任务
  结束即结束 authority 的使命。
- 冲突处理倾向让 authority 覆盖，但尚未最终确认。当前暂不为被覆盖的宿主版本
  保留可恢复副本；此前 materialize 提案中的 journal/备份恢复方案不作为现行
  要求。
- 写宿主文件的要求是：写前逐文件与基准比对，再原子替换。实现时必须检查性能，
  先比较文件大小和修改时间，仅在不一致时再计算哈希。这是待实现的要求。
- 嵌套挂载当前未实现，可以另找实现手段；当前 bwrap 参数层做不到不等于实际做
  不到，不把该能力写成不可能。

后台删除改为信号驱动：首次 discard 需要物理删除时向工作者发信号；工作者持续
扫描 trash 并删除，直到为空后静默阻塞等待下一次信号，多次信号合并。启动时若
trash 非空发一次信号回收崩溃遗留。删除失败不得静默丢弃：记录日志并保留条目，
在下一次信号或有界延迟后重试，不忙等。工作者不持有 FilesystemBackend 强引用；
backend 被 drop 时工作者退出，未删完的目录留在 trash 供下次启动回收。btrfs
递归删除、只读属性处理、`BACKGROUND_DELETE_LOCK` 与 `fast_delete` 语义保持不变。

损坏的 `operations.json` 改为失败关闭：改名隔离成功时以空记录继续并大声告警；
改名失败时加载返回包含路径与底层错误的明确错误，不以空记录继续，也不覆盖或
删除坏文件。元数据写入是 temp + `sync_all` + rename 原子写，因此“写到一半
崩溃”不是 `operations.json` 的损坏原因；损坏来源包括磁盘错误、人为编辑和版本
不一致。环境/快照元数据读不出来仍跳过并记录日志。

**废弃记录**：移除 filesystem diff 的“比较另一个环境”参数及其未实现分支。
diff 现在只表示该环境相对其 fork 基准的变化，行为不变；accept 的 target（目标
环境）是另一个概念，不受影响。移除前检查了 protocol contract、Filesystem
Backend 实现与分发、测试和文档，没有调用者传入该参数。

# 2026-11-10：状态目录重叠默认拒绝启动

决定：Filesystem Backend 的状态目录必须位于本次要 overlay 的挂载点之外。
`establish` 和 `launch_fragment` 在创建环境根、upper 目录或环境元数据之前检查
状态目录是否落在被隔离挂载点内；重叠时返回明确错误，错误包含状态目录路径、
冲突挂载点路径，以及修复建议“把状态目录放到被隔离挂载点之外，例如独立挂载的
文件系统”。

理由：此前的 flatten 降级会掩盖部署错误，并让 fork/diff/accept 退化为
O(upper)。flatten 路径保留，但只允许通过内部开关
`allow_overlapping_state_directory` 启用；该开关仅用于测试与诊断，默认 false，
不属于外部配置文件。

**废弃记录**：不再把“检测到重叠后自动 flatten”作为默认策略。能力字段
`state_directory_overlaps_mount` 与启动日志保留上报，日志措辞改为说明默认拒绝
启动、除非内部开关开启。

# 2026-11-10：fork 的逐挂载 Chain / Flatten 布局与链深度上限

决定：fork 时每个有产出的挂载（upper 非空）单独选择一次布局，不再用
“状态目录是否在该挂载内”这种对挂载恒定的判断。

- **Chain**：对父 upper 做只读快照/复制得到 S，S 加入子 lower（子 lower =
  父 lower + S），子 upper 为空。fork 便宜；子 upper 只含子自己的改动，
  diff/accept 成本随改动量增长；代价是每次 fork 让链加深一层。
- **Flatten**：父 upper 整个复制（或可写快照）成子 upper，子 lower = 父 lower，
  不加入 S。链不增长；代价是 fork 复制父 upper，diff/accept 随 upper 条目数增长。

选择规则：状态目录位于该挂载内（只可能由内部测试开关
`allow_overlapping_state_directory` 制造）时强制 Flatten；否则用
`upper_entry_count` 封顶计数，条目数 < `DEFAULT_FORK_FLATTEN_ENTRY_LIMIT` 用
Flatten，否则用 Chain。新配置字段 `FilesystemConfig::fork_flatten_entry_limit`
供测试与基准覆盖（`0` = 总是 Chain，`usize::MAX` = 总是 Flatten），默认取常量。
它与 `SNAPSHOT_CLONE_ENTRY_LIMIT` 是两件事：后者决定 S 用复制还是 btrfs 快照，
前者决定 S/父 upper 放在子 lower 还是子 upper。

快照基准：`SnapshotRecord.lower` 无论布局都显式记录“父 lower + S”（该挂载有
产出时），作为 diff/accept 三方合并的基准视图。Flatten 下不能直接取子挂载
lower，否则基准会缺 S。

diff/accept：`in_lower` 只判断 mount.lower 是否包含 base 路径（Chain 为真，
Flatten 为假）。Flatten 走现有的 upper 副本比较（overlay：
`changes_from_upper_copy` / `upper_delta_from_base`；btrfs：`btrfs_delta`）。
accept 中 Chain 的 `delta_since` 必须排在 btrfs 分支之前；否则用 S 全量与只含
本分支改动的子 upper 比较，会把父内容全部误判为删除。`needs_source_maps`
不再按 btrfs/flatten 只读子 upper，而是 lower 为空时只读 upper，否则读取
`read_chain(lower)` + upper。

链深度保护：新增 `MAX_LOWER_DEPTH = 96`。本机无特权实测 `bwrap --overlay-src`
叠层：231 个 lower 源成功，232 个 `Invalid argument`；bwrap 把源转成
`/proc/self/fd/N`，限制与底层路径长度无关。取低于实测上限一半并留余量的 96。
超过时 fork 在创建任何目录或元数据之前返回 `BackendError`，说明当前深度、上限
和原因；不静默降级为 Flatten，也不实现整链合并。

阈值标定：在同一非重叠布局下强制 Chain/Flatten，用 `examples/fs_bench.rs`
（ITERS=5，N=16/64/256/1024/4096）。Flatten 的 diff+accept p50 在 N=256 首次
比 Chain 高约 12.3ms（此前 N=16/64 分别约 5.5/2.2ms），向下取 2 的幂得到
`DEFAULT_FORK_FLATTEN_ENTRY_LIMIT = 256`。数据与命令写入
`docs/fs-backend/current-state.md`；这是本机测量，换环境需重测。

用户已批准 `ForkLayout`、`fork_flatten_entry_limit`、`MAX_LOWER_DEPTH`、
`allow_overlapping_state_directory` 命名；对应含义分别为逐次布局、Flatten 阈值、
lower 深度上限、内部允许状态目录重叠开关。父目录变化保留，不通过生产过滤修改
diff；测试必须精确计入对应数据面会报告的目录。

**验证边界**：用户第三轮 `log-3.txt` 已进入真实 btrfs Chain/Flatten/mixed，但
Chain/mixed 因 EXDEV、Flatten 因缺父目录预期失败；第四轮 `log-4.txt` 已验证修复后
三项成功路径，详见下方第四轮记录。

**废弃记录**：`FilesystemBackend::mount_uses_flattening` 及“flatten 只由状态目录
重叠决定”的规则被删除；`launch_lowers` 不再因该判断忽略 lower 层，始终使用
`[挂载点] + mount.lower` 的真实链。重叠场景仍强制 Flatten，但由逐次布局逻辑
显式判断，不再有对挂载恒定的辅助函数。

# 2026-11-10：第三轮特权失败修复与证据

此前 send dump 修复历史：基线 `a37e75c` 的 `log.txt` 中宿主原始能力检查通过，
Rust btrfs 测试却在 diff 断言失败、未到 accept。解析器未去掉 stream snapshot
根（实际 `./upper/`）、只接受箭头 rename 而非真实 `dest=`、输出 `modify` 而非
契约 `change`。修复按 snapshot 根和 inode 路径生成最终增量，临时创建/rename
不产生伪删除；真实目录 rename 只扫描目标子树补足未修改子项。真实 dump 格式
回归先失败后通过，accept 共用增量视图同步修复。随后 `log-2.txt` 319–324 行的
两个旧特权测试通过，仅覆盖当时 Flatten 与 direct-root，不外推其它布局。

- btrfs 挂载存储使用父 volume 子卷，内部含 upper/work。OverlayFS copy-up 的
  work→upper rename 必须位于同一子卷；普通用户真实回归已复现旧布局 errno 18，
  修复后覆盖建立、新挂载、空子挂载、Chain、Flatten、mark 与恢复。fork/mark/diff
  快照父 volume，send dump 只解释 upper 子树；其中父目录变化仍保留。
- 旧的 upper-as-subvolume 布局恢复时明确拒绝并保留原数据、元数据和 work，不做
  自动搬迁；upper/work 的同设备检查也在创建/启动前执行，防止再次延迟到 copy-up
  才暴露 EXDEV。
- 特权脚本使用 `--show-output --format=pretty --color=never`，避免 nocapture
  打断 libtest 结果行。`check-test-output.py` 分别验证精确名单、命令退出码、结果
  行、running 数和最终摘要；执行数=通过+失败，ignored 不算执行，任何 SKIP 失败。
  真实 libtest 交错回归先 red 后 green，保留原假命令场景，共 24 场景。
- launch 的挂载就绪要求每个 overlay 目标都存在（解码 mountinfo 路径），不再
  接受任意一个。目标未齐时每轮检查 owned child 的退出状态；只有确认它已退出才
  接受短命令完成，覆盖 mountinfo 可读但不完整、ENOENT 和退出未回收时的 EINVAL。
  其他读取错误上报，活进程仍须满足全部目标，保留 400 × 5ms 期限和真实退出码。
  两项退出回归先 red 后 green；完整测试最终库 140 项、bin 1 项通过、0 失败，
  其中无权限自行 SKIP 的 btrfs/特权项不作特权成功证据。
  live-target 增加初始观察标记与分阶段计时，共用原 30 秒期限；无证据将偶发超时
  归因为高负载。status pipe 首行仍阻塞，作为已知边界记录。

**废弃记录**：btrfs upper 本身为独立子卷、work 位于其外部的布局被父 volume
布局取代；从 `--nocapture` 日志精确 grep `test NAME ... ok` 的检查被捕获输出和
摘要交叉核对取代。`log-3.txt` 中 4 执行/1 通过与 0/7 误判不是实际只执行了
1 个或没有执行进程测试。

# 2026-11-10：第四轮特权验证通过与范围

用户 2026-11-10 第四轮验证中 Chain、Flatten、mixed、direct-root 四测试逐项 ok，
4 passed、0 failed、0 ignored；三项 btrfs 场景均选中真实 btrfs，
`incremental_diff=true fast_delete=true`。状态目录位于根 btrfs 挂载，workspace 在
独立 `/tmp` tmpfs，默认非重叠布局成立。7 项进程测试全通过、0 failed、0 ignored，
自动机制为 cgroup，强制 process_group fallback 也通过。原始子卷创建、只读快照、
增量 send dump、严格删除及初始 userns 直接 lower=/ 均通过；脚本退出 0，
全部 PASS，最终 `ALL PRIVILEGED CHECKS PASSED`。

亲读 `tests/privileged.rs` 与 `tests/support.rs` 确认：Chain/Flatten 检查实际布局、
真实进程修改/删除/新建、含父目录的精确 diff、accept 发布及后续真实视图与清理；
mixed 检查两代 Chain→Flatten、accept 回 child1 且 root 不变及清理。accept 传入
`target_mount_pid=None`，不是运行中 live-target 重放验收；不外推全部内核生成的
overlay 元数据、特殊文件/ACL/ownership、偶发超时根因或物化完成。

废弃的 `priv_btrfs_fork_diff_accept_discard` 已由 Chain/Flatten/mixed 三项替代，
不再作为当前测试名单；先前失败与修复过程留在本文件，正式状态以第四轮证据为准。

# 2026-11-10：删除用户日志与 FS 状态文档复查

用户明确授权删除根目录 `log.txt`、`log-2.txt`、`log-3.txt`、`log-4.txt`。
删除前已读取第四轮结果并与当前特权测试及 `652f60c`、`8c54403` 核对；当前文档
保留准确测试名、通过数、能力值和边界，不再依赖已删除日志的行号。上文旧日志名与
行号只作为失败/修复历史标识，原文件已按用户要求删除。

复查修正：状态目录重叠默认拒绝，只有内部测试/诊断开关允许 Flatten；小 upper
Flatten 只避免追加当前层，不会压平全部继承层，搬移状态目录也不会缩短既有链。
ProcessBackend 已承担启动、冻结、终止及 namespace 记录，不再将这些现成功能
列在未实现清单；跨 Backend 编排进一步统一与逃出进程组的后代仍保留为真实边界。
信号驱动 trash、元数据隔离失败关闭、diff 比较目标移除的既有决定保持不变。
# 2026-11-10：观察事件与增量读取

按已确认设计，弃用 `GET /api/run-view` 整体轮询及 `ui.rs` 每次 `state.tree = tree.clone()` 的整体替换。内部 SpeculativeTree 不再作为外部序列化模型；原始 Agent/Runtime/Ledger 消息不进入公共观察流。AScoutRunView 仅为从公共事件折叠的本地 TUI 视图；对话通过独立读取接口取得。现有简易页面改为首次事件快照 + SSE，后续 React + React Flow 前端尚未实现。具体接口与差距见 observation.md。
## 2026-11-10：状态事件集、Ledger 观察日志与 Tree 转换入口

废弃观察接口的 snapshot / “快照”命名、`GET /api/observation/snapshot` 和 `ObservationSnapshot`：它们读取的是完整有序事件，不是被保存的状态。当前统一称状态事件集，英文直译为 `StateEventSet`，路径为 `/api/observation/state-event-set`。

废弃 ui.rs 独立保存观察日志及通过 Tree 前后差异推断事件的做法。Ledger 在 events.jsonl 中以 `message: observation` 区分观察记录，直接提供只读句柄；Tree 在创建、生命周期、Authority 和环境转换入口产生待发事实，控制循环交给 Ledger 追加。保留同一次处理内的中间状态与入口明确给出的原因。观察日志不承担崩溃恢复整棵树的要求，不逐次保存完整树，也不实现折叠缓存。

## 2026-11-10：投机起点与工具开始分离

废弃“用 `SpeculativeAdmissionRequest.timestamp_ms` 合成 `ActionStarted` 作为工具开始时间”的做法：准入请求只登记 action 身份并表达投机起点，工具开始时间只来自接入侧真实的 `ActionStarted`。某 action 只有准入请求而尚无 `ActionStarted` 时，`tool_finished.started_at_ms` 为 null，耗时在起点和终点都可得时才计算。被准入投机的起点由 `branch_created.speculation_started_at_ms` 表达，取自对应准入请求的接入侧时刻；事件公共 `timestamp_ms` 仍是控制面记录时刻。`model_usage` 与 `tool_finished` 增加源侧 `started_at_ms` / `finished_at_ms`；`model_usage.action_id` 由可空改为必填字符串。

## 2026-11-10：结算的实测/估计分离与未命中时间语义

废弃 `settlement` 每个维度的 `{value, measured}` 形状（一个带符号数字加一个布尔标记），以及“未命中时间 = −(候选从投机起点到被丢弃的存活时长)”的语义。当前 `time_ms`、`money` 都是 `{measured, estimated}`：实测与估计各自独立，永远不相加，也不用其中一部分填充另一部分；未命中只记投机给整场运行（权威路径）造成的额外延迟的负值，不记候选存活时长或快速模型自身的“白耗”，没有可靠测量时对应部分为空。目标语义见 `docs/design/control-plane.md` §5，实现与数据缺口见 `docs/development/observation.md`。

## 2026-11-10：控制面 WebUI 的构建与嵌入

废弃 `src/bin/ascout-control-plane/web/index.html` 手写单页。浏览器界面改由仓库根目录 `webui/` 的 React + React Flow 应用实现，Vite 构建产物提交在 `webui/dist/`，由 `web.rs` 用 `include_bytes!` 嵌入二进制；`cargo build` 不依赖 Node，前端源码变化时重新构建并提交 `dist/`。控制面静态资源路由固定为 `/`、`/assets/app.js`、`/assets/app.css` 和 `/favicon.svg`；CSP 的脚本来源改为 `'self'`，不再需要内联脚本。界面只展示权威窗口对话，投机面板只读，结算的实测与估计分别显示；连续 `conversation_updated` 流式块在投机树中合并为一个就地更新的节点。实现说明见 `docs/development/web-ui.md`。

## 2026-11-10：Edge 改为抽象模型，由 Chance 取代“有 id 的机会对象”

废弃“Edge 是有 id、可枚举、被 `select` 选择、出现在 AssumptionSet（`Assumption=(edge_id, proposal_id)`）里的对象”以及“Edge 即投机机会”“嵌套 Edge 有自己的 Proposal、Branch 和过程状态”等表述。Edge 现在只是描述投机如何进行的抽象模型，不是运行时对象；其具体实例称为 Chance（被 `admit` 接纳的投机机会），从 `admit` 通过起拥有 id，直到被验证后接受或抛弃为止。待选集合、AssumptionSet 引用和运行时 Decision 作用对象均为 Chance。

## 2026-11-11：投机树以 Branch 树为唯一几何依据

废弃“以当前权威路径为居中主干、按子树宽度在权威两侧平衡、同一父节点的子枝头部统一排到父枝底部”的投机树布局。该布局让 Promote 改变几何：权威路径一变，主干、左右分配和分叉位置都会重排，已经长出的树形随之移动，分叉也被搬到新权威下方。

现在的目标语义是：根 Branch 的枝顶固定在原点，子枝的横纵位置只由 `parentBranchId` 与 `createdAtSequence` 决定；子枝从父枝上“创建它时的调用槽位”（最后一个 `firstSequence <= createdAtSequence` 的调用）分叉，之后新增调用只向下延长枝干，不改已有槽位与分叉。只有新增兄弟分支时才允许重新分配左右并整体重排，且兄弟子树的横向范围不得相交。权威路径只进入样式：权威枝干实色更粗，候选细描边；权威祖先在通往当前权威的分叉之后的调用变暗。调用语义固定为“一次模型调用一根小枝条，其后的工具调用是这根枝条上的叶子，第一次模型调用之前的工具调用长在枝干上”；连续同签名调用可在同一根小枝条内合并。

实现上每个 Branch 是一个 React Flow 节点，节点内部用一张 SVG 连续绘制枝干、小枝条、叶子、枝头标记和分叉起点；分叉本身用平滑曲线连接到子枝枝头。旧的独立枝干/小枝条/叶子节点类型与相关常量已删除。图例、悬停文案和详情点击行为随之改为新语义。
# WebUI 树形与权威窗口（2026-11-11）

## 已确定

- 用户视图保留独立的权威窗口；它沿 `authorityHistory` 连续展示已成为权威路径的对话，候选内容只有 Promote 后才进入该时间线。按 Branch 查看仍是独立的只读窗口。
- 投机树使用观察事件的逻辑时间和稳定哈希：Branch 是连续树干，模型调用是短叶茎，工具调用是末端叶片；长逻辑时间用单调平方根压缩并设单 Branch 上限，Chance 根部从父树干的对应时间点向侧面连接。
- 权威切换只改变样式，不移动既有几何；候选未采纳使用中性文字。
- 权威窗口的消息组件使用稳定身份和 memo；事件更新只刷新末尾内容，滚动只在用户贴近底部时跟随。

## 废弃

- 废弃把 Branch 作为垂直绝对槽位、把模型调用画成独立分支、以及按真实毫秒线性展开整棵树的做法；这些方案会破坏树干语义或使长任务不可读。

## 2026-11-12：Transcript 引用与持久化 session 树迁移

以下两项是已确定目标方案下的旧实现废弃记录；实现仍在迁移中，不表示目标能力已完整实现。

- 废弃 Ledger 的 Transcript 原样保存 Pi 完整消息对象或正文的旧实现。目标方案由 Pi 格式的 `session.jsonl` 保存正文，Ledger 仅保存 Branch id、树文件记录 id、role 等引用事实。
- 废弃 Candidate 使用仅存在内存的 Pi session，以及 Promote 只设置 `authority=branch` 的旧实现。目标方案中，Candidate 和 Promote 依赖持久化 session 树文件与权威路径标记；相关实现仍在迁移中。

## 2026-11-12：投机树长度、方向、宽度与配对规则重写

废弃以下旧实现与文档表述：把整枝长度写成“从 Branch 创建到当前的累计时间”的一次对数映射；把没有开始时间的调用跳过或用事件序号补时间；由“创建到最后事件（丢弃或清理更晚则取它）”决定枝长并追加固定短尾、最短长度和空枝固定长度；用指数回正加两根低频正弦决定航向；用 0.72 次幂、跨度 2.4%、4–90 的上限、26–56° 初始角、同侧 40 单位至少差 15° 等经验常数决定宽度和分叉角度。

现在的规则：布局前先把工具调用配对到它之前最近的模型调用；所有调用必须带开始时间，缺失显式报错。纵向长度逐截计算，每截等于相邻两次调用开始时间间隔的对数，位置逐截累加；Chance 不切段，只在相邻两截之间插值；枝长止于最后一次调用。方向改为逐截随机行走：横向位置决定回中概率，每截相对上一截转固定小角度，方向限制在 ±60°。宽度按该点之后剩余调用数占本枝总数的平方根缩放；根宽是独立的树内参数（默认 45 单位），不随段长映射变化；子枝根宽按剩余份额从父枝宽度推导；删除全局上限与 0.9 封顶。Chance 半径取子枝根宽的一半，叶片与空枝芽改为无尖端的椭圆。前端仍以事件折叠结果作布局输入，观察事件格式不变。
## 2026-11-13 NLE 初始化回归修正

e89533d 保存／恢复逻辑曾为每次新世界增加独立随机 lgen 种子，废弃该初始化选择；它改变既有 core seed 的地图生成行为。新世界保持 lgen=None（地图生成沿用 core RNG），保存记录仍按自身完整 initial 重建。不撤销保存／恢复及指纹验证，不改变候选共享世界语义。旧日期消息及旧权威历史差异尚未解决，不通过重新录制、伪造工具结果或放宽匹配／验收绕过。证据见 replay.md 的 2026-11-13 核实。
# 2026-11-13 叶茎完成位置与填充连接

废弃按调用开始位置放置叶茎，因为叶茎表示已完成的模型调用。
废弃 `stemAnchor` 及外沿偏移锚点，因为连接必须从完成位置中心线与最终可见轮廓求交。
废弃叶茎描边与线帽，因为显式等宽填充轮廓才能复用两侧平滑连接并限制局部宽度。
废弃只凭稳定随机偏好放侧，因为附近同侧子枝和叶茎需要参与角度间隔比较；稳定偏好仅用于同分。

## 2026-11-13 通用回放持久化范围纠正

废弃“回放与后续输入的 session/events 必须只在内存，因而改造 Ledger、SessionTree 和运行目录”的要求。用户明确允许正常新 run 的原生记录、日志和恢复资料照常落盘；不需要的是额外导出缓存、recording 副本。实现移除该过渡改造，直接复用正常运行路径；源 run 只读、每次重新读取，在内存准备回复队列。通用回放入口不包装 B3，也不推断其父目录配置布局。

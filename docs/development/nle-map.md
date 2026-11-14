# NLE Runtime 地图可视化（原型）

WebUI 的 `/map` 是独立于 AScout 对话和投机树的 NLE 地图浏览入口。它以动作历史为单位重建地图：起点、后退、前进和终点按钮只改变历史游标，不调用控制面、Runtime Adapter 或 NLE。

## 当前实现

- 地图使用固定的绝对 `(x, y)` 坐标网格。刷新、改变历史游标和窗口尺寸变化都不会重新计算格子的世界坐标。
- 每个历史前缀都从事件序列重新计算玩家位置、已知单元格和当前可见单元格。
- 未知格不绘制内容；曾经公开观测到、但当前不在视野的格子只显示已知状态；当前公开观测中的格子显示当前可见状态。
- 页面优先读取控制面 `state-event-set` 中的 `nle_map_action` 事件；事件由真实 NLE `invoke` 的公开 `evidence.before/after` 产生。Ledger 只复制 `chars`、`blstats` 和公开 `message`，不会把 Runtime evidence 原样转发。
- 没有真实地图事件时，页面才使用明确标注的手工公开观测演示序列，用于验证地图和回放语义，不包含隐藏地图。
- 地图是同一 `task_id` 的**权威窗口历史**，不是某个 Branch 的历史。页面按 Ledger `sequence` 合并所有真实 `nle_map_action`，Branch 只作为动作来源；因此 A → Promote B → B → Promote C → C 的真实动作会连续保留。不同 task 不合并；没有 `nle_map_action` 的执行候选也不会进入地图。
- 事件按 Ledger `sequence` 排序。无效 `after`、同一 Branch 内重复 action 身份或输入乱序会使真实历史标记为“不完整”，但只保留可解析事件；动作序号按实际加入历史的状态递增，不用原始数组下标。跨 Branch 的连续权威接续不是不完整原因。
- 页面在初次读取后订阅同一只读观察 SSE；收到新的连续事件会重新构建地图。发生序号缺口时重新读取状态事件集。用户停在历史末尾时自动跟随新末尾，正在回看时只更新可用历史和总长度，不改变当前游标；Promote 不重置地图历史或回看游标。

## 真实 Runtime 接入边界

控制面现在发布 `nle_map_action` 只读事件。它只在 Runtime 返回包含合法公开 `chars` / `blstats` 的 NLE evidence 时产生；页面会显示“真实 Runtime 公开观测”。如果运行没有动作事件，页面显示演示 fallback，而不会声称真实接通。NLE 特有的地图解释仍留在 WebUI，控制面只转发已经公开的观测事实，不计算坐标、不暴露隐藏地图。

坐标适配依据是仓库现有桥接和本地 NLE Policy：`examples/nle-runtime-bridge.py` 原样保留 `chars` 行，`assets/policy/nle/decisions.py` 把 `chars[ty][tx]` 与 `(dx, dy)` 配合使用，并明确 north 为 `dy=-1`、south 为 `dy=+1`。因此页面把屏幕行增加解释为世界 Y 增加（向南），把屏幕上方显示为北；`blstats[0]`、`blstats[1]` 被当作公开玩家绝对坐标。该适配只基于当前公开接口约定，不读取 NLE 隐藏状态。

## 运行

```sh
cd webui
npm ci
npm run dev
```

打开 Vite 地址的 `/map`。生产构建仍需把 `webui/dist` 重建后由控制面嵌入：

```sh
npm run build
cargo build
```

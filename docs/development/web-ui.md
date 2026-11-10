# 控制面 WebUI（2026-11-10）

控制面 WebUI 是 `--web` 模式下的正式浏览器界面。它只读取控制面已经发布的观察事实与按 Branch 的对话接口，不接触 Speculative Tree、控制命令或 Runtime 内部对象；输入仍走 `POST /api/request`。

## 技术组成

- React 19 + TypeScript，React Flow 绘制投机树，Zustand 保存观察状态。
- Vite 构建；源码位于仓库根目录 `webui/`。开发需要 Node 22.12 或更新的版本（当前验证使用 Node 26），运行 `cargo build` 则不需要 Node。
- 构建产物提交在 `webui/dist/`，由 `src/bin/ascout-control-plane/web.rs` 用 `include_bytes!` 嵌入 Rust 二进制。因此 `cargo build` 不要求安装 Node，也不会在构建时启动前端工具链。
- `webui/dist/index.html`、`webui/dist/assets/app.js`、`webui/dist/assets/app.css` 与 `webui/dist/favicon.svg` 是嵌入入口；控制面按固定路径提供它们。

如果 `webui/dist` 缺失，`cargo build` 会因找不到嵌入文件而失败。恢复方式：

```sh
cd webui
npm ci
npm run build
```

前端源码变更后必须重新构建并一并提交 `webui/dist`，否则二进制仍提供旧界面。

## 开发

先以 Web 模式启动控制面（绑定的地址会打印在控制面输出中）：

```sh
cargo run --bin ascout-control-plane -- /path/to/run-dir ../pi --web 127.0.0.1:8080
```

再启动 Vite 开发服务器：

```sh
cd webui
npm ci
npm run dev
```

默认代理目标是 `http://127.0.0.1:8080`；控制面使用其他地址或端口时：

```sh
ASCOUT_WEB_TARGET=http://127.0.0.1:8099 npm run dev
```

代理必须把 Host 和 `POST /api/request` 的 Origin 改写成控制面的值，因为控制面只接受 loopback、校验 Host，并要求同源 JSON 请求。开发服务器本身不实现观察接口，它只转发 `/api`。

其他命令：

```sh
npm run build      # tsc 工程检查 + Vite 生产构建到 dist/
npm run typecheck  # 只做 TypeScript 工程检查
```

## 界面行为

左侧是权威窗口对话，只显示权威 Branch 的对话内容；可以切换任务和权威窗口，历史权威窗口只读。第一个版本从观察事件推导任务列表，控制面没有单独的任务列表接口。

右侧是可隐藏的投机面板，默认显示投机树；点击树枝头部、枝干或叶子才进入该 Branch 的只读详情，可以返回树。树上不显示分支名；树枝头部显示生命周期状态图标与投机原因图标，悬停显示 Branch id、模型、token 与叶子/事件摘要。

面板顶部汇总当前投机数量、累计 token、累计费用、累计节约时间和累计节约金钱。`settlement.time_ms` 与 `settlement.money` 的实测部分和估计部分分开累加、分开显示，不合并成一个数字；没有结算数据时显示“暂无数据”，不用其他数字填充。

树纵向自上而下生长，不使用横坐标或时间刻度，长度不代表时间。一个 Branch 是一根较粗的纵向树枝：头部放较大的状态图标和投机原因图标，Branch 内部的事件是沿枝干按事件序号纵向排开的小叶子；枝干粗、叶子小，用尺寸区分层级。叶子按事件类型用形状和颜色区分（对话更新、模型用量、工具完成、状态变化、环境、结算、权威切换），悬停显示事件详情；连续且呈现信息相同的同类事件合并为一个叶子并带计数，例如流式回复的连续 `conversation_updated`、连续同结果的 `tool_finished`。子树枝从父树枝对应的分叉位置长出，分支之间只保证分叉位置的相对逻辑关系一致。新事件追加到各自树枝下方；叶子是独立的 React Flow 节点，只渲染可见部分。

权威路径的树枝为实色且更粗；投机候选为同色系空心描边，运行中轻微脉动；已抛弃的树枝和叶子变灰并降低透明度；晋级时颜色、粗细和描边平滑过渡。图例分别说明树枝、叶子与各叶子类型，并说明树上不显示分支名。

分支详情包含基本信息（角色、状态、模型、投机起点、存活时长）、隔离环境（独立 Instance、嵌套关系、配置摘要）、用量（逐次模型调用、工具调用）和该 Branch 的只读对话。环境字段为 null 时显示“未上报”，不推测值。

状态折叠是纯函数：`foldEvents(events, upToSequence)` 可以得到任意序号的前缀状态。当前版本没有实现回放界面。

## 目录

```text
webui/
  index.html          开发入口
  public/favicon.svg
  src/
    main.tsx          React 挂载
    App.tsx           页面骨架与面板开关
    api.ts            只读接口与请求
    types.ts          观察事件和对话的线上类型
    fold.ts           纯事件折叠、Branch/任务状态
    treeLayout.ts     叶子分组、树枝纵向布局与叶子悬停文案
    stats.ts          统计与实测/估计分项累加
    store.ts          SSE 连接、状态与界面选择
    useConversation.ts 按需读取并刷新 Branch 对话
    format.ts         数字、时间与金额格式
    icons.tsx         状态、原因和操作图标
    styles.css        设计变量、深浅色、布局与树样式
    components/       对话、树枝头部、枝干、叶子、详情、统计、图例
  dist/               提交的构建产物（嵌入 Rust）
```

## 当前缺口

- 点击叶子只进入该 Branch 详情，暂不定位到对应事件；树上没有事件列表或回放定位控件。
- 树的纵向距离只表达事件顺序，不是时间轴，也不能用于比较不同 Branch 的耗时。
- 没有任务列表接口；任务从事件中的 `task_id` 推导，首个事件到达后才会出现。任务切换状态保留在客户端。
- 没有事件回放界面；纯折叠函数已经可截取到任意序号，但未接入播放控件。
- 对话正文仍按需读取；Ledger 不保存正文，Branch 的 session id、路径与继承分界也没有进入观察事件。
- 模型投机当前没有生产路径；面板按事件结构显示，不假设模型投机一定发生。
- 统计只累加已上报事实。`model_usage` 缺 token 或费用时不补零；结算的实测/估计部分缺失时对应项显示“暂无”。

## 本轮验证

- `npm run build` 与 `npm run typecheck` 通过；`cargo build --bin ascout-control-plane` 通过，最终二进制包含本次构建的 `webui/dist` 资产（构建时由 `include_bytes!` 嵌入，已用新树样式字符串核对）。
- 使用临时只读服务回放手写事件序列，覆盖 4 个 Branch 的多分支树（含嵌套候选）、权威晋升、命中与未命中结算、连续事件合并、分支详情、叶子与树枝头部悬停、图例、深色模式和窄屏；另用 SSE 增量推送验证新叶子向下生长与局部更新。本轮没有调用任何真实模型，也没有改变控制面状态。
- 截图保存在 `/tmp/opencode/tree-light-panel.png`、`tree-dark-panel.png`、`tree-narrow-page.png`、`tree-legend.png`、`tree-leaf-hover.png`、`tree-head-hover.png`、`tree-branch-detail.png`、`tree-growth-partial.png`、`tree-growth-final.png`。
- 本轮没有为原型新增测试。

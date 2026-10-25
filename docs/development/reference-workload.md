# 首要验证 Workload

首个真实接入对象是 Pi，当前源码位于相邻的 `pi-code`。本文只记录当前交互实现，不定义完整 Runtime 或 coding workload。

## 当前可运行范围

运行方式见 [README](../../README.md)。Rust 控制面自动启动 Pi 接入进程，并提供 AScout 自己的终端界面；Pi 只管理本地 AgentSession、执行控制命令和报告事实，没有 Tree 或 Policy 裁决权。

- 初始 Authority 在本次独立管理目录中建立，连续请求使用同一 AgentSession。
- 模型选择与认证使用 Pi 现有配置，不写死 provider。
- 模型文本、工具启动与工具结果经接入协议实时回传，由 AScout 展示；模型错误如实显示。
- `/ascout` 展示当前控制面 Tree，可按编号深入只读查看 Branch 的记录和执行输出；选择观察对象不改变权威路径。
- Ctrl-C 请求中断；Ctrl-D 退出。运行目录和 `agent.log` 保留用于诊断。
- 不创建演示候选，不用最终模型回答充当 Parent operation outcome。

## 实际检查（2026-10-25）

在 tmux 真实终端启动，确认 Pi 接入成功、Authority 启动、主对话输入送达、模型错误实时回传、Tree 展示和 Branch 详情显示。Pi 默认模型 `gpt-5.5` 的当前服务返回 HTTP 503 `model_not_found`；没有伪造模型成功回答。中断后收到本轮结束事实。Rust 编译与已有两个控制状态测试通过，Pi 完整 `npm run check` 通过。

## 尚未完成

真实等待点的捕获、从共同上下文注入预测并续跑候选、Policy 的运行期接入、真实 Parent outcome 比较、Promote 的输出接替，以及相应的候选运行观察均待实现。当前界面明确显示投机尚未接通，不能作为完成真实投机执行的证据。

完整多行编辑、Markdown 和工具专门排版、宽字符排版、产品特有交互、断线恢复、资源硬限制与确认式退出清理尚未实现。当前是简单终端原型，不宣称已达到原 Agent 全部交互能力。

环境管理暂不接通完整 Runtime Adapter 是已确认的开发顺序；目录不代表任意 shell 或外部效果已隔离。接入进程退出只确认该进程退出，不证明任意工具后代均已停止或环境资源已清理。首轮不开放候选业务工具。

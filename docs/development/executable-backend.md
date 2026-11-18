# 接入独立可执行的 backend

在运行节点上安装可执行文件，在 Runtime JSON 的 `backends` 数组登记它。每个登记项对应一个由 base runtime 启动的进程，进程可以为多个 Environment 提供服务。同一 Runtime 二进制可以通过配置更换程序或增加登记项，重启 Runtime 后生效。文件系统、Firecracker、NLE 继续由内置实现装配。

```json
{
  "control_address": "127.0.0.1:8180",
  "adapter_instance": "local-runtime",
  "backends": [
    {
      "id": "my-backend",
      "type": "executable",
      "executable": "./bin/my-backend",
      "args": [],
      "interfaces": ["environment"],
      "capabilities": {"branch_environment": false},
      "config": {}
    }
  ]
}
```

部署时用 `ascout-runtime-adapter --config /path/to/runtime.json` 启动，或使用 control plane 的 `--runtime /path/to/runtime.json`。程序路径相对于 JSON 所在目录，也可以使用绝对路径。程序直接运行，`args` 逐项传入，工作目录和环境变量继承 Runtime。`config` 是原样发送的 JSON 值。单项配置默认路由到该项，多项沿用 `policy` 配置。registry 在启动进程前检查标识唯一、入口是有执行权限位的文件、接口和能力声明符合当前版本。请为 Runtime 配置有信任关系的程序，程序使用 Runtime 的宿主权限。

## 程序接口

握手、生命周期和 `operate` 使用 UTF-8 JSON 行格式。每条请求从 stdin 读取，在 stdout 写一条带换行的响应并刷新。每行上限 1 MiB，日志输出到 stderr。每实例串行执行普通请求，执行工具时仍须读取取消消息。成功响应为 `{"ok":true,"result":...}`，业务拒绝为 `{"ok":false,"error":"原因"}`。

首次请求为 `{"op":"hello","version":1,"config":{}}`。成功结果必须提供 `version`、`methods` 和 `capabilities`。

```json
{"ok":true,"result":{"version":1,"methods":["establish","operate","branch","promote","suspend","resume","terminate","cleanup","shutdown"],"capabilities":{"branch_environment":false}}}
```

base runtime 创建实例时核对版本、方法与配置中的能力。握手只核对接口声明。环境隔离等行为保证仍需单独验证。

除握手及 shutdown 外，程序请求包含 `environment`、`parent`、`operation_id`、`tool_call_id`、`task_id` 和 `generation`。可选身份字段可以为 `null`，Environment ID 由 Runtime 分配。`branch` 的 `environment` 是子环境，`parent` 是父环境。`operate` 额外提供字符串数组 `command`。普通生命周期成功返回 `result: null`，`operate` 返回 `{"exit_code":0,"stdout":"...","stderr":"..."}`。命令执行完成后的非零退出码仍使用成功信封和实际退出码。

| 方法 | 成功条件 |
| --- | --- |
| `establish` | 指定环境已经可用 |
| `operate` | 在可用环境执行命令并返回结果 |
| `branch` | 独立子环境已可用，父环境保持原状。配置为 `false` 时 Runtime 拒绝派生，程序也应返回不支持 |
| `promote` | 完成后端自身的提升动作，需要时可以直接确认 |
| `suspend` / `resume` | 停止该环境接受执行或恢复原环境的执行能力 |
| `terminate` / `cleanup` | 停止环境执行或释放环境资源，允许安全重试 |
| `shutdown` | 请求只有 `op` 字段。释放实例资源，返回成功，然后退出 |

### 工具请求和响应

在 `hello` 的 `methods` 中声明所支持的 `environment_operation` 和 `invoke_tool`。声明其中任何一种时也必须声明 `cancel`，否则 Runtime 拒绝启动该实例。Agent 工具的名称和参数声明通过现有 Instrumentation 配置交付，本协议不发布工具目录。

- `environment_operation` 请求增加 `operation` 和 `arguments`。`operation` 为 `read_file`、`write_file`、`replace_text`、`list_directory` 或 `exec`。请求参数和结果沿用执行端点格式。`write_file`、带输入的 `exec` 和 `replace_text` 随请求附带原始字节。`replace_text` 的每项参数包含 `old_length`、`new_length` 与可选的 `match`，内容帧依次保存旧文本和新文本。成功响应包含 `ok: true`，以及操作对应的 `value`、`exit_code` 或 `body_kind` 与内容。`file_bytes` 附带原始字节，`json_value` 附带 UTF-8 JSON 字节。
- `invoke_tool` 请求增加 `tool` 和结构化 `arguments`。成功响应使用 `result: {exit_code, stdout, stderr}`。`stdout` 中的 JSON 可提供 Agent 所需的 `result` 和 Policy 所需的 `evidence`。
- 两种请求均可在终态前返回 `stream: "stdout"` 或 `stream: "stderr"` 与内容帧。错误终态使用 `ok: false`、`error` 和可选的 `error_category`。

消息头由一行 JSON 构成。有二进制正文时，头部的 `body_length` 指定总长度，接着是一系列 4 字节大端长度和最多 64 KiB 的正文片段，再以 4 字节全零结束。正文长度为零仍需结束帧。程序必须及时刷新输出。一次请求只有一个终态回复。

### 取消单次工具执行

工具请求尚未写入时，Runtime 可以直接结束本次等待。一旦开始发送，即使上传期间收到取消，Runtime 也会发送完整请求及其正文结束帧，然后通过同一 stdin 发送一条没有正文的 JSON 行，例如 `{"op":"cancel","environment":"env-1","operation_id":"op-1"}`。

程序要在工具执行和输出期间继续读取控制消息。按 `environment` 与 `operation_id` 找到对应工作并通知它停止。工具在停止写者、释放本次操作持有的资源并发完已开始的输出帧后，返回唯一终态 `{"ok":false,"error":"操作已停止","error_category":"cancelled"}`。若工具在取消生效之前已完成，保留实际终态。已完成或身份不匹配的取消消息直接忽略。`cancel` 本身没有响应，程序应按接收顺序处理它，再读取后续普通请求。

Runtime 等待并转交工具的实际终态，按原顺序交付此前的输出。普通取消保持实例和其他 Environment 可用。业务拒绝应保持原状态或在返回前回滚本次操作。

工具的 30 秒请求预算涵盖排队、传输和等待终态。开始传输后若超时、遇到 EOF 或帧损坏，Runtime 报告超时或通信失败，终止进程组并回收直接子进程，将该实例的 Environment 标记为部分可用。关闭清理使用独立的 2 秒请求预算，收到 `shutdown` 确认后最多等待进程退出 2 秒。程序须自行管理外部资源和脱离进程组的进程。Runtime 会报告无法确认的回收或 drain。当前程序接口没有重启恢复或热加载。

仓库中的[可执行程序示例](../../tests/fixtures/executable-backend.py)用独立的工作线程执行工具，主线程继续读取取消消息。可在构建 `ascout-runtime-adapter` 后运行 `node --test tests/executable-backend-cancellation.mjs`，通过 Instrumentation 的 ASC-Proxy 检查双 Environment、分片、程序故障和关闭清理。

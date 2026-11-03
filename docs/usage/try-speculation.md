# 试一次 AScout 投机执行（NLE）

以下命令都从 AScout 仓库根目录执行（例如 `../AgentScout`）。

## 1. 准备

```sh
python3.13 --version    # 3.13
python3.13 -m venv ../.venv-nle
../.venv-nle/bin/python -m pip install -r requirements-nle.txt
node --version      # >= 22.19
cargo build
```

## 2. 启动

```sh
rm -rf /tmp/ascout-demo
mkdir -p /tmp/ascout-demo

./target/debug/ascout-control-plane \
  /tmp/ascout-demo \
  ../pi-code \
  assets/policy/nle.yaml \
  --model deepseek/deepseek-flash \
  --thinking low
```

`--model` 指定 Pi 使用的模型，认证使用 Pi 默认配置；`--thinking` 可选。

## 3. 交互

测试 prompt 见 `examples/nle-task.md`；把其中代码块作为首次消息发送。

```text
/ascout      Tree
/events      Events
Ctrl-C       中断当前回合
Ctrl-D       退出
```

## 4. 常见问题

- `503` 且提示 `gpt-5.5`：启动命令没有带 `--model deepseek/deepseek-flash`。
- NLE 启动失败：运行 `../.venv-nle/bin/python -c 'import nle, gymnasium'` 检查依赖。
- 同一个 socket 只能跑一个实例：先退出上一个 AScout。

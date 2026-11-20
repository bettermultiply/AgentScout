//! 一次投机的结算：在 Branch 的统一状态转换入口排队，由协调循环写入 Ledger。
//!
//! 目标语义见 `docs/design/control-plane.md` §5；实现与数据缺口见
//! `docs/development/observation.md`。这里只使用 control plane 已经收到的 Instrumentation 时间戳
//! 和用量事实，不根据 control plane 收到消息的时刻合成执行耗时。
//!
//! 实测与估计是两个独立部分：同一维度要么只填实测、要么只填估计，没有数据时
//! 两者都是 null。它们永远不相加，也不互相填充。

use super::*;
use crate::control_plane::observation::{ObservationAmount, ObservationContent};

impl SpeculativeTree {
    /// 每次已准入投机恰好结算一次：Promoted 记为命中，Discarded 记为未命中。
    /// 已经结算过的 Branch 不再重复结算（例如被替换的旧权威随后进入 Discarded）。
    pub(super) fn queue_settlement(&mut self, branch_id: &str, hit: bool) -> io::Result<()> {
        let Some(speculation) = self.speculations.get(branch_id) else {
            return Ok(());
        };
        if speculation.settled {
            return Ok(());
        }
        let started_at_ms = speculation.started_at_ms;
        let kind = speculation.kind;
        let branch = self.branch(branch_id)?.clone();
        let (time_ms, money, currency) = if hit {
            self.hit_settlement(&branch, kind, started_at_ms)
        } else {
            self.miss_settlement(&branch)
        };
        self.speculations
            .get_mut(branch_id)
            .expect("speculation exists")
            .settled = true;
        self.pending_observations
            .push(ObservationContent::Settlement {
                branch_id: branch_id.into(),
                hit,
                time_ms,
                money,
                currency,
            });
        Ok(())
    }

    fn hit_settlement(
        &self,
        branch: &BranchRecord,
        kind: Option<ActionKind>,
        started_at_ms: u64,
    ) -> (ObservationAmount, ObservationAmount, Option<String>) {
        match kind {
            Some(ActionKind::ToolExecution) => {
                // 权威侧同一工具 action 的真实到达时刻；若它晚于 Promote，
                // 结算已经发出，时间为 null，不回填。
                let finished = self.chance_parent(branch).or(branch.parent_branch_id.as_deref())
                    .and_then(|parent| self.tool_finished_at_ms.get(parent))
                    .and_then(|actions| {
                        branch
                            .execution_position
                            .operation_id
                            .as_ref()
                            .and_then(|operation| actions.get(operation))
                    })
                    .copied();
                let time = finished
                    .and_then(|finished| finished.checked_sub(started_at_ms))
                    .map(|duration| measured(duration as f64))
                    .unwrap_or_else(missing);
                // 工具投机没有可归属的模型费用差额；不把“没有数据”写成 0。
                (time, missing(), None)
            }
            Some(ActionKind::ModelInference) => {
                let operation = branch.execution_position.operation_id.as_deref();
                let fast = self.fast_model_usage(branch, operation);
                let authority = self.chance_parent(branch).or(branch.parent_branch_id.as_deref()).zip(operation).and_then(
                    |(parent, operation)| {
                        self.model_usage
                            .get(parent)?
                            .iter()
                            .find(|usage| usage.action_id.as_deref() == Some(operation))
                    },
                );
                match (fast, authority) {
                    (Some(fast), Some(authority)) => {
                        // 有权威侧同一 operation 的模型用量，按权威模型校验处理。
                        let time = authority
                            .duration_ms
                            .zip(fast.duration_ms)
                            .map(|(authority, fast)| measured(authority as f64 - fast as f64))
                            .unwrap_or_else(missing);
                        let money = authority
                            .cost
                            .zip(fast.cost)
                            .map(|(authority, fast)| measured(authority - fast))
                            .unwrap_or_else(missing);
                        let currency = currency_of(&money);
                        (time, money, currency)
                    }
                    (Some(fast), None) => {
                        // 没有权威模型可比：简单校验通过后被避免的权威调用，
                        // 只能以同一快速调用的 Instrumentation 耗时和费用作为估计依据。
                        let time = fast
                            .duration_ms
                            .map(|duration| estimated(duration as f64))
                            .unwrap_or_else(missing);
                        let money = fast.cost.map(estimated).unwrap_or_else(missing);
                        let currency = currency_of(&money);
                        (time, money, currency)
                    }
                    _ => (missing(), missing(), None),
                }
            }
            // 其他 action 类型没有定义结算公式；不猜测。
            _ => (missing(), missing(), None),
        }
    }

    /// 模型投机还没有生产者；这里定义唯一可判定的快速侧配对：候选 Branch
    /// 在同一 operation 下的用量优先，否则只有候选恰好一条模型用量时采用它。
    /// 多条又无法对应时视为无数据，不猜测哪一次是被投机的推理。
    fn fast_model_usage(
        &self,
        branch: &BranchRecord,
        operation: Option<&str>,
    ) -> Option<&ModelUsageFact> {
        let usages = self.model_usage.get(&branch.branch_id)?;
        if let Some(usage) = operation.and_then(|operation| {
            usages
                .iter()
                .find(|usage| usage.action_id.as_deref() == Some(operation))
        }) {
            return Some(usage);
        }
        let mut values = usages.iter();
        let only = values.next()?;
        values.next().is_none().then_some(only)
    }

    fn miss_settlement(
        &self,
        branch: &BranchRecord,
    ) -> (ObservationAmount, ObservationAmount, Option<String>) {
        // 未命中的时间项表示投机让权威路径（整场运行）多花的时间，而不是
        // 候选存活时长，也不是快速模型自身的“白耗”。当前没有对该额外延迟
        // 的可靠测量（已知来源：权威路径多等一次本地 Policy 求值，以及所有
        // 报告共用一条串行发送队列造成的背压），因此保持 null。
        let time = missing();
        // 未命中的金钱项是候选真实花掉的累计费用，标记实测；没有任何用量，
        // 或任一条用量缺少 cost.total 时不把部分成本当成完整成本。
        let costs = self.model_usage.get(&branch.branch_id).and_then(|usages| {
            if usages.is_empty() {
                return None;
            }
            usages
                .iter()
                .map(|usage| usage.cost)
                .collect::<Option<Vec<_>>>()
        });
        let money = costs
            .map(|costs| measured(-costs.iter().sum::<f64>()))
            .unwrap_or_else(missing);
        let currency = currency_of(&money);
        (time, money, currency)
    }
}

/// Pi SDK 的 `usage.cost.total` 以美元计费；只有金额存在时才写出币种。
const CURRENCY: &str = "USD";

fn measured(value: f64) -> ObservationAmount {
    ObservationAmount {
        measured: Some(value),
        estimated: None,
    }
}

fn estimated(value: f64) -> ObservationAmount {
    ObservationAmount {
        measured: None,
        estimated: Some(value),
    }
}

fn missing() -> ObservationAmount {
    ObservationAmount {
        measured: None,
        estimated: None,
    }
}

fn currency_of(money: &ObservationAmount) -> Option<String> {
    (money.measured.is_some() || money.estimated.is_some()).then(|| CURRENCY.to_string())
}

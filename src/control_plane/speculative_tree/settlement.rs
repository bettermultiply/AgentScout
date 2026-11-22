
use super::*;
use crate::control_plane::observation::{ObservationAmount, ObservationContent};

impl SpeculativeTree {
    pub(super) fn queue_settlement(&mut self, branch_id: &str, hit: bool) -> io::Result<()> {
        self.queue_disposition_settlement(branch_id, hit, if hit { "Promote" } else { "Discard" })
    }

    pub(super) fn queue_disposition_settlement(&mut self, branch_id: &str, hit: bool, disposition: &str) -> io::Result<()> {
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
                disposition: Some(disposition.into()),
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
                // Promote 后才到达的权威结果不回填已发出的结算。
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
                        // 若快速校验省掉了权威调用，以快速调用的实测值估算。
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
            _ => (missing(), missing(), None),
        }
    }

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
        let time = missing();
        // 缺少任一调用的费用时，累计费用未知。
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

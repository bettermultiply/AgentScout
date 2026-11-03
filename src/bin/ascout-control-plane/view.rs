//! AScout control facts only. Agent messages are rendered by the product in its PTY.
use ascout::agent_control::{BranchRecord, BranchRole, BranchStatus, SpeculativeTree};
use serde_json::Value;

pub fn role_label(role: BranchRole) -> &'static str {
    match role {
        BranchRole::Authority => "权威",
        BranchRole::Candidate => "候选",
    }
}

pub fn status_label(status: BranchStatus) -> &'static str {
    match status {
        BranchStatus::Created => "已创建，等待启动",
        BranchStatus::Running => "运行中",
        BranchStatus::AwaitingValidation => "等待验证",
        BranchStatus::Promoting => "正在晋级",
        BranchStatus::Promoted => "已晋级",
        BranchStatus::Discarding => "正在丢弃",
        BranchStatus::Discarded => "已丢弃",
        BranchStatus::Cleaning => "正在清理",
        BranchStatus::Cleaned => "已清理",
    }
}

pub fn branch_details(branch: &BranchRecord, tree: &SpeculativeTree, facts: &str) -> String {
    let note = match branch.status {
        BranchStatus::Cleaned => "\n说明：已清理只表示会话和资源已释放，不等于投机失败。\n",
        BranchStatus::Discarded => "\n说明：已丢弃表示该候选没有被接受，不表示它一定不可行。\n",
        _ => "",
    };
    let current = if tree.authority_branch_id.as_ref() == Some(&branch.branch_id) {
        " · 当前主对话"
    } else {
        " · 非当前权威"
    };
    format!(
        "Branch {} · 只读观察\n父分支：{}\n角色：{} · 状态：{}{current}\n{note}观察不会改变主对话或接受候选。\n\n运行事实\n{}",
        branch.branch_id,
        branch.parent_branch_id.as_deref().unwrap_or("无（根分支）"),
        role_label(branch.role),
        status_label(branch.status),
        facts
    )
}

pub fn select_branch(tree: &SpeculativeTree, input: &str) -> Result<String, String> {
    input
        .parse::<usize>()
        .ok()
        .and_then(|n| n.checked_sub(1))
        .and_then(|n| tree.branches.keys().nth(n))
        .cloned()
        .ok_or_else(|| {
            format!(
                "无效 Branch 编号：{input}。请输入 1–{}；Esc 返回主对话。",
                tree.branches.len()
            )
        })
}

pub fn structured(value: &Value) -> String {
    fn fields(value: &Value, indent: &str, out: &mut String) {
        match value {
            Value::Object(values) => {
                for (key, value) in values {
                    out.push_str(&format!("{indent}{key}:\n"));
                    fields(value, &format!("{indent}  "), out);
                }
            }
            Value::Array(values) => {
                for value in values {
                    fields(value, indent, out);
                }
            }
            Value::String(text) => out.push_str(&format!("{indent}{text}\n")),
            _ => out.push_str(&format!("{indent}{value}\n")),
        }
    }
    let mut text = String::new();
    fields(value, "", &mut text);
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use ascout::agent_control::{BranchRole, BranchStatus};
    #[test]
    fn invalid_selection_is_explicit_and_valid_selection_is_read_only() {
        let branch = BranchRecord {
            branch_id: "authority".into(),
            parent_branch_id: None,
            role: BranchRole::Authority,
            status: BranchStatus::Running,
            generation: 1,
            execution_position: Default::default(),
            proposal: None,
            pending_operations: Vec::new(),
            execution_state: Default::default(),
        };
        let mut tree = SpeculativeTree::new("agent".into());
        tree.authority_branch_id = Some("authority".into());
        tree.history_root_branch_id = Some("authority".into());
        tree.branches.insert("authority".into(), branch);
        for input in ["0", "2", "abc", "", "-1"] {
            assert!(select_branch(&tree, input)
                .unwrap_err()
                .contains("无效 Branch 编号"));
        }
        assert_eq!(select_branch(&tree, "1").unwrap(), "authority");
        assert_eq!(tree.authority_branch_id.as_deref(), Some("authority"));
    }
}

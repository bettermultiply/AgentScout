//! Policy 源文件、YAML/JSON 形状和 Decision 数据校验。

use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::{self, Read};
use std::os::fd::{AsRawFd, FromRawFd};
use std::path::{Component, Path};

use serde_json::Value;
use yaml_rust2::scanner::{Scanner, TokenType};

use super::compile::Binding;
use super::decision::Decision;

pub(super) fn invalid(message: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.to_string())
}
pub(super) fn require(condition: bool, message: &str) -> io::Result<()> {
    if condition {
        Ok(())
    } else {
        Err(invalid(message))
    }
}

pub(super) fn read_limited(file: File, limit: usize) -> io::Result<Vec<u8>> {
    require(file.metadata()?.is_file(), "source must be a regular file")?;
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1).read_to_end(&mut bytes)?;
    require(bytes.len() <= limit, "file exceeds Policy size limit")?;
    Ok(bytes)
}

pub(super) fn source_path(name: &str) -> io::Result<()> {
    require(
        !name.is_empty()
            && name
                .split('/')
                .all(|s| !s.is_empty() && s != "." && s != ".."),
        "source path must be a normalized relative path",
    )?;
    require(
        Path::new(name)
            .components()
            .all(|c| matches!(c, Component::Normal(_))),
        "source path escapes Policy directory",
    )?;
    require(
        !name.contains(['\\', '\0']) && name.is_ascii(),
        "source path must be ASCII without backslash or NUL",
    )?;
    if name.ends_with(".py") {
        for part in name.trim_end_matches(".py").split('/') {
            require(identifier(part), "Python module paths must be identifiers")?;
            require(
                ![
                    "sys",
                    "os",
                    "json",
                    "inspect",
                    "resource",
                    "ctypes",
                    "importlib",
                    "site",
                    "builtins",
                    "encodings",
                ]
                .contains(&part),
                "source shadows an execution module",
            )?;
        }
    }
    Ok(())
}

pub(super) fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .enumerate()
            .all(|(i, c)| c == b'_' || c.is_ascii_alphabetic() || (i > 0 && c.is_ascii_digit()))
}

pub(super) fn open_source(root: &File, name: &str) -> io::Result<File> {
    let mut directory = root.try_clone()?;
    let parts: Vec<_> = name.split('/').collect();
    for (index, part) in parts.iter().enumerate() {
        let part = std::ffi::CString::new(*part).map_err(invalid)?;
        let mut flags = libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK;
        if index + 1 != parts.len() {
            flags |= libc::O_DIRECTORY;
        }
        let fd = unsafe { libc::openat(directory.as_raw_fd(), part.as_ptr(), flags) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        directory = unsafe { File::from_raw_fd(fd) };
    }
    Ok(directory)
}

pub(super) fn check_yaml(text: &str) -> io::Result<()> {
    let mut scanner = Scanner::new(text.chars());
    let mut depth = 0_usize;
    for token in scanner.by_ref() {
        match token.1 {
            TokenType::Alias(_)
            | TokenType::Anchor(_)
            | TokenType::Tag(_, _)
            | TokenType::TagDirective(_, _)
            | TokenType::VersionDirective(_, _) => {
                return Err(invalid(
                    "YAML anchors, aliases, tags and directives are not supported",
                ));
            }
            TokenType::BlockSequenceStart
            | TokenType::BlockMappingStart
            | TokenType::FlowSequenceStart
            | TokenType::FlowMappingStart => {
                depth += 1;
                require(depth <= 32, "YAML nesting exceeds 32")?;
            }
            TokenType::BlockEnd | TokenType::FlowSequenceEnd | TokenType::FlowMappingEnd => {
                depth = depth.saturating_sub(1)
            }
            _ => {}
        }
    }
    if let Some(error) = scanner.get_error() {
        return Err(invalid(error));
    }
    // serde_yaml::Value rejects duplicate keys (including inside configuration).
    let value: serde_yaml::Value = serde_yaml::from_str(text).map_err(invalid)?;
    fn walk(value: &serde_yaml::Value) -> io::Result<()> {
        match value {
            serde_yaml::Value::Mapping(map) => {
                for (key, value) in map {
                    require(
                        key.as_str().is_some_and(|k| k != "<<"),
                        "YAML keys must be strings; merge keys are forbidden",
                    )?;
                    walk(value)?;
                }
            }
            serde_yaml::Value::Sequence(items) => {
                for item in items {
                    walk(item)?;
                }
            }
            serde_yaml::Value::Number(n) => require(
                n.as_i64().is_some()
                    || (n.as_u64().is_none() && n.as_f64().is_some_and(f64::is_finite)),
                "invalid YAML number",
            )?,
            serde_yaml::Value::Tagged(_) => return Err(invalid("YAML tags are forbidden")),
            _ => {}
        }
        Ok(())
    }
    walk(&value)
}

pub(super) fn check_json(value: &Value, depth: usize) -> io::Result<()> {
    require(depth <= 32, "JSON nesting exceeds 32")?;
    match value {
        Value::Array(items) => {
            for item in items {
                check_json(item, depth + 1)?;
            }
        }
        Value::Object(map) => {
            for item in map.values() {
                check_json(item, depth + 1)?;
            }
        }
        Value::Number(n) => require(
            n.as_i64().is_some()
                || (n.as_u64().is_none() && n.as_f64().is_some_and(f64::is_finite)),
            "numbers must be signed 64-bit integers or finite floats",
        )?,
        _ => {}
    }
    Ok(())
}

pub(super) fn validate_binding(
    decision: Decision,
    binding: &Binding,
    files: &BTreeMap<String, String>,
) -> io::Result<()> {
    require(
        files.contains_key(&binding.file) && binding.file.ends_with(".py"),
        "entry file must be a declared Python file",
    )?;
    require(
        identifier(&binding.function),
        "function must be a top-level identifier",
    )?;
    let (_, optional) = decision.fields();
    let mut seen = BTreeSet::new();
    for input in &binding.inputs {
        require(
            (optional.contains(&input.as_str())
                || (decision == Decision::Extent
                    && [
                        "process_state.hook_count",
                        "process_state.active_time",
                        "process_state.resource_counters",
                        "process_state.result_facts",
                        "process_state.semantic_facts",
                    ]
                    .contains(&input.as_str())))
                && seen.insert(input),
            "unknown or duplicate optional input",
        )?;
    }
    let parents: Vec<_> = binding.file.split('/').collect();
    for index in 1..parents.len() {
        require(
            files.contains_key(&format!("{}/__init__.py", parents[..index].join("/"))),
            "Python packages must declare __init__.py",
        )?;
    }
    check_json(&serde_json::to_value(&binding.policy_config)?, 0)
}

pub(super) fn validate_data(decision: Decision, binding: &Binding, data: &Value) -> io::Result<()> {
    check_json(data, 0)?;
    let map = data
        .as_object()
        .ok_or_else(|| invalid("Decision data must be an object"))?;
    let (required, _) = decision.fields();
    require(
        required.iter().all(|k| map.contains_key(*k))
            && binding.inputs.iter().all(|k| {
                let mut value = data;
                for part in k.split('.') {
                    if let Some(next) = value.get(part) {
                        value = next;
                    } else {
                        return false;
                    }
                }
                true
            }),
        "missing Decision input",
    )?;
    require(
        map.keys()
            .all(|k| required.contains(&k.as_str()) || binding.inputs.contains(k)),
        "input is outside this Decision's declared data",
    )?;
    if decision == Decision::Extent {
        let state = data["process_state"]
            .as_object()
            .ok_or_else(|| invalid("process_state must be an object"))?;
        require(
            state.keys().all(|k| {
                ["steps_completed", "phase"].contains(&k.as_str())
                    || binding.inputs.contains(&format!("process_state.{k}"))
            }),
            "undeclared process_state metric",
        )?;
        require(
            data["process_state"]["steps_completed"]
                .as_i64()
                .is_some_and(|v| v >= 0)
                && data["process_state"]["phase"].is_string(),
            "invalid process progress",
        )?;
        if let Some(value) = state.get("active_time") {
            require(
                value.as_f64().is_some_and(|n| n >= 0.0),
                "active_time must be nonnegative seconds",
            )?;
        }
        for key in ["resource_counters", "result_facts", "semantic_facts"] {
            if let Some(value) = state.get(key) {
                require(value.is_object(), "optional process facts must be objects")?;
            }
        }
    }
    match decision {
        Decision::OpportunityPass | Decision::FeasibilityPass => {
            let op = data["op"]
                .as_object()
                .ok_or_else(|| invalid("op must be an object"))?;
            require(
                op.get("kind").is_some_and(Value::is_string)
                    && ["has_predictor"]
                        .iter()
                        .all(|k| op.get(*k).is_some_and(Value::is_boolean)),
                "op must contain kind and the predictor fact",
            )?;
        }
        Decision::Select => {
            ids(&data["frontier_view"], "chance_id")?;
        }
        Decision::Rebalance => {
            let active = ids(&data["active_view"], "branch_id")?;
            let paused = ids(&data["paused_view"], "branch_id")?;
            require(
                active.is_disjoint(&paused),
                "Branch cannot be both active and paused",
            )?;
        }
        Decision::Predict => {
            require(
                data["assumption_set"].as_array().is_some_and(|a| {
                    let mut seen = BTreeSet::new();
                    a.iter().all(|v| {
                        v.as_object().is_some_and(|item| {
                            item.len() == 2
                                && item["chance_id"].is_string()
                                && item["proposal_id"].is_string()
                                && seen.insert((
                                    item["chance_id"].as_str().unwrap_or_default(),
                                    item["proposal_id"].as_str().unwrap_or_default(),
                                ))
                        })
                    })
                }),
                "assumption_set must contain unique (chance_id, proposal_id) references",
            )?;
        }
        Decision::Launch => require(
            data["candidate_view"].is_array(),
            "candidate_view must be an array",
        )?,
        Decision::Dispose => require(
            data["resolution"].as_str().is_some_and(|r| {
                [
                    "accepted",
                    "rejected",
                    "indeterminate",
                    "error",
                    "cancelled",
                    "preempted",
                    "extent_reached",
                    "failed",
                ]
                .contains(&r)
            }),
            "unknown resolution",
        )?,
        _ => {}
    }
    for key in required.iter().copied().chain(
        binding
            .inputs
            .iter()
            .map(String::as_str)
            .filter(|k| !k.contains('.')),
    ) {
        if ![
            "frontier_view",
            "active_view",
            "paused_view",
            "candidate_view",
            "assumption_set",
            "resolution",
            "history",
            "outcome",
        ]
        .contains(&key)
        {
            require(
                data[key].is_object() || (key == "evidence" && data[key].is_null()),
                &format!("{key} must be an object"),
            )?;
        }
    }
    Ok(())
}

pub(super) fn validate_result(decision: Decision, data: &Value, result: &Value) -> io::Result<()> {
    check_json(result, 0)?;
    let valid = match decision {
        Decision::OpportunityPass | Decision::FeasibilityPass | Decision::Launch => {
            result.is_boolean()
        }
        Decision::Extent => result
            .as_str()
            .is_some_and(|v| ["stop", "continue"].contains(&v)),
        Decision::Select => {
            let candidates = ids(&data["frontier_view"], "chance_id")?;
            let selected = result
                .as_array()
                .ok_or_else(|| invalid("select must return an array"))?;
            let mut seen = BTreeSet::new();
            selected.iter().all(|v| {
                v.as_str()
                    .is_some_and(|id| candidates.contains(id) && seen.insert(id))
            })
        }
        Decision::Rebalance => {
            let mut candidates = ids(&data["active_view"], "branch_id")?;
            candidates.extend(ids(&data["paused_view"], "branch_id")?);
            let actions = result
                .as_array()
                .ok_or_else(|| invalid("rebalance must return an array"))?;
            let mut seen = BTreeSet::new();
            actions.iter().all(|v| {
                v.as_object().is_some_and(|m| m.len() == 2)
                    && v["branch_id"]
                        .as_str()
                        .is_some_and(|id| candidates.contains(id) && seen.insert(id))
                    && v["action"]
                        .as_str()
                        .is_some_and(|a| ["pause", "resume", "preempt"].contains(&a))
            })
        }
        Decision::Predict => {
            (result.get("predict") == Some(&Value::String("branch_model".into()))
                && result.get("model").is_some_and(Value::is_string))
                || (result.as_object().is_some_and(|m| m.len() == 2)
                    && result["generation"] == "batch"
                    && result["proposals"].as_array().is_some_and(|items| {
                        items.iter().all(|p| {
                            p.as_object().is_some_and(|m| {
                                m.contains_key("payload")
                                    && m.keys()
                                        .all(|k| ["payload", "provenance"].contains(&k.as_str()))
                            })
                        })
                    }))
        }
        // Only plans without plugin-specific state/effect transfer can be checked yet.
        Decision::Effect => {
            result.as_object().is_some_and(|m| m.len() == 1)
                && result["mode"]
                    .as_str()
                    .is_some_and(|s| ["block", "deny", "allow"].contains(&s))
        }
        Decision::Match => {
            let facts_valid = data["match_context"].as_object().is_some_and(|context| {
                ["operation_id", "authority", "candidate"]
                    .iter()
                    .all(|key| context.get(*key).is_some())
            });
            facts_valid
                && result
                    .as_object()
                    .is_some_and(|m| m.keys().all(|k| ["verdict", "score"].contains(&k.as_str())))
                && result["verdict"].as_str().is_some_and(|v| {
                    ["compatible", "incompatible", "indeterminate", "error"].contains(&v)
                })
                && result.get("score").is_none()
        }
        Decision::Dispose => {
            result.as_object().is_some_and(|m| m.len() == 1)
                && result["actions"].as_array().is_some_and(|actions| {
                    if actions.is_empty() {
                        return data["resolution"].as_str().is_some_and(|r| {
                            ["indeterminate", "error", "extent_reached"].contains(&r)
                        });
                    }
                    actions.iter().all(|action| {
                        let valid_notification = action.get("notification").is_none_or(|value| {
                            value.as_str().is_some_and(|text| !text.trim().is_empty())
                        });
                        action.as_object().is_some_and(|m| {
                            (m.len() == 1 || (m.len() == 2 && action["intent"] == "Promote"))
                                && m.keys()
                                    .all(|key| ["intent", "notification"].contains(&key.as_str()))
                                && (action.get("notification").is_none()
                                    || action["intent"] == "Promote")
                                && valid_notification
                        }) && (action["intent"] == "Reuse"
                            || action["intent"] == "Salvage"
                            || action["intent"] == "Re-execute"
                            || action["intent"] == "Discard"
                            || action["intent"] == "Compensate"
                            || action["intent"] == "Overwrite"
                            || (action["intent"] == "Promote" && data["resolution"] == "accepted"))
                    })
                })
        }
    };
    require(
        valid,
        &format!(
            "invalid or currently unsupported {} result",
            decision.as_str()
        ),
    )
}

pub(super) fn ids(value: &Value, key: &str) -> io::Result<BTreeSet<String>> {
    let items = value
        .as_array()
        .ok_or_else(|| invalid("Decision view must be an array"))?;
    let mut ids = BTreeSet::new();
    for item in items {
        let id = item[key]
            .as_str()
            .ok_or_else(|| invalid(format!("view missing {key}")))?;
        require(ids.insert(id.to_owned()), "duplicate ID in input view")?;
    }
    Ok(ids)
}

#[cfg(test)]
mod notification_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn dispose_notification_is_optional_nonblank_and_promote_only() {
        let input = json!({"resolution":"accepted"});
        for action in [
            json!({"intent":"Promote"}),
            json!({"intent":"Promote","notification":"real result"}),
        ] {
            validate_result(Decision::Dispose, &input, &json!({"actions":[action]})).unwrap();
        }
        for action in [
            json!({"intent":"Promote","notification":null}),
            json!({"intent":"Promote","notification":" \n"}),
            json!({"intent":"Promote","notification":2}),
            json!({"intent":"Discard","notification":"no"}),
            json!({"intent":"Promote","other":"no"}),
        ] {
            assert!(
                validate_result(Decision::Dispose, &input, &json!({"actions":[action]})).is_err()
            );
        }
        let command: crate::control_plane::agent_control::AgentCommand =
            serde_json::from_value(json!({"command":"promote","branch_id":"winner"})).unwrap();
        assert!(
            serde_json::to_value(command)
                .unwrap()
                .get("notification")
                .is_none()
        );
        for value in [json!(null), json!(" \n"), json!(2)] {
            assert!(
                serde_json::from_value::<crate::control_plane::agent_control::AgentCommand>(
                    json!({"command":"promote","branch_id":"winner","notification":value})
                )
                .is_err()
            );
        }
    }
}

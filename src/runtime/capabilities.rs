
use std::collections::{BTreeMap, BTreeSet};

use super::selection::BackendConnection;
use crate::protocol::contract::{RuntimeCapabilities, RuntimeTool};

pub(super) fn merged_capabilities(connections: &[BackendConnection]) -> RuntimeCapabilities {
    let mut tools = BTreeMap::<String, RuntimeTool>::new();
    let mut conflicting_tools = BTreeSet::new();
    let mut branch_environment = true;
    let mut filesystem_fast_delete: Option<bool> = None;
    let mut fast_delete_conflict = false;
    let mut filesystem_fork_cost: Option<String> = None;
    let mut fork_cost_conflict = false;
    let mut filesystem_btrfs_incremental_diff: Option<bool> = None;
    let mut incremental_conflict = false;
    let mut freeze_mechanism: Option<String> = None;
    let mut freeze_conflict = false;
    let mut filesystem_passthrough: Option<Vec<String>> = None;
    let mut passthrough_conflict = false;
    let mut state_directory_overlaps_mount: Option<Vec<String>> = None;
    let mut overlaps_conflict = false;
    for connection in connections {
        let capabilities = connection.backend.capabilities();
        branch_environment &= capabilities.branch_environment;
        if let Some(value) = capabilities.filesystem_fast_delete {
            match filesystem_fast_delete {
                None => filesystem_fast_delete = Some(value),
                Some(previous) if previous != value => fast_delete_conflict = true,
                _ => {}
            }
        }
        if let Some(value) = capabilities.filesystem_fork_cost {
            match &filesystem_fork_cost {
                None => filesystem_fork_cost = Some(value),
                Some(previous) if previous != &value => fork_cost_conflict = true,
                _ => {}
            }
        }
        if let Some(value) = capabilities.filesystem_btrfs_incremental_diff {
            match filesystem_btrfs_incremental_diff {
                None => filesystem_btrfs_incremental_diff = Some(value),
                Some(previous) if previous != value => incremental_conflict = true,
                _ => {}
            }
        }
        if let Some(value) = capabilities.freeze_mechanism {
            match &freeze_mechanism {
                None => freeze_mechanism = Some(value),
                Some(previous) if previous != &value => freeze_conflict = true,
                _ => {}
            }
        }
        if let Some(value) = capabilities.filesystem_passthrough {
            match &filesystem_passthrough {
                None => filesystem_passthrough = Some(value),
                Some(previous) if previous != &value => passthrough_conflict = true,
                _ => {}
            }
        }
        if let Some(value) = capabilities.state_directory_overlaps_mount {
            match &state_directory_overlaps_mount {
                None => state_directory_overlaps_mount = Some(value),
                Some(previous) if previous != &value => overlaps_conflict = true,
                _ => {}
            }
        }
        for tool in capabilities.tools {
            if let Some(previous) = tools.get(&tool.name)
                && previous != &tool
            {
                conflicting_tools.insert(tool.name.clone());
            } else {
                tools.insert(tool.name.clone(), tool);
            }
        }
    }
    for name in conflicting_tools {
        tools.remove(&name);
    }
    RuntimeCapabilities {
        backends: connections
            .iter()
            .map(|connection| connection.id.clone())
            .collect(),
        tools: tools.into_values().collect(),
        branch_environment,
        filesystem_fast_delete: if fast_delete_conflict {
            None
        } else {
            filesystem_fast_delete
        },
        filesystem_fork_cost: if fork_cost_conflict {
            None
        } else {
            filesystem_fork_cost
        },
        filesystem_btrfs_incremental_diff: if incremental_conflict {
            None
        } else {
            filesystem_btrfs_incremental_diff
        },
        freeze_mechanism: if freeze_conflict {
            None
        } else {
            freeze_mechanism
        },
        filesystem_passthrough: if passthrough_conflict {
            None
        } else {
            filesystem_passthrough
        },
        state_directory_overlaps_mount: if overlaps_conflict {
            None
        } else {
            state_directory_overlaps_mount
        },
    }
}

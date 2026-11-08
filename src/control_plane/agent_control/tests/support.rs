//! Agent 控制树测试共享辅助。

use super::super::*;

    pub(super) fn tree() -> SpeculativeTree {
        let mut tree = SpeculativeTree::new("agent-1".into());
        tree.workspace_root = PathBuf::from("/memory");
        tree
    }


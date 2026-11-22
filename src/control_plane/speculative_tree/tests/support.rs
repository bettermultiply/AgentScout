
use super::super::*;

pub(super) fn tree() -> SpeculativeTree {
    let mut tree = SpeculativeTree::new("agent-1".into());
    tree.workspace_root = PathBuf::from("/memory");
    tree
}

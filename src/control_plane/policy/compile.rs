//! 编译后 Policy 的固定源码、绑定与摘要校验。

use std::collections::{BTreeMap, BTreeSet};
use std::io;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::decision::Decision;
use super::validation::{invalid, require, source_path, validate_binding};
use super::{MAX_BYTES, MAX_FILES, RUNNER};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Binding {
    pub(super) file: String,
    pub(super) function: String,
    #[serde(default)]
    pub(super) policy_config: BTreeMap<String, Value>,
    #[serde(default)]
    pub(super) inputs: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Definition {
    pub(super) version: u32,
    pub(super) language: String,
    pub(super) files: Vec<String>,
    pub(super) fidelity: String,
    #[serde(default)]
    pub(super) runtime: Option<String>,
    pub(super) decisions: BTreeMap<String, Value>,
}

/// Fixed source, bindings and Python environment identity. No author-side paths.
/// Deserialize only from an authenticated source; the digest is not authentication.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompiledPolicy {
    pub(super) version: u32,
    pub(super) python: String,
    pub(super) files: BTreeMap<String, String>,
    #[serde(default)]
    pub(super) runtime: Option<String>,
    pub(super) decisions: BTreeMap<Decision, Binding>,
    pub(super) digest: String,
}

impl CompiledPolicy {
    pub fn digest(&self) -> &str {
        &self.digest
    }

    /// Runtime configuration selected by this Policy, if any.
    pub fn runtime(&self) -> Option<&str> {
        self.runtime.as_deref()
    }

    /// A serializable Python script with all declared source and bindings embedded.
    /// Run through PolicyEngine, not as an unrestricted Python program.
    pub fn script(&self, decision: Decision) -> io::Result<String> {
        self.validate()?;
        let manifest = json!({
            "version": self.version, "python": self.python, "files": self.files,
            "binding": self.decisions.get(&decision).ok_or_else(|| invalid("missing Decision"))?,
            "decision": decision.as_str(), "digest": self.digest,
        });
        Ok(format!(
            "import os,json\nos.dup2(1,3)\nos.dup2(2,1)\nMANIFEST = json.loads({})\n{}",
            serde_json::to_string(&manifest.to_string())?,
            RUNNER
        ))
    }

    pub(super) fn content_digest(&self) -> io::Result<String> {
        let bytes = serde_json::to_vec(&(
            self.version,
            &self.python,
            &self.files,
            &self.runtime,
            &self.decisions,
            RUNNER,
        ))?;
        Ok(format!("{:x}", Sha256::digest(bytes)))
    }

    pub(super) fn validate(&self) -> io::Result<()> {
        require(self.version == 1, "unsupported compiled Policy version")?;
        require(
            self.decisions.keys().copied().collect::<BTreeSet<_>>() == Decision::ALL.into(),
            "incomplete Decision bindings",
        )?;
        require(
            !self.files.is_empty() && self.files.len() <= MAX_FILES,
            "invalid file count",
        )?;
        require(
            self.files.values().map(String::len).sum::<usize>() <= MAX_BYTES,
            "Policy source exceeds size limit",
        )?;
        for path in self.files.keys() {
            source_path(path)?;
        }
        for (&decision, binding) in &self.decisions {
            validate_binding(decision, binding, &self.files)?;
        }
        require(
            self.digest == self.content_digest()?,
            "compiled Policy digest mismatch",
        )
    }
}


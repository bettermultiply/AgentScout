//! 编译后 Policy 的固定源码、绑定与摘要校验。

use std::collections::{BTreeMap, BTreeSet};
use std::io;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::decision::Decision;
use super::validation::{invalid, require, source_path, validate_binding, validate_model_grant};
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) history: Option<HistoryInput>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) outcome: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct HistoryInput {
    pub mode: String,
    pub max_entries: usize,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy_model_grant: Option<PolicyModelGrant>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyModelGrant {
    pub decisions: Vec<Decision>,
    pub models: Vec<String>,
    pub max_requests: usize,
    pub timeout_ms: u64,
}

/// Fixed source, bindings, Python identity and author-side provenance.
/// Deserialize only from an authenticated source; the digest is not authentication.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompiledPolicy {
    // Provenance is retained from the same reads used by compilation. It does
    // not enter the executable digest, which remains the Decision identity.
    pub(super) source: std::path::PathBuf,
    pub(super) definition: String,
    pub(super) runtime_config_path: Option<std::path::PathBuf>,
    pub(super) version: u32,
    pub(super) python: String,
    pub(super) files: BTreeMap<String, String>,
    #[serde(default)]
    pub(super) runtime: Option<String>,
    pub(super) decisions: BTreeMap<Decision, Binding>,
    pub(super) digest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) policy_model_grant: Option<PolicyModelGrant>,
    // Derived from the bound source by the compile probe. Artifact bytes are
    // authenticated by the caller, and the executable identity stays stable.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(super) pure: BTreeMap<Decision, Value>,
}

impl CompiledPolicy {
    pub fn source(&self) -> &std::path::Path {
        &self.source
    }

    /// Deployment path resolved at compilation, not relative to the artifact.
    pub fn runtime_config_path(&self) -> Option<&std::path::Path> {
        self.runtime_config_path.as_deref()
    }

    pub fn load(path: &std::path::Path) -> io::Result<Self> {
        Self::from_bytes(&std::fs::read(path)?)
    }

    /// Decode the very bytes whose artifact identity the caller checked.
    pub fn from_bytes(bytes: &[u8]) -> io::Result<Self> {
        let policy: Self = serde_json::from_slice(bytes).map_err(invalid)?;
        policy.validate()?;
        policy.validate_opportunity_program()?;
        Ok(policy)
    }

    /// Publish only complete artifacts, and never replace an existing run's data.
    pub fn save(&self, path: &std::path::Path) -> io::Result<()> {
        use std::io::Write;
        self.validate()?;
        self.validate_opportunity_program()?;
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or(std::path::Path::new("."));
        let mut file = tempfile::NamedTempFile::new_in(parent)?;
        file.write_all(&serde_json::to_vec_pretty(self).map_err(invalid)?)?;
        file.as_file().sync_all()?;
        file.persist_noclobber(path).map_err(|error| error.error)?;
        std::fs::File::open(parent)?.sync_all()
    }

    pub fn digest(&self) -> &str {
        &self.digest
    }

    pub fn policy_model_grant(&self) -> Option<&PolicyModelGrant> {
        self.policy_model_grant.as_ref()
    }

    pub(crate) fn history_max_entries(&self) -> usize {
        self.decisions.get(&Decision::Predict).and_then(|binding| binding.history.as_ref())
            .map_or(16, |history| history.max_entries)
    }

    pub fn decision_inputs(&self, decision: Decision) -> Option<&[String]> {
        self.decisions
            .get(&decision)
            .map(|binding| binding.inputs.as_slice())
    }

    /// Whether the compiled Policy contains this Decision.
    pub fn has_decision(&self, decision: Decision) -> bool {
        self.decisions.contains_key(&decision)
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

    pub fn pure_decision(&self, decision: Decision) -> Option<&Value> {
        self.pure.get(&decision)
    }

    pub fn local_pure_decision(&self, decision: Decision) -> Option<Value> {
        fn expression(value: &Value) -> Value {
            match value {
                Value::Number(number) if number.is_f64() => json!(["float", value]),
                Value::Array(values) => Value::Array(std::iter::once(json!("list"))
                    .chain(values.iter().map(expression)).collect()),
                Value::Object(fields) => dictionary(fields),
                _ => json!(["const", value]),
            }
        }
        fn dictionary<'a>(fields: impl IntoIterator<Item = (&'a String, &'a Value)>) -> Value {
            Value::Array(std::iter::once(json!("dict"))
                .chain(fields.into_iter().map(|(key, value)| json!([key, expression(value)])))
                .collect())
        }
        Some(json!({
            "program": self.pure_decision(decision)?,
            "policy_config": dictionary(&self.decisions.get(&decision)?.policy_config),
        }))
    }

    /// Run an extended compile probe without changing the legacy runner bytes.
    pub(super) fn check_script(&self, decision: Decision) -> io::Result<String> {
        let script = self.script(decision)?;
        let helper = include_str!("pure_compile.py");
        let script = script.replacen(
            "\nif __name__ == \"__main__\":",
            &format!("\n{helper}\nif __name__ == \"__main__\":"),
            1,
        );
        // Return metadata before anything imports the Policy module. The
        // regular check still handles runtime signature checks for fallbacks.
        Ok(script.replacen(
            "    sys.meta_path.insert(0, Sources(sources))",
            "    if request.get('check') is True:\n        emit_pure_check(tree, functions[0], binding)\n        return\n    sys.meta_path.insert(0, Sources(sources))",
            1,
        ))
    }

    pub(super) fn content_digest(&self) -> io::Result<String> {
        let content = (
            self.version,
            &self.python,
            &self.files,
            &self.runtime,
            &self.decisions,
            RUNNER,
        );
        let bytes = match &self.policy_model_grant {
            Some(grant) => serde_json::to_vec(&(content, grant))?,
            None => serde_json::to_vec(&content)?,
        };
        Ok(format!("{:x}", Sha256::digest(bytes)))
    }

    fn validate_opportunity_program(&self) -> io::Result<()> {
        self.pure_decision(Decision::OpportunityPass)
            .filter(|value| !value.is_null())
            .map(|_| ())
            .ok_or_else(super::validation::opportunity_requires_pure)
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
        if let Some(grant) = &self.policy_model_grant {
            validate_model_grant(grant)?;
        }
        require(
            self.digest == self.content_digest()?,
            "compiled Policy digest mismatch",
        )
    }
}

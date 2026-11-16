# Predictor fact regression Policy

`predictor-fact-policy.json` freezes the real NLE Policy saved with the failed
2026-11-16 first-request run. Only `source` and `runtime_config_path` were
sanitized; definition, sources, bindings, Python identity, runtime name and
digest are unchanged. It contains no credentials, user prompts or model replies.
The original failed run remains separate evidence, not a test dependency.

`predictor-fact-policy.mjs` verifies the executable digest using the same tuple
and ordering as `CompiledPolicy::content_digest`. The original digest is
`839e90e6e92df9bd6c818cbc8c4fefccb8cc803df0eaf42a7bbb848abfa3fa92`.
The E2E recompiles the frozen YAML/sources and checks all semantic fields and
the digest. Python's complete version string participates in that digest: on
another installed Python build, only that field may differ in the new digest.

Both predictor tests use this repository fixture by default. An explicit
`ASCOUT_PREDICTOR_POLICY` may select another saved artifact. Remaining inputs
are repository sources, adjacent Pi and (for E2E) the existing NLE venv and built
control-plane/adapter binaries. The temporary run directory contains generated
outputs only. No input comes from the retained failed-run directory.

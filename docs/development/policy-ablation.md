# Coding policy ablation runs

The coding policy keeps the historical strategy when all ablation switches are false. The switches are in `assets/policy/coding/policy.yaml` and are passed to the corresponding policy decisions.

- `fast_model_speculation` admits model-inference opportunities.
- `candidate_thinking` selects its independent thinking level. The authority keeps its own thinking level.
- `max_candidate_model_steps` limits completed Flash model inferences per candidate branch when `fast_model_speculation` is enabled. `0` prevents a model candidate from starting, `1` permits one Flash inference, and larger values permit that many consecutive Flash inferences. Once the limit is reached, the candidate continues with the authority model until it is Promoted or Discarded. When the field is absent, the historical fast-speculation behavior is preserved.
- `partial_accept_salvage` permits independently verified results to use `Salvage`. The policy emits `Discard` after the salvage when `partial_accept_discard` is true.
- `skip_authority_after_independent_verification` makes `launch` return false only for recognized, non-external verification commands. The candidate result must also contain a successful observable `bash` verification result with no recognized failure marker. A candidate's text claiming success is not sufficient.

Use a fresh temporary policy copy for each arm so runs do not modify the shared policy.

```sh
root=$(mktemp -d)
cp -a assets/policy/coding/. "$root/"
# Edit only the switch values in "$root/policy.yaml".
# Start the ordinary offline fixture with --policy "$root/policy.yaml".
```

For a paired run, keep the task, workspace seed, fake-model script, runtime configuration, model order, and thinking levels fixed. Alternate the baseline and ablation arms, and repeat each pair for the planned number of repetitions. Use the event log and telemetry spans as the measurement record. Do not infer an effect from one run.

Each model action has start and finish timestamps. The usage event records input tokens, output tokens, cache-read tokens, cache-write tokens, total tokens, and cost when the provider supplies them. Telemetry also records `ascout.action.cancelled` and `ascout.authority.skipped`. Candidate and authority are identified by their branch role and branch ID. A cancelled request is an action with `result=cancelled`. A skipped authority action has no authority model request and carries `ascout.authority.skipped=true` on its control-plane resolution when that event is available.

Aggregate interleaved repetitions by arm and report the median, P25, P75, and the number of valid successful samples for elapsed time, model-action latency, candidate waiting, authority waiting, cancellation count, skipped-authority count, tokens, cache tokens, and cost. A result with a failed fixture, missing usage record, or incomplete event sequence is excluded from the successful-sample count and reported separately.

The local validation commands are:

```sh
python3 -m py_compile assets/policy/coding/decisions.py
cargo check
node --test tests/coding-policy-e2e.mjs
```

The offline fixture uses a fake model. It does not call a real model or authenticate to a provider.

## Current boundary

The control plane already validates file ownership, result dependencies, fork-base conflicts, and selective acceptance before applying `Salvage`. The candidate model now receives its configured thinking level independently from the authority.

The telemetry path records action latency and provider usage. Candidate and authority wait durations are derivable from the action and branch timestamps, but are not yet emitted as dedicated aggregate fields. A future report generator should derive them from the event sequence before calculating percentiles.

## Final campaign sessions 16–20

The generic coding-policy adjustment is in `assets/policy/coding/decisions.py` and is opt-in through `partial_accept_unverified_workspace` in `assets/policy/coding/policy.yaml`. With `require_authority_review`, `partial_accept_salvage`, and this switch enabled, a bounded candidate that made successful workspace mutations but lacks trustworthy post-change evidence can produce workspace-only `Salvage` followed by `Discard`. Its notification identifies an unverified partial draft, includes affected paths and available test evidence, and asks Authority to verify the entire public request and correct it. Direct `Promote` remains unavailable for rejected or failed evidence.

The offline regression in `tests/policy-ablation-fixture.mjs` covers this disposition and failed-test non-promotion. `python3 -m py_compile assets/policy/coding/decisions.py`, the five-test Node fixture, `cargo check`, and the Sphinx fake-model preflight passed. The acceptance runner was unchanged. The broader `tests/coding-policy-e2e.mjs` remained 4/5 because its semantic-audit scenario still launched one Authority request where the fixture expects zero. The three other policy flows and both partial-transfer E2E cases passed.

Sessions 16–18 used the Django fixture under `/var/tmp/ascout-bench/django-101112`.

- Session 16 used 5 candidate steps and 12 maximum steps. It launched successfully in 173.799 seconds, made 21 model actions, recorded 3 candidate branches, 1 Salvage, 2 Discards, and cost 0.123337508. Official acceptance was `FAIL_TO_PASS 4/5` and `PASS_TO_PASS 382/382`, with exit 1.
- Session 17 used 8 candidate steps and 18 maximum steps. It launched successfully in 115.951 seconds, made 17 model actions, recorded one accepted partial transfer and one discard, and cost 0.073853904. Official acceptance was `FAIL_TO_PASS 1/5` and `PASS_TO_PASS 382/382`, with exit 1.
- Session 18 repeated the 8-step arm because neither earlier arm was fully accepted. It launched successfully in 144.626 seconds, made 18 model actions, recorded 3 Promotes, 1 Salvage, and cost 0.154627792. Official acceptance was `FAIL_TO_PASS 1/5` and `PASS_TO_PASS 382/382`, with exit 1.

No Django arm was fully accepted, so the frozen Sphinx policy used the preserved arm 7 fallback. Its exact policy and hash snapshot are in `/var/tmp/ascout-bench/sphinx-1920/freeze`, with the fallback reason in `decision.txt` and reproducibility inputs in `hashes.txt`.

- Session 19 was the serial Sphinx baseline in `/var/tmp/ascout-bench/sphinx-session-19`. It launched successfully in 275.686 seconds, made 18 authority model actions, published once, cost 0.3529412, and official acceptance exited 0.
- Session 20 used the identical frozen arm 7 policy in `/var/tmp/ascout-bench/sphinx-1920`. It launched successfully in 576.864 seconds, made 47 model actions across 8 candidate branches, recorded 3 Promotes and 5 Discards, published once, cost 0.786278392, and official acceptance was `FAIL_TO_PASS 0/3` and `PASS_TO_PASS 59/59`, with exit 1.

Compared with Sphinx serial, the frozen policy increased wall time by 301.178 seconds and cost by 0.433337192. It did not produce an accepted Sphinx result, so no speed or quality benefit is claimed. The only failed campaign launch attempt was the pre-session-20 harness argument error. It created no session or model launch and was corrected before the real session. The campaign used exactly five real sessions, 16 through 20, with no retries.

The reusable frozen-policy location is `/var/tmp/ascout-bench/sphinx-1920/policies/coding`. Reproduce the Sphinx frozen run with the prepared root, `model-preflight --arm 7`, then `model-arm --arm 7 --session 20` using `ascout-swebench/run-django-ablation.py`.
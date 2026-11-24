import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { test } from 'node:test';
import { resolve } from 'node:path';

const policy = resolve('assets/policy/coding/decisions.py');
function call(functionName, data, config = {}) {
    const script = `import importlib.util, json, sys
s = importlib.util.spec_from_file_location('decisions', sys.argv[1])
m = importlib.util.module_from_spec(s)
s.loader.exec_module(m)
print(json.dumps(getattr(m, sys.argv[2])(json.loads(sys.argv[3]), policy_config=json.loads(sys.argv[4]), capabilities={})))`;
    return JSON.parse(execFileSync('python3', ['-c', script, policy, functionName,
        JSON.stringify(data), JSON.stringify(config)], {encoding: 'utf8',
        env: {...process.env, PYTHONPATH: resolve('policy_lib'), PYTHONDONTWRITEBYTECODE: '1'}}));
}

const model = {kind: 'model_inference', authority: true};
const bash = command => ({kind: 'bash', produces_result: true, result_pending: true, arguments: {command}});
const entry = (tool_name, arguments_, extra = {}) => ({tool_name, arguments: arguments_, status: 'succeeded', ...extra});
const read = (path, extra) => entry('read', {path}, extra);
const edit = entry('edit', {path: 'src/sample.py'}, {timestamp_ms: 50});
const verified = (command, extra) => entry('bash', {command}, extra);
const predict = (history, chance_context = model, config = {}, process_context = {}) =>
    call('predict', {chance_context, history, process_context}, config);

test('shared information matcher', () => {
    execFileSync('python3', ['-B', 'tests/policy-info-match.py'], {
        env: {...process.env, PYTHONPATH: resolve('policy_lib')}, stdio: 'inherit',
    });
});

test('Explore → Modify-Early → Modify-Ready → Verify → Explore', () => {
    const main = verified('cd ./src && pytest tests');
    const failure = {...main, status: 'failed'};
    const guide = {candidate_model: 'fixture/fast', candidate_guidance: 'Find relevant code.'};
    assert.deepEqual(predict([], model, guide), {predict: 'branch_model', model: 'fixture/fast', thinking: 'off', candidate_guidance: 'Find relevant code.'});
    assert.deepEqual(predict([edit]), {abstain: true, reason: 'modify_early'});
    assert.equal(predict([edit], model, {speculate_early_modify: true}).predict, 'branch_model');
    const ready = predict([failure, edit]);
    assert.deepEqual(ready.proposals[0], {
        payload: {kind: 'predicted_tool_calls', tool_calls: [{name: 'bash', arguments: main.arguments}]},
        provenance: {kind: 'active_post_edit_test'},
    });
    assert.ok(predict([failure, edit], model, {}, {authority_active_tools: [main]}).abstain);
    assert.ok(predict([failure, edit], model, {active_post_edit_test: false}).abstain);
    for (const command of ['cd .baseline && pytest', 'cd /tmp/work && pytest', 'cd ../baseline && pytest',
        'cd src && cd ".test copies/baseline" && pytest']) {
        assert.equal(predict([main, edit, verified(command, {status: 'failed'})]).proposals[0]
            .payload.tool_calls[0].arguments.command, main.arguments.command, command);
        assert.ok(predict([verified(command, {status: 'failed'}), edit]).abstain, command);
    }
    assert.equal(predict([failure, edit], bash(main.arguments.command)).proposals.length, 2);
    assert.equal(predict([failure, edit, main]).predict, 'branch_model');
    assert.equal(predict([edit, failure, read('a'), {kind: 'model_inference'}, read('b')]).predict, 'branch_model');
});

test('an active explorer only suppresses exploration predictions', () => {
    const explore = {branch_id: 'explorer', speculation_kind: 'model_inference', yield_only: true};
    const context = {active_branches: [explore]};
    assert.deepEqual(predict([], model, {}, context), {abstain: true, reason: 'active_explore_branch'});
    assert.equal(predict([], model, {}, {active_branches: [{...explore, speculation_kind: 'tool_execution'}]}).predict, 'branch_model');
    assert.equal(predict([], model, {}, {active_branches: [{...explore, proposal_provenance: {kind: 'active_post_edit_test'}}]}).predict, 'branch_model');
    const previous = verified('pytest tests', {status: 'failed'});
    assert.equal(predict([previous, edit], model, {}, context).proposals[0].provenance.kind, 'active_post_edit_test');
    assert.equal(predict([edit], bash('pytest'), {}, context).proposals[0].provenance.phase, 'post_source_edit');
});

test('shell history distinguishes Python inspection from explicit writes', () => {
    for (const command of ["python - <<'PY'\nimport inspect\nprint(inspect.getsource(print))\nPY",
        `python -c 'print("x > y")'`, `python -c 'open("src.py", "r").read()'`,
        'inspect-tool src.py', 'echo "x > y"', 'inspect-tool src.py 2>&1']) {
        assert.equal(predict([verified(command)]).predict, 'branch_model', command);
    }
    for (const command of ['echo hi > a.py', 'echo hi >> a.py', 'sed -i s/x/y/ a.py',
        'perl -pi -e s/x/y/ a.py', 'git apply changes.diff', 'cat a.py | tee b.py',
        `python -c 'open("src.py", mode="r+")'`,
        "python - <<'PY'\nfrom pathlib import Path\nPath('a.py').write_text('updated')\nPY"]) {
        assert.deepEqual(predict([verified(command)]), {abstain: true, reason: 'modify_early'}, command);
    }
});

test('workspace verification accepts absolute interpreters and hidden paths in pytest arguments', () => {
    for (const command of ['TMPDIR="$PWD/.test-tmp" python -m pytest --basetemp "$PWD/.pytest-work" tests/test_a.py',
        'pytest -o cache_dir=.test-tmp/cache tests/test_a.py', '/usr/bin/python3 -m pytest tests/test_a.py',
        'pytest .baseline/tests']) {
        const proposals = predict([edit], bash(command)).proposals;
        assert.deepEqual(proposals.map(p => p.payload.isError), [false, true], command);
        assert.deepEqual(predict([verified(command), edit]).proposals?.[0], {
            payload: {kind: 'predicted_tool_calls', tool_calls: [{name: 'bash', arguments: {command}}]},
            provenance: {kind: 'active_post_edit_test'},
        }, command);
    }
});

test('Modify-Ready selects the latest wide verification or falls back to the latest narrow verification', () => {
    const olderNarrow = verified('pytest tests/test_a.py::test_two');
    const narrow = verified('pytest tests/test_a.py::test_one');
    const olderWide = verified('pytest tests');
    const wide = verified('pytest tests/test_a.py');
    for (const [history, expected] of [[[olderNarrow, narrow], narrow], [[olderWide, wide, narrow], wide]]) {
        assert.deepEqual(predict([...history, edit]).proposals?.[0], {
            payload: {kind: 'predicted_tool_calls', tool_calls: [{name: 'bash', arguments: expected.arguments}]},
            provenance: {kind: 'active_post_edit_test'},
        });
    }
});

test('verification bets use native output and independent model settings', () => {
    const testEdit = entry('write', {path: 'tests/test_sample.py'});
    const proposals = (history, command = 'pytest tests', config = {}) => predict(history, bash(command), config).proposals;
    const bets = (...args) => proposals(...args).map(p => p.payload.isError);
    assert.deepEqual(bets([]), [false]);
    assert.deepEqual(bets([testEdit]), [true]);
    assert.deepEqual(bets([edit]), [false, true]);
    assert.deepEqual(bets([edit, verified('pytest tests', {is_error: true})]), [false, true]);
    assert.deepEqual(bets([edit, verified('python3 -m pytest tests')]), [false]);
    assert.deepEqual(bets([edit], 'pytest tests', {adaptive_dual_tool_speculation: false}), [false]);
    assert.deepEqual(bets([edit], 'cd .baseline && pytest tests'), [false]);
    for (const command of ['pytest -k test_foo', 'pytest -ktest_foo', 'pytest -k "test_one and not slow"',
        'pytest -k "not slow" tests/test_sample.py::test_one',
        'cd src && TMPDIR="$PWD/tmp" python -m pytest --basetemp="$PWD/tmp" tests/test_sample.py::TestClass::test_one']) {
        assert.deepEqual(bets([edit], command), [false], command);
    }
    for (const command of ['pytest tests/test_sample.py', 'pytest tests/test_sample.py::test_one tests/other',
        'pytest -- tests/test_sample.py::test_one tests', 'pytest -k "not test_cython"', 'pytest -k "not (a or b)"',
        'pytest -k "not foo and not bar"', 'pytest -k"(not foo) and (not bar)"', 'pytest -k="not slow"']) {
        assert.deepEqual(bets([edit], command), [false, true], command);
    }
    assert.deepEqual(bets([verified('pytest tests -k smoke'), edit], 'pytest tests'), [false, true]);
    assert.deepEqual(bets([edit, verified('pytest tests -k smoke')]), [false, true]);
    const [pass, fail] = proposals([edit], 'pytest tests', {fail_candidate_model: 'fixture/fast', fail_candidate_guidance: 'Inspect code.'});
    assert.deepEqual(pass.payload, {content: [{type: 'text', text: 'All tests passed.'}], isError: false});
    assert.equal(pass.model, undefined);
    assert.deepEqual(fail.payload, {content: [{type: 'text', text: 'Command exited with non-zero status.'}], isError: true});
    assert.equal(fail.model, 'fixture/fast');
    assert.equal(fail.thinking, 'off');
    assert.equal(fail.candidate_guidance, 'Inspect code.');
    assert.equal(fail.provenance.uses_candidate_model, true);
    const [mainFail] = proposals([testEdit], 'pytest tests', {fail_candidate_model: ''});
    assert.equal(mainFail.provenance.uses_candidate_model, false);
    assert.equal(mainFail.model, undefined);
});

const candidate = {speculation_kind: 'model_inference'};
const proactive = {provenance: {kind: 'active_post_edit_test'}, payload: {tool_calls: [{name: 'bash', arguments: {command: 'pytest tests'}}]}};
const match = (proposal, outcome, results, branch = candidate, config = {}) => call('match_proposal', {
    proposal, outcome, match_context: {candidate: branch, candidate_results: results},
}, config);
const dispose = (resolution, context, config = {}) => call('dispose', {resolution, context}, config).actions;
const result = (id, path, content) => read(path, {action_id: id, result: {content}});

test('Explore yields only novel read-only results, including an empty Yield while running', () => {
    const outcome = {tool_calls: [{name: 'read', arguments: {path: 'a', offset: 2, limit: 2}}]};
    const entries = [{...result('shared', 'a', 'one\ntwo\nthree\nfour'), arguments: {path: 'a', limit: 4}}, result('novel', 'b', 'fresh'),
        {...edit, action_id: 'write'}, result('unsafe', 'c', 'after candidate mutation')];
    const proposal = {payload: outcome};
    const hit = match(proposal, outcome, entries);
    assert.equal(hit.verdict, 'incompatible');
    const safe = hit.details.safe_results;
    assert.deepEqual(safe.map(r => r.id), ['shared', 'novel']);
    assert.match(safe[0].notification, /one/);
    assert.match(safe[0].notification, /four/);
    assert.doesNotMatch(safe[0].notification, /two|three/);
    const context = {candidate, candidate_proposal: proposal, authority_outcome: outcome, candidate_results: entries};
    assert.deepEqual(dispose('rejected', {...context, match: hit}), [{intent: 'Yield', results: safe}]);
    assert.deepEqual(dispose('rejected', {...context, candidate: {...candidate, yielded_action_ids: ['shared', 'novel']}}),
        [{intent: 'Yield', results: []}]);
    for (const resolution of ['extent_reached', 'preempted']) {
        assert.deepEqual(dispose(resolution, context), [{intent: 'Salvage', results: safe}, {intent: 'Discard'}]);
    }
});

test('Modify-Ready promotes matching verification and yields or salvages unmatched output', () => {
    const matching = {tool_calls: [{name: 'bash', arguments: {command: 'TMPDIR=/tmp python3 -m pytest -q tests'}}]};
    const hit = match(proactive, matching, []);
    assert.equal(hit.verdict, 'compatible');
    assert.deepEqual(dispose('accepted', {candidate, candidate_proposal: proactive, authority_outcome: matching, match: hit}),
        [{intent: 'Promote'}]);
    const context = {candidate: {...candidate, active_tools: [verified('pytest tests')]}, candidate_proposal: proactive,
        authority_outcome: {tool_calls: []}};
    assert.deepEqual(dispose('rejected', context), [{intent: 'Yield', results: []}]);
    const report = verified('pytest tests', {status: 'failed', action_id: 'test', result: {content: '1 failed'}});
    assert.match(dispose('rejected', {...context, candidate, candidate_results: [report]})[0].results[0].notification, /1 failed$/);
});

test('verification runs multiple inferences and allows edits until the next verification', () => {
    const extent = (next_action, options = {}) => call('extent', {process_state: {steps_completed: options.steps ?? 4,
        semantic_facts: {speculation_kind: options.kind ?? 'tool_execution', prediction_kind: options.prediction, next_action},
        result_facts: {completed_tools: options.completed ?? []}, resource_counters: {model_inferences: 3}}});
    const action = (name, args) => ({kind: 'tool_execution', name, arguments: args});
    assert.equal(extent({kind: 'model_inference'}), 'continue');
    assert.equal(extent(action('edit', {path: 'a'})), 'continue');
    assert.equal(extent(action('write', {path: 'a'}), {kind: 'model_inference'}), 'continue');
    assert.equal(extent(action('bash', {command: 'cargo test'})), 'stop');
    assert.equal(extent({kind: 'model_inference'}, {prediction: 'predicted_tool_calls'}), 'stop');
    assert.equal(extent({kind: 'model_inference'}, {steps: 12}), 'stop');
    assert.equal(extent(action('read', {path: './a'}), {completed: [{name: 'read', arguments: {path: 'a'}, status: 'succeeded'}]}), 'stop');
});

test('verification settlement preserves full output or salvages weak-model read-only facts', () => {
    const [pass, fail] = predict([edit], bash('pytest')).proposals;
    const output = 'FIRST\n' + 'x'.repeat(10_000) + '\nLAST';
    const authority_outcome = {status: 'succeeded', is_error: false, content: output};
    assert.equal(match(pass, authority_outcome, [], {}).verdict, 'compatible');
    const promote = dispose('accepted', {candidate_proposal: pass, authority_outcome})[0];
    assert.equal(promote.intent, 'Promote');
    assert.ok(promote.notification.endsWith(output));
    const huge = {...authority_outcome, content: 'FIRST' + 'x'.repeat(21_000) + 'LAST'};
    const bounded = dispose('accepted', {candidate_proposal: pass, authority_outcome: huge})[0].notification.split('\n').slice(1).join('\n');
    assert.equal(bounded.length, 20_000);
    assert.match(bounded, /^FIRST[\s\S]*LAST$/);
    const failure = {...authority_outcome, status: 'failed', is_error: true, content: 'AssertionError'};
    assert.equal(match(fail, failure, [], {}).verdict, 'compatible');
    const candidate_results = [result('r', 'src/a.py', 'useful'), {...edit, action_id: 'edit'}];
    const salvage = dispose('accepted', {candidate_proposal: fail, authority_outcome: failure, candidate_results});
    assert.deepEqual(salvage.map(a => a.intent), ['Salvage']);
    assert.deepEqual(salvage[0].results.map(r => r.id), ['r']);
    assert.deepEqual(dispose('accepted', {candidate_proposal: fail, authority_outcome: failure}), [{intent: 'Discard'}]);
    const mainFail = predict([edit], bash('pytest'), {fail_candidate_model: ''}).proposals[1];
    assert.equal(dispose('accepted', {candidate_proposal: mainFail, authority_outcome: failure})[0].intent, 'Promote');
});

test('safe results fit whole file or command output and deduplicate verification by signature', () => {
    const report = (id, command, content) => verified(command, {action_id: id, result: {content}});
    const large = 'x'.repeat(6_000) + '\nend of test';
    const entries = [report('old', 'pytest tests', 'old output'), report('full', 'python3 -m pytest -q tests', large),
        report('too-large', 'cargo test', 'z'.repeat(8_000)), result('small', 'a', 'small output')];
    const safe = match(proactive, {}, entries).details.safe_results;
    assert.deepEqual(safe.map(r => r.id), ['full', 'small']);
    assert.ok(safe[0].notification.endsWith(large));
    const files = Array.from({length: 10}, (_, i) => result(`r${i}`, `p${i}`, 'data'));
    assert.equal(match({}, null, files).details.safe_results.length, 4);
    const manifest = readFileSync(resolve('assets/policy/coding/policy.yaml'), 'utf8');
    assert.deepEqual([...manifest.matchAll(/safe_result_chars: (\d+)\s+safe_result_count: (\d+)/g)]
        .map(([, chars, count]) => [Number(chars), Number(count)]), [[8000, 4], [8000, 4]]);
    assert.deepEqual(match(proactive, {}, entries, {...candidate, yielded_action_ids: ['old']}).details.safe_results.map(r => r.id), ['small']);
});

test('rebalance retires stale explorers, failed branches, and oldest yield-only branch under pressure', () => {
    const branch = (branch_id, started_at_ms, kind = 'model_inference', yield_only = true, failed = 0) =>
        ({branch_id, started_at_ms, speculation_kind: kind, yield_only, tool_counts: {failed}});
    const rebalance = (active_view, scheduling_context, paused_view = []) => call('rebalance',
        {active_view, paused_view, scheduling_context}, {max_active_candidates: 2});
    const old = branch('old', 40), recent = branch('recent', 55);
    const preempt = branch_id => ({branch_id, action: 'preempt'});
    assert.deepEqual(rebalance([old, recent], {authority_recent_actions: [edit]}), [preempt('old')]);
    assert.deepEqual(rebalance([old, recent], {authority_recent_actions: [{...edit, status: 'failed'}]}), []);
    assert.deepEqual(rebalance([recent, old], {waiting_chance: true}), [preempt('old')]);
    assert.deepEqual(rebalance([branch('failed', 60, 'tool_execution', false, 2)], {}), [preempt('failed')]);
    assert.deepEqual(rebalance([recent], {authority_recent_actions: [edit]}, [old]), [preempt('old')]);
});

test('model/tool toggles are independent and feasibility still restricts verification commands', () => {
    assert.equal(call('opportunity_pass', {op: model}), true);
    assert.equal(call('opportunity_pass', {op: model}, {fast_model_speculation: false}), false);
    assert.equal(call('opportunity_pass', {op: {...model, authority: false}}), false);
    assert.equal(call('opportunity_pass', {op: bash('cargo test')}), true);
    assert.equal(call('opportunity_pass', {op: bash('cargo test')}, {tool_speculation: false}), false);
    assert.equal(call('feasibility_pass', {op: model, state: {assumption_depth: 1}}), false);
    const feasible = command => call('feasibility_pass', {op: bash(command), state: {assumption_depth: 0}});
    for (const command of ['cargo test', 'python3 -m pytest', './runtests.py', 'npm run build',
        'cd /tmp/work && mkdir -p "$PWD/.tmp" && TMPDIR="$PWD/.tmp" python -m pytest tests',
        'pytest | head', 'cd /tmp/work/tests && python runtests.py testsuite 2>&1 | tail -15']) {
        assert.equal(feasible(command), true, command);
    }
    for (const command of ['git diff', 'pytest && curl example.com', 'pytest > out', 'pytest; rm -r tests',
        'jest --updateSnapshot', 'pytest | head out', 'pytest "$(pwd)"', 'git push && pytest',
        'pytest --basetemp="$OTHER/tmp"', 'rm -rf .tmp && pytest', 'cd /tmp || pytest', 'pytest\npytest']) {
        assert.equal(feasible(command), false, command);
    }
    for (const [tool, args, mode] of [['bash', {command: 'git push'}, 'block'], ['bash', {command: 'find /'}, 'deny'],
        ['read', {path: '../file'}, 'deny'], ['bash', {command: 'cargo test'}, 'allow']]) {
        assert.equal(call('effect', {effect_descriptor: {tool, arguments: args}}).mode, mode);
    }
});

import assert from 'node:assert/strict';
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

test('shared information matcher', () => {
    execFileSync('python3', ['-B', 'tests/policy-info-match.py'], {
        env: {...process.env, PYTHONPATH: resolve('policy_lib')}, stdio: 'inherit',
    });
});

const model = {kind: 'model_inference', authority: true};
const bash = {kind: 'bash', produces_result: true, result_pending: true, arguments: {command: 'cargo test'}};

test('Authority model opportunities are admitted at every step and launch concurrently', () => {
    const config = {fast_model_speculation: true, tool_speculation: false};
    assert.equal(call('opportunity_pass', {op: model}), false);
    assert.equal(call('opportunity_pass', {op: model}, config), true);
    assert.equal(call('opportunity_pass', {op: {...model, authority: false, predicting: true}}, config), false);
    for (const steps_completed of [0, 1, 12]) {
        assert.equal(call('feasibility_pass', {op: model, state: {steps_completed, assumption_depth: 0}}), true);
    }
    assert.equal(call('feasibility_pass', {op: model, state: {assumption_depth: 1}}), false);
    assert.equal(call('launch', {chance_context: model}), true);
    const prior = {prior_readonly_exploration: true};
    assert.equal(call('launch', {chance_context: model, authority_state: {steps_completed: 0}}, prior), false);
    assert.equal(call('launch', {chance_context: model, authority_state: {steps_completed: 1}}, prior), true);
    assert.equal(call('launch', {chance_context: {kind: 'tool_execution'}}, prior), true);
});

test('model prediction starts known post-edit verification or falls back to branch_model', () => {
    const data = {chance_context: {kind: 'model_inference'}};
    const config = {candidate_model: 'fixture/fast'};
    const prediction = {predict: 'branch_model', model: 'fixture/fast', thinking: 'off'};
    const edited = {status: 'succeeded', tool_name: 'write', arguments: {path: 'src/a.py'}};
    const verified = {status: 'failed', tool_name: 'bash', arguments: {command: 'pytest tests'}};
    assert.deepEqual(call('predict', {...data, history: [edited]}, config), prediction);
    assert.deepEqual(call('predict', {...data, history: [verified, edited]}, config), {
        generation: 'batch', proposals: [{payload: {kind: 'predicted_tool_calls', tool_calls: [
            {name: 'bash', arguments: {command: 'pytest tests'}},
        ]}, provenance: {kind: 'active_post_edit_test'}}],
    });
    assert.deepEqual(call('predict', {...data, history: [verified, edited]}, {...config, active_post_edit_test: false}), prediction);
    assert.deepEqual(call('predict', {...data, history: [edited, verified]}, config), prediction);
});

const read = path => ({name: 'read', arguments: {path}});
const tool = (name, args) => ({name, arguments: args});
const match = (calls, outcome, context = {}, config = {}) => call('match_proposal', {
    proposal: {payload: {tool_calls: calls}}, outcome,
    match_context: {candidate: {speculation_kind: 'model_inference', ...context.candidate},
        authority: context.authority, candidate_results: context.results},
}, config);
test('model match promotes only verification batches that cover Authority', () => {
    const verify = tool('bash', {command: 'pytest tests/a.py'});
    const wrapped = tool('bash', {command: 'TMPDIR=/tmp python3 -m pytest -q tests/a.py'});
    const extra = tool('bash', {command: 'git diff --check'});
    const outcome = {tool_calls: [verify]};
    assert.deepEqual(match([verify], null), {verdict: 'indeterminate'});
    assert.equal(match([read('a')], {tool_calls: [read('a')]}).verdict, 'incompatible');
    const hit = match([wrapped], outcome);
    assert.equal(hit.verdict, 'compatible');
    assert.deepEqual(call('dispose', {resolution: 'accepted', context: {authority_outcome: outcome, match: hit}}),
        {actions: [{intent: 'Promote'}]});
    assert.equal(match([wrapped], {tool_calls: [verify, extra]}).verdict, 'incompatible');
    const executed = {tool_name: 'bash', arguments: wrapped.arguments, status: 'failed', action_id: 'verify',
        result: {content: '1 failed', is_error: true}};
    assert.equal(match([read('a')], outcome, {results: [executed]}).verdict, 'compatible');
    const miss = match([wrapped], {tool_calls: [verify, extra]}, {results: [executed]});
    assert.equal(miss.verdict, 'incompatible');
    assert.deepEqual(miss.details.safe_results.map(result => result.files), [[]]);
    assert.match(miss.details.safe_results[0].notification, /1 failed/);
    const mutated = {tool_name: 'edit', arguments: {path: 'a'}, status: 'succeeded'};
    assert.equal(match([wrapped], outcome, {results: [executed, mutated]}).verdict, 'incompatible');
});

test('multi-step weak-model matches salvage only novel results', () => {
    const result = {action_id: 'r1', tool_name: 'read', arguments: {path: 'a'}, status: 'succeeded',
        result: {content: [{type: 'text', text: 'authority-visible facts'}]}};
    const outcome = {tool_calls: [read('a')]};
    const context = {
        authority: {model: 'authority-model'},
        candidate: {usage: [{model: 'deepseek-flash'}, {model: 'deepseek-flash'}]},
        results: [result],
    };
    const novel = {...result, action_id: 'r2', arguments: {path: 'b'}};
    const proposal = match([read('a')], outcome, {...context, results: [result, novel]});
    assert.equal(proposal.verdict, 'incompatible');
    assert.equal(proposal.details.match_kind, 'partial_info');
    assert.deepEqual(proposal.details.safe_results, [{id: 'r2', files: [],
        notification: '[Speculative workspace output collected on current files]: read b\nauthority-visible facts'}]);
    const redundant = match([read('a')], outcome, context);
    assert.deepEqual(call('dispose', {resolution: 'rejected', context: {
        authority_outcome: outcome, match: redundant,
    }}), {actions: [{intent: 'Discard'}]});
    assert.deepEqual(call('dispose', {resolution: 'rejected', context: {
        authority_outcome: outcome, match: proposal,
    }}), {actions: [
        {intent: 'Salvage', results: proposal.details.safe_results},
        {intent: 'Discard'},
    ]});
});

test('extent permits isolated verification and stops observable degradation', () => {
    const extent = (next_action, completed_tools = [], steps_completed = 2, speculation_kind = 'model_inference') =>
        call('extent', {process_state: {steps_completed, semantic_facts: {speculation_kind, next_action},
            result_facts: {completed_tools}}});
    const action = (name, args) => ({kind: 'tool_execution', ...tool(name, args)});
    const verification = action('bash', bash.arguments);
    const mutation = action('edit', {path: 'a'});
    const prior = {...read('a'), status: 'succeeded'};
    assert.equal(extent(verification, [prior]), 'continue');
    assert.equal(extent(mutation), 'stop');
    assert.equal(extent(action('read', {path: './a'}), [prior]), 'stop');
    assert.equal(extent(action('read', {path: 'b'}), [prior]), 'continue');
    assert.equal(extent(verification, [], 12), 'stop');
    assert.equal(extent(verification, [], 2, 'tool_execution'), 'stop');
});

test('read-only Salvage requires novelty and an Authority anchor', () => {
    const good = {action_id: 'r', tool_name: 'read', status: 'succeeded', arguments: {path: 'b'},
        result: {content: 'useful facts'}};
    const overlap = {...good, action_id: 'a', arguments: {path: 'a'}};
    const failed = {...good, action_id: 'failed', status: 'failed'};
    const context = {results: [good, overlap, failed, {result: {content: 'assistant prose'}}]};
    const outcome = {tool_calls: [read('a')]};
    assert.deepEqual(match([read('a')], outcome, context).details.safe_results.map(r => r.id), ['r']);
    assert.deepEqual(match([], {tool_calls: [read('c')]}, context), {verdict: 'incompatible', score: 0});
    assert.deepEqual(match([], null, context, {prior_readonly_exploration: true})
        .details.safe_results.map(r => r.id), ['r', 'a']);
});

test('cross-tool search overlap can anchor Salvage but cannot Promote', () => {
    const search = tool('bash', {command: 'grep -rn "delete_cookie" src'});
    const repeated = read('src/response.py');
    const novel = read('tests/test_cookie.py');
    const outcome = {tool_calls: [tool('bash', {command: "rg -n 'delete_cookie\\(' ."}), repeated]};
    const results = [search, repeated, novel].map((fn, i) => ({action_id: `r${i}`,
        tool_name: fn.name, arguments: fn.arguments, status: 'succeeded', result: {content: 'output'}}));
    const result = match([search, repeated, novel], outcome, {results}, {min_partial_coverage: 0.75});
    assert.equal(result.verdict, 'incompatible');
    assert.equal(result.details.coverage, 1);
    assert.deepEqual(result.details.safe_results.map(r => r.id), ['r0', 'r2']);
    assert.deepEqual(call('dispose', {resolution: 'rejected', context: {authority_outcome: outcome, match: result}})
        .actions.map(a => a.intent), ['Salvage', 'Discard']);
});

test('read-only salvage keeps the latest result per target within both budgets', () => {
    const entry = (id, path, content) => ({action_id: id, tool_name: 'read', arguments: {path},
        status: 'succeeded', result: {content}});
    const latest = entry('latest', 'a', [{type: 'text', text: 'latest output'},
        {type: 'thinking', thinking: 'private'}]);
    latest.arguments = {path: 'a', offset: 140, limit: 100};
    const salvage = results => match([], null, {results}, {prior_readonly_exploration: true}).details.safe_results;
    const duplicate = salvage([{...entry('old', 'a', 'old output'), arguments: latest.arguments}, latest]);
    const header = '[Speculative workspace output collected on current files]: ';
    assert.deepEqual(duplicate, [{id: 'latest', files: [], notification: `${header}read a (offset=140, limit=100)\nlatest output`}]);

    const many = size => Array.from({length: 10}, (_, i) => entry(`r${i}`, `p${i}`, 'x'.repeat(size)));
    assert.equal(salvage(many(10)).length, 8);
    const long = salvage(many(5_000));
    assert.deepEqual(long.map(result => result.notification.length), [4_500, 4_500, 4_500, 500]);
    assert.ok(long.every(result => result.notification.endsWith('\n[truncated]')));
    const exact = 'x'.repeat(4_500 - `${header}read a\n`.length);
    assert.equal(salvage([entry('exact', 'a', exact)])[0].notification, `${header}read a\n${exact}`);
});

test('verification predictions adapt to test edits, source edits, and passing verification', () => {
    const entry = (tool_name, arguments_, status = 'succeeded', is_error = false) =>
        ({tool_name, arguments: arguments_, status, is_error});
    const source = entry('edit', {path: 'src/sample.py'});
    const tests = entry('write', {path: 'tests/test_sample.py'});
    const failed = entry('bash', bash.arguments, 'failed', true);
    const passed = entry('bash', bash.arguments);
    const predict = (history, config) => call('predict', {chance_context: bash, history}, config)
        .proposals.map(p => p.provenance.kind);
    const pass = 'verification_pass', fail = 'expected_test_failure';
    assert.deepEqual(predict([]), [pass]);
    assert.deepEqual(predict([tests]), [fail]);
    assert.deepEqual(predict([source]), [pass, fail]);
    assert.deepEqual(predict([tests, source, failed]), [pass, fail]);
    assert.deepEqual(predict([source, {...passed, is_error: true}]), [pass, fail]);
    assert.deepEqual(predict([source], {adaptive_dual_tool_speculation: false}), [pass]);
    assert.deepEqual(predict([source, passed]), [pass]);

    const branches = (command, history) => call('predict', {chance_context: {...bash, arguments: {command}}, history})
        .proposals.map(p => p.provenance.kind);
    assert.deepEqual(branches('pytest tests', [source, entry('bash', {command: 'pytest tests -k smoke'})]), [pass, fail]);
    const shellEdit = entry('bash', {command: "python - <<'PY'\nfrom pathlib import Path\nPath('src/sample.py').write_text('updated')\nPY"});
    assert.deepEqual(branches('pytest tests', [shellEdit]), [pass, fail]);
    assert.deepEqual(branches('pytest tests', [shellEdit, passed]), [pass, fail]);
    assert.deepEqual(branches('pytest tests', [shellEdit, entry('bash', {command: 'python3 -m pytest tests'})]), [pass]);
    assert.deepEqual(branches('pytest tests', [{...shellEdit, status: 'failed'}]), [pass]);
    assert.deepEqual(branches('pytest tests', [entry('bash', {command: 'git status'})]), [pass]);
    for (const command of ['pytest -k "first or second" tests',
        'cd src && TMPDIR="$PWD/tmp" python -m pytest --basetemp "$PWD/tmp" tests/test_sample.py::TestClass::test_one']) {
        assert.deepEqual(branches(command, [source]), [pass]);
        assert.deepEqual(branches(command, [failed, source]), [pass]);
        assert.deepEqual(branches(command, [{...passed, result: {is_error: true}}, source]), [pass]);
        assert.deepEqual(branches(command, [passed, source]), [pass]);
    }
    for (const command of ['pytest tests/test_sample.py', 'pytest tests/test_sample.py::test_one tests/other',
        'pytest -- tests/test_sample.py::test_one tests']) {
        assert.deepEqual(branches(command, [source]), [pass, fail]);
    }
});

test('Yield with no completed result keeps an in-flight verification alive', () => {
    const candidate = {active_tools: [{tool_name: 'bash', arguments: {command: 'pytest 2>&1 | tail -15'}}]};
    const data = {resolution: 'rejected', context: {candidate, authority_outcome: {}}};
    assert.deepEqual(call('dispose', data), {actions: [{intent: 'Yield', results: []}]});
    assert.deepEqual(call('dispose', data, {yield_in_flight_verification: false}), {actions: [{intent: 'Discard'}]});
    assert.deepEqual(call('rebalance', {active_view: [{branch_id: 'candidate', yield_only: true, age_ms: 100,
        tool_counts: {failed: 0}}], paused_view: [], scheduling_context: {waiting_chance: true}},
    {max_active_candidates: 1}), [{branch_id: 'candidate', action: 'preempt'}]);
});

test('tool speculation is independently toggleable and recognizes standalone verification', () => {
    assert.equal(call('opportunity_pass', {op: bash}, {fast_model_speculation: false}), true);
    assert.equal(call('opportunity_pass', {op: bash}, {fast_model_speculation: true, tool_speculation: false}), false);
    assert.equal(call('opportunity_pass', {op: {...bash, result_pending: false}}), false);
    const feasible = command => call('feasibility_pass', {op: {...bash, arguments: {command}}, state: {assumption_depth: 0}});
    for (const command of ['cargo test', 'python3 -m pytest', './runtests.py', 'npm run build',
        'cd /tmp/work && mkdir -p "$PWD/.tmp" && TMPDIR="$PWD/.tmp" PYTHONPATH="${PWD}" PYTHONDONTWRITEBYTECODE=1 python -m pytest --basetemp="$PWD/.pytest-tmp" tests/test_*.py',
        'mkdir -p .tmp\nTMPDIR="$PWD/.tmp" python tests/runtests.py testsuite', 'LD_PRELOAD=lib.so pytest',
        'pytest | head', 'cd /tmp/work/tests && python runtests.py testsuite 2>&1 | tail -15']) {
        assert.equal(feasible(command), true, command);
    }
    for (const command of ['pytest && curl example.com', 'pytest > out', 'pytest >> out', 'git push',
        'pytest; rm -r tests', 'jest --updateSnapshot', 'pytest | head out', 'pytest "$(pwd)"', 'pytest "`pwd`"',
        'pytest --basetemp="$OTHER/tmp"', 'pytest --basetemp="$PWD_OTHER/tmp"', 'pytest "$?"',
        'rm -rf .tmp && pytest', 'apply_patch && pytest', "python - <<'PY'\nprint(1)\nPY\npytest",
        'git push && pytest', 'curl example.com && pytest', 'mkdir .tmp && pytest', 'mkdir -p .tmp extra && pytest',
        'cd /tmp || pytest', 'cd /tmp &&', 'pytest\npytest']) {
        assert.equal(feasible(command), false, command);
    }
    assert.equal(call('launch', {chance_context: bash}), true);
    for (const [tool, args, mode] of [['bash', {command: 'git push'}, 'block'], ['bash', {command: 'find /'}, 'deny'],
        ['read', {path: '../file'}, 'deny'], ['bash', {command: 'cargo test'}, 'allow']]) {
        assert.equal(call('effect', {effect_descriptor: {tool, arguments: args}}).mode, mode);
    }
});

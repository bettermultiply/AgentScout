import assert from 'node:assert/strict';
import { once } from 'node:events';
import fs from 'node:fs';
import http from 'node:http';
import { tmpdir } from 'node:os';
import { join, relative, resolve } from 'node:path';
import { test } from 'node:test';
import { runPath, spawnControl } from './control-plane-fixture.mjs';
import { createInterposer } from '../instrumentation/core/index.ts';

test('extent stops a candidate, survives restore, and clears on promotion', async () => {
    const inputs = [];
    let suspended = Promise.withResolvers();
    const options = {
        taskId: 'extent-fixture',
        report: async event => { if (event.reason === 'extent_reached') suspended.resolve(); },
        evaluate: async (_artifact, decision, data) => {
            assert.equal(decision, 'extent');
            inputs.push(data.process_state);
            return inputs.length === 1 ? 'continue' : 'stop';
        },
        now: () => 1000,
    };
    const access = createInterposer(options);
    access.setPolicy('authority', { digest: 'fixture', decisions: {}, decision_inputs: { extent: [
        'process_state.hook_count', 'process_state.resource_counters', 'process_state.result_facts',
    ] } });
    await access.control({ command: 'start_candidate', branch_id: 'candidate', parent_branch_id: 'authority' });
    const point = { branch: 'candidate', actionKind: 'model_inference', name: 'model', op: {}, admit: false };
    await access.beforeAction({ ...point, actionId: 'one' });
    access.afterAction({ ...point, actionId: 'one', status: 'succeeded', isError: false });
    const stopped = access.beforeAction({ ...point, actionId: 'two' });
    await suspended.promise;
    assert.deepEqual(inputs.map(state => [state.steps_completed, state.hook_count, state.resource_counters]), [
        [0, 1, { model_inferences: 0, tool_executions: 0 }],
        [1, 2, { model_inferences: 1, tool_executions: 0 }],
    ]);
    assert.equal(inputs[1].result_facts.last_action.name, 'model');
    const restored = createInterposer(options);
    restored.restore(JSON.parse(JSON.stringify(access.snapshot())));
    const reconnected = restored.beforeAction({ ...point, actionId: 'two', admitted: true });
    await restored.control({ command: 'promote', branch_id: 'candidate' });
    assert.deepEqual(await reconnected, { kind: 'allow' });
    await restored.beforeAction({ ...point, actionId: 'three' });
    assert.equal(inputs.length, 2);
    await access.control({ command: 'cleanup', branch_id: 'candidate' });
    await stopped;
});

const bindings = {
    admit: { opportunity_pass: { function: 'opportunity' }, feasibility_pass: { function: 'feasibility' } },
    select: {}, rebalance: {}, extent: { inputs: ['process_state.semantic_facts'] },
    predict: { inputs: ['history'] }, launch: {}, effect: {},
    match: { function: 'match_result', inputs: ['outcome'] }, dispose: {},
};
for (const [name, binding] of Object.entries(bindings)) {
    if (name === 'admit') {
        for (const entry of Object.values(binding)) entry.file = 'decisions.py';
    } else Object.assign(binding, { file: 'decisions.py', function: binding.function ?? name });
}
const policy = `
def complete(capabilities, prompt):
    return capabilities.model_complete({'provider': 'fixture', 'model': 'policy', 'prompt': prompt, 'max_tokens': 16})['text']

def opportunity(data, *, policy_config, capabilities):
    op = data['op']
    return (op['produces_result'] and op['result_pending'] and op['has_predictor']
            and op['authority'] and op['name'] == 'bash')

def feasibility(data, *, policy_config, capabilities):
    state = data['state']
    return (not state['is_candidate'] and state['assumption_depth'] == 0
            and state['tool_counts'].get('read', {}).get('succeeded') == 1
            and any(a['kind'] == 'tool_execution' and a['name'] == 'read' for a in state['recent_actions']))

def select(data, *, policy_config, capabilities):
    context = data['scheduling_context']
    assert context['pending_chance_count'] >= len(data['frontier_view'])
    assert context['active_candidate_count'] == len(context['active_branches'])
    assert context['blocked_candidate_count'] <= context['active_candidate_count']
    for branch in context['active_branches']:
        assert set(branch) == {'branch_id', 'chance_id', 'assumption_depth', 'status'}
    return [c['chance_id'] for c in data['frontier_view']]

def rebalance(data, *, policy_config, capabilities):
    return []

def extent(data, *, policy_config, capabilities):
    state = data['process_state']
    assert set(state) == {'steps_completed', 'phase', 'semantic_facts'}
    facts = state['semantic_facts']
    assert facts['next_action_kind'] == state['phase']
    assert complete(capabilities, 'extent:' + str(state['steps_completed']) + ':' + facts['next_action_name']) == 'ok'
    return 'continue'

def predict(data, *, policy_config, capabilities):
    assert 0 < len(data['history']) <= 16
    assert any(item.get('role') == 'user' for item in data['history'])
    payload = complete(capabilities, 'predict:' + data['chance_context']['arguments']['command'])
    return {'proposals': [{'payload': payload}], 'generation': 'batch'}

def launch(data, *, policy_config, capabilities):
    authority = data['authority_state']
    assert set(authority) == {'branch_id', 'status', 'assumption_depth', 'turn_active'}
    assert authority['branch_id'] == 'authority' and authority['turn_active']
    for branch in data['candidate_view']:
        assert set(branch) == {'branch_id', 'chance_id', 'status', 'assumption_depth'}
    return True

def effect(data, *, policy_config, capabilities):
    return {'mode': 'allow'}

def match_result(data, *, policy_config, capabilities):
    outcome = data['outcome']
    assert outcome['tool_name'] == 'bash' and 'details' in outcome
    assert 'command' in outcome['arguments']
    actual = outcome['payload']
    # The second proposal is ready to accept after the candidate's next
    # inference sees its write result. Use reported progress as evidence.
    if actual == 'actual-2' and len(data['match_context']['candidate']['usage'] or []) < 3:
        return {'verdict': 'indeterminate'}
    assert complete(capabilities, 'match:' + actual) == 'ok'
    return {'verdict': 'compatible' if actual == 'actual-2' else 'incompatible'}

def dispose(data, *, policy_config, capabilities):
    for result in data['context']['candidate_results']:
        if result.get('tool_name') == 'read':
            assert result['arguments']['path'] == 'input.txt' and 'details' in result
    if data['resolution'] == 'accepted':
        assert any(result.get('tool_name') == 'write' for result in data['context']['candidate_results'])
        return {'actions': [{'intent': 'Promote', 'notification': 'Verified the candidate file write.'}]}
    if data['resolution'] == 'indeterminate':
        return {'actions': []}
    return {'actions': [{'intent': 'Discard'}]}
`;

async function freePort() {
    const server = http.createServer();
    server.listen(0, '127.0.0.1');
    await once(server, 'listening');
    const { port } = server.address();
    await new Promise(done => server.close(done));
    return port;
}

const tool = (id, name, args) => ({ tool_calls: [{ index: 0, id, type: 'function',
    function: { name, arguments: JSON.stringify(args) } }] });

for (const [failingTools, nativeModels] of [[false, false], [true, false], [false, true]]) test(nativeModels
    ? 'ordinary startup uses the Pi openai override for Agent and Policy requests'
    : failingTools ? 'failed bash results resolve against successful and failed predictions'
    : 'compiled Decision grants and facts reach real control plane, Pi and Filesystem Runtime', { timeout: 45000 }, async t => {
    const root = fs.mkdtempSync(join(tmpdir(), 'ascout-policy-decision-'));
    for (const dir of ['workspace', 'policy', 'pi']) fs.mkdirSync(join(root, dir));
    fs.writeFileSync(join(root, 'workspace/input.txt'), 'fixture input');
    const provider = nativeModels ? 'openai' : 'fixture';
    const config = {
        version: 1, language: 'python', files: ['decisions.py'], fidelity: 'Best-effort', decisions: bindings,
        policy_model_grant: { decisions: ['predict', 'match', 'extent'], models: [`${provider}/policy`], max_requests: 1, timeout_ms: 10000 },
    };
    const policyPath = join(root, 'policy/policy.yaml');
    fs.writeFileSync(policyPath, JSON.stringify(config));
    const source = failingTools ? policy
        .replace("    return {'proposals': [{'payload': payload}], 'generation': 'batch'}", `    step = payload[-1]
    payload = {'content': [{'type': 'text', 'text': 'actual-' + step + '\\n\\nCommand exited with code 1'}], 'isError': step == '2'}
    return {'proposals': [{'payload': payload}], 'generation': 'batch'}`)
        .replace("    actual = outcome['payload']", `    actual = outcome['content'][0]['text'].split('\\n', 1)[0]
    predicted = data['proposal']['payload']
    assert outcome['details']['exitCode'] == 1 and outcome['is_error']
    assert predicted['content'] == outcome['content']`)
        .replace("actual == 'actual-2' and len(data['match_context']['candidate']['usage'] or []) < 3", "len(data['match_context']['candidate']['usage'] or []) < (3 if actual == 'actual-2' else 1)")
        .replace("'compatible' if actual == 'actual-2' else 'incompatible'", "'compatible' if predicted['isError'] == outcome['is_error'] else 'incompatible'")
        : policy;
    fs.writeFileSync(join(root, 'policy/decisions.py'), source.replace("'provider': 'fixture'", `'provider': '${provider}'`));
    const prompts = [], agentRequests = [], firstPrediction = new Set();
    const fake = http.createServer(async (req, res) => {
        const body = JSON.parse(Buffer.concat(await Array.fromAsync(req)).toString());
        let delta;
        if (body.model === 'policy') {
            const prompt = body.messages.at(-1).content;
            prompts.push(prompt);
            delta = { content: prompt.startsWith('predict:') ? `guess-${prompt.match(/actual-([12])/)[1]}` : 'ok' };
        } else {
            const toolResults = body.messages.filter(m => m.role === 'tool');
            const guess = failingTools ? toolResults.findLast(r => /^actual-[12]\n\nCommand exited with code 1$/.test(r.content)
                && (!firstPrediction.has(r.tool_call_id) || toolResults.some(result => result.tool_call_id.startsWith('candidate-'))))
                : toolResults.find(r => /^guess-[12]$/.test(r.content));
            if (guess) firstPrediction.add(guess.tool_call_id);
            agentRequests.push({ candidate: Boolean(guess), toolCallIds: toolResults.map(r => r.tool_call_id) });
            if (guess) {
                if (toolResults.at(-1) === guess) delta = tool('candidate-read', 'read', { path: 'input.txt' });
                else if (toolResults.at(-1).tool_call_id === 'candidate-read' && guess.content.startsWith(failingTools ? 'actual-2' : 'guess-2')) {
                    delta = tool('candidate-write', 'write', { path: 'accepted.txt', content: 'candidate content' });
                } else delta = { content: 'candidate done' };
            } else if (!toolResults.length) delta = tool('initial-read', 'read', { path: 'input.txt' });
            else if (toolResults.length <= 2) {
                const step = toolResults.length;
                delta = tool(`authority-${step}`, 'bash', { command: `printf actual-${step}${failingTools ? '; exit 1' : ''}` });
            } else delta = { content: 'authority done' };
        }
        const chunk = (value, finish_reason = null) => ({ id: 'fixture', object: 'chat.completion.chunk', created: 0,
            model: body.model, choices: [{ index: 0, delta: value, finish_reason }] });
        res.writeHead(200, { 'content-type': 'text/event-stream' });
        res.write(`data: ${JSON.stringify(chunk(delta))}\n\n`);
        res.end(`data: ${JSON.stringify(chunk({}, delta.tool_calls ? 'tool_calls' : 'stop'))}\n\ndata: [DONE]\n\n`);
    });
    fake.listen(0, '127.0.0.1');
    await once(fake, 'listening');
    t.after(() => { fake.closeAllConnections(); fake.close(); });
    const models = join(nativeModels ? join(root, 'pi') : root, 'models.json');
    fs.writeFileSync(models, JSON.stringify({ providers: { [provider]: {
        baseUrl: `http://127.0.0.1:${fake.address().port}/v1`, api: 'openai-completions', apiKey: 'fake',
        models: ['agent', 'policy'].map(id => ({ id, reasoning: false, input: ['text'], contextWindow: 32768, maxTokens: 256 })),
    } } }));
    const [port, runtimePort] = await Promise.all([freePort(), freePort()]);
    const address = `127.0.0.1:${port}`;
    const runtime = join(root, 'runtime.json');
    fs.writeFileSync(runtime, JSON.stringify({ backend: 'filesystem', control_address: `127.0.0.1:${runtimePort}`,
        adapter_instance: 'decision-fixture', filesystem: { state_directory: join(root, 'fs'), isolated_paths: [join(root, 'workspace')], passthrough_rw: [root] } }));
    const piDir = join(root, 'pi');
    const agentDir = nativeModels ? relative(process.cwd(), piDir) : piDir;
    const child = spawnControl(resolve('target/debug/ascout-control-plane'), [root, resolve('../pi'), policyPath,
        '--web', address, '--model', `${provider}/agent`, ...(nativeModels ? [] : ['--models', models]), '--thinking', 'off'], {
        env: { ...process.env, PI_OFFLINE: '1', PI_CODING_AGENT_DIR: agentDir, ASCOUT_RUNTIME_CONFIG: runtime }, stdio: ['ignore', 'pipe', 'pipe'],
    });
    let logs = '';
    for (const stream of [child.stdout, child.stderr]) stream.on('data', b => { logs += b; });
    const exit = once(child, 'exit');
    const post = (action, extra = {}) => fetch(`http://${address}/api/request`, { method: 'POST',
        headers: { origin: `http://${address}`, 'content-type': 'application/json' }, body: JSON.stringify({ action, ...extra }) });
    t.after(async () => {
        if (child.exitCode !== null) return;
        const timer = setTimeout(() => child.kill('SIGKILL'), 3000);
        try { await post('shutdown'); } catch {}
        await exit;
        clearTimeout(timer);
    });
    const events = () => {
        const path = runPath(root, 'events.jsonl');
        return fs.existsSync(path) ? fs.readFileSync(path, 'utf8').trimEnd().split('\n').filter(Boolean).map(JSON.parse) : [];
    };
    async function waitFor(predicate, label) {
        for (const end = Date.now() + 20000; Date.now() < end;) {
            assert.equal(child.exitCode, null, `${root}\n${logs}`);
            const found = events().find(predicate);
            if (found) return found;
            await new Promise(done => setTimeout(done, 20));
        }
        assert.fail(`Timed out waiting for ${label}: ${root}\n${logs}\n${JSON.stringify(prompts)}`);
    }
    const assertNoDuplicateProvider = () => {
        const agentLog = runPath(root, 'agent.log');
        const duplicate = /Additional provider already exists/;
        assert.ok(!duplicate.test(logs) && !(fs.existsSync(agentLog) && duplicate.test(fs.readFileSync(agentLog, 'utf8'))),
            'Pi reported Additional provider already exists');
    };
    try {
        await waitFor(e => e.message === 'branch_ready', 'Pi startup');
    } finally {
        assertNoDuplicateProvider();
    }
    if (nativeModels) assert.equal(fs.existsSync(runPath(root, 'models.json')), false);
    assert.equal((await post('prompt', { prompt: 'Read a file and run two commands.' })).status, 202);
    const done = await waitFor(e => e.message === 'task_completed', 'task completion');
    assert.equal(done.status, 'succeeded', `${root}\n${logs}`);
    const rows = events();
    assert.equal(rows.filter(e => e.message === 'speculative_admission_request' && e.branch_id === 'authority').length, 2);
    assert.ok(!rows.some(e => e.message === 'branch_suspended' && e.branch_id === 'authority' && e.reason === 'extent_reached'));
    assert.ok(!rows.some(e => e.message === 'policy_error' || e.policy_error), `${root}\n${JSON.stringify(prompts)}`);
    for (const step of [1, 2]) {
        assert.ok(prompts.includes(`predict:printf actual-${step}${failingTools ? '; exit 1' : ''}`), JSON.stringify(prompts));
        assert.ok(prompts.includes(`match:actual-${step}`), JSON.stringify(prompts));
    }
    for (const prompt of ['extent:0:model', 'extent:1:read', 'extent:2:model']) assert.ok(prompts.includes(prompt), JSON.stringify(prompts));
    if (failingTools) {
        const toolResults = rows.filter(e => e.message === 'action_finished' && /^authority-[12]$/.test(e.action_id));
        assert.deepEqual(toolResults.map(e => [e.status, e.result.details.exitCode, e.result.is_error]), [['failed', 1, true], ['failed', 1, true]]);
        assert.ok(rows.some(e => e.verdict === 'incompatible' && e.disposition === 'Discard'));
    }
    const winner = rows.find(e => e.verdict === 'compatible' && e.disposition === 'Promote');
    assert.ok(winner, `${root}\n${JSON.stringify(prompts)}`);
    assert.equal(winner.notification, 'Verified the candidate file write.');
    assert.equal(done.branch_id, winner.branch_id);
    const write = rows.findIndex(e => e.message === 'action_finished' && e.branch_id === winner.branch_id
        && e.action_id === 'candidate-write' && e.status === 'succeeded');
    assert.ok(write >= 0 && write < rows.indexOf(winner));
    const promoted = rows.find(e => e.message === 'command_result' && e.branch_id === winner.branch_id
        && e.operation === 'promote' && e.status === 'applied');
    assert.ok(promoted);
    const session = fs.readFileSync(runPath(root, 'session.jsonl'), 'utf8').trim().split('\n').map(JSON.parse);
    const notices = session.filter(e => e.customType === 'notification' && e.details?.command_id === promoted.command_id);
    assert.deepEqual(notices.map(e => e.content), [winner.notification]);
    assert.equal(rows.filter(e => e.message === 'action_started' && e.branch_id === 'authority'
        && e.action_kind === 'model_inference').length, 3);
    assert.equal(agentRequests.filter(request => !request.candidate).length, 3, JSON.stringify(agentRequests));
    assert.equal(fs.readFileSync(runPath(root, 'workspace/accepted.txt'), 'utf8'), 'candidate content');
    assert.equal((await post('shutdown')).status, 202);
    assert.equal((await exit)[0], 0, logs);
    fs.rmSync(root, { recursive: true, force: true });
});

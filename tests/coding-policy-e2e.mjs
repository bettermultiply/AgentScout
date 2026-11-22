import assert from 'node:assert/strict';
import { once } from 'node:events';
import fs from 'node:fs';
import http from 'node:http';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { test } from 'node:test';
import { setTimeout as delay } from 'node:timers/promises';
import { runPath, spawnControl } from './control-plane-fixture.mjs';

const coding = resolve('assets/policy/coding');
const binary = resolve('target/debug/ascout-control-plane');

const tool = (id, name, args) => ({tool_calls: [{index: 0, id, type: 'function',
    function: {name, arguments: JSON.stringify(args)}}]});

async function listen(server) {
    server.listen(0, '127.0.0.1');
    await once(server, 'listening');
    return server.address().port;
}

async function freePort() {
    const server = http.createServer();
    const port = await listen(server);
    await new Promise(done => server.close(done));
    return port;
}

async function fixture(t, respond, prepare, {checkUiNotice = false} = {}) {
    const root = fs.mkdtempSync(join(tmpdir(), 'ascout-coding-'));
    for (const dir of ['workspace', 'pi']) fs.mkdirSync(join(root, dir));
    const write = (name, content, executable = false) => fs.writeFileSync(join(root, 'workspace', name), content,
        {mode: executable ? 0o755 : 0o644});
    const policy = prepare(root, write);
    const fake = http.createServer(async (req, res) => {
        try {
            const body = JSON.parse(Buffer.concat(await Array.fromAsync(req)).toString());
            assert.equal(body.model, 'agent');
            const delta = respond(body.messages);
            const chunk = (value, finish_reason) => ({id: 'fixture', object: 'chat.completion.chunk',
                created: 0, model: body.model, choices: [{index: 0, delta: value, finish_reason}]});
            res.writeHead(200, {'content-type': 'text/event-stream'});
            res.end(`data: ${JSON.stringify(chunk(delta, null))}\n\ndata: ${JSON.stringify(chunk({}, delta.tool_calls ? 'tool_calls' : 'stop'))}\n\ndata: [DONE]\n\n`);
        } catch (error) {
            res.writeHead(500);
            res.end(String(error));
        }
    });
    const [modelPort, port, runtimePort] = await Promise.all([listen(fake), freePort(), freePort()]);
    const address = `127.0.0.1:${port}`;
    const models = join(root, 'models.json'), runtime = join(root, 'runtime.json');
    fs.writeFileSync(models, JSON.stringify({providers: {fixture: {
        baseUrl: `http://127.0.0.1:${modelPort}/v1`, api: 'openai-completions', apiKey: 'fake',
        models: [{id: 'agent', reasoning: false, input: ['text'], contextWindow: 32768, maxTokens: 256}],
    }}}));
    fs.writeFileSync(runtime, JSON.stringify({backend: 'filesystem', control_address: `127.0.0.1:${runtimePort}`,
        adapter_instance: 'coding-fixture', filesystem: {state_directory: join(root, 'fs'),
            isolated_paths: [join(root, 'workspace')], passthrough_rw: [root]}}));
    const child = spawnControl(binary, [root, resolve('../pi'), policy,
        '--web', address, '--model', 'fixture/agent', '--models', models, '--thinking', 'off'], {
        env: {...process.env, PI_CODING_AGENT_DIR: join(root, 'pi'), ASCOUT_RUNTIME_CONFIG: runtime},
        stdio: ['ignore', 'pipe', 'pipe'],
    });
    let logs = '', stderr = '';
    child.stdout.on('data', data => { logs += data; });
    child.stderr.on('data', data => { logs += data; stderr += data; });
    const exit = once(child, 'exit');
    const post = (action, extra = {}) => fetch(`http://${address}/api/request`, {method: 'POST',
        headers: {origin: `http://${address}`, 'content-type': 'application/json'}, body: JSON.stringify({action, ...extra})});
    const rows = (file = 'events.jsonl') => {
        const path = runPath(root, file);
        if (!fs.existsSync(path)) return [];
        return fs.readFileSync(path, 'utf8').split('\n').slice(0, -1).filter(Boolean).map(JSON.parse);
    };
    const wait = async (predicate, label) => {
        for (const deadline = Date.now() + 22000; Date.now() < deadline && child.exitCode === null;) {
            const found = rows().find(predicate);
            if (found) return found;
            await delay(25);
        }
        assert.fail(`${label}: ${root}\n${logs}\n${JSON.stringify(rows().slice(-15))}`);
    };
    t.after(async () => {
        fake.closeAllConnections();
        fake.close();
        if (child.exitCode === null) {
            const timeout = setTimeout(() => child.kill('SIGKILL'), 3000);
            try { await post('shutdown'); } catch {}
            await exit;
            clearTimeout(timeout);
        }
        if (t.passed) fs.rmSync(root, {recursive: true, force: true});
    });
    await wait(event => event.message === 'branch_ready', 'Pi startup');
    assert.equal((await post('prompt', {prompt: 'Verify the software and report the outcome.'})).status, 202);
    const done = await wait(event => event.message === 'task_completed', 'task completion');
    if (checkUiNotice) {
        const urlNotices = stderr.split('\n').filter(line => line.includes(`http://${address}`));
        assert.deepEqual(urlNotices, [`AScout browser UI: http://${address}`]);
    }
    assert.equal(done.status, 'succeeded', `${root}\n${logs}`);
    return {done, rows, root, shutdown: async () => {
        assert.equal((await post('shutdown')).status, 202);
        assert.equal((await exit)[0], 0, logs);
    }};
}

const lastTool = messages => messages.findLast(message => message.role === 'tool');

test('coding Policy promotes a reproduced failure, discards bad work, then promotes a successful verification', {timeout: 45000}, async t => {
    const {done, rows, root, shutdown} = await fixture(t, messages => {
        const last = lastTool(messages);
        const edit = (id, path, oldText, newText) => tool(id, 'edit', {path, edits: [{oldText, newText}]});
        const bash = (id, command) => tool(id, 'bash', {command});
        const id = last?.tool_call_id?.split('|')[0];
        const predicted = last?.content?.includes('Predicted verification result:');
        switch (id) {
            case undefined: return tool('initial-read', 'read', {path: 'test_sample.py'});
            case 'initial-read': return edit('test-edit', 'test_sample.py', '# todo', 'assert value == 1');
            case 'test-edit': return bash('reproduce', './pytest');
            case 'reproduce':
                assert.ok(predicted);
                assert.match(last.content, /fails as expected/);
                return edit('source-fix', 'sample.py', 'value = 0', 'value = 1');
            case 'source-fix': return bash('broken-check', './pytest --broken');
            case 'broken-check': return predicted
                ? tool('bad-write', 'write', {path: 'unwanted.txt', content: 'discard me'})
                : edit('second-fix', 'sample.py', 'value = 1', 'value = 2');
            case 'bad-write': return {content: 'incorrect candidate'};
            case 'second-fix': return bash('final-check', './runtests.py');
            case 'final-check':
                assert.ok(predicted);
                return tool('final-write', 'write', {path: 'verified.txt', content: 'verified'});
            case 'final-write': return {content: 'Verified and finished.'};
            default: assert.fail(`unexpected tool result ${id}`);
        }
    }, (_, write) => {
        write('sample.py', 'value = 0\n');
        write('test_sample.py', '# todo\n');
        write('pytest', '#!/bin/bash\nsleep 1.2\nif [[ "$1" == --broken ]]; then echo SyntaxError: invalid syntax; else echo AssertionError: expected 1; fi\nexit 1\n', true);
        write('runtests.py', '#!/bin/bash\nsleep 1.2\nprintf "OK (skipped=2, expected failures=1)\\n"\n', true);
        return join(coding, 'policy.yaml');
    }, {checkUiNotice: true});
    const events = rows();
    const decisions = events.filter(e => e.disposition);
    assert.deepEqual(decisions.map(e => e.disposition), ['Promote', 'Discard', 'Promote']);
    assert.equal(done.branch_id, decisions.at(-1).branch_id);
    assert.ok(!events.some(e => e.message === 'policy_error' || e.policy_error));
    assert.equal(events.filter(e => e.message === 'speculative_admission_request').length, 3);
    for (const action of ['initial-read', 'test-edit']) {
        assert.ok(events.some(e => e.message === 'action_finished' && e.action_id.startsWith(action) && e.status === 'succeeded'));
    }
    assert.ok(events.some(e => e.message === 'action_finished' && e.action_id.startsWith('bad-write') && e.branch_id === decisions[1].branch_id));
    assert.equal(fs.readFileSync(runPath(root, 'workspace/sample.py'), 'utf8'), 'value = 2\n');
    assert.equal(fs.readFileSync(runPath(root, 'workspace/verified.txt'), 'utf8'), 'verified');
    assert.ok(!fs.existsSync(runPath(root, 'workspace/unwanted.txt')));
    const session = rows('session.jsonl');
    assert.equal(session.filter(e => e.customType === 'notification' && e.content.includes('OK (skipped=2, expected failures=1)')).length, 1);
    assert.ok(session.some(e => e.message?.role === 'assistant' && e.message.content.some(part => part.text === 'Verified and finished.')));
    await shutdown();
});

for (const intent of ['Reuse', 'Salvage']) test(`${intent} transfers a file and preserves the authority session`, {timeout: 45000}, async t => {
    const {done, rows, root, shutdown} = await fixture(t, messages => {
        const last = lastTool(messages);
        if (!last) return tool('verify', 'bash', {command: './pytest'});
        if (last.content.includes('Predicted verification result:')) {
            return tool('candidate-write', 'write', {path: 'selected.txt', content: 'candidate result'});
        }
        return {content: 'done'};
    }, (root, write) => {
        write('pytest', `#!/bin/bash\nsleep 1.2\n${intent === 'Reuse' ? "echo '1 passed'" : "echo 'SyntaxError'; exit 1"}\n`, true);
        const dir = join(root, 'policy');
        fs.cpSync(coding, dir, {recursive: true});
        fs.appendFileSync(join(dir, 'decisions.py'), `

def match_partial(data, *, policy_config, capabilities):
    result = match_proposal(data, policy_config=policy_config, capabilities=capabilities)
    result['details'] = {'partial': True}
    return result

def dispose_partial(data, *, policy_config, capabilities):
    assert data['context']['match']['details']['partial'] is True
    assert data['branch']['proposal']['provenance']['phase'] == 'baseline_check'
    result = next(item for item in data['context']['candidate_results'] if item.get('tool_name') == 'write')
    action = {'intent': '${intent}', 'results': [{'id': result['action_id'], 'notification': 'Selected artifact transferred.'}]}
    return {'actions': [action${intent === 'Salvage' ? ", {'intent': 'Discard'}" : ''}]}
`);
        const config = fs.readFileSync(join(dir, 'policy.yaml'), 'utf8')
            .replace('function: match_proposal', 'function: match_partial').replace('function: dispose', 'function: dispose_partial');
        fs.writeFileSync(join(dir, 'policy.yaml'), config);
        return join(dir, 'policy.yaml');
    });
    assert.equal(done.branch_id, 'authority');
    const events = rows();
    const disposition = events.find(e => e.disposition === intent);
    assert.ok(disposition, JSON.stringify(events.filter(e => e.disposition || e.policy_error)));
    const settlements = events.filter(e => e.message === 'observation').map(e => e.observation)
        .filter(e => e.event === 'settlement' && e.branch_id === disposition.branch_id);
    assert.deepEqual(settlements.map(({disposition, hit}) => ({disposition, hit})), [{disposition: intent, hit: intent === 'Reuse'}]);
    assert.equal(fs.readFileSync(runPath(root, 'workspace/selected.txt'), 'utf8'), 'candidate result');
    assert.equal(rows('session.jsonl').filter(e => e.customType === 'notification' && e.content.includes('Selected artifact transferred.')).length, 1);
    await shutdown();
});

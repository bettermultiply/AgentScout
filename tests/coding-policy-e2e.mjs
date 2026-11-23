import assert from 'node:assert/strict';
import { once } from 'node:events';
import fs from 'node:fs';
import http from 'node:http';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { test } from 'node:test';
import { setTimeout as delay } from 'node:timers/promises';
import { execFileSync } from 'node:child_process';
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

async function fixture(t, respond, prepare, {checkUiNotice = false, prompt, modelIds = ['agent']} = {}) {
    const root = fs.mkdtempSync(join(tmpdir(), 'ascout-coding-'));
    for (const dir of ['workspace', 'pi']) fs.mkdirSync(join(root, dir));
    const write = (name, content, executable = false) => fs.writeFileSync(join(root, 'workspace', name), content,
        {mode: executable ? 0o755 : 0o644});
    const policy = prepare(root, write);
    const fake = http.createServer(async (req, res) => {
        try {
            const body = JSON.parse(Buffer.concat(await Array.fromAsync(req)).toString());
            assert.ok(modelIds.includes(body.model));
            const delta = await respond(body.messages, body.model, root);
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
        models: modelIds.map(id => ({id, reasoning: false, input: ['text'], contextWindow: 32768, maxTokens: 1024})),
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
    assert.equal((await post('prompt', {prompt: prompt ?? 'Verify the software and report the outcome.'})).status, 202);
    const done = await wait(event => event.message === 'task_completed', 'task completion');
    if (checkUiNotice) {
        const urlNotices = stderr.split('\n').filter(line => line.includes(`http://${address}`));
        assert.deepEqual(urlNotices, [`AScout browser UI: http://${address}`]);
    }
    assert.equal(done.status, 'succeeded', `${root}\n${logs}`);
    return {done, rows, root, followUp: async prompt => {
        const previous = rows().filter(event => event.message === 'task_completed').length;
        assert.equal((await post('prompt', {prompt})).status, 202);
        await wait(() => rows().filter(event => event.message === 'task_completed').length > previous, 'follow-up completion');
    }, shutdown: async () => {
        assert.equal((await post('shutdown')).status, 202);
        assert.equal((await exit)[0], 0, logs);
    }};
}

const lastTool = messages => messages.findLast(message => message.role === 'tool');

test('branch_model guidance is request-local across promotion', {timeout: 45000}, async t => {
    const prompt = 'Read the fixture.\n\nKeep this wording.\n';
    const requests = [];
    const followUpPrompt = 'Confirm the result.\n';
    const {done, rows, followUp, shutdown} = await fixture(t, (messages, model) => {
        requests.push({model, messages});
        return lastTool(messages) ? {content: 'Finished.'} : tool('same-read', 'read', {path:'value.txt'});
    }, (root, write) => {
        write('value.txt', 'fixture\n');
        fs.cpSync(coding, join(root, 'policy'), {recursive:true});
        const manifest = join(root, 'policy/policy.yaml');
        fs.writeFileSync(manifest, fs.readFileSync(manifest, 'utf8')
            .replaceAll('fast_model_speculation: false', 'fast_model_speculation: true')
            .replace('candidate_model: deepseek/deepseek-flash', 'candidate_model: fixture/candidate')
            .replace('candidate_thinking: high', 'candidate_thinking: off')
            .replace('max_model_speculation_step: null', 'max_model_speculation_step: 0')
            .replaceAll('max_candidate_model_steps: 1', 'max_candidate_model_steps: 6')
            .replace('max_steps: 4', 'max_steps: 16'));
        return manifest;
    }, {prompt, modelIds:['agent', 'candidate']});
    assert.ok(rows().some(row => row.operation === 'promote' && row.status === 'applied' && row.branch_id === done.branch_id));
    assert.ok(requests.some(({model}) => model === 'candidate'));
    assert.ok(requests.some(({model, messages}) => model === 'agent' && !lastTool(messages)));
    await followUp(followUpPrompt);
    const userText = message => typeof message.content === 'string' ? message.content
        : message.content.map(part => part.text).join('');
    assert.ok(requests.some(({model, messages}) => model === 'agent' && messages.some(message =>
        message.role === 'user' && userText(message) === followUpPrompt)), 'promoted authority continued');
    for (const {model, messages} of requests) {
        const users = messages.filter(message => message.role === 'user').map(userText);
        assert.deepEqual(users, users.includes(followUpPrompt) ? [prompt, followUpPrompt] : [prompt]);
        const system = messages.filter(message => ['system', 'developer'].includes(message.role)).map(message => message.content).join('\n');
        assert.equal(system.includes('Batch independent symbol, call-site, and test lookups'), model === 'candidate');
    }
    await shutdown();
});

test('semantic audit discards a rejected sibling and publishes the passing sibling without Authority inference', {timeout: 45000}, async t => {
    const prompt = 'Set the fixture value to winner, including the edge case.';
    const auditReplies = [];
    let authorityCalls = 0, candidates = 0;
    const {done, rows, root, shutdown} = await fixture(t, async (messages, model, root) => {
        if (model === 'agent') {
            authorityCalls++;
            return {content: 'Unexpected Authority inference'};
        }
        if (model === 'audit') {
            const text = messages.at(-1).content;
            assert.ok(text.includes(prompt));
            assert.match(text, /Passing test command:\n\.\/pytest\nTest output:\n1 passed/);
            assert.match(text, /Captured git diff:/);
            const reply = text.includes('+winner') ? 'PASS' : 'REJECT: missing edge case';
            auditReplies.push(reply);
            return {content: reply};
        }
        const last = lastTool(messages);
        const id = last?.tool_call_id?.split('|')[0];
        if (!id) {
            const number = ++candidates;
            if (number === 2) {
                const deadline = Date.now() + 10000;
                while (Date.now() < deadline) {
                    const events = runPath(root, 'events.jsonl');
                    if (fs.existsSync(events) && fs.readFileSync(events, 'utf8').includes('"disposition":"Discard"')) break;
                    await delay(25);
                }
            }
            return tool(`write-${number}`, 'write', {path: join(root, 'workspace/value.txt'),
                content: number === 1 ? 'missing\n' : 'winner\n'});
        }
        if (id.startsWith('write-')) return tool('diff', 'bash', {command: 'git diff'});
        if (id === 'diff') return tool('verify', 'bash', {command: './pytest'});
        assert.equal(id, 'verify');
        return {content: 'Candidate finished.'};
    }, (root, write) => {
        write('value.txt', 'original\n');
        write('pytest', '#!/bin/sh\necho "1 passed"\n', true);
        const cwd = join(root, 'workspace');
        execFileSync('git', ['init', '-q'], {cwd});
        execFileSync('git', ['add', '.'], {cwd});
        execFileSync('git', ['-c', 'user.name=Fixture', '-c', 'user.email=fixture@example.test', 'commit', '-qm', 'fixture'], {cwd});
        fs.cpSync(coding, join(root, 'policy'), {recursive: true});
        const manifest = join(root, 'policy/policy.yaml');
        fs.writeFileSync(manifest, fs.readFileSync(manifest, 'utf8')
            .replaceAll('openai/gpt-6.2-sol', 'fixture/audit')
            .replaceAll('fast_model_speculation: false', 'fast_model_speculation: true')
            .replaceAll('skip_authority_after_independent_verification: false', 'skip_authority_after_independent_verification: true')
            .replace('candidate_model: deepseek/deepseek-flash', 'candidate_model: fixture/candidate')
            .replace('candidate_count: 1', 'candidate_count: 2')
            .replace('candidate_thinking: high', 'candidate_thinking: off')
            .replace('max_model_speculation_step: null', 'max_model_speculation_step: 0')
            .replaceAll('max_candidate_model_steps: 1', 'max_candidate_model_steps: 6')
            .replace('max_steps: 4', 'max_steps: 16')
            .replace('semantic_audit: false', 'semantic_audit: true')
            .replace('partial_accept_salvage: false', 'partial_accept_salvage: true'));
        return manifest;
    }, {prompt, modelIds: ['agent', 'candidate', 'audit']});
    assert.equal(authorityCalls, 0);
    assert.equal(candidates, 2);
    assert.deepEqual(auditReplies, ['REJECT: missing edge case', 'PASS']);
    const events = rows();
    const branches = events.filter(e => e.message === 'candidate_created');
    assert.equal(branches.length, 2);
    assert.equal(branches[0].chance_id, branches[1].chance_id);
    const audits = events.filter(e => e.message === 'policy_audit');
    assert.deepEqual(audits.map(e => e.audit.finding), auditReplies);
    assert.deepEqual(events.filter(e => e.disposition).map(e => [e.branch_id, e.disposition]),
        [[audits[0].branch_id, 'Discard'], [audits[1].branch_id, 'Promote']]);
    assert.equal(done.branch_id, audits[1].branch_id);
    await shutdown();
    assert.equal(fs.readFileSync(join(root, 'workspace/value.txt'), 'utf8'), 'winner\n');
});

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

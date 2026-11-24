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
const binary = resolve('target/release/ascout-control-plane');

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

async function fixture(t, respond, prepare, {
    checkUiNotice = false, modelIds = ['agent'], reasoning = false, authorityModel = 'agent', responses = false, cliModel = true,
    recoveryBarrier,
} = {}) {
    const root = fs.mkdtempSync(join(tmpdir(), 'ascout-coding-'));
    for (const dir of ['workspace', 'pi']) fs.mkdirSync(join(root, dir));
    const write = (name, content, executable = false) => fs.writeFileSync(join(root, 'workspace', name), content,
        {mode: executable ? 0o755 : 0o644});
    const policy = prepare(root, write);
    const providerErrors = [], requests = [];
    const fake = http.createServer(async (req, res) => {
        try {
            const body = JSON.parse(Buffer.concat(await Array.fromAsync(req)).toString());
            assert.ok(modelIds.includes(body.model));
            if (responses) assert.equal(req.url, '/v1/responses');
            const messages = responses ? body.input.map(item => item.type === 'function_call_output'
                ? {role: 'tool', tool_call_id: item.call_id.split('_fc_')[0], content: item.output} : item) : body.messages;
            requests.push({model: body.model, toolId: lastTool(messages)?.tool_call_id?.split('|')[0],
                cacheKey: body.prompt_cache_key, sessionId: req.headers.session_id});
            const id = `resp_${requests.length}`;
            const delta = await respond(messages, body.model, root, responses ? body.reasoning?.effort : body.reasoning_effort,
                callback => res.once('close', () => { if (!res.writableEnded) callback(); }));
            if (res.destroyed) return;
            if (responses) {
                const items = delta.tool_calls?.map(({id, function: call}) => ({type: 'function_call',
                    id: `fc_${id}`, call_id: id, name: call.name, arguments: call.arguments, status: 'completed'})) ??
                    [{type: 'message', id: `msg_${id}`, role: 'assistant', status: 'completed',
                        content: [{type: 'output_text', text: delta.content, annotations: []}]}];
                const events = [{type: 'response.created', response: {id}}, ...items.flatMap((item, output_index) => [
                    {type: 'response.output_item.added', output_index, item},
                    {type: 'response.output_item.done', output_index, item},
                ]), {type: 'response.completed', response: {id, status: 'completed'}}];
                res.writeHead(200, {'content-type': 'text/event-stream'});
                res.end(events.map(event => `data: ${JSON.stringify(event)}\n\n`).join(''));
                return;
            }
            const chunk = (value, finish_reason) => ({id: 'fixture', object: 'chat.completion.chunk',
                created: 0, model: body.model, choices: [{index: 0, delta: value, finish_reason}]});
            res.writeHead(200, {'content-type': 'text/event-stream'});
            res.end(`data: ${JSON.stringify(chunk(delta, null))}\n\ndata: ${JSON.stringify(chunk({}, delta.tool_calls ? 'tool_calls' : 'stop'))}\n\ndata: [DONE]\n\n`);
        } catch (error) {
            providerErrors.push(error);
            res.writeHead(500);
            res.end(String(error));
        }
    });
    const [modelPort, port, runtimePort] = await Promise.all([listen(fake), freePort(), freePort()]);
    const address = `127.0.0.1:${port}`;
    const models = join(root, ...(cliModel ? [] : ['pi']), 'models.json'), runtime = join(root, 'runtime.json');
    if (!cliModel) fs.writeFileSync(join(root, 'pi', 'settings.json'), JSON.stringify({
        defaultProvider: 'fixture', defaultModel: authorityModel,
    }));
    fs.writeFileSync(models, JSON.stringify({providers: {fixture: {
        baseUrl: `http://127.0.0.1:${modelPort}/v1`, api: responses ? 'openai-responses' : 'openai-completions', apiKey: 'fake',
        compat: responses ? {sessionAffinityFormat: 'openai'} : {supportsReasoningEffort: true},
        models: modelIds.map(id => ({id, reasoning, input: ['text'], contextWindow: 32768, maxTokens: 1024,
            cost: {input: 0, output: 0, cacheRead: 0, cacheWrite: 0}})),
    }}}));
    fs.writeFileSync(runtime, JSON.stringify({backend: 'filesystem', control_address: `127.0.0.1:${runtimePort}`,
        adapter_instance: 'coding-fixture', filesystem: {state_directory: join(root, 'fs'),
            isolated_paths: [join(root, 'workspace')], passthrough_rw: [root]}}));
    const start = (resume = false) => spawnControl(binary, [root, resolve('../pi'), policy,
        '--web', address, ...(cliModel ? ['--model', `fixture/${authorityModel}`, '--models', models] : []),
        '--thinking', reasoning ? 'medium' : 'off', ...(resume ? ['--resume'] : [])], {
        env: {...process.env, PI_OFFLINE: '1', PI_CODING_AGENT_DIR: join(root, 'pi'), ASCOUT_RUNTIME_CONFIG: runtime},
        stdio: ['ignore', 'pipe', 'pipe'],
    });
    let logs = '', stderr = '';
    const collect = child => {
        child.stdout.on('data', data => { logs += data; });
        child.stderr.on('data', data => { logs += data; stderr += data; });
        return once(child, 'exit');
    };
    let child = start(), exit = collect(child);
    const post = (action, extra = {}) => fetch(`http://${address}/api/request`, {method: 'POST',
        headers: {origin: `http://${address}`, 'content-type': 'application/json'}, body: JSON.stringify({action, ...extra})});
    const rows = (file = 'events.jsonl') => {
        const path = runPath(root, file);
        if (!fs.existsSync(path)) return [];
        return fs.readFileSync(path, 'utf8').split('\n').slice(0, -1).filter(Boolean).map(JSON.parse);
    };
    const wait = async (predicate, label) => {
        for (const deadline = Date.now() + 22000; Date.now() < deadline && child.exitCode === null;) {
            if (providerErrors.length) throw providerErrors[0];
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
        if (t.passed) {
            try { fs.rmSync(root, {recursive: true, force: true}); }
            catch (error) {
                if (!recoveryBarrier || error.code !== 'EACCES') throw error;
                t.diagnostic(`Recovery fixture retained at ${root}`);
            }
        }
    });
    await wait(event => event.message === 'branch_ready', 'Pi startup');
    assert.equal((await post('prompt', {prompt: 'Verify the software and report the outcome.'})).status, 202);
    if (recoveryBarrier) {
        const release = await recoveryBarrier();
        await wait(event => event.disposition === 'Promote', 'candidate promotion before save');
        assert.equal((await post('pause')).status, 202);
        release();
        await wait(event => event.message === 'recovery_complete', 'candidate session saved');
        child.kill('SIGKILL');
        await exit;
        child = start(true);
        exit = collect(child);
        const loaded = await wait(event => event.message === 'recovery_loaded', 'candidate session loaded');
        assert.equal(loaded.error ?? null, null);
        assert.equal((await post('prompt', {prompt: 'Confirm the recovered verification.'})).status, 202);
        assert.equal((await post('continue')).status, 202);
    }
    const done = await wait(event => event.message === 'task_completed', 'task completion');
    if (checkUiNotice) {
        const urlNotices = stderr.split('\n').filter(line => line.includes(`http://${address}`));
        assert.deepEqual(urlNotices, [`AScout browser UI: http://${address}`]);
    }
    assert.deepEqual(providerErrors, []);
    assert.equal(done.status, 'succeeded', `${root}\n${logs}`);
    return {done, rows, root, requests, shutdown: async () => {
        assert.equal((await post('shutdown')).status, 202);
        assert.equal((await exit)[0], 0, logs);
    }};
}

const lastTool = messages => messages.findLast(message => message.role === 'tool');

function assertSharedCacheKey(root, requests, branchIds) {
    const rootFile = fs.realpathSync(runPath(root, 'session.jsonl'));
    const readSession = file => fs.readFileSync(file, 'utf8').trim().split('\n').map(JSON.parse);
    const rootId = readSession(rootFile)[0].id;
    for (const {model, toolId, cacheKey, sessionId} of requests) {
        assert.equal(cacheKey, rootId, `${model} after ${toolId} prompt_cache_key`);
        assert.equal(sessionId, rootId, `${model} after ${toolId} session_id`);
    }
    const sessionsDir = join(root, 'pi', 'sessions');
    const branchHeaders = new Map();
    for (const relative of fs.readdirSync(sessionsDir, {recursive: true})) {
        const file = join(sessionsDir, relative);
        if (file === rootFile || !file.endsWith('.jsonl')) continue;
        const [header, ...entries] = readSession(file);
        const branch = entries.findLast(e => e.customType === 'authority')?.data.branch_id;
        if (branch) branchHeaders.set(branch, {header, file});
    }
    const identities = new Set([rootId]);
    const files = new Set([rootFile]);
    for (const branch of branchIds) {
        assert.ok(branchHeaders.has(branch), `missing native session for ${branch}`);
        const {header, file} = branchHeaders.get(branch);
        assert.ok(header.id);
        identities.add(header.id);
        files.add(file);
    }
    assert.equal(identities.size, branchIds.length + 1);
    assert.equal(files.size, branchIds.length + 1);
}

function modelPolicy(root, write, {prior = false, active = true} = {}) {
    write('value.txt', 'one\ntwo\nthree\nfour\n');
    write('other.txt', 'different\n');
    const dir = join(root, 'policy');
    fs.cpSync(coding, dir, {recursive: true});
    const manifest = join(dir, 'policy.yaml');
    const config = fs.readFileSync(manifest, 'utf8')
        .replace('fast_model_speculation: false', 'fast_model_speculation: true')
        .replace('tool_speculation: true', 'tool_speculation: false')
        .replace('candidate_model: deepseek/deepseek-flash', 'candidate_model: fixture/candidate')
        .replace('active_post_edit_test: true', `active_post_edit_test: ${active}`)
        .replaceAll('prior_readonly_exploration: false', `prior_readonly_exploration: ${prior}`);
    fs.writeFileSync(manifest, config);
    return manifest;
}

async function waitFor(predicate, label) {
    for (const deadline = Date.now() + 10000; Date.now() < deadline;) {
        if (predicate()) return;
        await delay(25);
    }
    assert.fail(label);
}

test('coding Policy promotes observed verification outcomes and discards mismatches', {timeout: 45000}, async t => {
    const {done, rows, root, requests, shutdown} = await fixture(t, messages => {
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
                ? (last.content.includes('fails as expected')
                    ? edit('second-fix', 'sample.py', 'value = 1', 'value = 2')
                    : tool('bad-write', 'write', {path: 'unwanted.txt', content: 'discard me'}))
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
    }, {checkUiNotice: true, responses: true, cliModel: false});
    const events = rows();
    const decisions = events.filter(e => e.disposition);
    const settlements = events.filter(e => e.observation?.event === 'settlement').map(e => e.observation);
    // The broken invocation discards both predictions. The assertion failure and clean pass each promote one.
    assert.deepEqual(settlements.map(e => e.disposition).sort(), ['Discard', 'Discard', 'Discard', 'Promote', 'Promote']);
    assert.equal(done.branch_id, decisions.filter(e => e.disposition === 'Promote').at(-1).branch_id);
    assert.ok(!events.some(e => e.message === 'policy_error' || e.policy_error));
    assert.equal(events.filter(e => e.message === 'speculative_admission_request').length, 3);
    for (const action of ['initial-read', 'test-edit']) {
        assert.ok(events.some(e => e.message === 'action_finished' && e.action_id.startsWith(action) && e.status === 'succeeded'));
    }
    assert.equal(fs.readFileSync(runPath(root, 'workspace/sample.py'), 'utf8'), 'value = 2\n');
    assert.equal(fs.readFileSync(runPath(root, 'workspace/verified.txt'), 'utf8'), 'verified');
    assert.ok(!fs.existsSync(runPath(root, 'workspace/unwanted.txt')));
    assert.ok(requests.some(r => r.toolId === 'reproduce'));
    assertSharedCacheKey(root, requests, ['authority', ...decisions.filter(e => e.disposition === 'Promote').map(e => e.branch_id)]);
    const session = rows('session.jsonl');
    assert.equal(session.filter(e => e.customType === 'notification' && e.content.includes('OK (skipped=2, expected failures=1)')).length, 1);
    assert.ok(session.some(e => e.message?.role === 'assistant' && e.message.content.some(part => part.text === 'Verified and finished.')));
    await shutdown();
});

test('Salvage settles during a delayed second inference and delivers only novel results', {timeout: 45000}, async t => {
    let candidatePending = false;
    let authorityReturned;
    let notificationReceived;
    let candidateAborted;
    const {rows, shutdown} = await fixture(t, async (messages, model, _root, _thinking, onAbort) => {
        const id = lastTool(messages)?.tool_call_id?.split('|')[0];
        if (model === 'candidate' && !id) return {tool_calls: [
            ...tool('shared-read', 'read', {path: 'value.txt', limit: 2}).tool_calls,
            {...tool('novel-read', 'read', {path: 'other.txt'}).tool_calls[0], index: 1},
        ]};
        if (model === 'candidate' && id !== 'authority-read') {
            candidatePending = true;
            onAbort(() => { candidateAborted = performance.now(); });
            await delay(4_000, undefined, {ref: false});
            return {content: 'Delayed candidate response'};
        }
        if (model === 'agent' && !id) {
            await waitFor(() => candidatePending, 'second candidate inference should be pending');
            authorityReturned = performance.now();
            return tool('authority-read', 'read', {path: 'value.txt', limit: 4});
        }
        if (model === 'agent') {
            notificationReceived ??= performance.now();
            const input = JSON.stringify(messages);
            assert.match(input, /Accepted result: \[Speculative workspace output collected on current files\]: read other\.txt/);
            assert.doesNotMatch(input, /Accepted result: \[Speculative workspace output collected on current files\]: read value\.txt/);
        }
        return {content: 'Done'};
    }, (root, write) => modelPolicy(root, write, {}), {modelIds: ['agent', 'candidate']});
    const elapsed = notificationReceived - authorityReturned;
    t.diagnostic(`Salvage notification reached Authority in ${Math.round(elapsed)} ms`);
    assert.ok(elapsed < 1_500, `Salvage took ${elapsed} ms`);
    await waitFor(() => candidateAborted !== undefined, 'candidate HTTP request should abort');
    assert.ok(candidateAborted - authorityReturned < 1_500);
    assert.ok(rows().some(e => e.disposition === 'Salvage'));
    await shutdown();
});

test('Yield delivers workspace and verification results while the candidate keeps running', {timeout: 45000}, async t => {
    let verificationRequested = false;
    let authorityAdvanced = false;
    const workspace = '[Speculative workspace output collected on current files]: read other.txt';
    const verification = 'YIELD_VERIFICATION_COMPLETED';
    const {rows, done, shutdown} = await fixture(t, async (messages, model, root) => {
        const id = lastTool(messages)?.tool_call_id?.split('|')[0];
        if (model === 'candidate' && !id) return {tool_calls: [
            ...tool('yield-shared', 'read', {path: 'value.txt', limit: 2}).tool_calls,
            {...tool('yield-novel', 'read', {path: 'other.txt'}).tool_calls[0], index: 1},
        ]};
        if (model === 'candidate' && id !== 'yield-test') {
            verificationRequested = true;
            return tool('yield-test', 'bash', {command: './pytest'});
        }
        if (model === 'candidate') return {content: 'Candidate finished.'};
        if (!id) {
            await waitFor(() => verificationRequested && fs.readFileSync(runPath(root, 'events.jsonl'), 'utf8')
                .split('\n').some(line => line.includes('"message":"action_started"') && line.includes('yield-test')),
            'candidate verification did not start');
            return tool('yield-authority-read', 'read', {path: 'value.txt', limit: 4});
        }
        const input = JSON.stringify(messages);
        if (id === 'yield-authority-read') {
            assert.ok(input.includes(workspace), input);
            assert.ok(!input.includes(verification), input);
            authorityAdvanced = true;
            return tool('yield-authority-work', 'bash', {command: 'sleep 2.5'});
        }
        assert.equal(input.split(workspace).length - 1, 1, input);
        assert.equal(input.split(verification).length - 1, 1, input);
        return {content: 'Authority finished.'};
    }, (root, write) => {
        write('pytest', `#!/bin/bash\nsleep 2\necho ${verification}\n`, true);
        return modelPolicy(root, write);
    }, {modelIds: ['agent', 'candidate']});
    assert.ok(authorityAdvanced);
    assert.equal(done.status, 'succeeded');
    const events = rows();
    const yields = events.filter(event => event.disposition === 'Yield');
    assert.equal(yields.length, 2);
    const candidate = yields[0].branch_id;
    assert.equal(yields[1].branch_id, candidate);
    const finished = events.find(event => event.message === 'action_finished' && event.action_id.startsWith('yield-test'));
    assert.equal(finished?.status, 'succeeded');
    assert.ok(events.indexOf(yields[0]) < events.indexOf(finished));
    assert.ok(events.indexOf(finished) < events.indexOf(yields[1]));
    const cleaned = events.findIndex(event => event.message === 'command_result' && event.branch_id === candidate
        && event.operation === 'cleanup' && event.status === 'applied');
    assert.ok(cleaned > events.indexOf(yields[1]) && cleaned < events.findIndex(event => event.message === 'materialization_started'));
    assert.ok(!events.some(event => event.policy_error));
    await shutdown();
});

for (const prior of [false, true]) test(`Mode ${prior ? 1 : 2} salvages only read-only facts before edit`, {timeout: 45000}, async t => {
    const started = [];
    const marker = 'PRIVATE_CANDIDATE_TEXT', thinking = 'PRIVATE_CANDIDATE_THINKING';
    const candidateReply = delta => ({content: marker, reasoning_content: thinking, ...delta});
    const {rows, done, root, shutdown} = await fixture(t, async (messages, model, root) => {
        const id = lastTool(messages)?.tool_call_id?.split('|')[0];
        started.push({model, id});
        const seen = (model, id) => started.some(call => call.model === model && call.id === id);
        if (model === 'candidate') {
            if (id === 'authority-read') {
                await waitFor(() => seen('agent', id), 'Authority must run concurrently on the subsequent turn');
                return candidateReply({});
            }
            if (!id) {
                if (prior && started.filter(call => call.model === 'candidate' && !call.id).length === 1) {
                    assert.equal(seen('agent', undefined), false);
                } else if (!prior) await waitFor(() => seen('agent', undefined), 'Authority must start concurrently');
                return candidateReply(tool('candidate-first', 'read', {path: 'other.txt'}));
            }
            if (id === 'candidate-first') return candidateReply(tool('candidate-second', 'read', {path: 'value.txt', limit: 2}));
            assert.equal(id, 'candidate-second');
            return candidateReply(tool('braked-edit', 'edit', {path: 'value.txt', edits: [{oldText: 'one', newText: 'bad'}]}));
        }
        const input = JSON.stringify(messages);
        assert.ok(!input.includes(marker) && !input.includes(thinking));
        if (prior || id) {
            assert.ok(input.includes('read other.txt') && input.includes('different'), input);
            if (prior) assert.ok(input.includes('read value.txt') && input.includes('one\\ntwo'), input);
            else assert.ok(!input.includes('[Speculative workspace output collected on current files]: read value.txt'), input);
        }
        if (!id) {
            if (!prior) await waitFor(() => {
                const path = runPath(root, 'events.jsonl');
                return fs.existsSync(path) && fs.readFileSync(path, 'utf8').includes('extent_reached');
            }, 'candidate should park before Authority returns');
            return tool('authority-read', 'read', {path: 'value.txt', limit: 4});
        }
        assert.equal(id, 'authority-read');
        await waitFor(() => seen('candidate', id), 'subsequent step should admit a candidate');
        return {content: 'done'};
    }, (root, write) => modelPolicy(root, write, {prior}), {modelIds: ['agent', 'candidate'], reasoning: true});
    const events = rows();
    assert.equal(done.branch_id, 'authority');
    assert.ok(!events.some(e => e.policy_error || e.message === 'policy_error'));
    const first = events.find(e => e.message === 'candidate_created').branch_id;
    assert.equal(events.find(e => e.branch_id === first && e.disposition)?.disposition, 'Salvage');
    assert.ok(events.some(e => e.message === 'observation' && e.observation.event === 'settlement'
        && e.observation.branch_id === first && e.observation.disposition === 'Salvage' && !e.observation.hit));
    assert.ok(!events.some(e => e.message === 'action_started' && e.action_id.startsWith('braked-edit')));
    for (const id of ['candidate-first', 'candidate-second']) {
        assert.ok(events.some(e => e.message === 'action_finished' && e.action_id.startsWith(id)
            && e.branch_id === first && e.status === 'succeeded'));
    }
    assert.equal(fs.readFileSync(runPath(root, 'workspace/value.txt'), 'utf8'), 'one\ntwo\nthree\nfour\n');
    await shutdown();
    const tree = JSON.parse(fs.readFileSync(runPath(root, 'final-tree.json'), 'utf8'));
    assert.ok(tree.transcripts[first].some(message => message.role === 'assistant'
        && message.content.some(part => part.type === 'thinking' && part.thinking === thinking)
        && message.content.some(part => part.type === 'text' && part.text === marker)));
});

test('adaptive tool speculation promotes the failure branch after a source edit', {timeout: 45000}, async t => {
    let predictedCalls = 0;
    const {rows, done, shutdown} = await fixture(t, messages => {
        const last = lastTool(messages);
        switch (last?.tool_call_id?.split('|')[0]) {
            case undefined: return tool('source-write', 'write', {path: 'app.py', content: 'value = 1\n'});
            case 'source-write': return tool('verify', 'bash', {command: './pytest'});
            case 'verify':
                if (last.content.includes('Predicted verification result:')) predictedCalls++;
                return {content: last.content.includes('fails as expected') ? 'failure branch' : 'pass branch'};
            default: assert.fail(`unexpected tool ${last.tool_call_id}`);
        }
    }, (_, write) => {
        write('pytest', '#!/bin/bash\nsleep 1.2\necho "AssertionError: expected 2"\nexit 1\n', true);
        return join(coding, 'policy.yaml');
    });
    const events = rows();
    assert.ok(!events.some(e => e.policy_error || e.message === 'policy_error'));
    const branches = events.filter(e => e.message === 'candidate_created');
    assert.equal(branches.length, 2);
    assert.equal(branches[0].chance_id, branches[1].chance_id);
    assert.equal(predictedCalls, 2);
    const decisions = events.filter(e => e.disposition);
    assert.equal(decisions.length, 2);
    assert.equal(decisions.find(e => e.hypothesis?.isError === false)?.disposition, 'Discard');
    const winner = decisions.find(e => e.hypothesis?.isError === true);
    assert.equal(winner?.disposition, 'Promote');
    assert.equal(done.branch_id, winner.branch_id);
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

def extent_partial(data, *, policy_config, capabilities):
    # This synthetic policy stages a candidate write to exercise artifact transfer.
    return 'continue'

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
            .replace('function: extent', 'function: extent_partial')
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

for (const scenario of ['running', 'finished', 'yield', 'salvage', 'fast-model', 'recovery']) {
    test('post-edit verification ' + scenario, {timeout: 45000}, async t => {
        const active = scenario !== 'fast-model';
        const salvage = scenario === 'salvage' || !active;
        const yielding = scenario === 'yield';
        const diverges = salvage || yielding;
        const running = scenario === 'running' || scenario === 'recovery';
        const command = './pytest';
        const authorityModel = active ? 'gpt-6.2-sol' : 'agent';
        const toolStarted = event => event.message === 'action_started' && event.action_id.startsWith('pred-candidate');
        const parked = event => event.message === 'branch_suspended' && event.reason === 'extent_reached';
        async function waitEvent(root, predicate, label) {
            let found;
            await waitFor(() => found = fs.readFileSync(runPath(root, 'events.jsonl'), 'utf8')
                .split('\n').slice(0, -1).map(JSON.parse).find(predicate), label);
            return found;
        }
        let continuation, aborted, release, predictedExecution, releaseRunning;
        const pending = new Promise(resolve => { release = resolve; });
        const {done, rows, root, requests, shutdown} = await fixture(t, async (messages, model, root, thinking, onAbort) => {
            const id = lastTool(messages)?.tool_call_id?.split('|')[0];
            if (JSON.stringify(messages).includes('Confirm the recovered verification.')) {
                assert.match(JSON.stringify(messages), /POST_EDIT_RESULT/);
                return {content: 'Recovered verification confirmed'};
            }
            const predicted = id?.startsWith('pred-') || id === 'fast-test';
            if (predicted && active && model === 'candidate') return {content: 'No speculative work'};
            if (predicted) {
                assert.equal(model, active ? authorityModel : 'candidate');
                assert.match(lastTool(messages).content, /POST_EDIT_RESULT/);
                continuation = true;
                if (!active || scenario === 'recovery') {
                    onAbort(() => { aborted = true; release(); });
                    await pending;
                }
                return {content: 'Candidate verification finished'};
            }
            if (model === 'candidate') {
                if (id === 'source-write') {
                    assert.equal(active, false);
                    assert.equal(thinking, undefined);
                    return tool('fast-test', 'bash', {command});
                }
                return {content: 'No speculative work'};
            }
            if (!id) return tool('baseline', 'bash', {command});
            if (id === 'baseline') return tool('source-write', 'write', {path: 'value.txt', content: 'edited'});
            if (id === 'source-write') {
                if (active) {
                    predictedExecution = await waitEvent(root, toolStarted, 'predicted test should start while Authority is waiting');
                    if (running) {
                        // Runtime can finish transferring only after the in-flight tool returns.
                        releaseRunning = waitEvent(root, event => event.observation?.branch_id === predictedExecution.branch_id
                            && event.observation.event === 'status_changed' && event.observation.status === 'promoting',
                            'promotion requested while verification is running')
                            .then(() => fs.writeFileSync(join(root, 'release-test'), ''));
                        releaseRunning.catch(() => {});
                    } else if (!yielding) {
                        await waitEvent(root, event => event.branch_id === predictedExecution.branch_id && parked(event),
                            'candidate should park before inference while Authority is waiting');
                    }
                    assert.equal(continuation, undefined);
                } else {
                    await waitFor(() => continuation, 'fast-model candidate should reach its next inference');
                }
                return diverges ? tool('authority-read', 'read', {path: 'other.txt'})
                    : tool('authority-test', 'bash', {command});
            }
            if (yielding && id === 'authority-read') {
                await waitEvent(root, event => event.branch_id === predictedExecution.branch_id
                    && event.disposition === 'Yield' && event.part.results.length === 0, 'initial empty Yield');
                assert.doesNotMatch(JSON.stringify(messages), /POST_EDIT_RESULT/);
                fs.writeFileSync(join(root, 'release-test'), '');
                await waitEvent(root, event => event.branch_id === predictedExecution.branch_id
                    && event.operation === 'cleanup' && event.status === 'applied', 'cleanup after final Yield');
                return tool('authority-confirm', 'read', {path: 'value.txt'});
            }
            assert.equal(id, yielding ? 'authority-confirm' : 'authority-read');
            assert.match(JSON.stringify(messages), /Accepted result: \[Speculative verification output.*POST_EDIT_RESULT/);
            assert.doesNotMatch(JSON.stringify(messages), /Candidate verification finished/);
            return {content: 'Authority finished'};
        }, (root, write) => {
            const policy = modelPolicy(root, write, {active});
            write('pytest', '#!/bin/sh\nif [ "$(cat value.txt)" != edited ]; then echo BASELINE; exit; fi\n'
                + (running || yielding ? `while [ ! -f '${join(root, 'release-test')}' ]; do sleep 0.05; done\n` : '')
                + 'echo POST_EDIT_RESULT\n' + (diverges ? 'exit 1\n' : ''), true);
            return policy;
        }, {authorityModel, modelIds: [authorityModel, 'candidate'], responses: true, recoveryBarrier: scenario === 'recovery' ? async () => {
            await waitFor(() => continuation, 'promoted candidate should reach its next inference');
            return release;
        } : undefined});
        await releaseRunning;
        const events = rows();
        const execution = events.find(e => toolStarted(e) || (e.message === 'action_started' && e.action_id.startsWith('fast-test')));
        assert.ok(execution);
        const finished = events.find(e => e.message === 'action_finished' && e.action_id === execution.action_id);
        assert.ok(finished);
        const candidate = execution.branch_id;
        const disposition = events.find(e => e.branch_id === candidate
            && e.disposition === (yielding ? 'Yield' : salvage ? 'Salvage' : 'Promote'));
        assert.ok(disposition);
        const extent = events.find(e => e.branch_id === candidate && parked(e));
        if (!diverges) {
            assert.equal(continuation, true);
            if (running) {
                const promoting = events.findIndex(e => e.observation?.branch_id === candidate
                    && e.observation.event === 'status_changed' && e.observation.status === 'promoting');
                assert.ok(promoting > events.indexOf(execution) && promoting < events.indexOf(finished));
            }
            if (scenario === 'finished') {
                assert.ok(extent);
                assert.ok(events.indexOf(finished) < events.indexOf(extent));
                assert.ok(events.indexOf(extent) < events.indexOf(disposition));
            }
            assert.equal(done.branch_id, candidate);
            assert.ok(!events.some(e => e.message === 'action_started' && e.action_id.startsWith('authority-test')));
            const session = rows('session.jsonl');
            const call = session.find(e => e.message?.content?.some?.(p => p.id === execution.action_id));
            const result = session.find(e => e.message?.toolCallId === execution.action_id);
            assert.equal(result.parentId, call.id);
        } else if (yielding) {
            assert.equal(done.branch_id, 'authority');
            const yields = events.filter(e => e.branch_id === candidate && e.disposition === 'Yield');
            assert.equal(yields.length, 2);
            assert.deepEqual(yields[0].part.results, []);
            assert.match(yields[1].part.results[0].notification, /POST_EDIT_RESULT/);
            const cleanup = events.findIndex(e => e.branch_id === candidate && e.message === 'command_result'
                && e.operation === 'cleanup' && e.status === 'applied');
            assert.ok(extent);
            assert.ok(events.indexOf(yields[0]) < events.indexOf(finished));
            assert.ok(events.indexOf(finished) < events.indexOf(extent));
            assert.ok(events.indexOf(extent) < events.indexOf(yields[1]));
            assert.ok(events.indexOf(yields[1]) < cleanup);
            assert.ok(cleanup < events.findIndex(e => e.message === 'materialization_started'));
        } else {
            if (!active) await waitFor(() => aborted, 'pending fast-model reasoning should abort');
            assert.equal(done.branch_id, 'authority');
            assert.ok(events.some(e => e.observation?.branch_id === candidate && e.observation.status === 'discarded'));
            assert.ok(disposition.part.results.every(result => result.files.length === 0));
        }
        if (active) {
            const postTestRequests = requests.filter(request => request.toolId === execution.action_id
                && request.model === authorityModel);
            if (scenario !== 'recovery') assert.equal(postTestRequests.length, diverges ? 0 : 1);
            const modelStarts = events.filter(e => e.branch_id === candidate && e.message === 'action_started'
                && e.action_kind === 'model_inference');
            if (scenario !== 'recovery') assert.equal(modelStarts.length, diverges ? 0 : 1);
            if (!diverges) assert.ok(modelStarts[0] && events.indexOf(disposition) < events.indexOf(modelStarts[0]));
        }
        if (scenario === 'recovery') {
            assert.equal(events.filter(e => e.message === 'action_started' && e.action_id === execution.action_id).length, 1);
            assert.equal(new Set(requests.map(request => request.cacheKey)).size, 1);
        } else {
            assertSharedCacheKey(root, requests, diverges ? ['authority'] : ['authority', candidate]);
        }
        await shutdown();
    });
}

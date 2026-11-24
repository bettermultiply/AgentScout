import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
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

function modelPolicy(root, write, {fast = true, tools = false} = {}) {
    write('value.txt', 'one\ntwo\nthree\nfour\n');
    write('other.txt', 'different\n');
    const dir = join(root, 'policy');
    fs.cpSync(coding, dir, {recursive: true});
    const manifest = join(dir, 'policy.yaml');
    const config = fs.readFileSync(manifest, 'utf8')
        .replace('fast_model_speculation: true', `fast_model_speculation: ${fast}`)
        .replace('tool_speculation: true', `tool_speculation: ${tools}`)
        .replaceAll('candidate_model: deepseek/deepseek-flash', 'candidate_model: fixture/candidate');
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

for (const failure of [false, true]) test(`Verify ${failure ? 'Fail salvages verified edits' : 'Pass promotes with complete output'}`, {timeout: 45000}, async t => {
    const realOutput = `START_OF_TEST\n${'test output line\n'.repeat(420)}END_OF_TEST`;
    const expected = failure ? 'Command exited with non-zero status.' : 'All tests passed.';
    const {done, rows, root, shutdown} = await fixture(t, (messages, model, root) => {
        const last = lastTool(messages);
        const id = last?.tool_call_id?.split('|')[0];
        const input = JSON.stringify(messages);
        assert.doesNotMatch(input, /Predicted verification result:/);
        if (!id) return failure ? tool('test-edit', 'write', {path: 'test_app.py', content: 'assert value == 1'})
            : tool('verify', 'bash', {command: './pytest'});
        if (id === 'test-edit') return tool('verify', 'bash', {command: './pytest'});
        if (id === 'verify' && last.content === expected) {
            assert.equal(model, failure ? 'candidate' : 'agent');
            if (failure) assert.match(input, /Do not re-run the test suite right away; first inspect/);
            return tool('inspect', 'read', {path: 'app.py'});
        }
        if (id === 'inspect') return tool('candidate-edit', 'write', {path: 'app.py', content: 'value = 1\n'});
        if (id === 'candidate-edit') {
            fs.writeFileSync(join(root, 'candidate-ready'), '');
            return tool('candidate-retest', 'bash', {command: './pytest second'});
        }
        if (id === 'candidate-retest') {
            assert.match(last.content, /CANDIDATE_VERIFIED/);
            return {content: 'Candidate verified the fix'};
        }
        if (failure) {
            assert.equal(id, 'verify');
            assert.equal(model, 'agent');
            assert.match(input, /read app\.py/);
            assert.match(input, /END_OF_TEST/);
            assert.doesNotMatch(input, /Start with high-confidence checks/);
            assert.match(input, /CANDIDATE_VERIFIED/);
        } else {
            assert.fail(`unexpected continuation after ${id}`);
        }
        return {content: 'Done'};
    }, (root, write) => {
        write('app.py', 'value = 0\n');
        write('pytest', `#!/bin/sh\nif [ "$1" = second ]; then echo CANDIDATE_VERIFIED; touch '${join(root, 'candidate-verified')}'; exit 0; fi\nfor i in $(seq 1 100); do [ -f '${join(root, 'candidate-verified')}' ] && break; sleep .05; done\nsleep .4\nprintf '%s\\n' '${realOutput}'\nexit ${Number(failure)}\n`, true);
        const manifest = modelPolicy(root, write, {fast: false, tools: true});
        fs.appendFileSync(join(root, 'policy', 'decisions.py'), `

def initial_verification_only(data, *, policy_config, capabilities):
    return data['op'].get('arguments', {}).get('command') == './pytest'
`);
        fs.writeFileSync(manifest, fs.readFileSync(manifest, 'utf8')
            .replace('function: feasibility_pass', 'function: initial_verification_only'));
        return manifest;
    }, {modelIds: ['agent', 'candidate']});
    const events = rows();
    assert.ok(!events.some(e => e.policy_error || e.message === 'policy_error'));
    const settlement = events.find(e => e.observation?.event === 'settlement')?.observation;
    assert.equal(settlement.disposition, failure ? 'Salvage' : 'Promote');
    assert.equal(settlement.hit, true);
    assert.equal(done.branch_id, failure ? 'authority' : settlement.branch_id);
    const edited = events.findIndex(e => e.action_id?.startsWith('candidate-edit') && e.message === 'action_finished');
    const retest = events.findIndex(e => e.action_id?.startsWith('candidate-retest') && e.message === 'action_finished');
    assert.ok(edited >= 0 && retest > edited);
    assert.equal(fs.readFileSync(runPath(root, 'workspace/app.py'), 'utf8'), 'value = 1\n');
    if (!failure) {
        const notices = rows('session.jsonl').filter(e => e.customType === 'notification');
        assert.ok(notices.some(e => e.content.includes(realOutput)));
    }
    await shutdown();
});

test('Explore yields novel reads before inference and retires after an Authority edit', {timeout: 45000}, async t => {
    let candidateWaiting = false;
    let aborted = false;
    const {rows, done, root, shutdown} = await fixture(t, async (messages, model, _root, _thinking, onAbort) => {
        const id = lastTool(messages)?.tool_call_id?.split('|')[0];
        if (model === 'candidate') {
            if (!id) {
                assert.match(JSON.stringify(messages), /Use read-only tools/);
                return tool('candidate-read', 'read', {path: 'other.txt'});
            }
            if (id === 'candidate-read') return tool('candidate-write', 'write', {path: 'value.txt', content: 'candidate only'});
            assert.equal(id, 'candidate-write');
            candidateWaiting = true;
            onAbort(() => { aborted = true; });
            await delay(7000, undefined, {ref: false});
            return {content: 'Private reasoning'};
        }
        if (!id) {
            await waitFor(() => candidateWaiting, 'candidate write should run before Authority responds');
            return tool('authority-read', 'read', {path: 'value.txt'});
        }
        const input = JSON.stringify(messages);
        assert.match(input, /read other\.txt/);
        assert.doesNotMatch(input, /candidate only|Private reasoning/);
        if (id === 'authority-read') return tool('authority-edit', 'write', {path: 'value.txt', content: 'authority only'});
        await waitFor(() => aborted, 'Authority edit should preempt the running explorer');
        return {content: 'Done'};
    }, (root, write) => {
        const manifest = modelPolicy(root, write);
        fs.appendFileSync(join(root, 'policy', 'decisions.py'), `\n\ndef initial_only(data, *, policy_config, capabilities):\n    return data['state']['steps_completed'] == 0\n`);
        fs.writeFileSync(manifest, fs.readFileSync(manifest, 'utf8')
            .replace('function: feasibility_pass', 'function: initial_only'));
        return manifest;
    }, {modelIds: ['agent', 'candidate']});
    assert.equal(done.branch_id, 'authority');
    const events = rows();
    const yielded = events.find(e => e.disposition === 'Yield');
    assert.ok(yielded);
    assert.equal(yielded.part.results.length, 1);
    assert.ok(!events.some(e => e.disposition === 'Promote' || e.policy_error));
    assert.equal(fs.readFileSync(runPath(root, 'workspace/value.txt'), 'utf8'), 'authority only');
    await shutdown();
});

for (const admit of [false, true]) test(`Yield batches results before ${admit ? 'a fresh' : 'a locally denied'} model opportunity`, {timeout: 45000}, async t => {
    let candidateWaiting = false;
    let authorityAdvanced = false;
    const workspace = '[Speculative workspace output collected on current files]: read other.txt';
    const first = 'YIELD_FIRST_COMPLETED';
    const second = 'YIELD_SECOND_COMPLETED';
    const log = root => fs.readFileSync(runPath(root, 'events.jsonl'), 'utf8').split('\n').slice(0, -1).map(JSON.parse);
    const {rows, done, root, shutdown} = await fixture(t, async (messages, model, root) => {
        const id = lastTool(messages)?.tool_call_id?.split('|')[0];
        if (model === 'candidate' && !id) return {tool_calls: [
            ...tool('yield-shared', 'read', {path: 'value.txt', limit: 2}).tool_calls,
            {...tool('yield-novel', 'read', {path: 'other.txt'}).tool_calls[0], index: 1},
        ]};
        if (model === 'candidate' && id === 'yield-authority-work') return {content: 'Fresh exploration'};
        if (model === 'candidate' && !id.startsWith('yield-late')) {
            candidateWaiting = true;
            await waitFor(() => authorityAdvanced, 'Authority should continue after the initial Yield');
            return {tool_calls: [
                ...tool('yield-late-first', 'read', {path: 'late-first.txt'}).tool_calls,
                {...tool('yield-late-second', 'read', {path: 'late-second.txt'}).tool_calls[0], index: 1},
            ]};
        }
        if (model === 'candidate') {
            // Stay running so only the authority boundary can trigger the second Yield.
            await delay(6000, undefined, {ref: false});
            return {content: 'Candidate finished.'};
        }
        if (!id) {
            await waitFor(() => candidateWaiting, 'candidate should be waiting in its second inference');
            return tool('yield-authority-read', 'read', {path: 'value.txt', limit: 4});
        }
        const input = JSON.stringify(messages);
        if (id === 'yield-authority-read') {
            assert.ok(input.includes(workspace), input);
            assert.ok(!input.includes(first), input);
            authorityAdvanced = true;
            void waitFor(() => log(root).some(e => e.message === 'action_finished' && e.action_id === 'yield-late-second'),
                'both late reads should finish').then(() => fs.promises.writeFile(join(root, 'authority-read-fifo'), 'ready'));
            return tool('yield-authority-work', 'bash', {command: `cat '${join(root, 'authority-read-fifo')}'`});
        }
        assert.equal(input.split(workspace).length - 1, 1, input);
        assert.equal(input.split(first).length - 1, 1, input);
        assert.equal(input.split(second).length - 1, 1, input);
        if (id === 'yield-authority-work') return tool('yield-authority-edit', 'write', {path: 'value.txt', content: 'edited after Yield'});
        assert.equal(id, 'yield-authority-edit');
        return {content: 'Authority finished.'};
    }, (root, write) => {
        execFileSync('mkfifo', [join(root, 'authority-read-fifo')]);
        write('late-first.txt', first);
        write('late-second.txt', second);
        const manifest = modelPolicy(root, write);
        const dir = join(root, 'policy');
        fs.appendFileSync(join(dir, 'decisions.py'), `

def feasibility_initial_only(data, *, policy_config, capabilities):
    return data['state']['steps_completed'] == 0

def feasibility_next_explore(data, *, policy_config, capabilities):
    return data['state']['steps_completed'] < 3

def extent_keep_running(data, *, policy_config, capabilities):
    return 'continue'
`);
        const config = fs.readFileSync(manifest, 'utf8').replace('function: extent', 'function: extent_keep_running');
        fs.writeFileSync(manifest, config.replace('function: feasibility_pass', admit ? 'function: feasibility_next_explore' : 'function: feasibility_initial_only'));
        return manifest;
    }, {modelIds: ['agent', 'candidate']});
    assert.ok(authorityAdvanced);
    assert.equal(done.status, 'succeeded');
    const events = rows();
    const candidate = events.find(event => event.disposition === 'Yield').branch_id;
    const yields = events.filter(event => event.disposition === 'Yield' && event.branch_id === candidate);
    assert.equal(yields.length, 2);
    assert.equal(yields[1].branch_id, candidate);
    const completed = events.filter(event => event.message === 'action_finished' && event.action_id.startsWith('yield-late'));
    assert.deepEqual(completed.map(event => event.status), ['succeeded', 'succeeded']);
    const boundary = events.slice(events.indexOf(completed[1]) + 1).find(event => admit
        ? event.message === 'speculative_admission_request' && event.branch_id === 'authority'
        : event.message === 'model_inference_boundary' && event.branch_id === 'authority');
    assert.ok(boundary, 'the next Authority boundary must settle Yield before inference');
    if (admit) {
        const chance = events.find(event => event.observation?.event === 'chance_created'
            && event.observation.parent_branch_id === 'authority' && event.observation.operation_id === boundary.action_id);
        assert.ok(chance, 'the batched Yield boundary admits a Chance');
        assert.ok(events.some(event => event.message === 'candidate_created'
            && event.chance_id === chance.observation.chance_id && event.branch_id !== candidate));
    }
    if (!admit) assert.equal(events.filter(event => event.message === 'candidate_created').length, 1);
    assert.ok(events.indexOf(yields[0]) < events.indexOf(completed[0]));
    assert.ok(events.indexOf(completed[1]) < events.indexOf(boundary));
    assert.ok(events.indexOf(boundary) < events.indexOf(yields[1]));
    assert.equal(yields[1].part.results.length, 2);
    const edit = events.find(event => event.message === 'action_finished' && event.action_id === 'yield-authority-edit');
    assert.equal(edit?.status, 'succeeded');
    const inference = events.find(event => event.message === 'action_finished' && event.branch_id === 'authority'
        && event.action_id === boundary.action_id);
    assert.equal(inference?.result.tool_calls?.[0]?.id, 'yield-authority-edit');
    assert.ok(events.indexOf(yields[1]) < events.indexOf(inference) && events.indexOf(inference) < events.indexOf(edit));
    assert.equal(fs.readFileSync(runPath(root, 'workspace/value.txt'), 'utf8'), 'edited after Yield');
    const cleaned = events.findIndex(event => event.message === 'command_result' && event.branch_id === candidate
        && event.operation === 'cleanup' && event.status === 'applied');
    assert.ok(cleaned > events.indexOf(yields[1]) && cleaned < events.findIndex(event => event.message === 'materialization_started'));
    assert.ok(!events.some(event => event.policy_error));
    await shutdown();
});

for (const conflict of [false, true]) test(`draining Yield salvages verified files (${conflict ? 'conflict' : 'clean transfer'})`, {timeout: 45000}, async t => {
    const command = './pytest drain';
    const waitForEvent = (root, predicate, label) => waitFor(() => fs.readFileSync(runPath(root, 'events.jsonl'), 'utf8')
        .split('\n').slice(0, -1).map(JSON.parse).some(predicate), label);
    let fresh = false, parked;
    const {done, rows, root, requests, shutdown} = await fixture(t, async (messages, model, root) => {
        const id = lastTool(messages)?.tool_call_id?.split('|')[0];
        const input = JSON.stringify(messages);
        if (model === 'candidate') {
            if (!id) return tool('drain-read', 'read', {path: 'other.txt'});
            if (id === 'drain-read') return tool('drain-edit', 'write', {path: 'value.txt', content: 'candidate fix'});
            if (id === 'drain-edit') return {tool_calls: [
                ...tool('drain-verify', 'bash', {command}).tool_calls,
                {...tool('drain-last-tool', 'read', {path: 'last.txt'}).tool_calls[0], index: 1},
            ]};
            assert.equal(id, 'drain-authority-read', 'draining candidate must park before another inference');
            assert.match(input, /Avoid duplicating commands already running in other candidates/);
            assert.match(input, /pytest drain/);
            fresh = true;
            return {content: 'Fresh exploration finished'};
        }
        assert.doesNotMatch(input, /Avoid duplicating commands already running in other candidates/);
        if (!id) {
            await waitForEvent(root, event => event.message === 'action_started' && event.action_id === 'drain-verify',
                'first candidate verification in flight');
            return tool('drain-authority-read', 'read', {path: 'value.txt'});
        }
        if (id === 'drain-authority-read') {
            await waitFor(() => fresh, 'fresh exploration while the old candidate drains');
            assert.doesNotMatch(input, /candidate fix|VERIFIED_DRAIN/);
            parked = waitForEvent(root, event => event.message === 'branch_suspended' && event.reason === 'extent_reached',
                'draining candidate parks after both tools').then(() => fs.writeFileSync(join(root, 'parked'), ''));
            void parked.catch(() => {});
            return tool('drain-authority-work', 'bash', {command: './settle'});
        }
        assert.equal(id, 'drain-authority-work');
        assert.match(input, /fast speculative candidate.*VERIFIED_DRAIN/);
        if (conflict) assert.doesNotMatch(input, /Accepted result: value.txt/);
        else assert.match(input, /Accepted result: value.txt.*candidate fix/);
        return {content: 'Authority finished'};
    }, (root, write) => {
        write('last.txt', 'last tool output');
        write('pytest', `#!/bin/sh\nwhile [ ! -f '${join(root, 'release')}' ]; do sleep .025; done\n`
            + 'grep -qx "candidate fix" value.txt || exit 1\necho VERIFIED_DRAIN\n', true);
        write('settle', '#!/bin/sh\n' + (conflict ? "printf 'authority update' > value.txt\n" : '')
            + `touch '${join(root, 'release')}'\nwhile [ ! -f '${join(root, 'parked')}' ]; do sleep .025; done\n`, true);
        const manifest = modelPolicy(root, write);
        fs.appendFileSync(join(root, 'policy', 'decisions.py'), `

def first_two_rounds(data, *, policy_config, capabilities):
    return data['state']['steps_completed'] < 2
`);
        fs.writeFileSync(manifest, fs.readFileSync(manifest, 'utf8')
            .replace('function: feasibility_pass', 'function: first_two_rounds')
            .replace('share_inflight_commands: false', 'share_inflight_commands: true'));
        return manifest;
    }, {modelIds: ['agent', 'candidate']});
    await parked;
    assert.equal(done.branch_id, 'authority');
    const events = rows();
    const yielded = events.find(e => e.disposition === 'Yield' && e.part.results.some(result => result.id === 'drain-read'));
    assert.ok(yielded);
    const branch = yielded.branch_id;
    const freshBranch = events.find(e => e.message === 'candidate_created' && e.branch_id !== branch);
    const blocked = events.find(e => e.branch_id === branch && e.message === 'branch_suspended' && e.reason === 'extent_reached');
    const salvaged = events.find(e => e.branch_id === branch && e.disposition === 'Salvage');
    assert.ok(freshBranch && blocked && salvaged);
    assert.ok(events.indexOf(yielded) < events.indexOf(freshBranch) && events.indexOf(freshBranch) < events.indexOf(blocked));
    for (const id of ['drain-verify', 'drain-last-tool']) {
        const finished = events.find(e => e.action_id === id && e.message === 'action_finished');
        assert.equal(finished?.status, 'succeeded');
        assert.ok(events.indexOf(finished) < events.indexOf(blocked));
    }
    assert.ok(events.indexOf(blocked) < events.indexOf(salvaged));
    assert.equal(salvaged.accepted, false);
    assert.ok(!events.some(e => e.branch_id === branch && e.disposition === 'Promote'));
    assert.ok(events.some(e => e.branch_id === branch && e.operation === 'cleanup' && e.status === 'applied'));
    assert.ok(!requests.some(request => request.model === 'candidate' && request.toolId === 'drain-verify'));
    assert.equal(fs.readFileSync(runPath(root, 'workspace/value.txt'), 'utf8'), conflict ? 'authority update' : 'candidate fix');
    await shutdown();
});

test('tool Proposal selects candidate inference settings and restores Authority on Promote', {timeout: 45000}, async t => {
    let sawCandidate = false;
    let sawRestored = false;
    const guide = 'Only the speculative model should receive this guidance.';
    const {done, rows, root, requests, shutdown} = await fixture(t, (messages, model, _, thinking) => {
        const last = lastTool(messages);
        if (!last) return tool('verify', 'bash', {command: './pytest'});
        if (model === 'candidate') {
            assert.equal(thinking, 'low');
            assert.ok(JSON.stringify(messages).includes(guide));
            sawCandidate = true;
            return tool('after-promotion', 'read', {path: 'result.txt'});
        }
        assert.equal(thinking, 'medium');
        assert.ok(!JSON.stringify(messages).includes(guide));
        assert.match(last.tool_call_id, /after-promotion/);
        sawRestored = true;
        return {content: 'done'};
    }, (root, write) => {
        write('result.txt', 'restored model');
        write('pytest', '#!/bin/sh\nsleep 1.5\necho "1 passed"\n', true);
        const manifest = modelPolicy(root, write, {fast: false, tools: true});
        const dir = join(root, 'policy');
        fs.appendFileSync(join(dir, 'decisions.py'), `

def predict_with_settings(data, *, policy_config, capabilities):
    prediction = predict(data, policy_config=policy_config, capabilities=capabilities)
    for proposal in prediction.get('proposals', []):
        proposal.update(model='fixture/candidate', thinking='low', candidate_guidance=${JSON.stringify(guide)})
    return prediction

def dispose_with_promotion(data, *, policy_config, capabilities):
    # Exercise model restoration under a policy that explicitly allows this promotion.
    return {'actions': [{'intent': 'Promote'}]} if data['resolution'] == 'accepted' else dispose(data, policy_config=policy_config, capabilities=capabilities)
`);
        const config = fs.readFileSync(manifest, 'utf8').replace(/function: predict\b/, 'function: predict_with_settings')
            .replace('function: dispose', 'function: dispose_with_promotion')
            .replace('max_steps: 12', 'max_steps: 1');
        fs.writeFileSync(manifest, config);
        return manifest;
    }, {modelIds: ['agent', 'candidate'], reasoning: true, responses: true});
    assert.ok(sawCandidate && sawRestored);
    assert.ok(rows().some(event => event.disposition === 'Promote' && event.branch_id === done.branch_id));
    assertSharedCacheKey(root, requests, ['authority', done.branch_id]);
    await shutdown();
});

for (const accepted of [true, false]) test(`Salvage (${accepted ? 'accepted' : 'rejected'}) transfers a file and preserves the authority session`, {timeout: 45000}, async t => {
    const {done, rows, root, shutdown} = await fixture(t, messages => {
        const last = lastTool(messages);
        if (!last) return tool('verify', 'bash', {command: './pytest'});
        if (last.content === 'All tests passed.') {
            return tool('candidate-write', 'write', {path: 'selected.txt', content: 'candidate result'});
        }
        return {content: 'done'};
    }, (root, write) => {
        write('pytest', `#!/bin/bash\nsleep 1.2\n${accepted ? "echo '1 passed'" : "echo 'SyntaxError'; exit 1"}\n`, true);
        const manifest = modelPolicy(root, write, {fast: false, tools: true});
        const dir = join(root, 'policy');
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
    action = {'intent': 'Salvage', 'results': [{'id': result['action_id'], 'notification': 'Selected artifact transferred.'}]}
    return {'actions': [action${!accepted ? ", {'intent': 'Discard'}" : ''}]}
`);
        const config = fs.readFileSync(join(dir, 'policy.yaml'), 'utf8')
            .replace('function: extent', 'function: extent_partial')
            .replace('function: match_proposal', 'function: match_partial').replace('function: dispose', 'function: dispose_partial');
        fs.writeFileSync(join(dir, 'policy.yaml'), config);
        return join(dir, 'policy.yaml');
    });
    assert.equal(done.branch_id, 'authority');
    const events = rows();
    const disposition = events.find(e => e.disposition === 'Salvage');
    assert.ok(disposition, JSON.stringify(events.filter(e => e.disposition || e.policy_error)));
    const settlements = events.filter(e => e.message === 'observation').map(e => e.observation)
        .filter(e => e.event === 'settlement' && e.branch_id === disposition.branch_id);
    assert.deepEqual(settlements.map(({disposition, hit}) => ({disposition, hit})), [{disposition: 'Salvage', hit: accepted}]);
    assert.equal(fs.readFileSync(runPath(root, 'workspace/selected.txt'), 'utf8'), 'candidate result');
    assert.equal(rows('session.jsonl').filter(e => e.customType === 'notification' && e.content.includes('Selected artifact transferred.')).length, 1);
    await shutdown();
});

for (const scenario of ['running', 'finished', 'yield', 'salvage', 'recovery']) {
    test('post-edit verification ' + scenario, {timeout: 45000}, async t => {
        const salvage = scenario === 'salvage';
        const yielding = scenario === 'yield';
        const diverges = salvage || yielding;
        const running = scenario === 'running' || scenario === 'recovery';
        const command = './pytest';
        const authorityModel = 'gpt-6.2-sol';
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
            if (model === 'candidate') return {content: 'No speculative work'};
            if (id?.startsWith('pred-')) {
                assert.equal(model, authorityModel);
                assert.match(lastTool(messages).content, /POST_EDIT_RESULT/);
                continuation = true;
                if (scenario === 'recovery') {
                    onAbort(() => { aborted = true; release(); });
                    await pending;
                }
                return {content: 'Candidate verification finished'};
            }
            if (!id) return tool('baseline', 'bash', {command});
            if (id === 'baseline') return tool('source-write', 'write', {path: 'value.txt', content: 'edited'});
            if (id === 'source-write') {
                {
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
                }
                return diverges ? tool('authority-read', 'read', {path: 'other.txt'})
                    : tool('authority-test', 'bash', {command});
            }
            if (yielding && id === 'authority-read') {
                await waitEvent(root, event => event.branch_id === predictedExecution.branch_id
                    && event.disposition === 'Yield' && event.part.results.length === 0, 'initial empty Yield');
                assert.doesNotMatch(JSON.stringify(messages), /POST_EDIT_RESULT/);
                fs.writeFileSync(join(root, 'release-test'), '');
                await waitEvent(root, event => event.branch_id === predictedExecution.branch_id && parked(event), 'park after verification');
                return tool('authority-confirm', 'read', {path: 'value.txt'});
            }
            assert.ok((yielding ? ['authority-confirm'] : ['authority-read', 'authority-debug']).includes(id));
            assert.match(JSON.stringify(messages), /\[Speculative verification output.*POST_EDIT_RESULT/);
            assert.doesNotMatch(JSON.stringify(messages), /Candidate verification finished/);
            if (salvage && id === 'authority-read') return tool('authority-debug', 'bash', {command: 'ls value.txt'});
            return {content: 'Authority finished'};
        }, (root, write) => {
            const policy = modelPolicy(root, write);
            fs.appendFileSync(join(root, 'policy', 'decisions.py'), `

def after_source_edit_only(data, *, policy_config, capabilities):
    return data['state']['steps_completed'] ${salvage ? '>=' : '=='} 2
`);
            fs.writeFileSync(policy, fs.readFileSync(policy, 'utf8')
                .replace('function: feasibility_pass', 'function: after_source_edit_only'));
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
        const execution = events.find(toolStarted);
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
            assert.deepEqual(yields.map(e => e.part.results), [[]]);
            const salvage = events.find(e => e.branch_id === candidate && e.disposition === 'Salvage');
            assert.match(salvage.part.results[0].notification, /POST_EDIT_RESULT/);
            const cleanup = events.findIndex(e => e.branch_id === candidate && e.message === 'command_result'
                && e.operation === 'cleanup' && e.status === 'applied');
            assert.ok(extent);
            assert.ok(events.indexOf(yields[0]) < events.indexOf(finished));
            assert.ok(events.indexOf(finished) < events.indexOf(extent));
            assert.ok(events.indexOf(extent) < events.indexOf(salvage));
            assert.ok(events.indexOf(salvage) < cleanup);
            assert.ok(cleanup < events.findIndex(e => e.message === 'materialization_started'));
        } else {
            assert.equal(done.branch_id, 'authority');
            assert.ok(events.some(e => e.observation?.branch_id === candidate && e.observation.status === 'discarded'));
            assert.ok(disposition.part.results.every(result => result.files.length === 0));
            assert.equal(events.filter(toolStarted).length, 1);
            assert.equal(rows('session.jsonl').filter(e => e.customType === 'notification'
                && e.content.includes('POST_EDIT_RESULT')).length, 1);
            assert.ok(requests.some(request => request.model === 'candidate' && request.toolId === 'authority-debug'));
        }
        {
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

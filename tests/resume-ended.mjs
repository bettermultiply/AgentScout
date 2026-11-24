import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import fs from 'node:fs';
import http from 'node:http';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { test } from 'node:test';
import { setTimeout as delay } from 'node:timers/promises';

async function listen(server) {
    await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
    return server.address().port;
}
async function freePort() {
    const server = http.createServer();
    const port = await listen(server);
    await new Promise(resolve => server.close(resolve));
    return port;
}

test('normal CLI opens finished Pi history and can save and resume its next task', {timeout: 90000}, async t => {
    const root = fs.mkdtempSync('/tmp/as-resume-');
    console.log('evidence', root);
    for (const dir of ['home/.config/ascout', 'pi', 'policy/policy', 'tmp', 'workspace']) fs.mkdirSync(`${root}/${dir}`, {recursive:true});
    fs.writeFileSync(`${root}/home/.config/ascout/config.toml`, `runs_dir = "${root}/runs"\npolicy_dir = "${root}/policy"\nlog_path = "${root}/system.log"\n`);
    fs.copyFileSync('assets/policy/coding/decisions.py', `${root}/policy/policy/decisions.py`);
    const policy = fs.readFileSync('assets/policy/coding/policy.yaml', 'utf8')
        .replaceAll('tool_speculation: true', 'tool_speculation: false');
    fs.writeFileSync(`${root}/policy/policy/policy.yaml`, policy);
    const requests = [], sessionIds = [];
    let finish;
    const fake = http.createServer(async (req, res) => {
        let body = '';
        for await (const chunk of req) body += chunk;
        requests.push(JSON.parse(body));
        sessionIds.push(req.headers.session_id);
        fs.appendFileSync(`${root}/requests.jsonl`, `${body}\n`);
        res.writeHead(200, {'content-type':'text/event-stream'});
        const send = event => res.write(`data: ${JSON.stringify(event)}\n\n`);
        const id = `resp_${requests.length}`;
        const item = requests.length === 1
            ? {type:'function_call', id:'fc_original', call_id:'original-tool', name:'bash',
                arguments:JSON.stringify({command:'printf old-tool-output'}), status:'completed'}
            : {type:'message', id:`msg_${id}`, role:'assistant', status:'completed',
                content:[{type:'output_text', text:requests.length === 3 ? 'second answer' : 'saved answer', annotations:[]}]};
        send({type:'response.created', response:{id}});
        send({type:'response.output_item.added', output_index:0, item});
        const complete = () => {
            send({type:'response.output_item.done', output_index:0, item});
            send({type:'response.completed', response:{id, status:'completed'}});
            res.end();
        };
        if (requests.length === 3) finish = complete;
        else complete();
    });
    const fakePort = await listen(fake);
    t.after(() => {fake.closeAllConnections(); fake.close();});
    const modelFile = `${root}/additional.json`;
    fs.writeFileSync(modelFile, JSON.stringify({providers:{local:{baseUrl:`http://127.0.0.1:${fakePort}/v1`,api:'openai-responses',apiKey:'fake',compat:{sessionAffinityFormat:'openai'},models:[{id:'fake',reasoning:false,input:['text'],contextWindow:32768,maxTokens:256,cost:{input:0,output:0,cacheRead:0,cacheWrite:0}}]}}}));
    fs.writeFileSync(`${root}/runtime.json`, JSON.stringify({backend:'nle',control_address:`127.0.0.1:${await freePort()}`,adapter_instance:'fixture',python:resolve('../.venv-nle/bin/python'),bridge:resolve('examples/nle-runtime-bridge.py'),guidebook:resolve('examples/nle/Guidebook.txt'),seed:7,filesystem:{state_directory:`${root}/fs`,isolated_paths:[`${root}/runs`, `${root}/workspace`],passthrough_rw:[root]}}));
    const web = `http://127.0.0.1:${await freePort()}`;
    const children = [];
    t.after(() => {for (const child of children) if (child.exitCode === null && child.signalCode === null) child.kill('SIGKILL');});
    // The commands_listed Ledger record proves the controller handled earlier Pi frames.
    const gatedEntry = `${root}/hold-history.mjs`;
    const releaseHistory = `${root}/release-history`;
    const historyBarrier = 'fixture-hold-history';
    fs.writeFileSync(gatedEntry, `
        import { existsSync } from 'node:fs';
        import { setTimeout as delay } from 'node:timers/promises';
        import { AscProxy } from ${JSON.stringify(pathToFileURL(resolve('instrumentation/pi/asc-proxy.ts')).href)};
        const send = AscProxy.prototype.send;
        let held = false;
        AscProxy.prototype.send = async function(wire) {
            if (!held && wire.kind === 'message' && wire.payload.message === 'session_history') {
                held = true;
                await send.call(this, {kind:'message', payload:{source:'agent_adapter', message:'commands_listed',
                    branch_id:wire.payload.branch_id, request_id:${JSON.stringify(historyBarrier)}, commands:[]}});
                while (!existsSync(${JSON.stringify(releaseHistory)})) await delay(20);
            }
            return send.call(this, wire);
        };
        await import(${JSON.stringify(pathToFileURL(resolve('instrumentation/pi/main.ts')).href)});
    `);
    function start(runId, holdHistory = false) {
        const entry = holdHistory ? {ASCOUT_AGENT_ENTRY:gatedEntry} : {};
        const child = spawn('target/debug/ascout-control-plane', [resolve('../pi'), '--web', web.slice(7), ...(runId ? ['--resume', runId] : ['--cwd', `${root}/workspace`, '--policy', 'policy', '--model', 'local/fake', '--models', modelFile])], {
            env:{...process.env, ...entry, PI_OFFLINE:'1', HOME:`${root}/home`, TMPDIR:`${root}/tmp`, PI_CODING_AGENT_DIR:`${root}/pi`, ASCOUT_RUNTIME_CONFIG:`${root}/runtime.json`}, stdio:['ignore','ignore','pipe'],
        });
        child.log = '';
        child.stderr.on('data', chunk => {child.log += chunk; fs.appendFileSync(`${root}/cli.log`, chunk);});
        children.push(child);
        return child;
    }
    async function wait(child, predicate, label) {
        for (let i = 0; i < 750; i++) {
            assert.equal(child.exitCode, null, child.log);
            if (await predicate()) return;
            await delay(20);
        }
        assert.fail(`${label}: ${root}`);
    }
    function post(action, prompt) {
        return fetch(`${web}/api/request`, {method:'POST',headers:{origin:web,'content-type':'application/json'},body:JSON.stringify({action,prompt})}).then(response => assert.equal(response.status, 202));
    }
    const inputStatus = async () => (await fetch(`${web}/api/input-status`)).json();
    async function ready(child) {
        await wait(child, async () => {
            try {return (await inputStatus()).ready;}
            catch {return false;}
        }, 'input ready');
    }
    async function currentAuthority() {
        const state = await (await fetch(`${web}/api/observation/state-event-set`)).json();
        const authority = state.events.findLast(event => event.event === 'authority_changed');
        assert.ok(authority, 'public state identifies the current authority');
        return authority.branch_id;
    }
    async function shutdown(child) {
        const exited = once(child, 'exit');
        await post('shutdown');
        assert.deepEqual(await exited, [0, null], child.log);
    }
    let child = start();
    await ready(child);
    const [runId] = fs.readdirSync(`${root}/runs`);
    const run = `${root}/runs/${runId}`;
    const events = () => fs.readFileSync(`${run}/events.jsonl`, 'utf8').trim().split('\n').map(JSON.parse);
    const count = message => events().filter(event => event.message === message).length;
    const history = () => fs.readFileSync(`${run}/session.jsonl`, 'utf8');
    await post('prompt', 'first prompt');
    await wait(child, () => count('task_completed') === 1, 'first task completed');
    await shutdown(child);
    assert.equal(count('recovery_complete'), 0);
    const original = history();
    const originalFile = fs.realpathSync(`${run}/session.jsonl`);
    const rootSessionId = JSON.parse(original.split('\n')[0]).id;
    assert.match(original, /old-tool-output/);
    // Simulate legacy records with only final-tree bindings and a native session link.
    const ledgerPath = `${run}/events.jsonl`;
    const historicalLedger = fs.readFileSync(ledgerPath);
    fs.writeFileSync(ledgerPath, events().filter(event => event.message !== 'agent_released').map(event => {
        if (event.message === 'session_file') delete event.bindings;
        return JSON.stringify(event);
    }).join('\n') + '\n');
    child = start(runId, true);
    await wait(child, () => events().some(event => event.request_id === historyBarrier), 'Pi history held');
    const reopenedBranch = await currentAuthority();
    const conversationFor = async branch => (await fetch(`${web}/api/branches/${branch}/conversation`)).json();
    assert.deepEqual((await conversationFor(reopenedBranch)).inherited, [], 'current authority history is held');
    assert.equal((await inputStatus()).ready, false, 'input stays closed while inherited history is held');
    fs.writeFileSync(releaseHistory, '');
    await ready(child);
    assert.equal(await currentAuthority(), reopenedBranch);
    const conversation = await conversationFor(reopenedBranch);
    const originalMessages = original.trim().split('\n').map(JSON.parse)
        .filter(entry => entry.type === 'message').map(entry => entry.message);
    assert.deepEqual(conversation.inherited, originalMessages);
    assert.match(JSON.stringify(conversation.inherited), /first prompt.*old-tool-output.*saved answer/s);
    const competing = start(runId);
    assert.deepEqual(await once(competing, 'exit'), [1, null]);
    assert.match(competing.log, /Cannot lock run/);
    assert.equal(requests.length, 2);
    assert.equal(history(), original);
    await shutdown(child);
    assert.equal(history(), original);
    assert.equal(fs.realpathSync(`${run}/session.jsonl`), originalFile);
    fs.writeFileSync(ledgerPath, historicalLedger);
    fs.unlinkSync(`${run}/session.jsonl`);
    fs.symlinkSync(`${root}/missing-compatibility-link`, `${run}/session.jsonl`);
    child = start(runId);
    await ready(child);
    assert.equal(fs.realpathSync(`${run}/session.jsonl`), originalFile);
    assert.equal(requests.length, 2);
    await post('prompt', 'second prompt');
    await wait(child, () => finish, 'second prompt reaches provider');
    assert.match(JSON.stringify(requests[2].input), /first prompt.*old-tool-output.*saved answer.*second prompt/s);
    await post('pause');
    finish();
    await wait(child, () => count('recovery_complete') === 1, 'next task saved');
    const checkpoint = events().find(event => event.message === 'recovery_complete');
    assert.equal(checkpoint.control.authority_branch_id, await currentAuthority());
    assert.equal(checkpoint.task_id, events().filter(event => event.message === 'task_started').at(-1).task_id);
    assert.notEqual(checkpoint.task_id, events().filter(event => event.message === 'task_started')[0].task_id);
    const exited = once(child, 'exit');
    child.kill('SIGKILL');
    await exited;
    child = start(runId);
    await wait(child, () => events().some(event => event.message === 'recovery_loaded' && !event.error), 'next task restored');
    await post('prompt', 'queued prompt');
    await delay(300);
    assert.equal(requests.length, 3);
    await post('continue');
    await wait(child, () => count('task_completed') >= 2 && requests.length === 4, 'continue runs queued prompt');
    assert.match(JSON.stringify(requests[3].input), /second prompt.*second answer.*queued prompt/s);
    assert.deepEqual(requests.map(request => request.prompt_cache_key), Array(4).fill(rootSessionId));
    assert.deepEqual(sessionIds, Array(4).fill(rootSessionId));
    assert.notEqual(JSON.parse(history().split('\n')[0]).id, rootSessionId,
        'recovery gives the assembled session a new native ID');
    await shutdown(child);
    const loads = count('recovery_loaded');
    child = start(runId);
    await ready(child);
    assert.equal(count('recovery_loaded'), loads, 'old checkpoint no longer restores the ended task');
    assert.equal(requests.length, 4);
    assert.match(history(), /queued prompt/);
    assert.equal(events().filter(e => e.message === 'action_started' && e.action_id?.split('|')[0] === 'original-tool').length, 1);
    assert.deepEqual(fs.readdirSync(`${root}/runs`), [runId]);
    await shutdown(child);
    const last = events().filter(event => event.message === 'task_completed').at(-1);
    const ledgerBeforeFailure = events();
    const release = ledgerBeforeFailure.find(event => event.message === 'agent_released' && event.task_id === last.task_id);
    assert.ok(release);
    const beforeFailure = history();
    const invalid = ledgerBeforeFailure.map(event => event === release ? {...event, session:{...event.session,file:`${root}/missing-authority`}} : event);
    fs.writeFileSync(ledgerPath, invalid.map(event => JSON.stringify(event)).join('\n') + '\n');
    child = start(runId);
    assert.deepEqual(await once(child, 'exit'), [1, null]);
    assert.match(child.log, /Pi session reference is unavailable/);
    assert.equal(history(), beforeFailure);
    assert.equal(requests.length, 4);
});

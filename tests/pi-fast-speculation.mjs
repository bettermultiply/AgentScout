import assert from 'node:assert/strict';
import { once } from 'node:events';
import fs from 'node:fs';
import http from 'node:http';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { test } from 'node:test';
import { runPath, spawnControl } from './control-plane-fixture.mjs';

const success = 'Successfully replaced 1 block(s) in a.txt.';
const decisions = {
    admit: {
        opportunity_pass: {file:'decisions.py', function:'opportunity_pass'},
        feasibility_pass: {file:'decisions.py', function:'feasibility_pass'},
    },
    ...Object.fromEntries(['select','rebalance','extent','predict','launch','effect','match','dispose']
        .map(name => [name, {file:'decisions.py', function:name, ...(name === 'match' ? {inputs:['outcome']} : {})}])),
};
const implementations = {
    opportunity_pass: "return data['op']['name'] == 'edit' and data['op']['authority']",
    feasibility_pass: 'return True',
    select: "return [chance['chance_id'] for chance in data['frontier_view']]",
    launch: 'return True',
    predict: `return {'generation':'batch','proposals':[{'payload':${JSON.stringify(success)}}]}`,
    extent: "return 'continue'",
    effect: "return {'mode':'allow'}",
    rebalance: 'return []',
    match: `return {'verdict':'compatible' if data['outcome']['payload'] == ${JSON.stringify(success)} else 'incompatible'}`,
    dispose: "return {'actions':[{'intent':'Promote'}]} if data['resolution'] == 'accepted' else {'actions':[{'intent':'Discard'}]}",
};
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

for (const scenario of ['hit','miss','bind']) test(`local edit speculation ${scenario}`, {timeout:30000}, async t => {
    const miss = scenario === 'miss', bind = scenario === 'bind', hit = scenario === 'hit';
    const root = fs.mkdtempSync(join(tmpdir(), 'ascout-fast-edit-'));
    for (const dir of ['workspace','pi','policy']) fs.mkdirSync(join(root, dir));
    fs.writeFileSync(join(root,'workspace/a.txt'), 'before\n');
    fs.writeFileSync(join(root,'policy/decisions.py'), Object.entries(implementations)
        .map(([name, body]) => `def ${name}(data, *, policy_config, capabilities):\n    ${bind && name === 'match'
            ? "if len(data['match_context']['candidate']['usage'] or []) < 2:\n        return {'verdict':'indeterminate'}\n    " : ''}${body}\n`).join('\n'));
    const policy = join(root,'policy/policy.yaml');
    fs.writeFileSync(policy, JSON.stringify({version:1, language:'python', files:['decisions.py'], fidelity:'Best-effort', decisions}));
    const requests = [];
    const fake = http.createServer(async (req,res) => {
        const body = JSON.parse(Buffer.concat(await Array.fromAsync(req)).toString());
        const result = body.messages.findLast(message => message.role === 'tool');
        requests.push(result?.content);
        if (!result) assert.equal(child.kill('SIGSTOP'), true);
        const call = (id, name, args) => ({tool_calls:[{index:0,id,type:'function',function:{name,arguments:JSON.stringify(args)}}]});
        let delta;
        if (!result) delta = call('edit-1', 'edit', {path:'a.txt', edits:[{oldText:miss?'missing':'before',newText:'after'}]});
        else if (hit && result.tool_call_id.startsWith('edit-1')) {
            await waitFor(e => e.message === 'command_result' && e.operation === 'promote' && e.status === 'applied');
            delta = call('winner-read', 'read', {path:'a.txt'});
        } else if (hit && result.tool_call_id.startsWith('winner-read')) {
            delta = call('edit-2', 'edit', {path:'a.txt', edits:[{oldText:'after',newText:'finished'}]});
        } else if (bind && result.content === success && result.tool_call_id.startsWith('edit-1')) {
            delta = call('candidate-read', 'read', {path:'a.txt'});
        } else delta = {content:'done'};
        const chunk = (value, finish_reason) => ({id:'fixture', object:'chat.completion.chunk', created:0,
            model:body.model, choices:[{index:0,delta:value,finish_reason}]});
        res.writeHead(200, {'content-type':'text/event-stream'});
        res.end(`data: ${JSON.stringify(chunk(delta,null))}\n\ndata: ${JSON.stringify(chunk({},delta.tool_calls?'tool_calls':'stop'))}\n\ndata: [DONE]\n\n`);
    });
    const modelPort = await listen(fake);
    const [port,runtimePort] = await Promise.all([freePort(),freePort()]);
    const address = `127.0.0.1:${port}`;
    const models = join(root,'models.json'), runtime = join(root,'runtime.json');
    fs.writeFileSync(models, JSON.stringify({providers:{fixture:{baseUrl:`http://127.0.0.1:${modelPort}/v1`,api:'openai-completions',apiKey:'fake',
        models:[{id:'agent',reasoning:false,input:['text'],contextWindow:32768,maxTokens:256}]}}}));
    fs.writeFileSync(runtime, JSON.stringify({backend:'filesystem',control_address:`127.0.0.1:${runtimePort}`,adapter_instance:'fast-edit',
        filesystem:{state_directory:join(root,'fs'),isolated_paths:[join(root,'workspace')],passthrough_rw:[root]}}));
    const child = spawnControl(resolve('target/debug/ascout-control-plane'), [root,resolve('../pi'),policy,'--web',address,'--model','fixture/agent','--models',models,'--thinking','off'], {
        env:{...process.env, PI_CODING_AGENT_DIR:join(root,'pi'), ASCOUT_RUNTIME_CONFIG:runtime}, stdio:['ignore','pipe','pipe'],
    });
    let logs = '';
    for (const stream of [child.stdout,child.stderr]) stream.on('data', data => {logs += data;});
    const exit = once(child,'exit');
    const post = (action, extra = {}) => fetch(`http://${address}/api/request`, {method:'POST',
        headers:{origin:`http://${address}`,'content-type':'application/json'}, body:JSON.stringify({action,...extra})});
    t.after(async () => {
        child.kill('SIGCONT');
        fake.closeAllConnections(); fake.close();
        if (child.exitCode !== null) return;
        const timer = setTimeout(() => child.kill('SIGKILL'),3000);
        try {await post('shutdown');} catch {}
        await exit;
        clearTimeout(timer);
    });
    const events = () => {
        const path = runPath(root,'events.jsonl');
        return fs.existsSync(path) ? fs.readFileSync(path,'utf8').trim().split('\n').filter(Boolean).map(JSON.parse) : [];
    };
    async function waitFor(predicate) {
        for (let end = Date.now()+15000; Date.now()<end && child.exitCode === null;) {
            const event = events().find(predicate);
            if (event) return event;
            await new Promise(done => setTimeout(done,15));
        }
        assert.fail(`${root}\n${logs}\n${JSON.stringify(events().slice(-24))}`);
    }
    await waitFor(e => e.message === 'branch_ready');
    assert.equal((await post('prompt',{prompt:'Edit a.txt'})).status,202);
    const end = Date.now()+5000;
    while (Date.now()<end && !requests.includes(success)) await new Promise(done => setTimeout(done,10));
    assert.ok(requests.includes(success),'candidate requested inference while control plane was stopped');
    assert.ok(!events().some(e => e.message === 'speculative_admission_request'));
    if (!miss) {
        const state = join(root,'fs');
        const edited = () => fs.globSync('environments/*/mounts/*/**/upper/**/a.txt',{cwd:state,exclude:['**/work','**/work/**']})
            .some(file => fs.readFileSync(join(state,file),'utf8') === 'after\n');
        while (Date.now()<end && !edited()) await new Promise(done => setTimeout(done,10));
        assert.ok(edited(),'authority edited its workspace while control plane was stopped');
    }
    child.kill('SIGCONT');
    const done = await waitFor(e => e.message === 'task_completed');
    assert.equal(done.status,'succeeded');
    const rows = events();
    const admission = rows.find(e => e.message === 'speculative_admission_request' && e.fast_start);
    assert.ok(admission, JSON.stringify(rows));
    const candidate = `candidate-${admission.fast_start.chance_id}`;
    assert.ok(rows.some(e => e.message === 'candidate_created' && e.branch_id === candidate));
    assert.ok(rows.some(e => e.message === 'action_started' && e.branch_id === candidate && e.action_kind === 'model_inference'));
    assert.ok(requests.includes(success));
    const edit = rows.find(e => e.message === 'action_finished' && e.branch_id === 'authority' && e.name !== 'model' && e.action_id.startsWith('edit-1'));
    assert.equal(edit.status, miss ? 'failed' : 'succeeded');
    if (hit) {
        assert.ok(rows.some(e => e.message === 'action_finished' && e.branch_id === candidate
            && e.action_id.startsWith('winner-read') && e.status === 'succeeded'));
        const next = rows.find(e => e.message === 'speculative_admission_request' && e.branch_id === candidate && e.fast_start);
        assert.ok(next, 'the promoted authority can speculate again');
        assert.equal(done.branch_id, `candidate-${next.fast_start.chance_id}`);
    } else assert.equal(done.branch_id, miss ? 'authority' : candidate);
    assert.ok(rows.some(e => e.operation === (miss?'discard':'promote') && e.branch_id === candidate && e.status === 'applied'));
    const observed = rows.filter(e => e.message === 'observation').map(e => e.observation);
    assert.ok(observed.some(e => e.event === 'chance_created' && e.chance_id === admission.fast_start.chance_id));
    assert.ok(observed.some(e => e.event === 'branch_created' && e.branch_id === candidate));
    assert.ok(observed.some(e => e.event === 'settlement' && e.branch_id === candidate && e.hit === !miss
        && (miss || Number.isFinite(e.time_ms.measured))));
    assert.equal(rows.some(e => e.event?.payload?.operation?.operation === 'branch_environment'),bind);
    assert.ok(rows.every(e => !JSON.stringify(e).includes('environment_pending')));
    if (bind) {
        const requested = observed.findIndex(e => e.event === 'environment_changed' && e.branch_id === candidate && e.state === 'environment_requested');
        assert.ok(requested >= 0);
        assert.equal(observed.slice(0, requested).findLast(e => e.event === 'status_changed' && e.branch_id === candidate)?.status, 'running');
        assert.ok(observed.slice(requested + 1).some(e => e.event === 'environment_changed' && e.branch_id === candidate && e.state === 'ready'));
        assert.ok(rows.some(e => e.message === 'action_finished' && e.branch_id === candidate && e.action_id.startsWith('candidate-read') && e.status === 'succeeded'));
    }
    assert.equal(fs.readFileSync(runPath(root,'workspace/a.txt'),'utf8'), miss?'before\n':hit?'finished\n':'after\n');
    assert.equal((await post('shutdown')).status,202);
    assert.equal((await exit)[0],0,logs);
    fs.rmSync(root,{recursive:true,force:true});
});

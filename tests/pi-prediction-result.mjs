import {test} from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import http from 'node:http';
import {once} from 'node:events';
import {isDeepStrictEqual} from 'node:util';
import {spawnControl, runPath} from './control-plane-fixture.mjs';

const image = {type:'image', mimeType:'image/png', data:'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jMrkAAAAASUVORK5CYII='};
const content = [{type:'text', text:'before image'}, image, {type:'text', text:'after image'}];
const predictions = [
    {role:'toolResult', toolCallId:'wrong-id', toolName:'wrong-name', content, isError:true,
        details:{reason:'fixture failure'}, timestamp:1234, durationMs:5, nestedCalls:{calls:[], complete:true}},
    false,
    'An image of a cat',
    {answer:42},
    image,
    [{type:'text', text:'list text'}, image],
    {content:[{type:'text', text:'explicit success'}], isError:false, details:{fixture:'success'}},
];

async function freePort() {
    const server = http.createServer();
    await new Promise(r => server.listen(0, '127.0.0.1', r));
    const {port} = server.address();
    await new Promise(r => server.close(r));
    return port;
}

test('Policy predictions reach the next native Pi request without replaying the tool', {timeout:60000}, async t => {
    const project = process.cwd(), root = fs.mkdtempSync('/tmp/opencode/pi-prediction-');
    for (const dir of ['workspace', 'pi', 'policy']) fs.mkdirSync(`${root}/${dir}`);
    const policy = fs.readFileSync('assets/policy/pi-tool/policy.yaml', 'utf8')
        .replace('example_tools_only: true', 'example_tools_only: false')
        .replace('max_chances: 4', 'max_chances: 1');
    fs.writeFileSync(`${root}/policy/policy.yaml`, policy);
    const decisions = fs.readFileSync('assets/policy/pi-tool/decisions.py', 'utf8')
        .replace('def predict(', 'def example_predict(')
        .replace('def match_proposal(', 'def example_match_proposal(');
    fs.writeFileSync(`${root}/policy/decisions.py`, decisions + `

def predict(data, *, policy_config, capabilities):
    import json
    payloads = json.loads(${JSON.stringify(JSON.stringify(predictions))})
    return {"generation": "batch", "proposals": [{"payload": payload} for payload in payloads]}

def match_proposal(data, *, policy_config, capabilities):
    return {"verdict": "incompatible"}
`);
    const requests = [];
    const fake = http.createServer(async (req, res) => {
        let body = '';
        for await (const chunk of req) body += chunk;
        const request = JSON.parse(body);
        requests.push(request);
        const first = !request.messages.some(m => m.role === 'tool');
        const delta = first ? {tool_calls:[{index:0, id:'fixture-tool', type:'function',
            function:{name:'nle_action', arguments:'{"action":4}'}}]} : {content:'fixture done'};
        const chunk = (delta, finish_reason = null) => ({id:'fixture', object:'chat.completion.chunk',
            created:0, model:'fixture', choices:[{index:0, delta, finish_reason}]});
        res.writeHead(200, {'content-type':'text/event-stream'});
        res.end(`data: ${JSON.stringify(chunk(delta))}\n\ndata: ${JSON.stringify(chunk({}, first ? 'tool_calls' : 'stop'))}\n\ndata: [DONE]\n\n`);
    });
    await new Promise(r => fake.listen(0, '127.0.0.1', r));
    t.after(() => { fake.closeAllConnections(); fake.close(); });
    fs.writeFileSync(`${root}/pi/models.json`, JSON.stringify({providers:{fixture:{
        baseUrl:`http://127.0.0.1:${fake.address().port}/v1`, api:'openai-completions', apiKey:'local-fake',
        models:[{id:'fixture', reasoning:false, input:['text','image'], contextWindow:32768, maxTokens:256}],
    }}}));
    const [port, runtimePort] = [await freePort(), await freePort()];
    fs.writeFileSync(`${root}/runtime.json`, JSON.stringify({backend:'nle', control_address:`127.0.0.1:${runtimePort}`,
        adapter_instance:'pi-prediction', python:`${project}/../.venv-nle/bin/python`, bridge:`${project}/examples/nle-runtime-bridge.py`,
        guidebook:`${project}/examples/nle/Guidebook.txt`, seed:7, action_delay_seconds:3, env_id:'NetHackScore-v0'}));
    const control = spawnControl(`${project}/target/debug/ascout-control-plane`, [root, `${project}/../pi`,
        `${root}/policy/policy.yaml`, '--web', `127.0.0.1:${port}`, '--model', 'fixture/fixture', '--thinking', 'off'],
    {env:{...process.env, ASCOUT_RUNTIME_CONFIG:`${root}/runtime.json`, PI_CODING_AGENT_DIR:`${root}/pi`}, stdio:['ignore','pipe','pipe']});
    for (const stream of [control.stdout, control.stderr]) stream.on('data', b => fs.appendFileSync(`${root}/control.log`, b));
    const base = `http://127.0.0.1:${port}`;
    const post = body => fetch(`${base}/api/request`, {method:'POST', headers:{origin:base,'content-type':'application/json'}, body:JSON.stringify(body)});
    t.after(async () => {
        if (control.exitCode !== null) return;
        const exited = once(control, 'exit');
        const timer = setTimeout(() => control.kill('SIGKILL'), 5000);
        try { await post({action:'shutdown'}); } catch {}
        await exited;
        clearTimeout(timer);
    });
    const events = () => fs.readFileSync(runPath(root, 'events.jsonl'), 'utf8').split('\n').slice(0,-1).filter(Boolean).map(JSON.parse);
    async function wait(predicate, label) {
        const end = Date.now() + 30000;
        while (Date.now() < end) {
            assert.equal(control.exitCode, null, `control plane exited (${root})`);
            if (fs.existsSync(`${root}/runs`) && fs.readdirSync(`${root}/runs`).length && fs.existsSync(runPath(root, 'events.jsonl')) && predicate(events())) return;
            await new Promise(r => setTimeout(r, 20));
        }
        assert.fail(`timeout waiting for ${label} (${root})`);
    }
    await wait(rows => rows.some(e => e.message === 'branch_ready'), 'Agent ready');
    assert.equal((await post({action:'prompt', prompt:'Call nle_action once, then finish.'})).status, 202);
    await wait(rows => rows.some(e => e.message === 'task_completed'), 'task completion');
    const rows = events();
    const tools = rows.filter(e => e.message === 'action_started' && e.action_kind === 'tool_execution');
    assert.deepEqual(tools.map(e => [e.branch_id, e.action_id]), [['authority','fixture-tool']]);
    const finishedTools = rows.filter(e => e.message === 'action_finished' && e.action_kind === 'tool_execution');
    assert.deepEqual(finishedTools.map(e => [e.branch_id, e.action_id, e.status]), [['authority','fixture-tool','succeeded']]);
    assert.equal(rows.find(e => e.message === 'task_completed').status, 'succeeded');
    const entries = fs.readFileSync(runPath(root, 'session.jsonl'), 'utf8').trim().split('\n').map(JSON.parse);
    const results = entries.flatMap(e => e.message?.role === 'toolResult' ? [e.message] : []);
    const prediction = results.find(m => m.details?.reason === 'fixture failure');
    assert.deepEqual(prediction, {...predictions[0], toolCallId:'fixture-tool', toolName:'nle_action'},
        'native session retains ordered blocks, status, and details with checkpoint identity');
    const text = value => [{type:'text', text:value}];
    for (const expected of [text('false'), text('An image of a cat'), text('{"answer":42}'), [image], predictions[5]]) {
        assert.ok(results.some(m => !m.isError && isDeepStrictEqual(m.content, expected)),
            `native successful result ${JSON.stringify(expected)}`);
    }
    const success = results.find(m => m.details?.fixture === 'success');
    assert.deepEqual(success?.content, text('explicit success'));
    assert.equal(success?.isError, false);
    const continuation = requests.find(r => r.messages.some(m => m.role === 'tool' && m.content === 'before image\nafter image'));
    assert.ok(continuation?.messages.some(m => Array.isArray(m.content) && m.content.some(
        p => p.image_url?.url === `data:${image.mimeType};base64,${image.data}`)), 'Pi sends the actual image on continuation');
    assert.ok(requests.some(r => r.messages.some(m => m.role === 'tool' && m.content === 'false')));
    const exited = once(control, 'exit');
    assert.equal((await post({action:'shutdown'})).status, 202);
    assert.deepEqual(await exited, [0, null]);
});

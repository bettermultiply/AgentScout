// Run from the repository root against a local Jaeger OTLP/HTTP receiver.
import assert from 'node:assert/strict';
import { once } from 'node:events';
import fs from 'node:fs';
import http from 'node:http';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { runPath, spawnControl } from '../tests/control-plane-fixture.mjs';

const endpoint = process.env.ASCOUT_E2E_OTLP ?? 'http://127.0.0.1:4318';
const jaeger = process.env.ASCOUT_E2E_JAEGER ?? 'http://127.0.0.1:16686';
const pi = resolve(process.env.PI_ROOT ?? '../pi');
const marker = 'OTEL_E2E_PRIVATE_16F79';
const toolResult = 'Successfully replaced 1 block(s) in a.txt.';
const encode = JSON.stringify;
const tags = span => Object.fromEntries(span.tags.map(({key,value}) => [key,value]));
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
async function until(check, description, timeout = 12000) {
    const end = Date.now() + timeout;
    while (Date.now() < end) {
        const value = await check();
        if (value) return value;
        await delay(30);
    }
    assert.fail(`Timed out waiting for ${description}`);
}
function writePolicy(root, speculation) {
    const bindings = Object.fromEntries(['select','rebalance','extent','predict','launch','effect','match','dispose']
        .map(name => [name, {file:'decisions.py', function:name, ...(name === 'match' ? {inputs:['outcome']} : {})}]));
    bindings.admit = Object.fromEntries(['opportunity_pass','feasibility_pass'].map(name => [name, {file:'decisions.py',function:name}]));
    const expressions = {
        opportunity_pass: speculation ? "data['op']['name'] == 'edit' and data['op']['authority']" : 'False',
        feasibility_pass: 'True', select: "[chance['chance_id'] for chance in data['frontier_view']]", launch:'True',
        predict:`{'generation':'batch','proposals':[{'payload':${encode(toolResult)}}]}`,
        extent:"'continue'", effect:"{'mode':'allow'}", rebalance:'[]',
        match:`{'verdict':'compatible' if data['outcome']['payload'] == ${encode(toolResult)} else 'incompatible'}`,
        dispose:"{'actions':[{'intent':'Promote'}]} if data['resolution'] == 'accepted' else {'actions':[{'intent':'Discard'}]}",
    };
    fs.writeFileSync(join(root,'policy/decisions.py'), Object.entries(expressions).map(([name,expr]) => `def ${name}(data, *, policy_config, capabilities):\n    return ${expr}\n`).join(''));
    const policy = join(root,'policy/policy.yaml');
    fs.writeFileSync(policy, encode({version:1, language:'python', files:['decisions.py'], fidelity:'Best-effort', decisions:bindings}));
    return policy;
}
async function readSse(url, count, lastId) {
    const response = await fetch(`${url}/api/observation/events?after=0`, {
        headers:lastId ? {'Last-Event-ID':String(lastId)} : {}, signal:AbortSignal.timeout(5000),
    });
    assert.equal(response.status, 200);
    const reader = response.body.getReader();
    let buffer = '', events = [];
    try {
        while (events.length < count) {
            const {value,done} = await reader.read();
            assert.equal(done, false, `SSE closed after ${events.length} events with ${JSON.stringify(buffer.slice(0, 200))} buffered`);
            buffer += Buffer.from(value).toString();
            const frames = buffer.split('\n\n');
            buffer = frames.pop();
            events.push(...frames.filter(frame => frame.startsWith('id:')).map(frame => ({
                id: Number(/^id:\s*(\d+)/.exec(frame)[1]),
                data: JSON.parse(/^data:\s*(.*)$/m.exec(frame)[1]),
            })));
        }
        return events.slice(0, count);
    } finally { await reader.cancel(); }
}
async function traceById(id, predicate = () => true) {
    return until(async () => {
        const response = await fetch(`${jaeger}/api/traces/${id}`, { signal:AbortSignal.timeout(2000) });
        if (response.status === 404) return;
        assert.ok(response.ok, `Jaeger HTTP ${response.status}`);
        const trace = (await response.json()).data[0];
        if (trace && predicate(trace.spans)) return trace;
    }, `Jaeger trace ${id}`);
}
function verifyTrace(trace, ledger, fakeRequests, speculation) {
    const {spans, traceID} = trace;
    const byName = name => spans.filter(span => span.operationName === name);
    const root = byName('ascout.task');
    assert.equal(root.length, 1);
    assert.equal(tags(root[0]).result, 'succeeded');
    assert.equal(root[0].references.length, 0);
    const byId = new Map(spans.map(span => [span.spanID, span]));
    for (const span of spans) {
        if (span === root[0]) continue;
        const visited = new Set();
        let child = span;
        while (child !== root[0]) {
            assert.ok(!visited.has(child.spanID), `cycle at ${span.operationName}`);
            visited.add(child.spanID);
            const parent = child.references.find(ref => ref.refType === 'CHILD_OF');
            assert.ok(parent && parent.traceID === traceID && byId.has(parent.spanID), `missing parent of ${child.operationName}`);
            child = byId.get(parent.spanID);
        }
    }
    const services = new Set(Object.values(trace.processes).map(process => process.serviceName));
    assert.deepEqual(services, new Set(['ascout-control','ascout-pi','ascout-runtime']));
    const models = byName('ascout.model');
    const usage = ledger.map(row => row.observation).filter(row => row?.event === 'model_usage');
    const usageFor = (rows, key, get) => rows.reduce((sum,row) => sum + (get(row)[key] ?? 0), 0);
    const totals = {};
    for (const [ledgerKey, spanKey] of Object.entries({input_tokens:'gen_ai.usage.input_tokens', output_tokens:'gen_ai.usage.output_tokens', cache_read_tokens:'ascout.usage.cache_read_tokens', cache_write_tokens:'ascout.usage.cache_write_tokens', total_tokens:'ascout.usage.total_tokens', cost:'ascout.usage.cost'})) {
        totals[ledgerKey] = usageFor(models, spanKey, tags);
        assert.equal(totals[ledgerKey], usageFor(usage, ledgerKey, row => row));
    }
    assert.equal(models.length, fakeRequests);
    assert.equal(totals.total_tokens, 18 * fakeRequests);
    assert.ok(byName('ascout.tool').length >= 1);
    assert.ok(byName('ascout.runtime.tool_execute').some(span => {
        const rpc = byId.get(span.references[0].spanID);
        return byId.get(rpc.references[0].spanID).operationName === 'ascout.tool';
    }));
    const candidate = byName('ascout.branch.candidate');
    const dispositions = byName('ascout.policy.evaluate').filter(span => tags(span).decision === 'dispose' && tags(span).result === 'Promote');
    const applied = byName('ascout.agent.command').filter(span => tags(span).command === 'promote' && tags(span).result === 'applied');
    if (speculation) {
        assert.ok(candidate.some(span => tags(span).local === true));
        assert.ok(dispositions.length && applied.length);
        assert.ok(byName('ascout.policy.evaluate').some(span => tags(span).path === 'local_fast'));
    } else assert.equal(candidate.length + dispositions.length + applied.length, 0);
    assert.ok(!encode(trace).includes(marker), 'private fixture content leaked into trace');
    return {traceID, spans:spans.length, models:models.length, tools:byName('ascout.tool').length, candidate:candidate.length, dispositions:dispositions.length, applied:applied.length, ...totals};
}

async function scenario(mode) {
    const startedAt = Date.now();
    const speculation = mode === 'speculation';
    const paused = mode === 'resume' || mode === 'legacy';
    const root = fs.mkdtempSync(join(tmpdir(), `ascout-otel-${mode}-`));
    for (const dir of ['workspace','pi','policy']) fs.mkdirSync(join(root, dir));
    fs.writeFileSync(join(root,'workspace/a.txt'), `before\n`);
    const policy = writePolicy(root, speculation);
    let requests = 0, release;
    const waitForPause = new Promise(resolve => { release = resolve; });
    let child, exit, logs = '', collectorRequests = 0;
    const fake = http.createServer(async (req,res) => {
        const body = JSON.parse(Buffer.concat(await Array.fromAsync(req)).toString());
        requests++;
        const hasTool = body.messages.some(message => message.role === 'tool');
        if (paused && requests === 1) await waitForPause;
        const delta = hasTool ? {content:`done ${marker}`} : {tool_calls:[{index:0,id:'edit-1',type:'function',function:{name:'edit',arguments:encode({path:'a.txt',edits:[{oldText:'before',newText:`after ${marker}`}]})}}]};
        const chunk = (value, finish_reason) => ({id:'fixture', object:'chat.completion.chunk', created:1,
            model:body.model, choices:[{index:0,delta:value,finish_reason}]});
        const usage = {id:'fixture',object:'chat.completion.chunk',created:1,model:body.model,choices:[],usage:{prompt_tokens:13,completion_tokens:5,total_tokens:18,prompt_tokens_details:{cached_tokens:3}}};
        res.writeHead(200, {'content-type':'text/event-stream'});
        res.end([chunk(delta,null),chunk({},hasTool?'stop':'tool_calls'),usage].map(value => `data: ${encode(value)}\n\n`).join('') + 'data: [DONE]\n\n');
    });
    const collector = http.createServer((req) => { collectorRequests++; req.resume(); });
    const modelPort = await listen(fake);
    const [port,runtimePort] = await Promise.all([freePort(),freePort()]);
    const localEndpoint = mode === 'stalled' || mode === 'disabled' || mode === 'stopped' ? `http://127.0.0.1:${await listen(collector)}` : endpoint;
    if (mode === 'stopped') await new Promise(done => collector.close(done));
    const address = `127.0.0.1:${port}`, url = `http://${address}`;
    const models = join(root,'models.json'), runtime = join(root,'runtime.json');
    fs.writeFileSync(models, encode({providers:{fixture:{baseUrl:`http://127.0.0.1:${modelPort}/v1`,api:'openai-completions',apiKey:'fake',
        models:[{id:'agent',reasoning:false,input:['text'],contextWindow:32768,maxTokens:256,cost:{input:2,output:5,cacheRead:1,cacheWrite:1}}]}}}));
    fs.writeFileSync(runtime, encode({backend:'filesystem',control_address:`127.0.0.1:${runtimePort}`,adapter_instance:'otel-fixture',
        filesystem:{state_directory:join(root,'fs'),isolated_paths:[join(root,'workspace')],passthrough_rw:[root]}}));
    const rows = () => {
        try { return fs.readFileSync(runPath(root,'events.jsonl'),'utf8').split('\n').flatMap(line => { try { return [JSON.parse(line)]; } catch { return []; } }); }
        catch (error) { if (error.code === 'ENOENT') return []; throw error; }
    };
    const post = async (action, extra = {}) => assert.equal((await fetch(`${url}/api/request`, {method:'POST',signal:AbortSignal.timeout(5000),
        headers:{origin:url,'content-type':'application/json'}, body:encode({action,...extra})})).status, 202);
    async function start(resume = false) {
        const env = {...process.env, PI_CODING_AGENT_DIR:join(root,'pi'), ASCOUT_RUNTIME_CONFIG:runtime,
            OTEL_EXPORTER_OTLP_ENDPOINT:localEndpoint, OTEL_EXPORTER_OTLP_TRACES_ENDPOINT:'', OTEL_EXPORTER_OTLP_HEADERS:'',
            OTEL_SDK_DISABLED:mode === 'disabled' ? 'true' : 'false'};
        if (mode === 'no-endpoint' || (mode === 'legacy' && !resume)) delete env.OTEL_EXPORTER_OTLP_ENDPOINT;
        child = spawnControl(resolve('target/debug/ascout-control-plane'), [root,pi,policy,'--web',address,'--model','fixture/agent','--models',models,'--thinking','off',...(resume?['--resume']:[])], {
            env, stdio:['ignore','pipe','pipe'],
        });
        for (const stream of [child.stdout,child.stderr]) stream.on('data',data => { logs += data; });
        exit = once(child,'exit');
        await until(async () => {
            assert.equal(child.exitCode,null,logs);
            try { return (await fetch(`${url}/api/observation/state-event-set`)).ok && rows().some(row => row.message === (resume ? 'recovery_loaded' : 'branch_ready')); }
            catch { return false; }
        }, 'ready');
    }
    try {
        await start();
        await post('prompt',{prompt:`Edit a.txt ${marker}`});
        if (paused) {
            await until(() => requests === 1, 'active fake model');
            await post('pause');
            release();
            const saved = await until(() => rows().find(row => row.message === 'recovery_complete'), 'paused generation');
            const sidecar = join(dirname(saved.manifest.files['agent.json'].path),'trace-context.json');
            if (mode === 'legacy') assert.ok(!fs.existsSync(sidecar));
            else await until(() => fs.existsSync(sidecar), 'trace sidecar');
            child.kill('SIGKILL');
            await exit;
            await delay(500);
            await start(true);
            const before = await readSse(url, 4);
            assert.deepEqual(await readSse(url, 2, before[1].id), before.slice(2));
            await post('continue');
        }
        const completed = await until(() => rows().find(row => row.message === 'task_completed'), 'task completion');
        assert.equal(completed.status, 'succeeded');
        const replay = await readSse(url, rows().filter(row => row.observation).length);
        const action = replay.findIndex(event => event.data.event === 'action_started');
        assert.ok(action > 0, 'action observations must be replayable');
        assert.deepEqual(await readSse(url, 2, replay[action - 1].id), replay.slice(action, action + 2));
        const started = rows().find(row => row.message === 'task_started');
        assert.ok(started);
        const began = Date.now();
        await post('shutdown');
        const exitResult = await Promise.race([exit, delay(4500, null, {ref:false})]);
        assert.deepEqual(exitResult, [0,null], logs || 'shutdown exceeded 4500 ms');
        const shutdownMs = Date.now() - began;
        assert.ok(shutdownMs < 4500, `shutdown took ${shutdownMs}ms`);
        const otlpLogs = [logs,...fs.readdirSync(dirname(runPath(root,'events.jsonl'))).filter(file => file.endsWith('.log')).map(file => fs.readFileSync(join(dirname(runPath(root,'events.jsonl')),file),'utf8'))].join('\n');
        assert.ok(!rows().some(row => row.message?.startsWith('otel') || row.message?.startsWith('trace')));
        let result = {mode, requests, shutdownMs, root};
        if (mode === 'baseline' || speculation) {
            assert.match(started.trace_id, /^[0-9a-f]{32}$/);
            const trace = await traceById(started.trace_id, spans => spans.some(span => span.operationName === 'ascout.runtime.tool_execute') && spans.filter(span => span.operationName === 'ascout.model').length === requests);
            result = {...result,...verifyTrace(trace, rows(), requests, speculation)};
        } else if (mode === 'resume') {
            const trace = await traceById(started.trace_id, spans => spans.some(span => span.operationName === 'ascout.task.resume'));
            const span = trace.spans.find(span => span.operationName === 'ascout.task.resume');
            assert.equal(tags(span).result,'succeeded');
            assert.equal(span.references[0].traceID,started.trace_id);
            assert.ok(trace.spans.some(span => span.operationName === 'ascout.branch.authority' && tags(span).recovered === true));
            result.traceID = started.trace_id;
        } else if (mode === 'legacy') {
            assert.equal(started.trace_id,undefined);
            const trace = await until(async () => {
                const query = await fetch(`${jaeger}/api/traces?service=ascout-control&operation=ascout.task&limit=20`, {signal:AbortSignal.timeout(2000)});
                assert.ok(query.ok);
                const traces = (await query.json()).data;
                return traces.find(trace => trace.spans.some(span => span.operationName === 'ascout.task'
                    && span.startTime >= startedAt * 1000 && tags(span).resumed === true && tags(span).result === 'succeeded'));
            }, 'fresh trace for a legacy paused run');
            result.traceID = trace.traceID;
        } else if (mode === 'disabled' || mode === 'no-endpoint') {
            assert.equal(started.trace_id,undefined);
            assert.equal(collectorRequests,0);
            assert.doesNotMatch(otlpLogs,/OTLP ended=/);
        } else {
            assert.match(started.trace_id,/^[0-9a-f]{32}$/);
            assert.match(otlpLogs,/OTLP ended=\d+ exported=0 failed=[1-9]\d*/);
            if (mode === 'stalled') assert.ok(collectorRequests > 0);
        }
        console.log(encode(result));
    } catch (error) {
        console.error(`Fixture ${mode} at ${root}\n${logs}\n${encode(rows().slice(-8))}`);
        throw error;
    } finally {
        release();
        if (child && child.exitCode === null && child.signalCode === null) child.kill('SIGKILL');
        for (const server of [fake,collector]) { server.closeAllConnections(); server.close(); }
    }
}
const modes = process.argv.slice(2);
for (const mode of modes.length ? modes : ['baseline','speculation','stopped','stalled','disabled','no-endpoint','resume','legacy']) await scenario(mode);

import {createInterposer} from '../instrumentation/core/index.ts';
import {writeExecution} from '../instrumentation/pi/execution-wire.ts';
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, mkdir, writeFile, rm, symlink } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { createServer } from 'node:net';
import { spawn, spawnSync } from 'node:child_process';

const REAL_PI = '/home/betmul/BETMUL/AgentScout-suit/pi';

function readTree(file) {
    const script = `import { SessionManager } from '${REAL_PI}/packages/coding-agent/src/core/session-manager.ts'; const m=SessionManager.open(process.argv[1]); console.log(JSON.stringify({entries:m.getEntries(),messages:m.buildSessionContext().messages}));`;
    const result = spawnSync(process.execPath, ['--no-warnings', '--experimental-strip-types', '--import', `${REAL_PI}/packages/coding-agent/src/experimental/source-resolver.ts`, '--input-type=module', '-e', script, file], {encoding:'utf8'});
    assert.equal(result.status, 0, result.stderr);
    return JSON.parse(result.stdout);
}

for (const shutdownAt of ['completed', 'tool_wait', 'candidate_gate', 'admission', 'eof_tool_wait']) {
test(`runtime result commands and graceful shutdown at ${shutdownAt}`, async t => {
    const root = await mkdtemp(join(tmpdir(), 'ascout-pi-'));
    const socketPath = join(root, 'agent.sock');
    await mkdir(join(root, 'packages/coding-agent/src'), { recursive: true });
    await symlink(join(REAL_PI, 'packages/pi-integration'), join(root, 'packages/pi-integration'), 'dir');
    await mkdir(join(root, 'node_modules/typebox'), { recursive: true });
    await writeFile(join(root, 'package.json'), '{"type":"module"}');
    await writeFile(join(root, 'node_modules/typebox/index.js'), 'exports.Type={Unsafe:x=>x};');
    await writeFile(join(root, 'packages/coding-agent/src/index.ts'), `
 import { SessionManager } from '${REAL_PI}/packages/coding-agent/src/core/session-manager.ts';
 export { SessionManager };
 export const ModelRuntime = {}, resolveCliModel = () => ({});
export async function createAgentSession(options) {
    const tool = options.customTools.find(tool=>tool.name==='runtime_tool');
    const controller = new AbortController();
    let subscriber = () => {};
    const session = {
        async bindExtensions({uiContext}) {
            await uiContext.confirm('Startup', 'Load resources?');
            session.promptTemplates=[{name:'loaded',description:'after startup'}];
        },
        promptTemplates:[], resourceLoader:{getSkills:()=>({skills:[]})},
        getCommands() { if(session.catalogRead) throw Error('catalog read twice'); session.catalogRead=true; return session.promptTemplates; },
        extensionRunner:{hasHandlers:()=>true,async emit(event){if(event.type==='session_shutdown'){await new Promise(r=>setImmediate(r));options.sessionManager.appendCustomEntry('shutdown-final',{reason:event.reason})}}},
         agent: {}, sessionManager:options.sessionManager, messages: options.sessionManager ? options.sessionManager.buildSessionContext().messages : [], subscribe(fn) {subscriber=fn}, dispose() {}, async abort() {controller.abort()},
        async continue() { return session.prompt('candidate'); },
        async prompt() {
             const isCandidate = options.sessionManager?.buildSessionContext().messages.some(m => m.role === 'toolResult');
             const toolCall = {id:isCandidate?'operation-2':'operation-1',name:'runtime_tool'};
            const finishModelInference = await options.beforeModelInference?.(
                {provider:'test',id:'test'},
                {messages:session.messages},
                controller.signal,
            );
            const message={role:'assistant',content:[{type:'toolCall',...toolCall}],stopReason:'toolUse',model:'test',usage:{}};
            subscriber({type:'message_start',message});
            session.messages.push(message);
            subscriber({type:'message_end',message});
            options.sessionManager.appendMessage(message);
            await finishModelInference?.('succeeded');
            await session.agent.beforeToolCall({toolCall,args:{}}, controller.signal);
            const result = await tool.execute(toolCall.id, {}, controller.signal);
            await session.agent.afterToolCall({toolCall,result,isError:false});
            const toolResult={role:'toolResult',toolCallId:toolCall.id,toolName:toolCall.name,content:result.content,isError:false,timestamp:Date.now()};
            session.messages.push(toolResult);subscriber({type:'message_end',message:toolResult});options.sessionManager.appendMessage(toolResult);
        }
    };
    return {session};
}
`);
    const messages = [];
    let client;
    const server = createServer(socket => {
        client = socket;
        let bytes = Buffer.alloc(0);
        socket.on('data', chunk => {
            bytes = Buffer.concat([bytes, chunk]);
            while (bytes.length >= 4 && bytes.length >= bytes.readUInt32BE(0) + 4) {
                const length = bytes.readUInt32BE(0);
                messages.push(JSON.parse(bytes.subarray(4, length + 4)).payload);
                bytes = bytes.subarray(length + 4);
            }
        });
    });
    await new Promise(resolve => server.listen(socketPath, resolve));
    const executionPath=join(root,'execution.sock'),executions=[];
    const executionServer=createServer(socket=>{
        let input='';
        socket.on('data',async chunk=>{
            input+=chunk;
            const end=input.indexOf('\n');
            if(end<0)return;
            const request=JSON.parse(input.slice(0,end));
            input=input.slice(end+1);
            executions.push(request);
            if(request.command==='execute'){
                const stdout = Buffer.from(JSON.stringify({result:{payload:request.environment_id==='authority-environment'?'real-result':'candidate-real-result'},evidence:null}));
                await writeExecution(socket,{event:'accepted'});
                await writeExecution(socket,{event:'terminal',status:'succeeded',body_kind:'business_output',stdout_length:stdout.length,exit_code:0},[stdout]);
                socket.end();
            }
            else socket.end(JSON.stringify({event:request.command==='confirm'?'confirmed':'terminal',status:'cancelled'})+'\n');
        });
    });
    await new Promise(resolve=>executionServer.listen(executionPath,resolve));
    const authorityAccess={transport:'unix-json-lines',address:executionPath,environment_id:'authority-environment'};
    const candidateAccess={...authorityAccess,environment_id:'candidate-environment'};
    const child = spawn(process.execPath, ['--no-warnings', '--experimental-strip-types', '--import', `${REAL_PI}/packages/coding-agent/src/experimental/source-resolver.ts`, resolve('instrumentation/pi/main.ts'), socketPath, 'agent'], {
         env: { ...process.env, ASCOUT_PI_ROOT: root, ASCOUT_SESSION_FILE: join(root, 'session.jsonl'), ASCOUT_RUNTIME_TOOLS: JSON.stringify([
            {name:'runtime_tool',description:'test',parameters:{type:'object'}}
        ]) }, stdio: ['ignore', 'pipe', 'pipe']
    });
    let stderr = '';
    child.stderr.on('data', chunk => stderr += chunk);
    t.after(async () => {
        client?.destroy();
        if (child.exitCode === null) {
            const exited = new Promise(resolve => child.once('exit', resolve));
            child.kill();
            await exited;
        }
        await new Promise(resolve => server.close(resolve));
        await new Promise(resolve => executionServer.close(resolve));
        await rm(root, {recursive:true, force:true});
    });
    async function expectMessage(predicate) {
        const deadline = Date.now() + 2000;
        while (Date.now() < deadline) {
            const message = messages.find(predicate);
            if (message) return message;
            if (child.exitCode !== null) throw Error(stderr);
            await new Promise(resolve => setTimeout(resolve, 10));
        }
        assert.fail(`Timed out waiting for protocol message; received ${JSON.stringify(messages)}; ${stderr}`);
    }
    function send(payload) {
        const body = Buffer.from(JSON.stringify({kind:'command',payload}));
        const header = Buffer.alloc(4); header.writeUInt32BE(body.length);
        client.write(Buffer.concat([header, body]));
    }
    async function finish(branches, expectedShutdowns=branches.length) {
        for (const branch_id of branches) {
            send({command:'cleanup',branch_id,workspace:root,command_id:`cleanup-${branch_id}`,
                generation:branch_id==='authority'?1:2});
            await expectMessage(m => m.message === 'branch_released' && m.branch_id === branch_id);
            await expectMessage(m => m.message === 'agent_session_released' && m.branch_id === branch_id);
            await expectMessage(m => m.message === 'command_result' && m.command_id === `cleanup-${branch_id}` && m.status === 'applied');
            assert.deepEqual(messages.filter(m => m.message === 'command_result' && m.command_id === `cleanup-${branch_id}`).map(m => m.status), ['accepted','started','applied']);
        }
        const exited = new Promise(resolve => child.once('exit', (code, signal) => resolve({code,signal})));
        client.end();
        const exit = await Promise.race([exited, new Promise((_,reject) => {
            const timer=setTimeout(()=>reject(Error('integration did not exit after EOF')),2000);
            timer.unref();
        })]);
        assert.deepEqual(exit,{code:0,signal:null},stderr);
        assert.equal(stderr,'');
        const final=readTree(join(root,'session.jsonl'));
        assert.equal(final.entries.filter(e=>e.customType==='shutdown-final').length,expectedShutdowns);
        assert.ok(final.entries.filter(e=>e.customType==='shutdown-final').every(e=>e.data.reason==='quit'));
    }
    const connected = await expectMessage(m => m.message === 'agent_connected');
    assert.deepEqual(connected.capabilities, {
        preExecutionInterception: true,
        localDecisionEvaluation: true,
        modelInferencePreExecutionInterception: true,
        subagentPreExecutionInterception: false,
        checkpointContinuation: true,
        independentCandidateSession: true,
        cancelCompletion: true,
        toolResultInjection: true,
    });
    {
        const runner = 'import sys,json\nfor line in sys.stdin.buffer:\n payload=json.dumps({"type":"result","request_id":json.loads(line)["request_id"],"result":True}).encode()\n sys.stdout.buffer.write(len(payload).to_bytes(4,"big")+payload)\n sys.stdout.buffer.flush()\n';
        send({command:'local_policy',branch_id:'authority',policy_digest:'test-policy',has_predictor:true,decisions:{
            'admit.opportunity_pass':'import json,sys\nfor line in sys.stdin.buffer:\n request=json.loads(line)\n payload=json.dumps({"type":"result","request_id":request["request_id"],"result":request["data"]["op"]["kind"]!="model_inference"}).encode()\n sys.stdout.buffer.write(len(payload).to_bytes(4,"big")+payload)\n sys.stdout.buffer.flush()\n','admit.feasibility_pass':runner,'extent':runner,'effect':runner}});
    }
    send({command:'start_branch',branch_id:'authority',workspace:root,prompt:'',environment_id:authorityAccess.environment_id,execution_access:authorityAccess});
    await expectMessage(m => m.message === 'branch_started');
    const startup=await expectMessage(m=>m.message==='ui_prompt_requested'&&m.branch_id==='authority');
    assert.equal(messages.some(m=>m.message==='commands_listed'),false);
    send({command:'ui_prompt_answer',branch_id:'authority',prompt_id:startup.prompt_id,answer:true,cancelled:false});
    const catalog=await expectMessage(m=>m.message==='commands_listed'&&m.request_id==='startup');
    assert.equal(catalog.commands[0].name,'loaded');
    for(const request_id of ['slash-1','slash-2']) {
        send({command:'get_commands',branch_id:'authority',request_id});
        const reply=await expectMessage(m=>m.message==='commands_listed'&&m.request_id===request_id);
        assert.deepEqual(reply.commands,catalog.commands);
    }
    send({command:'prompt',branch_id:'authority',prompt:'one tool call'});
    const inference = await expectMessage(m => m.message === 'action_started' && m.action_kind === 'model_inference' && m.branch_id === 'authority');
    const inferred = await expectMessage(m => m.message === 'action_finished' && m.action_id === inference.action_id);
    assert.equal(inferred.status, 'succeeded');
    if (shutdownAt === 'admission') {
        const admission = await expectMessage(m => m.message === 'speculative_admission_request');
        assert.equal(admission.policy_digest, 'test-policy');
        assert.equal(typeof admission.timestamp_ms, 'number');
        return finish(['authority']);
    }
    const admission = await expectMessage(m => m.message === 'speculative_admission_request');
    const pending={...admission,action_id:'operation-1'};
    assert.equal(executions.length,0,'backend must wait for launch authorization');
    if (shutdownAt === 'eof_tool_wait') return finish([],1);
    if (shutdownAt === 'tool_wait') return finish(['authority']);
    // Vary JSON key order to match Rust and check that candidate setup leaves the queue free.
    send({command:'start_candidate',branch_id:'candidate',parent_branch_id:'authority',
        workspace:root,operation_id:'operation-1',tool_name:'runtime_tool',prediction:'guess',
        checkpoint:{adapter:pending.checkpoint.adapter,kind:pending.checkpoint.kind,opaqueId:pending.checkpoint.opaqueId},
        generation:2,command_id:'start-candidate',environment_id:candidateAccess.environment_id,execution_access:candidateAccess});
    await expectMessage(m => m.message === 'branch_started' && m.branch_id === 'candidate');
    const candidateStartup=await expectMessage(m=>m.message==='ui_prompt_requested'&&m.branch_id==='candidate');
    send({command:'ui_prompt_answer',branch_id:'candidate',prompt_id:candidateStartup.prompt_id,answer:true,cancelled:false});
    if (shutdownAt === 'candidate_gate') return finish(['candidate','authority']);
    send({command:'resolve_pending',branch_id:'authority',operation_id:'operation-1',
        command_id:'resolve-1',generation:1,outcome:{phase:'execute'}});
    const outcome = await expectMessage(m => m.message === 'action_finished' && m.action_kind === 'tool_execution');
    assert.match(outcome.result.payload, /real-result/);
    assert.equal(outcome.status, 'succeeded');
    assert.equal(outcome.action_id, pending.action_id);
    await expectMessage(m => m.message === 'turn_completed');
    await expectMessage(m => m.message === 'command_result' && m.status === 'applied');
    send({command:'promote',branch_id:'candidate',command_id:'promote-1',generation:2});
    await expectMessage(m => m.message === 'speculative_admission_request' && m.branch_id === 'candidate');
    send({command:'resolve_pending',branch_id:'candidate',operation_id:'operation-2',
        command_id:'resolve-2',generation:2,outcome:{phase:'execute'}});
    await expectMessage(m => m.message === 'action_finished' && m.action_kind === 'tool_execution' && m.branch_id === 'candidate');
    await expectMessage(m => m.message === 'turn_completed' && m.branch_id === 'candidate');
    if (shutdownAt === 'completed') {
        const treeFile = join(root, 'session.jsonl');
        const tree = readTree(treeFile);
        const transcripts = messages.filter(m => m.message === 'transcript');
        assert.ok(transcripts.length >= 3);
        for (const transcript of transcripts) {
            assert.equal(typeof transcript.session_entry_id, 'string');
            const entry = tree.entries.find(e => e.id === transcript.session_entry_id);
            assert.deepEqual(entry?.message, transcript.entry);
        }
        const authorityTranscript = transcripts.find(m => m.branch_id === 'authority' && m.entry.content?.some(p => p.type === 'toolCall' && p.id === 'operation-1'));
        const paired = transcripts.find(m => m.branch_id === 'candidate' && m.entry.role === 'toolResult');
        assert.ok(authorityTranscript && paired);
        const authorityEntry = tree.entries.find(e => e.id === authorityTranscript.session_entry_id);
        const pairedEntry = tree.entries.find(e => e.id === paired.session_entry_id);
        assert.equal(pairedEntry.parentId, authorityEntry.id);
        const marker = tree.entries.at(-1);
        assert.equal(marker.customType, 'authority');
        assert.equal(marker.data.branch_id, 'candidate');
        const candidateTail = transcripts.filter(m => m.branch_id === 'candidate').at(-1);
        assert.equal(marker.data.parentId, candidateTail.session_entry_id);
        assert.equal(marker.parentId, candidateTail.session_entry_id);
        assert.deepEqual(tree.messages.map(m => m.role), ['assistant', 'toolResult', 'assistant', 'toolResult']);
        const resumedResult=tree.messages.find(m=>m.role==='toolResult');
        assert.match(resumedResult.content[0].text,/real-result/);
        for (const transcript of transcripts) assert.ok(tree.entries.some(e => e.id === transcript.session_entry_id));
        const authorityRecords = transcripts.filter(m => m.branch_id === 'authority');
        assert.ok(authorityRecords.length >= 1);
        for (const record of authorityRecords) assert.ok(tree.entries.some(e => e.id === record.session_entry_id));
    }
    await finish(['candidate','authority']);
});
}

test('model usage survives completion before message_end and reports an unpaired call', async t => {
    const root = await mkdtemp(join(tmpdir(), 'ascout-pi-'));
    const socketPath = join(root, 'agent.sock');
    await mkdir(join(root, 'packages/coding-agent/src'), { recursive: true });
    await symlink(join(REAL_PI, 'packages/pi-integration'), join(root, 'packages/pi-integration'), 'dir');
    await mkdir(join(root, 'node_modules/typebox'), { recursive: true });
    await writeFile(join(root, 'package.json'), '{"type":"module"}');
    await writeFile(join(root, 'node_modules/typebox/index.js'), 'exports.Type={Unsafe:x=>x};');
    // Pi completes the hook before message_end. The second prompt bypasses the hook.
     await writeFile(join(root, 'packages/coding-agent/src/index.ts'), `
 import { SessionManager } from '${REAL_PI}/packages/coding-agent/src/core/session-manager.ts';
 export { SessionManager };
 export const ModelRuntime = {}, resolveCliModel = () => ({});
export async function createAgentSession(options) {
    const controller = new AbortController();
    let subscriber = () => {};
    const session = {
        async bindExtensions() {},
        agent: {}, messages: [], subscribe(fn) {subscriber=fn}, dispose() {}, async abort() {controller.abort()},
        async prompt(prompt) {
            const use = prompt === 'unpaired'
                ? {input:1,output:2,cacheRead:0,cacheWrite:0,totalTokens:3,cost:{total:0.25}}
                : {input:10,output:20,cacheRead:0,cacheWrite:0,totalTokens:30,cost:{total:1.5}};
            if (prompt === 'unpaired') {
                const message = {role:'assistant',content:[{type:'text',text:'unpaired'}],stopReason:'stop',model:'test',usage:use,timestamp:Date.now()};
                subscriber({type:'message_start',message});
                session.messages.push(message);
                subscriber({type:'message_end',message});
                return;
            }
            const started = Date.now();
            const finishModelInference = await options.beforeModelInference?.(
                {provider:'test',id:'test'},
                {messages:session.messages},
                controller.signal,
            );
            const message = {role:'assistant',content:[{type:'text',text:'paired'}],stopReason:'stop',model:'test',usage:use,timestamp:started,durationMs:12};
            subscriber({type:'message_start',message});
            session.messages.push(message);
            await finishModelInference?.('succeeded');
            subscriber({type:'message_end',message});
        }
    };
    return {session};
}
`);
    const messages = [];
    let client;
    const server = createServer(socket => {
        client = socket;
        let bytes = Buffer.alloc(0);
        socket.on('data', chunk => {
            bytes = Buffer.concat([bytes, chunk]);
            while (bytes.length >= 4 && bytes.length >= bytes.readUInt32BE(0) + 4) {
                const length = bytes.readUInt32BE(0);
                messages.push(JSON.parse(bytes.subarray(4, length + 4)).payload);
                bytes = bytes.subarray(length + 4);
            }
        });
    });
    await new Promise(resolve => server.listen(socketPath, resolve));
    const child = spawn(process.execPath, ['--no-warnings', '--experimental-strip-types', '--import', `${REAL_PI}/packages/coding-agent/src/experimental/source-resolver.ts`, resolve('instrumentation/pi/main.ts'), socketPath, 'agent'], {
         env: { ...process.env, ASCOUT_PI_ROOT: root, ASCOUT_SESSION_FILE: join(root, 'session.jsonl'), ASCOUT_RUNTIME_TOOLS: '[]' }, stdio: ['ignore', 'pipe', 'pipe']
    });
    let stderr = '';
    child.stderr.on('data', chunk => stderr += chunk);
    t.after(async () => {
        client?.destroy();
        if (child.exitCode === null) {
            const exited = new Promise(resolve => child.once('exit', resolve));
            child.kill();
            await exited;
        }
        await new Promise(resolve => server.close(resolve));
        await rm(root, {recursive:true, force:true});
    });
    async function expectMessage(predicate) {
        const deadline = Date.now() + 2000;
        while (Date.now() < deadline) {
            const message = messages.find(predicate);
            if (message) return message;
            if (child.exitCode !== null) throw Error(stderr);
            await new Promise(resolve => setTimeout(resolve, 10));
        }
        assert.fail(`Timed out waiting for protocol message; received ${JSON.stringify(messages)}; ${stderr}`);
    }
    function send(payload) {
        const body = Buffer.from(JSON.stringify({kind:'command',payload}));
        const header = Buffer.alloc(4); header.writeUInt32BE(body.length);
        client.write(Buffer.concat([header, body]));
    }
    await expectMessage(m => m.message === 'agent_connected');
    send({command:'start_branch',branch_id:'authority',workspace:root,prompt:''});
    await expectMessage(m => m.message === 'branch_started' && m.branch_id === 'authority');
    send({command:'prompt',branch_id:'authority',prompt:'paired'});
    const inference = await expectMessage(m => m.message === 'action_started' && m.action_kind === 'model_inference' && m.branch_id === 'authority');
    const paired = await expectMessage(m => m.message === 'usage' && m.branch_id === 'authority' && m.action_id === inference.action_id);
    assert.equal(paired.started_at_ms + 12, paired.finished_at_ms);
    assert.equal(paired.usage.input, 10);
    assert.equal(paired.usage.totalTokens, 30);
    assert.equal(paired.usage.cost.total, 1.5);
    send({command:'prompt',branch_id:'authority',prompt:'unpaired'});
    const unpaired = await expectMessage(m => m.message === 'usage' && m.branch_id === 'authority' && m.action_id === null);
    assert.equal(unpaired.model, 'test');
    assert.equal(unpaired.usage.totalTokens, 3);
    assert.equal(unpaired.usage.cost.total, 0.25);
    send({command:'cleanup',branch_id:'authority',workspace:root,command_id:'cleanup-authority',generation:1});
    await expectMessage(m => m.message === 'branch_released' && m.branch_id === 'authority');
    const exited = new Promise(resolve => child.once('exit', (code, signal) => resolve({code,signal})));
    client.end();
    const exit = await Promise.race([exited, new Promise((_,reject) => {
        const timer=setTimeout(()=>reject(Error('integration did not exit after EOF')),2000);
        timer.unref();
    })]);
    assert.deepEqual(exit,{code:0,signal:null},stderr);
    assert.equal(stderr,'');
});

for (const injected of [false, true]) test(`round counting survives recovery (${injected ? 'injected tools' : 'model and tools'})`, async () => {
    const inputs = [];
    const options = {
        taskId: 'rounds', report: async () => {},
        evaluate: async (_, decision, {process_state}) => {
            assert.equal(decision, 'extent');
            inputs.push(process_state);
            return process_state.semantic_facts.yield_only && process_state.phase === 'model_inference' ? 'stop' : 'continue';
        },
    };
    let access = createInterposer(options);
    access.setPolicy('authority', {digest: 'test', decisions: {}, decision_inputs: {
        extent: ['process_state.semantic_facts', 'process_state.resource_counters'],
    }});
    await access.control({command: 'start_candidate', branch_id: 'child', parent_branch_id: 'authority',
        prediction: injected ? {kind: 'predicted_tool_calls'} : {predict: 'branch_model'}});
    const point = (id, kind, signal) => ({branch: 'child', actionId: id, actionKind: kind, name: 'read',
        op: {}, admit: false, signal});
    const completed = (id, kind, hasToolCalls) => access.afterAction({branch: 'child', actionId: id,
        actionKind: kind, name: 'read', status: 'succeeded', isError: false, hasToolCalls});
    if (!injected) {
        await access.beforeAction(point('model-1', 'model_inference'));
        completed('model-1', 'model_inference', true);
    }
    // Release the effect hold for the two simulated tool hooks while retaining candidate progress.
    const state = access.snapshot();
    state.effectHolds = [];
    access = createInterposer(options);
    access.restore(state);
    await access.beforeAction(point('tool-1', 'tool_execution'));
    completed('tool-1', 'tool_execution');
    await access.control({command: 'resolve_pending', branch_id: 'authority', outcome: {yield_candidate: 'child'}});
    const saved = access.snapshot();
    access = createInterposer(options);
    access.restore(saved);
    await access.beforeAction(point('tool-2', 'tool_execution'));
    completed('tool-2', 'tool_execution');
    const abort = new AbortController();
    const stopped = access.beforeAction(point('model-2', 'model_inference', abort.signal));
    const rejected = assert.rejects(stopped, /Extent stop cancelled/);
    await new Promise(resolve => setImmediate(resolve));
    abort.abort();
    await rejected;
    assert.deepEqual(inputs.slice(-3).map(state => [state.steps_completed, state.semantic_facts.yield_only]),
        [[0, false], [0, true], [1, true]]);
    assert.deepEqual(inputs.at(-1).resource_counters, {model_inferences: Number(!injected), tool_executions: 2, failed_tools: 0});

    completed('text', 'model_inference', false);
    access.completeTurn('child');
    assert.equal(access.snapshot().progress.find(([id]) => id === 'child')[1].state.steps_completed, 2);
});

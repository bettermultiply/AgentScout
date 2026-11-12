import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, mkdir, writeFile, rm } from 'node:fs/promises';
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
    await mkdir(join(root, 'node_modules/typebox'), { recursive: true });
    await writeFile(join(root, 'package.json'), '{"type":"module"}');
    await writeFile(join(root, 'node_modules/typebox/index.js'), 'exports.Type={Unsafe:x=>x};');
    // A deterministic SDK double: one turn waits for a real runtime result.
    await writeFile(join(root, 'packages/coding-agent/src/index.ts'), `
 import { SessionManager } from '${REAL_PI}/packages/coding-agent/src/core/session-manager.ts';
 export { SessionManager };
 export const ModelRuntime = {}, resolveCliModel = () => ({});
export async function createAgentSession(options) {
    const tool = options.customTools[0];
    const controller = new AbortController();
    let subscriber = () => {};
    const session = {
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
    const child = spawn(process.execPath, ['--no-warnings', '--experimental-strip-types', '--import', `${REAL_PI}/packages/coding-agent/src/experimental/source-resolver.ts`, resolve('agent-access/pi/main.ts'), socketPath, 'agent'], {
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
    if (shutdownAt === 'admission') {
        const runner = 'import sys\npayload=b"true"\nsys.stdout.buffer.write(len(payload).to_bytes(4,"big")+payload)\n';
        send({command:'local_policy',branch_id:'authority',policy_digest:'test-policy',decisions:{
            'admit.opportunity_pass':runner,'admit.feasibility_pass':runner,'extent':runner}});
    }
    send({command:'start_branch',branch_id:'authority',workspace:root,prompt:''});
    await expectMessage(m => m.message === 'branch_started');
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
    const pending = await expectMessage(m => m.message === 'action_started' && m.action_kind === 'tool_execution');
    assert.equal(pending.action_id, 'operation-1');
    assert.equal(pending.result_pending, true);
    if (shutdownAt === 'eof_tool_wait') return finish([],1);
    if (shutdownAt === 'tool_wait') return finish(['authority']);
    // Rust's JSON encoder sorts object keys; checkpoint identity must not depend
    // on insertion order. Candidate setup also must not hold the command queue.
    send({command:'start_candidate',branch_id:'candidate',parent_branch_id:'authority',
        workspace:root,operation_id:'operation-1',tool_name:'runtime_tool',prediction:'guess',
        checkpoint:{adapter:pending.checkpoint.adapter,kind:pending.checkpoint.kind,opaqueId:pending.checkpoint.opaqueId},
        generation:2,command_id:'start-candidate'});
    await expectMessage(m => m.message === 'branch_started' && m.branch_id === 'candidate');
    if (shutdownAt === 'candidate_gate') return finish(['candidate','authority']);
    send({command:'resolve_pending',branch_id:'authority',operation_id:'operation-1',
        command_id:'resolve-1',generation:1,outcome:{payload:'real-result'}});
    const outcome = await expectMessage(m => m.message === 'action_finished' && m.action_kind === 'tool_execution');
    assert.match(outcome.result.payload, /real-result/);
    assert.equal(outcome.status, 'succeeded');
    assert.equal(outcome.action_id, pending.action_id);
    await expectMessage(m => m.message === 'turn_completed');
    await expectMessage(m => m.message === 'command_result' && m.status === 'applied');
    send({command:'promote',branch_id:'candidate',command_id:'promote-1',generation:2});
    await expectMessage(m => m.message === 'action_started' && m.action_kind === 'tool_execution' && m.branch_id === 'candidate');
    send({command:'resolve_pending',branch_id:'candidate',operation_id:'operation-2',
        command_id:'resolve-2',generation:2,outcome:{payload:'candidate-real-result'}});
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
        // Every reported record stays in the file, including the Authority's own
        // records that are not on the promoted path.
        for (const transcript of transcripts) assert.ok(tree.entries.some(e => e.id === transcript.session_entry_id));
        const authorityRecords = transcripts.filter(m => m.branch_id === 'authority');
        assert.ok(authorityRecords.length >= 1);
        for (const record of authorityRecords) assert.ok(tree.entries.some(e => e.id === record.session_entry_id));
    }
    // Closing the controller's write half must let the integration drain its
    // output and exit without an unhandled rejected write.
    await finish(['candidate','authority']);
});
}

test('model usage survives completion before message_end and reports an unpaired call', async t => {
    const root = await mkdtemp(join(tmpdir(), 'ascout-pi-'));
    const socketPath = join(root, 'agent.sock');
    await mkdir(join(root, 'packages/coding-agent/src'), { recursive: true });
    await mkdir(join(root, 'node_modules/typebox'), { recursive: true });
    await writeFile(join(root, 'package.json'), '{"type":"module"}');
    await writeFile(join(root, 'node_modules/typebox/index.js'), 'exports.Type={Unsafe:x=>x};');
    // Pi settles the model stream and invokes the beforeModelInference completion
    // callback before the agent loop emits message_end. A second prompt reports a
    // model message that never went through the hook at all.
     await writeFile(join(root, 'packages/coding-agent/src/index.ts'), `
 import { SessionManager } from '${REAL_PI}/packages/coding-agent/src/core/session-manager.ts';
 export { SessionManager };
 export const ModelRuntime = {}, resolveCliModel = () => ({});
export async function createAgentSession(options) {
    const controller = new AbortController();
    let subscriber = () => {};
    const session = {
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
    const child = spawn(process.execPath, ['--no-warnings', '--experimental-strip-types', '--import', `${REAL_PI}/packages/coding-agent/src/experimental/source-resolver.ts`, resolve('agent-access/pi/main.ts'), socketPath, 'agent'], {
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

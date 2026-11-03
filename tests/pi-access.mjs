import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, mkdir, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { createServer } from 'node:net';
import { spawn } from 'node:child_process';

for (const shutdownAt of ['completed', 'tool_wait', 'candidate_gate']) {
test(`runtime result commands and graceful shutdown at ${shutdownAt}`, async t => {
    const root = await mkdtemp(join(tmpdir(), 'ascout-pi-'));
    const socketPath = join(root, 'agent.sock');
    await mkdir(join(root, 'packages/coding-agent/src'), { recursive: true });
    await mkdir(join(root, 'node_modules/typebox'), { recursive: true });
    await writeFile(join(root, 'package.json'), '{"type":"module"}');
    await writeFile(join(root, 'node_modules/typebox/index.js'), 'exports.Type={Unsafe:x=>x};');
    // A deterministic SDK double: one turn waits for a real runtime result.
    await writeFile(join(root, 'packages/coding-agent/src/index.ts'), `
export const SessionManager = {inMemory:()=>({messages:[],appendMessage(m){this.messages.push(m)}})}, ModelRuntime = {}, resolveCliModel = () => ({});
export async function createAgentSession(options) {
    const tool = options.customTools[0];
    const controller = new AbortController();
    const session = {
        agent: {}, messages: options.sessionManager?.messages ?? [], subscribe() {}, dispose() {}, async abort() {controller.abort()},
        async continue() { return session.prompt('candidate'); },
        async prompt() {
            const toolCall = {id:options.sessionManager?'operation-2':'operation-1',name:'runtime_tool'};
            session.messages.push({role:'assistant',content:[{type:'toolCall',...toolCall}]});
            await session.agent.beforeToolCall({toolCall,args:{}}, controller.signal);
            const result = await tool.execute(toolCall.id, {}, controller.signal);
            await session.agent.afterToolCall({toolCall,result,isError:false});
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
    const child = spawn(process.execPath, [resolve('agent-access/pi/main.ts'), socketPath, 'agent'], {
        env: { ...process.env, ASCOUT_PI_ROOT: root, ASCOUT_RUNTIME_TOOLS: JSON.stringify([
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
    async function finish(branches) {
        for (const branch_id of branches) {
            send({command:'cleanup',branch_id,workspace:root,command_id:`cleanup-${branch_id}`,
                generation:branch_id==='authority'?1:2});
            await expectMessage(m => m.message === 'cleanup_completed' && m.branch_id === branch_id);
            await expectMessage(m => m.message === 'command_result' && m.command_id === `cleanup-${branch_id}` && m.status === 'applied');
        }
        const exited = new Promise(resolve => child.once('exit', (code, signal) => resolve({code,signal})));
        client.end();
        const exit = await Promise.race([exited, new Promise((_,reject) => {
            const timer=setTimeout(()=>reject(Error('integration did not exit after EOF')),2000);
            timer.unref();
        })]);
        assert.deepEqual(exit,{code:0,signal:null},stderr);
        assert.equal(stderr,'');
    }
    await expectMessage(m => m.message === 'hello');
    send({command:'start_branch',branch_id:'authority',workspace:root,prompt:''});
    await expectMessage(m => m.message === 'branch_started');
    send({command:'prompt',branch_id:'authority',prompt:'one tool call'});
    const pending = await expectMessage(m => m.message === 'tool_pending');
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
    const outcome = await expectMessage(m => m.message === 'parent_outcome');
    assert.match(outcome.outcome.payload, /real-result/);
    await expectMessage(m => m.message === 'turn_completed');
    await expectMessage(m => m.message === 'command_result' && m.status === 'applied');
    send({command:'promote',branch_id:'candidate',command_id:'promote-1',generation:2});
    await expectMessage(m => m.message === 'tool_pending' && m.branch_id === 'candidate');
    send({command:'resolve_pending',branch_id:'candidate',operation_id:'operation-2',
        command_id:'resolve-2',generation:2,outcome:{payload:'candidate-real-result'}});
    await expectMessage(m => m.message === 'parent_outcome' && m.branch_id === 'candidate');
    await expectMessage(m => m.message === 'turn_completed' && m.branch_id === 'candidate');
    // Closing the controller's write half must let the integration drain its
    // output and exit without an unhandled rejected write.
    await finish(['candidate','authority']);
});
}

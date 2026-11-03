/** AScout-owned integration using Pi's public SDK, without Pi UI internals. */
// This is a protocol boundary: Pi's SDK types are intentionally loaded from
// the selected checkout at runtime, so the Rust-side wire schema remains the
// compile-time contract for this small launcher.
// @ts-nocheck
import { connect } from 'node:net';
import { pathToFileURL } from 'node:url';
import { resolve } from 'node:path';

const root = process.env.ASCOUT_PI_ROOT;
if (!root) throw Error('ASCOUT_PI_ROOT is required');
// Public package entry. The source resolver supports the local Pi checkout.
const { createAgentSession, SessionManager, ModelRuntime, resolveCliModel } = await import(pathToFileURL(resolve(root, 'packages/coding-agent/src/index.ts')).href);
const { createRequire } = await import('node:module');
const require = createRequire(resolve(root, 'package.json'));
const { Type } = require('typebox');
const args = process.argv.slice(2);
const socketPath = args.shift();
const agentId = args.shift() ?? 'agent';
if (!socketPath) throw Error('usage: main.ts <socket> <agent> [--model provider/model] [--thinking level]');
let modelPattern, thinkingLevel = 'low';
while (args.length) {
    const flag = args.shift(), value = args.shift();
    if (!value) throw Error(`${flag} requires a value`);
    if (flag === '--model') modelPattern = value;
    else if (flag === '--thinking') thinkingLevel = value;
    else throw Error(`Unknown argument ${flag}`);
}
if (!['off','minimal','low','medium','high','xhigh'].includes(thinkingLevel)) throw Error('Invalid thinking level');
let modelRuntime, model;
if (modelPattern) {
    modelRuntime = await ModelRuntime.create();
    const resolved = resolveCliModel({ cliModel:modelPattern, cliThinking:thinkingLevel, modelRuntime });
    if (resolved.error || !resolved.model) throw Error(resolved.error ?? 'Unknown model');
    model = resolved.model;
}
const sessions = new Map(), checkpoints = new Map(), results = new Map(), waiters = new Map(), resultWaiters = new Map();
const cancelled = new Set(), gates = new Map(), generations = new Map(), commands = new Map();
const runtimeTools = JSON.parse(process.env.ASCOUT_RUNTIME_TOOLS ?? '[]');
let authority = 'authority', closing = false, outgoing = Promise.resolve();
const socket = connect(socketPath);
await new Promise((resolve,reject)=>{socket.once('connect',resolve);socket.once('error',reject)});
const report = payload => {
    const bytes = Buffer.from(JSON.stringify({kind:'message',payload})), header=Buffer.alloc(4);
    header.writeUInt32BE(bytes.length);
    outgoing=outgoing.then(()=>new Promise((resolve,reject)=>socket.write(Buffer.concat([header,bytes]),error=>error?reject(error):resolve())));
    return outgoing;
};
const key = (branch, operation) => JSON.stringify([branch,operation]);
const text = value => typeof value==='string'?value:JSON.stringify(value);
function tools(branch) {
    return runtimeTools.map(spec=>({name:spec.name,label:spec.name,description:spec.description,parameters:Type.Unsafe(spec.parameters),execute:async(operation,_args,signal)=>{
        if (branch!==authority) throw Error('Candidate action is unavailable before acceptance');
        const id=key(branch,operation);
        let value;
        if(results.has(id)){value=results.get(id);results.delete(id)}
        else value=await new Promise((resolve,reject)=>{
            const abort=()=>{waiters.delete(id);reject(Error('Runtime operation cancelled'))};
            waiters.set(id,value=>{signal?.removeEventListener('abort',abort);resolve(value)});
            signal?.addEventListener('abort',abort,{once:true});
            if(signal?.aborted||closing)abort();
        });
        return {content:[{type:'text',text:text(value)}],details:undefined};
    }}));
}
function install(session,branch) {
    const before=session.agent.beforeToolCall, after=session.agent.afterToolCall;
    session.agent.beforeToolCall=async(context,signal)=>{
        const gate=gates.get(branch);
        if(gate)await new Promise((resolve,reject)=>{
            const abort=()=>reject(Error('Candidate cancelled'));
            signal?.addEventListener('abort',abort,{once:true});
            gate.promise.then(resolve,reject).finally(()=>signal?.removeEventListener('abort',abort));
            if(signal?.aborted)abort();
        });
        const previous=await before?.(context,signal);
        if(previous?.block||signal?.aborted||branch!==authority)return previous;
        const saved=structuredClone(session.messages.filter(m=>m.role!=='custom'));
        const checkpoint={kind:'checkpoint',adapter:'pi',opaqueId:`${branch}:${context.toolCall.id}:${saved.length}`};
        checkpoints.set(key(branch,context.toolCall.id),{saved,checkpoint});
        await report({message:'tool_pending',branch_id:branch,operation_id:context.toolCall.id,tool_name:context.toolCall.name,arguments:context.args,checkpoint});
        return previous;
    };
    session.agent.afterToolCall=async(context,signal)=>{
        const previous=await after?.(context,signal);
        const content=previous?.content??context.result.content,isError=previous?.isError??context.isError;
        const result={role:'toolResult',toolCallId:context.toolCall.id,toolName:context.toolCall.name,content:structuredClone(content),isError,timestamp:Date.now()};
        const checkpoint=checkpoints.get(key(branch,context.toolCall.id));
        if(checkpoint){
            checkpoint.result=result;
            const resultKey=key(branch,context.toolCall.id);
            for(const resolve of resultWaiters.get(resultKey)??[])resolve(result);
            resultWaiters.delete(resultKey);
            await report({message:'parent_outcome',branch_id:branch,operation_id:context.toolCall.id,outcome:{payload:content.length===1&&content[0].type==='text'?content[0].text:null,content,is_error:isError}})
        }
        return previous;
    };
    session.subscribe(event=>{
        if(event.type==='message_update'&&event.assistantMessageEvent.type==='text_delta')void report({message:'output',branch_id:branch,text:event.assistantMessageEvent.delta});
        if(event.type==='message_end'){
            void report({message:'transcript',branch_id:branch,branch_role:branch===authority?'authority':'candidate',entry:event.message});
            if(event.message.role==='assistant')void report({message:'usage',branch_id:branch,model:event.message.model,thinking_level:event.message.thinkingLevel,usage:event.message.usage});
        }
    });
}
async function run(session,branch,operation) {
    await operation();
    const last=[...session.messages].reverse().find(m=>m.role==='assistant');
    if(last?.stopReason==='error')await report({message:'branch_failed',branch_id:branch,reason:last.errorMessage??'Model failed'});
    await report({message:'turn_completed',branch_id:branch});
}
// A turn may wait for a later control command (runtime result, Promote or
// Abort). Keep setup ordered, but never hold the command queue for the turn.
function executeTurn(session,branch,operation) {
    const task=run(session,branch,operation)
        .catch(error=>report({message:'branch_failed',branch_id:branch,reason:String(error instanceof Error?error.message:error)}));
    tasks.add(task);
    void task.finally(()=>tasks.delete(task)).catch(()=>socket.destroy());
}
async function handle(command) {
    const branch=command.branch_id;
    switch(command.command){
        case 'start_branch': {
            const {session}=await createAgentSession({cwd:command.workspace,model,modelRuntime,thinkingLevel,customTools:tools(branch)});
            if(closing||cancelled.has(branch)){session.dispose();return}
            sessions.set(branch,session);install(session,branch);
            await report({message:'branch_started',branch_id:branch});
            if(command.prompt)executeTurn(session,branch,()=>session.prompt(command.prompt));
            return;
        }
        case 'prompt': {
            if(branch!==authority)throw Error('Only the authority accepts user input');
            const session=sessions.get(branch);if(!session)throw Error('Session missing');
            executeTurn(session,branch,()=>session.prompt(command.prompt,{streamingBehavior:'followUp'}));return;
        }
        case 'start_candidate': {
            const parent=sessions.get(command.parent_branch_id),checkpoint=checkpoints.get(key(command.parent_branch_id,command.operation_id));
            if(!parent||!checkpoint||checkpoint.checkpoint.adapter!==command.checkpoint?.adapter||checkpoint.checkpoint.kind!==command.checkpoint?.kind||checkpoint.checkpoint.opaqueId!==command.checkpoint?.opaqueId)throw Error('Candidate requires its registered checkpoint');
            const messages=structuredClone(checkpoint.saved),last=messages.at(-1);
            if(last?.role!=='assistant'||!last.content.some(p=>p.type==='toolCall'&&p.id===command.operation_id&&p.name===command.tool_name))throw Error('Invalid pending checkpoint');
            const paired: any[]=[];
            for(const part of last.content){
                if(part.type!=='toolCall')continue;
                if(part.id===command.operation_id)paired.push({role:'toolResult',toolCallId:part.id,toolName:part.name,content:[{type:'text',text:text(command.prediction)}],isError:false,timestamp:Date.now()});
                else {
                    const sibling=checkpoints.get(key(command.parent_branch_id,part.id))?.result;
                    if(sibling) paired.push(structuredClone(sibling));
                    else paired.push(await new Promise((resolve,reject)=>{
                        const resultKey=key(command.parent_branch_id,part.id);
                        const pending=resultWaiters.get(resultKey)??[];
                        pending.push(resolve);
                        resultWaiters.set(resultKey,pending);
                        if(closing||cancelled.has(command.branch_id))reject(Error('Candidate cancelled while waiting for sibling result'));
                    }));
                }
            }
            const manager=SessionManager.inMemory(command.workspace);
            for(const message of [...messages,...paired])manager.appendMessage(message);
            let release;const promise=new Promise(resolve=>release=resolve);gates.set(branch,{promise,release});
            const {session}=await createAgentSession({cwd:command.workspace,sessionManager:manager,model:parent.model,modelRuntime:parent.modelRuntime,thinkingLevel:parent.thinkingLevel,customTools:tools(branch)});
            if(closing||cancelled.has(branch)){session.dispose();return}
            sessions.set(branch,session);install(session,branch);
            // Injected result is new candidate content, not copied from the Parent.
            for(const entry of paired)await report({message:'transcript',branch_id:branch,branch_role:'candidate',entry});
            await report({message:'branch_started',branch_id:branch});
            executeTurn(session,branch,()=>session.continue());return;
        }
        case 'resolve_pending': {
            if(command.outcome!==undefined){const id=key(branch,command.operation_id),waiter=waiters.get(id);if(waiter){waiters.delete(id);waiter(command.outcome)}else results.set(id,command.outcome)}
            return;
        }
        case 'promote':
            if(cancelled.has(branch)||!sessions.has(branch))throw Error('Branch unavailable');
            authority=branch;gates.get(branch)?.release();gates.delete(branch);return;
        case 'abort': return sessions.get(branch)?.abort();
        case 'discard': cancelled.add(branch);return sessions.get(branch)?.abort();
        case 'cleanup':
            cancelled.add(branch);await sessions.get(branch)?.abort();sessions.get(branch)?.dispose();sessions.delete(branch);gates.delete(branch);
            for(const id of checkpoints.keys())if(JSON.parse(id)[0]===branch)checkpoints.delete(id);
            await report({message:'cleanup_completed',branch_id:branch});return;
        default:throw Error(`Unknown command ${command.command}`);
    }
}
async function dispatch(command){
    const operation=['promote','resolve_pending','discard','cleanup'].includes(command.command)?command.command:undefined;
    let reason;
    try{
        const generation=generations.get(command.branch_id);
        if(generation!==undefined&&command.generation!==undefined&&generation!==command.generation)throw Error('Stale command generation');
        if(command.generation!==undefined)generations.set(command.branch_id,command.generation);
        await handle(command);
    }catch(error){reason=String(error instanceof Error?error.message:error)}
    if(operation&&command.command_id)return {message:'command_result',command_id:command.command_id,branch_id:command.branch_id,operation,status:reason?'failed':'applied',generation:command.generation??0,...(command.operation_id?{operation_id:command.operation_id}:{}),...(reason?{reason}:{})};
    if(reason)return {message:'branch_failed',branch_id:command.branch_id,reason};
}
await report({message:'hello',agent_id:agentId});
let buffer=Buffer.alloc(0);const tasks=new Set();
let commandQueue: Promise<unknown>=Promise.resolve();
try{
    for await(const chunk of socket){
        buffer=Buffer.concat([buffer,chunk]);
        while(buffer.length>=4){const length=buffer.readUInt32BE(0);if(length>16777216)throw Error('Protocol frame too large');if(buffer.length<length+4)break;
            const wire=JSON.parse(buffer.subarray(4,length+4).toString());buffer=buffer.subarray(length+4);if(wire.kind!=='command')throw Error('Unexpected message direction');
            const command=wire.payload;
            const id=command.command_id;
            const old=id?commands.get(id):undefined;
            if(old&&old.serialized!==JSON.stringify(command)){await report({message:'branch_failed',branch_id:command.branch_id,reason:'Command id reused with different payload'});continue}
            const result=old?.result??(commandQueue=commandQueue.then(()=>dispatch(command)));
            if(id&&!old)commands.set(id,{serialized:JSON.stringify(command),result});
            const task=result.then(reply=>reply?report(reply):undefined);tasks.add(task);void task.finally(()=>tasks.delete(task)).catch(()=>socket.destroy());
        }
    }
}finally{
    closing=true;for(const gate of gates.values())gate.release();
    await Promise.allSettled([...sessions.values()].map(s=>s.abort()));
    await Promise.allSettled(tasks);for(const session of sessions.values())session.dispose();
}

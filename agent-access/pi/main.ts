/** AScout-owned integration using Pi's public SDK, without Pi UI internals. */
// This is a protocol boundary: Pi's SDK types are intentionally loaded from
// the selected checkout at runtime, so the Rust-side wire schema remains the
// compile-time contract for this small launcher.
// @ts-nocheck
import { connect } from 'node:net';
import { createRequire } from 'node:module';
import { resolve } from 'node:path';
import { evaluatePolicyArtifact } from '../../../pi/packages/ascout-agent-access/src/local-policy.ts';
import { createAgentAccess } from '../core/index.ts';
import { SessionTree } from './session-tree.ts';
import { requestKey } from '../../tools/replay/normalize.mjs';
import { Notifications } from './notification.ts';

const root = process.env.ASCOUT_PI_ROOT;
if (!root) throw Error('ASCOUT_PI_ROOT is required');
const require = createRequire(resolve(root, 'package.json'));
const { createAgentSession, SessionManager, ModelRuntime, resolveCliModel } = require(resolve(root, 'packages/coding-agent/src/index.ts'));
const sessionFile = process.env.ASCOUT_SESSION_FILE;
if (!sessionFile) throw Error('ASCOUT_SESSION_FILE is required');
const tree = new SessionTree(SessionManager.open(sessionFile));
const { Type } = require('typebox');
const args = process.argv.slice(2);
const socketPath = args.shift();
const agentId = args.shift() ?? 'agent';
const taskId = process.env.ASCOUT_TASK_ID ?? '';
let nextAction = 1;
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
tree.initialize(model,thinkingLevel);
const sessions = new Map(), checkpoints = new Map(), results = new Map(), waiters = new Map(), resultWaiters = new Map();
const cancelled = new Set(), generations = new Map(), commands = new Map();
const activeActions = new Map(), reportedActions = new Set();
const aborting = new Set();
const modelActions = new Map();
const transcriptQueues = new Map();
const replayKeys = new Map();
const reportedEntries = new Set();
const candidateWaiters = new Map();
const branchTasks = new Map();
const setupTasks = new Map();
const notifications = new Notifications();
async function stopSession(branch) {
    const session=sessions.get(branch);
    await session?.abort();
    await Promise.allSettled([...(branchTasks.get(branch)??[])]);
    // dispose() is synchronous and does not emit session_shutdown. Await the
    // native extension lifecycle while its append context is still valid.
    if(session?.extensionRunner?.hasHandlers('session_shutdown'))
        await session.extensionRunner.emit({type:'session_shutdown',reason:'quit'});
    await session?.waitForIdle?.();
    if(session)await notifications.drain(session);
    syncTranscript(branch);
    session?.dispose();sessions.delete(branch);modelActions.delete(branch);
    tree.stop(branch);tree.settle();
}
function releaseCandidateWaiters(branch) {
    for(const reject of candidateWaiters.get(branch)??[])reject(Error('Candidate cancelled while waiting for sibling result'));
    candidateWaiters.delete(branch);
}
function syncTranscript(branch) {
    const manager=tree.branches.get(branch)?.manager;
    if(!manager)return;
    notifications.confirm(manager);
    const queue=transcriptQueues.get(branch)??[];
    const pending=[];
    for(const item of [...queue]){
        const entry=manager.getEntries().find(e=>{
            if(reportedEntries.has(e.id))return false;
            if(e.type==='custom_message'&&item.message.role==='custom')return e.customType===item.message.customType&&JSON.stringify(e.content)===JSON.stringify(item.message.content)&&JSON.stringify(e.details)===JSON.stringify(item.message.details);
            return e.type==='message'&&(e.message===item.message||JSON.stringify(e.message)===JSON.stringify(item.message));
        });
        if(!entry)continue;
        queue.splice(queue.indexOf(item),1);
        reportedEntries.add(entry.id);
        if(item.message.role==='assistant'){
            const request_key=replayKeys.get(branch)?.shift();
            if(request_key)tree.requests.set(entry.id,request_key);
        }
        tree.tails.set(branch,manager.getLeafId());
        pending.push({message:'transcript',branch_id:branch,entry:item.message,session_entry_id:entry.id});
    }
    tree.settle();
    for(const payload of pending)void report(payload);
}
const runtimeTools = JSON.parse(process.env.ASCOUT_RUNTIME_TOOLS ?? '[]');
const capabilities = {
    preExecutionInterception:true,
    localDecisionEvaluation:true,
    modelInferencePreExecutionInterception:true,
    subagentPreExecutionInterception:false,
    checkpointContinuation:true,
    independentCandidateSession:true,
    cancelCompletion:true,
    toolResultInjection:true,
};
let authority = tree.authority, closing = false, outgoing = Promise.resolve();
const socket = connect({path:socketPath,allowHalfOpen:true});
await new Promise((resolve,reject)=>{socket.once('connect',resolve);socket.once('error',reject)});
let transportError;
socket.on('error',error=>{transportError=error;socket.destroy()});
const report = payload => {
    if(payload.message==='action_started'){activeActions.set(key(payload.branch_id,payload.action_id),payload);reportedActions.add(key(payload.branch_id,payload.action_id));}
    if(payload.message==='action_finished')activeActions.delete(key(payload.branch_id,payload.action_id));
    payload={source:'agent_adapter',...payload};
    if(socket.destroyed)return Promise.resolve();
    const bytes = Buffer.from(JSON.stringify({kind:'message',payload})), header=Buffer.alloc(4);
    header.writeUInt32BE(bytes.length);
    outgoing=outgoing.then(()=>new Promise((resolve,reject)=>{
        if(socket.destroyed){resolve();return}
        socket.write(Buffer.concat([header,bytes]),error=>error?reject(error):resolve());
    })).catch(error=>{transportError=error;socket.destroy()});
    return outgoing;
};
const access = createAgentAccess({taskId,report,evaluate:evaluatePolicyArtifact});
const key = (branch, operation) => JSON.stringify([branch,operation]);
const text = value => typeof value==='string'?value:JSON.stringify(value);
async function finishActions(branch,status) {
    for(const action of [...activeActions.values()])if(action.branch_id===branch)
        await report({message:'action_finished',task_id:taskId,branch_id:branch,action_id:action.action_id,action_kind:action.action_kind,status,result:null,timestamp_ms:Date.now()});
}
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
async function beforeModelInference(branch, modelValue, context, signal) {
    syncTranscript(branch);
    const keys=replayKeys.get(branch)??[];keys.push(requestKey(context.messages));replayKeys.set(branch,keys);
    // Model inference is a PURE step: the generic layer evaluates `admit` but
    // never applies the effect gate, reports an admission, or counts extent.
    if(closing||cancelled.has(branch)||signal?.aborted)throw Error('Model inference cancelled before execution');
    const op={kind:'model_inference',produces_result:true,result_pending:true,has_predictor:false};
    await access.beforeAction({
        branch,
        actionId:'model',
        actionKind:'model_inference',
        name:'model',
        arguments:{model:modelValue.provider+'/'+modelValue.id,message_count:context.messages.length},
        op,
        contractContext:{model:`${modelValue.provider}/${modelValue.id}`,message_count:context.messages.length,runtime_capabilities:runtimeTools},
        admit:true,
        signal,
    });
    if(closing||cancelled.has(branch)||signal?.aborted)throw Error('Model inference cancelled before execution');
    const actionId=`model-${nextAction++}`, startedAt=Date.now();
    const queue=modelActions.get(branch)??[];
    const action={action_id:actionId,started_at_ms:startedAt,seen_message:false};
    queue.push(action);
    modelActions.set(branch,queue);
    await report({message:'action_started',task_id:taskId,branch_id:branch,action_id:actionId,action_kind:'model_inference',result_pending:true,name:'model',arguments:{model:modelValue.provider+'/'+modelValue.id,message_count:context.messages.length},timestamp_ms:startedAt});
    return async status=>{
        // Pi settles the stream and runs this callback before the agent loop
        // emits the assistant message_end. Keep the action while its message
        // has started so message_end can still pair the usage. Only forget an
        // action that produced no message at all (the request threw before the
        // stream started), so it cannot be paired with a later message.
        if(!action.seen_message){
            const pending=modelActions.get(branch),index=pending?.indexOf(action)??-1;
            if(index>=0)pending.splice(index,1);
            if(pending?.length===0)modelActions.delete(branch);
        }
        await report({message:'action_finished',task_id:taskId,branch_id:branch,action_id:actionId,action_kind:'model_inference',status:status??(signal?.aborted?'cancelled':'succeeded'),result:null,timestamp_ms:Date.now()});
    };
}
function install(session,branch) {
    const before=session.agent.beforeToolCall, after=session.agent.afterToolCall;
    session.agent.beforeToolCall=async(context,signal)=>{
        const previous=await before?.(context,signal);
        if(previous?.block||signal?.aborted)return previous;
        let saved, checkpoint;
        const prepareCheckpoint=()=>{
            if(checkpoint)return checkpoint;
            syncTranscript(branch);
            saved=structuredClone(session.messages);
            checkpoint={kind:'checkpoint',adapter:'pi',opaqueId:`${branch}:${context.toolCall.id}:${saved.length}`};
            const treeTail=tree.assertCheckpoint(branch,saved);
            checkpoints.set(key(branch,context.toolCall.id),{saved,checkpoint,treeTail,manager:tree.branches.get(branch).manager});
            return checkpoint;
        };
        await access.beforeAction({
            branch,
            actionId:context.toolCall.id,
            actionKind:'tool_execution',
            name:context.toolCall.name,
            arguments:context.args,
            op:{kind:context.toolCall.name,produces_result:true,result_pending:true,has_predictor:true},
            contractContext:{arguments:context.args,runtime_capabilities:runtimeTools},
            admit:()=>branch===authority,
            admissionFacts:()=>({checkpoint:prepareCheckpoint()}),
            signal,
        });
        if(signal?.aborted||branch!==authority)return previous;
        const toolCheckpoint=prepareCheckpoint();
        await report({message:'action_started',task_id:taskId,branch_id:branch,action_id:context.toolCall.id,action_kind:'tool_execution',result_pending:true,name:context.toolCall.name,arguments:context.args,checkpoint:toolCheckpoint,timestamp_ms:Date.now()});
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
            if(reportedActions.delete(key(branch,context.toolCall.id)))await report({message:'action_finished',task_id:taskId,branch_id:branch,action_id:context.toolCall.id,action_kind:'tool_execution',status:signal?.aborted?'cancelled':isError?'failed':'succeeded',result:{payload:content.length===1&&content[0].type==='text'?content[0].text:null,content,is_error:isError},timestamp_ms:Date.now()})
        }
        return previous;
    };
    session.subscribe(event=>{
        if(event.type==='message_start'&&event.message.role==='assistant'){
            // The completion callback can run before message_end, so the
            // message itself marks which queued action it belongs to.
            const action=modelActions.get(branch)?.find(action=>!action.seen_message);
            if(action)action.seen_message=true;
        }
        if(event.type==='message_update'&&event.assistantMessageEvent.type==='text_delta')void report({message:'output',branch_id:branch,text:event.assistantMessageEvent.delta});
        if(event.type==='message_end'){
            const queue=transcriptQueues.get(branch)??[];
            queue.push({message:event.message});transcriptQueues.set(branch,queue);
            queueMicrotask(()=>syncTranscript(branch));
            if(event.message.role==='assistant'){
                const pending=modelActions.get(branch),index=pending?.findIndex(action=>action.seen_message)??-1;
                const action=index>=0?pending.splice(index,1)[0]:undefined;
                if(pending?.length===0)modelActions.delete(branch);
                const started_at_ms=Number.isFinite(event.message.timestamp)?event.message.timestamp:(action?.started_at_ms??Date.now());
                const duration_ms=Number.isFinite(event.message.durationMs)?event.message.durationMs:Math.max(0,Date.now()-started_at_ms);
                // Usage is never dropped: a model call that cannot be paired
                // with an action reports a null action_id instead of inventing one.
                void report({message:'usage',branch_id:branch,action_id:action?.action_id??null,started_at_ms,finished_at_ms:started_at_ms+duration_ms,model:event.message.model,thinking_level:event.message.thinkingLevel,usage:event.message.usage??{}});
            }
        }
    });
}
async function run(session,branch,operation) {
    try { await operation(); }
    finally { syncTranscript(branch);await finishActions(branch,cancelled.has(branch)||aborting.has(branch)?'cancelled':'failed'); }
    const last=[...session.messages].reverse().find(m=>m.role==='assistant');
    if(last?.stopReason==='error')await report({message:'branch_failed',branch_id:branch,reason:last.errorMessage??'Model failed'});
    await report({message:'turn_completed',branch_id:branch});
    if(branch===authority)tree.settle();
}
// A turn may wait for a later control command (runtime result, Promote or
// Abort). Keep setup ordered, but never hold the command queue for the turn.
function executeTurn(session,branch,operation) {
    const task=run(session,branch,operation)
        .catch(error=>report({message:'branch_failed',branch_id:branch,reason:String(error instanceof Error?error.message:error)}));
    tasks.add(task);
    const running=branchTasks.get(branch)??new Set();running.add(task);branchTasks.set(branch,running);
    void task.finally(()=>{running.delete(task);if(!running.size)branchTasks.delete(branch)}).catch(()=>socket.destroy());
    void task.finally(()=>tasks.delete(task)).catch(()=>socket.destroy());
}
async function handle(command) {
    const branch=command.branch_id;
    // Hold bookkeeping (local Policy, effect/pause/extent gates) belongs to the
    // generic access layer; each product only connects its own execution points.
    if(command.command!=='start_candidate')await access.control(command);
    switch(command.command){
        case 'local_policy':
            return;
        case 'start_branch': {
            const manager=tree.createBranch(branch,command.workspace,tree.manager,tree.tail(branch)??tree.manager.getLeafId());
            const {session}=await createAgentSession({cwd:command.workspace,sessionManager:manager,model,modelRuntime,thinkingLevel,customTools:tools(branch),beforeModelInference:(modelValue,context,signal)=>beforeModelInference(branch,modelValue,context,signal)});
            if(closing||cancelled.has(branch)){sessions.set(branch,session);await stopSession(branch);return}
            sessions.set(branch,session);install(session,branch);
            await report({message:'branch_created',branch_id:branch});
            await report({message:'branch_started',branch_id:branch});
            await report({message:'branch_ready',branch_id:branch});
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
            const manager=tree.createBranch(branch,command.workspace,checkpoint.manager,checkpoint.treeTail);
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
                         const rejects=candidateWaiters.get(branch)??new Set();
                         const done=value=>{rejects.delete(cancel);resolve(value)};
                         const cancel=()=>{const index=pending.indexOf(done);if(index>=0)pending.splice(index,1);reject(Error('Candidate cancelled while waiting for sibling result'))};
                         rejects.add(cancel);candidateWaiters.set(branch,rejects);
                         pending.push(done);
                        resultWaiters.set(resultKey,pending);
                        if(closing||cancelled.has(command.branch_id))reject(Error('Candidate cancelled while waiting for sibling result'));
                    }));
                }
            }
            const pairedEntryIds=paired.map(entry=>tree.append(branch,entry));
            tree.inputs.set(branch,{parent:command.parent_branch_id,source:checkpoint.manager,operation:command.operation_id,checkpoint:checkpoint.treeTail,prediction:pairedEntryIds[paired.findIndex(e=>e.toolCallId===command.operation_id)],pairedTail:pairedEntryIds.at(-1),operations:paired.map(e=>e.toolCallId)});
            tree.settle();
            await access.control(command);
            const {session}=await createAgentSession({cwd:command.workspace,sessionManager:manager,model:parent.model,modelRuntime:parent.modelRuntime,thinkingLevel:parent.thinkingLevel,customTools:tools(branch),beforeModelInference:(modelValue,context,signal)=>beforeModelInference(branch,modelValue,context,signal)});
            if(closing||cancelled.has(branch)){sessions.set(branch,session);await stopSession(branch);return}
            sessions.set(branch,session);install(session,branch);
            await report({message:'branch_created',branch_id:branch});
            // Injected result is new candidate content, not copied from the Parent.
            for(let i=0;i<paired.length;i++)await report({message:'transcript',branch_id:branch,entry:paired[i],session_entry_id:pairedEntryIds[i]});
            await report({message:'branch_started',branch_id:branch});
            await report({message:'branch_ready',branch_id:branch});
            executeTurn(session,branch,()=>session.continue());return;
        }
        case 'pause':
        case 'resume':
            return;
        case 'resolve_pending': {
            if(command.outcome===undefined)return;
            const id=key(branch,command.operation_id),waiter=waiters.get(id);if(waiter){waiters.delete(id);waiter(command.outcome)}else results.set(id,command.outcome);
            return;
        }
        case 'promote':
            if(cancelled.has(branch)||!sessions.has(branch))throw Error('Branch unavailable');
            tree.accepted??=new Set();tree.accepted.add(branch);
            authority=branch;tree.promote(branch);tree.settle();
            if(command.notification!==undefined){
                try {
                    await notifications.accept(sessions.get(branch),command);
                    syncTranscript(branch);
                } catch(error) {
                    // Promote is already applied. Do not turn a notification
                    // persistence failure into a false Promote failure, and do
                    // not emit a delivery record for a message not accepted by Pi.
                    console.error(`Notification persistence failed for ${command.command_id}:`,error);
                    await report({message:'adapter_capability_gap',branch_id:branch,capability:'notification',status:'failed',reason:`${command.command_id}: ${error}`});
                }
            }
            return;
        case 'abort':
            aborting.add(branch);
            try { await sessions.get(branch)?.abort();await finishActions(branch,'cancelled'); }
            finally { aborting.delete(branch); }
            return;
        case 'discard': cancelled.add(branch);releaseCandidateWaiters(branch);await sessions.get(branch)?.abort();await finishActions(branch,'cancelled');tree.settle();return;
        case 'cleanup':
            cancelled.add(branch);releaseCandidateWaiters(branch);
            await setupTasks.get(branch)?.catch(()=>{});
            await stopSession(branch);
            await finishActions(branch,'cancelled');
            for(const id of checkpoints.keys())if(JSON.parse(id)[0]===branch)checkpoints.delete(id);
            await report({message:'agent_session_released',branch_id:branch});
            await report({message:'branch_released',branch_id:branch});tree.settle();return;
        default:throw Error(`Unknown command ${command.command}`);
    }
}
async function dispatch(command){
    const operation=['promote','resolve_pending','discard','cleanup'].includes(command.command)?command.command:undefined;
    let reason;
    const acknowledgement=status=>({message:'command_result',command_id:command.command_id,branch_id:command.branch_id,operation,status,generation:command.generation??0,...(command.operation_id?{operation_id:command.operation_id}:{})});
    try{
        const generation=generations.get(command.branch_id);
        if(generation!==undefined&&command.generation!==undefined&&generation!==command.generation)throw Error('Stale command generation');
        if(command.generation!==undefined)generations.set(command.branch_id,command.generation);
        if(operation&&command.command_id){await report(acknowledgement('accepted'));await report(acknowledgement('started'))}
        await handle(command);
    }catch(error){reason=String(error instanceof Error?error.message:error)}
    if(operation&&command.command_id)return {message:'command_result',command_id:command.command_id,branch_id:command.branch_id,operation,status:reason?'failed':'applied',generation:command.generation??0,...(command.operation_id?{operation_id:command.operation_id}:{}),...(reason?{reason}:{})};
    if(reason)return {message:'branch_failed',branch_id:command.branch_id,reason};
}
await report({message:'agent_connected',agent_id:agentId,capabilities});
await report({message:'agent_ready',agent_id:agentId});
let buffer=Buffer.alloc(0);const tasks=new Set();
let commandQueue: Promise<unknown>=Promise.resolve();
try{
    // Do not let the async iterator's clean EOF destroy the socket with a
    // synthetic AbortError while the writable half is still ending. Keep the
    // explicit half-close below as the only shutdown path.
    for await(const chunk of socket.iterator({destroyOnReturn:false})){
        buffer=Buffer.concat([buffer,chunk]);
        while(buffer.length>=4){const length=buffer.readUInt32BE(0);if(length>16777216)throw Error('Protocol frame too large');if(buffer.length<length+4)break;
            const wire=JSON.parse(buffer.subarray(4,length+4).toString());buffer=buffer.subarray(length+4);if(wire.kind!=='command')throw Error('Unexpected message direction');
            const command=wire.payload;
            const id=command.command_id;
            const old=id?commands.get(id):undefined;
            if(old&&old.serialized!==JSON.stringify(command)){await report({message:'branch_failed',branch_id:command.branch_id,reason:'Command id reused with different payload'});continue}
            // A candidate may wait for sibling tool results. It must not hold
            // up resolve_pending / cleanup on the transport's command queue.
            const result=old?.result??(command.command==='start_candidate'
                ?commandQueue.then(()=>dispatch(command))
                :(commandQueue=commandQueue.then(()=>dispatch(command))));
            if(command.command==='start_candidate'){
                setupTasks.set(command.branch_id,result);
                void result.finally(()=>{if(setupTasks.get(command.branch_id)===result)setupTasks.delete(command.branch_id)}).catch(()=>{});
            }
            if(id&&!old)commands.set(id,{serialized:JSON.stringify(command),result});
            const task=result.then(reply=>reply?report(reply):undefined);tasks.add(task);void task.finally(()=>tasks.delete(task)).catch(()=>socket.destroy());
        }
    }
}catch(error){
    transportError??=error;
}finally{
    closing=true;access.releaseAll();
    for(const branch of candidateWaiters.keys())releaseCandidateWaiters(branch);
    await commandQueue.catch(()=>{});
    await Promise.allSettled([...sessions.values()].map(s=>s.abort()));
    await Promise.allSettled(tasks);
    for(const branch of [...tree.branches.keys()]){
        try{await stopSession(branch);await finishActions(branch,'cancelled')}
        catch(error){
            transportError??=error;
            console.error(`Session shutdown failed for ${branch}:`,error);
            await report({message:'agent_failed',agent_id:agentId,reason:`Session shutdown failed for ${branch}: ${error}`});
        }
    }
    tree.settle();
    if(!transportError)await report({message:'agent_released',agent_id:agentId});
    await outgoing;
    if(!socket.destroyed)await new Promise(resolve=>socket.end(resolve));
    // An aborted local-policy evaluation can leave the evaluator child's stdio
    // handles open after it rejects; those handles would otherwise keep this
    // adapter alive even though every report is flushed and the socket is
    // closed. Shutdown is complete here, so terminate explicitly.
    if(transportError)console.error('Agent control connection failed:',transportError);
    process.exit(transportError?1:0);
}

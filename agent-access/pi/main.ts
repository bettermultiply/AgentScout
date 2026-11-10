/** AScout-owned integration using Pi's public SDK, without Pi UI internals. */
// This is a protocol boundary: Pi's SDK types are intentionally loaded from
// the selected checkout at runtime, so the Rust-side wire schema remains the
// compile-time contract for this small launcher.
// @ts-nocheck
import { connect } from 'node:net';
import { createRequire } from 'node:module';
import { resolve } from 'node:path';
import { evaluatePolicyArtifact } from '../../../pi/packages/ascout-agent-access/src/local-policy.ts';

const root = process.env.ASCOUT_PI_ROOT;
if (!root) throw Error('ASCOUT_PI_ROOT is required');
const require = createRequire(resolve(root, 'package.json'));
const { createAgentSession, SessionManager, ModelRuntime, resolveCliModel } = require(resolve(root, 'packages/coding-agent/src/index.ts'));
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
const sessions = new Map(), checkpoints = new Map(), results = new Map(), waiters = new Map(), resultWaiters = new Map();
const cancelled = new Set(), gates = new Map(), generations = new Map(), commands = new Map();
const activeActions = new Map(), reportedActions = new Set();
const aborting = new Set();
const policies = new Map(), hookCounts = new Map(), countedHooks = new Set(), stopGates = new Map(), pauseGates = new Map();
const modelActions = new Map();
const runtimeTools = JSON.parse(process.env.ASCOUT_RUNTIME_TOOLS ?? '[]');
async function evaluatePolicy(artifact, decision, data, signal) {
    return evaluatePolicyArtifact(artifact, decision, data, signal);
}
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
let authority = 'authority', closing = false, outgoing = Promise.resolve();
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
async function localDecision(branch, operationId, toolCallId, toolName, args, checkpoint, signal) {
    const artifact = policies.get(branch);
    if (!artifact) return {kind:'allow'};
    const input={op:{kind:toolName,produces_result:true,result_pending:true,has_predictor:true},state:{},contract_context:{arguments:args,runtime_capabilities:runtimeTools}};
    const opportunity=await evaluatePolicy(artifact,'admit.opportunity_pass',{op:input.op},signal);
    if (opportunity !== true) return {kind:'allow'};
    const feasibility=await evaluatePolicy(artifact,'admit.feasibility_pass',input,signal);
    return feasibility === true ? {kind:'admit_speculation'} : {kind:'allow'};
}
async function beforeModelInference(branch, modelValue, context, signal) {
    // Model inference is a PURE step; only external effects need the effect gate.
    await waitPause(branch,signal);
    if(closing||cancelled.has(branch)||signal?.aborted)throw Error('Model inference cancelled before execution');
    const artifact=policies.get(branch);
    if(artifact){
        const op={kind:'model_inference',produces_result:true,result_pending:true,has_predictor:false};
        const opportunity=await evaluatePolicy(artifact,'admit.opportunity_pass',{op},signal);
        if(opportunity===true)await evaluatePolicy(artifact,'admit.feasibility_pass',{op,state:{},contract_context:{model:`${modelValue.provider}/${modelValue.id}`,message_count:context.messages.length,runtime_capabilities:runtimeTools}},signal);
    }
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
// A Branch that has not been accepted yet may not produce effects. The `effect`
// Decision is evaluated at the boundary; while the gate holds, the Branch is
// reported as blocked by the effect gate.
async function effectGate(branch, descriptor, message, signal) {
    const gate=gates.get(branch);
    if(!gate)return;
    const artifact=policies.get(branch);
    if(artifact){
        const plan=await evaluatePolicy(artifact,'effect',{effect_descriptor:descriptor,context:{branch_id:branch}},signal);
        if(plan?.mode==='deny')throw Error('Effect denied by Policy');
    }
    await report({message:'branch_suspended',branch_id:branch,reason:'effect_gate'});
    await new Promise((resolve,reject)=>{
        const abort=()=>reject(Error(message));
        gate.promise.then(resolve,reject).finally(()=>signal?.removeEventListener('abort',abort));
        signal?.addEventListener('abort',abort,{once:true});
        if(signal?.aborted)abort();
    });
    await report({message:'branch_resumed',branch_id:branch});
}
// `rebalance` pause: hold the Branch at its next boundary until resumed.
async function waitPause(branch, signal) {
    const gate=pauseGates.get(branch);
    if(!gate)return;
    await report({message:'branch_suspended',branch_id:branch,reason:'rebalanced'});
    await new Promise((resolve,reject)=>{
        const abort=()=>reject(Error('Branch cancelled while paused'));
        gate.promise.then(resolve,reject).finally(()=>signal?.removeEventListener('abort',abort));
        signal?.addEventListener('abort',abort,{once:true});
        if(signal?.aborted)abort();
    });
    await report({message:'branch_resumed',branch_id:branch});
}
async function waitStop(branch, signal) {
    const gate=stopGates.get(branch);
    if (!gate) return;
    await new Promise((resolve,reject)=>{
        const abort=()=>reject(Error('Extent stop cancelled'));
        gate.promise.then(resolve,reject).finally(()=>signal?.removeEventListener('abort',abort));
        signal?.addEventListener('abort',abort,{once:true});
        if(signal?.aborted)abort();
    });
}
function install(session,branch) {
    const before=session.agent.beforeToolCall, after=session.agent.afterToolCall;
    session.agent.beforeToolCall=async(context,signal)=>{
        await effectGate(branch,{kind:'tool_execution',tool:context.toolCall.name,arguments:context.args},'Candidate cancelled',signal);
        await waitPause(branch,signal);
        const previous=await before?.(context,signal);
        if(previous?.block||signal?.aborted||branch!==authority)return previous;
        const saved=structuredClone(session.messages.filter(m=>m.role!=='custom'));
        const checkpoint={kind:'checkpoint',adapter:'pi',opaqueId:`${branch}:${context.toolCall.id}:${saved.length}`};
        checkpoints.set(key(branch,context.toolCall.id),{saved,checkpoint});
        const eventKey=key(branch,context.toolCall.id);
        if(!countedHooks.has(eventKey)){
            countedHooks.add(eventKey);
            hookCounts.set(branch,(hookCounts.get(branch)??0)+1);
        }
        const decision=await localDecision(branch,context.toolCall.id,context.toolCall.id,context.toolCall.name,context.args,checkpoint,signal);
        const admitted=decision.kind==='admit_speculation',policy=policies.get(branch);
        if(admitted&&policy)await report({message:'speculative_admission_request',task_id:taskId,policy_digest:policy.digest,branch_id:branch,action_id:context.toolCall.id,action_kind:'tool_execution',name:context.toolCall.name,arguments:context.args,checkpoint,timestamp_ms:Date.now()});
        if(admitted&&policy){
            const extent=await evaluatePolicy(policy,'extent',{process_state:{steps_completed:hookCounts.get(branch)??0,hook_count:hookCounts.get(branch)??0,phase:'tool_execution'}},signal);
            if(extent==='stop'){
                let release;const promise=new Promise(resolve=>release=resolve);stopGates.set(branch,{promise,release});
                await report({message:'branch_suspended',branch_id:branch,reason:'extent_reached'});
                await waitStop(branch,signal);
            }
        }
        await report({message:'action_started',task_id:taskId,branch_id:branch,action_id:context.toolCall.id,action_kind:'tool_execution',result_pending:true,name:context.toolCall.name,arguments:context.args,checkpoint,timestamp_ms:Date.now()});
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
            void report({message:'transcript',branch_id:branch,entry:event.message});
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
    finally { await finishActions(branch,cancelled.has(branch)||aborting.has(branch)?'cancelled':'failed'); }
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
        case 'local_policy':
            if(typeof command.policy_digest!=='string'||!command.decisions||typeof command.decisions!=='object')throw Error('Invalid local Policy artifact');
            policies.set(branch,{digest:command.policy_digest,decisions:command.decisions});
            return;
        case 'start_branch': {
            const {session}=await createAgentSession({cwd:command.workspace,model,modelRuntime,thinkingLevel,customTools:tools(branch),beforeModelInference:(modelValue,context,signal)=>beforeModelInference(branch,modelValue,context,signal)});
            if(closing||cancelled.has(branch)){session.dispose();return}
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
            const parentPolicy=policies.get(command.parent_branch_id);
            if(parentPolicy)policies.set(branch,parentPolicy);
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
            const {session}=await createAgentSession({cwd:command.workspace,sessionManager:manager,model:parent.model,modelRuntime:parent.modelRuntime,thinkingLevel:parent.thinkingLevel,customTools:tools(branch),beforeModelInference:(modelValue,context,signal)=>beforeModelInference(branch,modelValue,context,signal)});
            if(closing||cancelled.has(branch)){session.dispose();return}
            sessions.set(branch,session);install(session,branch);
            await report({message:'branch_created',branch_id:branch});
            // Injected result is new candidate content, not copied from the Parent.
            for(const entry of paired)await report({message:'transcript',branch_id:branch,entry});
            await report({message:'branch_started',branch_id:branch});
            await report({message:'branch_ready',branch_id:branch});
            executeTurn(session,branch,()=>session.continue());return;
        }
        case 'pause': {
            if(!pauseGates.has(branch)){let release;const promise=new Promise(resolve=>release=resolve);pauseGates.set(branch,{promise,release});}
            return;
        }
        case 'resume':
            pauseGates.get(branch)?.release();pauseGates.delete(branch);return;
        case 'resolve_pending': {
            stopGates.get(branch)?.release();stopGates.delete(branch);
            if(command.outcome===undefined){await report({message:'branch_resumed',branch_id:branch});return;}
            if(command.outcome!==undefined){const id=key(branch,command.operation_id),waiter=waiters.get(id);if(waiter){waiters.delete(id);waiter(command.outcome)}else results.set(id,command.outcome)}
            return;
        }
        case 'promote':
            if(cancelled.has(branch)||!sessions.has(branch))throw Error('Branch unavailable');
            authority=branch;gates.get(branch)?.release();gates.delete(branch);stopGates.get(branch)?.release();stopGates.delete(branch);return;
        case 'abort':
            aborting.add(branch);
            try { await sessions.get(branch)?.abort();await finishActions(branch,'cancelled'); }
            finally { aborting.delete(branch); }
            return;
        case 'discard': cancelled.add(branch);stopGates.get(branch)?.release();stopGates.delete(branch);pauseGates.get(branch)?.release();pauseGates.delete(branch);await sessions.get(branch)?.abort();await finishActions(branch,'cancelled');return;
        case 'cleanup':
            cancelled.add(branch);stopGates.get(branch)?.release();stopGates.delete(branch);pauseGates.get(branch)?.release();pauseGates.delete(branch);await sessions.get(branch)?.abort();sessions.get(branch)?.dispose();sessions.delete(branch);gates.delete(branch);policies.delete(branch);hookCounts.delete(branch);modelActions.delete(branch);
            await finishActions(branch,'cancelled');
            for(const id of checkpoints.keys())if(JSON.parse(id)[0]===branch)checkpoints.delete(id);
            await report({message:'agent_session_released',branch_id:branch});
            await report({message:'branch_released',branch_id:branch});return;
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
            const result=old?.result??(commandQueue=commandQueue.then(()=>dispatch(command)));
            if(id&&!old)commands.set(id,{serialized:JSON.stringify(command),result});
            const task=result.then(reply=>reply?report(reply):undefined);tasks.add(task);void task.finally(()=>tasks.delete(task)).catch(()=>socket.destroy());
        }
    }
}catch(error){
    transportError??=error;
}finally{
    closing=true;for(const gate of gates.values())gate.release();for(const gate of stopGates.values())gate.release();for(const gate of pauseGates.values())gate.release();
    await commandQueue.catch(()=>{});
    await Promise.allSettled([...sessions.values()].map(s=>s.abort()));
    await Promise.allSettled(tasks);for(const session of sessions.values())session.dispose();
    for(const branch of sessions.keys())await finishActions(branch,'cancelled');
    await report({message:'agent_released',agent_id:agentId});
    await outgoing;
    if(!socket.destroyed)await new Promise(resolve=>socket.end(resolve));
    // An aborted local-policy evaluation can leave the evaluator child's stdio
    // handles open after it rejects; those handles would otherwise keep this
    // adapter alive even though every report is flushed and the socket is
    // closed. Shutdown is complete here, so terminate explicitly.
    if(transportError)console.error('Agent control connection failed:',transportError);
    process.exit(transportError?1:0);
}

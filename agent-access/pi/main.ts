/** AScout-owned integration using Pi's public SDK, without Pi UI internals. */
// This is a protocol boundary: Pi's SDK types are intentionally loaded from
// the selected checkout at runtime, so the Rust-side wire schema remains the
// compile-time contract for this small launcher.
// @ts-nocheck
import { connect } from 'node:net';
import { createRequire } from 'node:module';
import { resolve, dirname } from 'node:path';
import { evaluatePolicyArtifact } from '../../../pi/packages/ascout-agent-access/src/local-policy.ts';
import { createAgentAccess } from '../core/index.ts';
import { SessionTree } from './session-tree.ts';
import { Notifications } from './notification.ts';
import {save as saveRecovery,load as loadRecovery} from './recovery.ts';
import {recoveryBinding,snapshotResources} from './recovery-binding.ts';
import { readFileSync } from 'node:fs';
import {filesystemTools} from './filesystem-tools.ts';

const root = process.env.ASCOUT_PI_ROOT;
if (!root) throw Error('ASCOUT_PI_ROOT is required');
const require = createRequire(resolve(root, 'package.json'));
const { createAgentSession, SessionManager, ModelRuntime, resolveCliModel } = require(resolve(root, 'packages/coding-agent/src/index.ts'));
const sessionFile = process.env.ASCOUT_SESSION_FILE;
if (!sessionFile) throw Error('ASCOUT_SESSION_FILE is required');
let tree = new SessionTree(SessionManager.open(sessionFile));
const { Type } = require('typebox');
const args = process.argv.slice(2);
const socketPath = args.shift();
const agentId = args.shift() ?? 'agent';
const taskId = process.env.ASCOUT_TASK_ID ?? '';
let nextAction = 1;
if (!socketPath) throw Error('usage: main.ts <socket> <agent> [--model provider/model] [--thinking level]');
let modelPattern, modelsPath, additionalModels, thinkingLevel = 'low';
while (args.length) {
    const flag = args.shift(), value = args.shift();
    if (!value) throw Error(`${flag} requires a value`);
    if (flag === '--model') modelPattern = value;
    else if (flag === '--models') modelsPath = value;
    else if (flag === '--thinking') thinkingLevel = value;
    else throw Error(`Unknown argument ${flag}`);
}
if (!['off','minimal','low','medium','high','xhigh'].includes(thinkingLevel)) throw Error('Invalid thinking level');
let modelRuntime, model;
if (modelPattern || modelsPath) {
    modelRuntime = await ModelRuntime.create();
    if (modelsPath) {
        const extra = JSON.parse(readFileSync(modelsPath, 'utf8'));
        additionalModels = extra;
        for (const [id, config] of Object.entries(extra.providers ?? {})) {
            if (modelRuntime.getProvider(id)) throw Error(`Additional provider already exists: ${id}`);
            modelRuntime.registerProvider(id, config);
        }
    }
    if (modelPattern) {
    const resolved = resolveCliModel({ cliModel:modelPattern, cliThinking:thinkingLevel, modelRuntime });
    if (resolved.error || !resolved.model) throw Error(resolved.error ?? 'Unknown model');
    model = resolved.model;
    }
}
tree.initialize(model,thinkingLevel);
const sessions = new Map(), checkpoints = new Map(), results = new Map(), waiters = new Map(), resultWaiters = new Map();
const cancelled = new Set(), generations = new Map(), commands = new Map();
const activeActions = new Map(), reportedActions = new Set();
const aborting = new Set();
const modelActions = new Map();
const transcriptQueues = new Map();
const reportedEntries = new Set();
const candidateWaiters = new Map();
const branchTasks = new Map();
const setupTasks = new Map();
let saving=false;
const pendingHooks=new Map();
const suspendedHooks=new Map(), candidateSetups=new Map();
const reconnectControllers=new Map();
const interruptedBranches=new Set();
let recoveryPaused=false;
function captureHooks(){
    for(const branch of branchTasks.keys())interruptedBranches.add(branch);
    for(const [id,hook] of pendingHooks){
        // Native non-Runtime tools have Pi's terminal cancellation result.
        // A started external action cannot safely be replayed after restart.
        if(hook.started&&!runtimeTools.some(spec=>spec.name===hook.toolCall.name))continue;
        suspendedHooks.set(id,{...hook,admitted:checkpoints.has(id)});
    }
    for(const [branch,session] of sessions){
        const assistant=[...session.messages].reverse().find(m=>m.role==='assistant');
        if(!assistant?.content?.some(p=>p.type==='toolCall'))continue;
        const position=session.messages.indexOf(assistant),following=session.messages.slice(position+1);
        for(const part of assistant.content)if(part.type==='toolCall'&&!following.some(m=>m.role==='toolResult'&&m.toolCallId===part.id)){
            const id=key(branch,part.id);
            if(pendingHooks.get(id)?.started&&!runtimeTools.some(spec=>spec.name===part.name))continue;
            if(!suspendedHooks.has(id))suspendedHooks.set(id,{branch,toolCall:structuredClone(part),args:structuredClone(part.arguments),assistantMessage:structuredClone(assistant),admitted:checkpoints.has(id)});
        }
    }
}
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
    const cwd=tree.branches.get(branch)?.manager.getHeader().cwd??process.cwd();
    const fileTools=filesystemTools(process.env.ASCOUT_FILESYSTEM_TOOL_PLANS,branch,cwd,require(resolve(root,'packages/coding-agent/src/core/tools/index.ts')));
    return [...fileTools,...runtimeTools.map(spec=>({name:spec.name,label:spec.name,description:spec.description,parameters:Type.Unsafe(spec.parameters),execute:async(operation,_args,signal)=>{
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
    }}))];
}
// FS execution must never fall back to host implementations selected through
// user settings or extensions. Only the isolated built-ins and Runtime tools
// have an execution plan in this integration.
const filesystemToolSelection=process.env.ASCOUT_FILESYSTEM_TOOL_PLANS
    ? {tools:['read','bash','edit','write',...runtimeTools.map(tool=>tool.name)]}
    : {};
async function beforeModelInference(branch, modelValue, context, signal) {
    syncTranscript(branch);
    // Model inference is a PURE step: the generic layer evaluates `admit` but
    // never applies the effect gate, reports an admission, or counts extent.
    if(saving||closing||cancelled.has(branch)||signal?.aborted)throw Error('Model inference cancelled before execution');
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
        if(saving)throw Error('Save requested before tool execution');
        const hookKey=key(branch,context.toolCall.id);
        pendingHooks.set(hookKey,{branch,toolCall:structuredClone(context.toolCall),args:structuredClone(context.args),assistantMessage:structuredClone(context.assistantMessage)});
        try {
        const previous=await before?.(context,signal);
        if(previous?.block||signal?.aborted){pendingHooks.delete(key(branch,context.toolCall.id));return previous;}
        let saved, checkpoint;
        const prepareCheckpoint=()=>{
            if(checkpoint)return checkpoint;
            if(suspendedHooks.get(hookKey)?.admitted)return checkpoints.get(hookKey).checkpoint;
            syncTranscript(branch);
            const reconnect=suspendedHooks.get(hookKey);
            const index=reconnect?session.messages.findIndex(m=>m.role==='assistant'&&m.content?.some(p=>p.type==='toolCall'&&p.id===context.toolCall.id)):-1;
            saved=structuredClone(index>=0?session.messages.slice(0,index+1):session.messages);
            checkpoint={kind:'checkpoint',adapter:'pi',opaqueId:`${branch}:${context.toolCall.id}:${saved.length}`};
            const manager=tree.branches.get(branch).manager;
            const treeTail=reconnect?manager.getEntries().find(e=>e.type==='message'&&JSON.stringify(e.message)===JSON.stringify(saved.at(-1)))?.id:tree.assertCheckpoint(branch,saved);
            if(!treeTail)throw Error('Suspended tool checkpoint missing');
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
            admitted:suspendedHooks.get(hookKey)?.admitted??false,
            signal,
        });
        if(signal?.aborted||branch!==authority)return previous;
        const toolCheckpoint=prepareCheckpoint();
        await report({message:'action_started',task_id:taskId,branch_id:branch,action_id:context.toolCall.id,action_kind:'tool_execution',result_pending:true,name:context.toolCall.name,arguments:context.args,checkpoint:toolCheckpoint,timestamp_ms:Date.now()});
        const hook=pendingHooks.get(hookKey);if(hook)hook.started=true;
        return previous;
        } catch(error) {
            if(!saving)pendingHooks.delete(key(branch,context.toolCall.id));
            throw error;
        }
    };
    session.agent.afterToolCall=async(context,signal)=>{
        if(saving&&signal?.aborted)return;
        pendingHooks.delete(key(branch,context.toolCall.id));
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
    finally { syncTranscript(branch);await finishActions(branch,saving||cancelled.has(branch)||aborting.has(branch)?'cancelled':'failed'); }
    const last=[...session.messages].reverse().find(m=>m.role==='assistant');
    if(last?.stopReason==='error'&&!saving)await report({message:branch===authority?'turn_failed':'branch_failed',branch_id:branch,reason:last.errorMessage??'Model failed'});
    await report({message:'turn_completed',branch_id:branch});
    if(branch===authority)tree.settle();
}
// A turn may wait for a later control command (runtime result, Promote or
// Abort). Keep setup ordered, but never hold the command queue for the turn.
function executeTurn(session,branch,operation) {
    const task=run(session,branch,operation)
        .catch(error=>saving?undefined:report({message:'branch_failed',branch_id:branch,reason:String(error instanceof Error?error.message:error)}));
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
        case 'model': {
            try {
                if(branch!==authority)throw Error('Only the authority accepts model changes');
                const session=sessions.get(branch);if(!session)throw Error('Session missing');
                if(branchTasks.has(branch)||setupTasks.size)throw Error('Model change requires an idle authority');
                const resolved=resolveCliModel({cliModel:command.model,cliThinking:session.thinkingLevel,modelRuntime:session.modelRuntime});
                if(resolved.error||!resolved.model)throw Error(resolved.error??'Unknown model');
                await session.setModel(resolved.model);model=resolved.model;modelPattern=command.model;
                await report({message:'model_changed',branch_id:branch,model:command.model,error:null});
            } catch(error){await report({message:'model_changed',branch_id:branch,model:command.model,error:String(error.message)});}
            return;
        }
        case 'cancel_save':
            saving=false;
            recoveryPaused=true;
            return;
        case 'save': {
            saving=true;
            let error;
            try {
                captureHooks();
                for(const branch of candidateWaiters.keys())releaseCandidateWaiters(branch);
                // Cancellation is requested concurrently on every session,
                // before waiting for any individual inference to settle.
                const aborts=[...sessions.values()].map(session=>session.abort());
                for(const controller of reconnectControllers.values())controller.abort();
                await Promise.all(aborts);
                await commandQueue;
                await Promise.all([...branchTasks.values()].flatMap(set=>[...set]));
                await Promise.allSettled([...setupTasks.values()]);
                for(const [branch,session] of sessions){await notifications.drain(session);syncTranscript(branch)}
                const commandRecords=[];
                for(const [id,item] of commands){
                    const value=JSON.parse(item.serialized);
                    if(['save','load'].includes(value.command))continue;
                    commandRecords.push([id,{serialized:item.serialized,reply:await item.result}]);
                }
                const snapshot=tree.snapshot(dirname(command.path),[...checkpoints.values()].map(c=>c.manager));
                const sourceFiles=new Map(snapshot.sources.map(source=>[source.originalFile,source.file]));
                saveRecovery(command.path,{version:1,taskId,tree:snapshot,authority,nextAction,
                    resourceBinding:await recoveryBinding(root,snapshot.sources.map(source=>source.header.cwd)),
                    snapshotResources:snapshotResources(snapshot.sources),
                    executionConfig:{model:model??null,modelPattern:modelPattern??null,thinkingLevel,runtimeTools},
                    additionalModels:additionalModels??null,
                    access:access.snapshot(),generations:[...generations],results:[...results],
                    cancelled:[...cancelled],reportedEntries:[...reportedEntries],commands:commandRecords,
                    suspendedHooks:[...suspendedHooks],candidateSetups:[...candidateSetups],
                    interruptedBranches:[...interruptedBranches],
                    checkpoints:[...checkpoints].map(([id,c])=>[id,{...c,manager:sourceFiles.get(c.manager.getSessionFile())}])});
            }catch(reason){error=String(reason)}
            await report({message:'recovery_saved',path:command.path,error:error??null});
            if(error){saving=false;recoveryPaused=true;}
            return;
        }
        case 'load': {
            // A failed load must not settle/delete the saved native sources.
            saving=true;
            let error;
            try {
                if(sessions.size)throw Error('Load requires an empty Agent adapter');
                const record=loadRecovery(command.path);
                if(record.taskId!==taskId)throw Error('Recovery task identity mismatch');
                if(record.resourceBinding&&JSON.stringify(record.resourceBinding)!==JSON.stringify(await recoveryBinding(root,record.tree.sources.map(source=>source.header.cwd))))throw Error('Recovery Pi code, extension or configuration resources changed');
                snapshotResources(record.tree.sources,record.snapshotResources);
                const executionConfig={model:model??null,modelPattern:modelPattern??null,thinkingLevel,runtimeTools};
                const canonical=value=>Array.isArray(value)?value.map(canonical):value&&typeof value==='object'?Object.fromEntries(Object.keys(value).sort().map(key=>[key,canonical(value[key])])):value;
                if(record.additionalModels&&JSON.stringify(canonical(record.additionalModels))!==JSON.stringify(canonical(additionalModels)))throw Error('Recovery additional model configuration changed');
                if(record.executionConfig&&JSON.stringify(canonical(record.executionConfig))!==JSON.stringify(canonical(executionConfig)))throw Error('Recovery model, thinking level or tool configuration changed');
                if(record.authority!==record.tree.authority)throw Error('Recovery authority identity mismatch');
                const sources=new Map(record.tree.sources.map(source=>[source.file,source]));
                for(const [id,checkpoint] of record.checkpoints){
                    const [owner,operation]=JSON.parse(id),source=sources.get(checkpoint.manager);
                    if(!source||!record.tree.branches.some(([branch])=>branch===owner))throw Error('Recovery checkpoint owner missing');
                    const entry=source.entries.find(entry=>entry.id===checkpoint.treeTail);
                    if(entry?.type!=='message'||JSON.stringify(entry.message)!==JSON.stringify(checkpoint.saved.at(-1))||!entry.message.content?.some(part=>part.type==='toolCall'&&part.id===operation))throw Error('Recovery checkpoint context mismatch');
                }
                const restored=SessionTree.restore(record.tree,SessionManager,sessionFile);
                const managers=restored.recoveryManagers;
                for(const [,checkpoint] of record.checkpoints)if(!managers.has(checkpoint.manager))throw Error('Recovery checkpoint manager missing');
                tree=restored;authority=record.authority;nextAction=record.nextAction;
                access.restore(record.access);
                for(const [id,value] of record.generations)generations.set(id,value);
                for(const [id,value] of record.results)results.set(id,value);
                for(const value of record.cancelled)cancelled.add(value);
                for(const value of record.reportedEntries)reportedEntries.add(value);
                for(const [id,value] of record.commands)commands.set(id,{serialized:value.serialized,result:Promise.resolve(value.reply)});
                for(const [id,c] of record.checkpoints)checkpoints.set(id,{...c,manager:managers.get(c.manager)});
                for(const [id,hook] of record.suspendedHooks??[])suspendedHooks.set(id,hook);
                for(const [id,setup] of record.candidateSetups??[])candidateSetups.set(id,setup);
                for(const branch of record.interruptedBranches??[])interruptedBranches.add(branch);
                for(const [branch,{manager,stopped}] of tree.branches){
                    if(stopped||candidateSetups.has(branch))continue;
                    const {session}=await createAgentSession({...filesystemToolSelection,cwd:manager.getHeader().cwd,sessionManager:manager,model,modelRuntime,thinkingLevel,customTools:tools(branch),beforeModelInference:(modelValue,context,signal)=>beforeModelInference(branch,modelValue,context,signal)});
                    sessions.set(branch,session);install(session,branch);
                }
                saving=false;
                recoveryPaused=true;
            }catch(reason){error=String(reason)}
            await report({message:'recovery_loaded',error:error??null});return;
        }
        case 'local_policy':
            return;
        case 'suspend_recovery': {
            saving=true;
            captureHooks();
            for(const branch of candidateWaiters.keys())releaseCandidateWaiters(branch);
            for(const controller of reconnectControllers.values())controller.abort();
            await Promise.all([...sessions.values()].map(session=>session.abort()));
            await Promise.allSettled([...setupTasks.values()]);
            await report({message:'recovery_suspended'});return;
        }
        case 'start_branch': {
            // The control plane explicitly starts this Branch as the authority;
            // a leftover authority marker in a reused session file must not
            // keep a previous run's winner in charge.
            if(command.role==='authority'){authority=branch;tree.adoptAuthority(branch)}
            const manager=tree.createBranch(branch,command.workspace,tree.manager,tree.tail(branch)??tree.manager.getLeafId());
            const {session}=await createAgentSession({...filesystemToolSelection,cwd:command.workspace,sessionManager:manager,model,modelRuntime,thinkingLevel,customTools:tools(branch),beforeModelInference:(modelValue,context,signal)=>beforeModelInference(branch,modelValue,context,signal)});
            if(closing||cancelled.has(branch)){sessions.set(branch,session);await stopSession(branch);return}
            sessions.set(branch,session);install(session,branch);
            await report({message:'branch_created',branch_id:branch});
            await report({message:'branch_started',branch_id:branch});
            await report({message:'branch_ready',branch_id:branch});
            if(command.prompt)executeTurn(session,branch,()=>session.prompt(command.prompt));
            return;
        }
        case 'prompt': {
            if(saving)throw Error('Execution is saved and waiting for exit');
            if(branch!==authority)throw Error('Only the authority accepts user input');
            const session=sessions.get(branch);if(!session)throw Error('Session missing');
            if(recoveryPaused){
                recoveryPaused=false;
                // Native cancelled tool results remain in history. Reconnect
                // only suspended calls, appending their eventual real result;
                // never replay the already issued Runtime action.
                for(const [id,setup] of [...candidateSetups])startSetup(setup);
                const grouped=new Map();
                for(const [id,hook] of suspendedHooks){const hooks=grouped.get(hook.branch)??[];hooks.push([id,hook]);grouped.set(hook.branch,hooks)}
                for(const [owner,hooks] of grouped){
                    const target=sessions.get(owner);if(!target||cancelled.has(owner))continue;
                    executeTurn(target,owner,async()=>{
                        const {runToolCall}=require(resolve(root,'packages/agent/src/index.ts'));
                        const controller=new AbortController();
                        reconnectControllers.set(owner,controller);
                        await Promise.all(hooks.map(async([id,hook])=>{
                            const outcome=await runToolCall(hook.toolCall,{tools:target.agent.state.tools,assistantMessage:hook.assistantMessage,context:{messages:target.messages,tools:target.agent.state.tools},signal:controller.signal,
                                beforeToolCall:target.agent.beforeToolCall,afterToolCall:target.agent.afterToolCall});
                            if(saving)return;
                            const message={role:'toolResult',toolCallId:hook.toolCall.id,toolName:hook.toolCall.name,content:outcome.result.content,isError:outcome.isError,timestamp:Date.now()};
                            const entry=tree.append(owner,message);target.agent.state.messages=tree.context(owner).messages;
                            suspendedHooks.delete(id);pendingHooks.delete(id);
                            await report({message:'transcript',branch_id:owner,entry:message,session_entry_id:entry});
                        }));
                        reconnectControllers.delete(owner);
                        if(saving||controller.signal.aborted)return;
                        const operations=new Set(hooks.map(([,hook])=>hook.toolCall.id));
                        const latest=new Map();
                        for(const message of target.messages)if(message.role==='toolResult'&&operations.has(message.toolCallId))latest.set(message.toolCallId,message);
                        target.agent.state.messages=target.messages.filter(message=>
                            !(message.role==='toolResult'&&operations.has(message.toolCallId)&&latest.get(message.toolCallId)!==message)
                            && !(message.role==='assistant'&&['aborted','error'].includes(message.stopReason)));
                        if(owner===authority)await target.prompt(command.prompt,{streamingBehavior:'followUp'});else await target.continue();
                    });
                }
                for(const [owner,target] of sessions)if(owner!==authority&&interruptedBranches.has(owner)&&!grouped.has(owner)&&!candidateSetups.has(owner)&&!cancelled.has(owner)){
                    const last=target.messages.at(-1);
                    // Pi excludes aborted assistant output from model context;
                    // keep its native record but continue from its prior input.
                    if(last?.role==='assistant'&&['aborted','error'].includes(last.stopReason))target.agent.state.messages=target.messages.slice(0,-1);
                    if(target.messages.at(-1)?.role!=='assistant')executeTurn(target,owner,()=>target.continue());
                }
                interruptedBranches.clear();
                if(grouped.has(branch))return;
            }
            executeTurn(session,branch,()=>session.prompt(command.prompt,{streamingBehavior:'followUp'}));return;
        }
        case 'start_candidate': {
            candidateSetups.set(branch,structuredClone(command));
            const parent=sessions.get(command.parent_branch_id),checkpoint=checkpoints.get(key(command.parent_branch_id,command.operation_id));
            if(!parent||!checkpoint||checkpoint.checkpoint.adapter!==command.checkpoint?.adapter||checkpoint.checkpoint.kind!==command.checkpoint?.kind||checkpoint.checkpoint.opaqueId!==command.checkpoint?.opaqueId)throw Error('Candidate requires its registered checkpoint');
            const messages=structuredClone(checkpoint.saved),last=messages.at(-1);
            if(last?.role!=='assistant'||!last.content.some(p=>p.type==='toolCall'&&p.id===command.operation_id&&p.name===command.tool_name))throw Error('Invalid pending checkpoint');
            const manager=tree.branches.get(branch)?.manager??tree.createBranch(branch,command.workspace,checkpoint.manager,checkpoint.treeTail);
            if(saving)return;
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
            const {session}=await createAgentSession({...filesystemToolSelection,cwd:command.workspace,sessionManager:manager,model:parent.model,modelRuntime:parent.modelRuntime,thinkingLevel:parent.thinkingLevel,customTools:tools(branch),beforeModelInference:(modelValue,context,signal)=>beforeModelInference(branch,modelValue,context,signal)});
            if(closing||cancelled.has(branch)){sessions.set(branch,session);await stopSession(branch);return}
            sessions.set(branch,session);install(session,branch);
            candidateSetups.delete(branch);
            await report({message:'branch_created',branch_id:branch});
            // Injected result is new candidate content, not copied from the Parent.
            for(let i=0;i<paired.length;i++)await report({message:'transcript',branch_id:branch,entry:paired[i],session_entry_id:pairedEntryIds[i]});
            await report({message:'branch_started',branch_id:branch});
            await report({message:'branch_ready',branch_id:branch});
            if(!saving&&!recoveryPaused)executeTurn(session,branch,()=>session.continue());return;
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
            reconnectControllers.get(branch)?.abort();
            try { await sessions.get(branch)?.abort();await finishActions(branch,'cancelled');await report({message:'turn_completed',branch_id:branch}); }
            finally { aborting.delete(branch); }
            return;
        case 'discard': cancelled.add(branch);reconnectControllers.get(branch)?.abort();candidateSetups.delete(branch);releaseCandidateWaiters(branch);await sessions.get(branch)?.abort();await finishActions(branch,'cancelled');tree.settle();return;
        case 'cleanup':
            cancelled.add(branch);releaseCandidateWaiters(branch);
            reconnectControllers.get(branch)?.abort();
            candidateSetups.delete(branch);
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
        if(saving&&!['save','suspend_recovery','cancel_save','resolve_pending','promote','discard','cleanup','start_candidate'].includes(command.command))throw Error('Save in progress; command not applied');
        const generation=generations.get(command.branch_id);
        if(generation!==undefined&&command.generation!==undefined&&generation!==command.generation)throw Error('Stale command generation');
        if(command.generation!==undefined)generations.set(command.branch_id,command.generation);
        if(operation&&command.command_id){await report(acknowledgement('accepted'));await report(acknowledgement('started'))}
        await handle(command);
    }catch(error){reason=String(error instanceof Error?error.message:error)}
    if(operation&&command.command_id)return {message:'command_result',command_id:command.command_id,branch_id:command.branch_id,operation,status:reason?'failed':'applied',generation:command.generation??0,...(command.operation_id?{operation_id:command.operation_id}:{}),...(reason?{reason}:{})};
    if(reason&&!(saving&&command.command==='start_candidate'))return {message:'branch_failed',branch_id:command.branch_id,reason};
}
function startSetup(command){
    const task=dispatch(command);setupTasks.set(command.branch_id,task);
    void task.then(reply=>reply?report(reply):undefined).finally(()=>setupTasks.delete(command.branch_id));
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
            // Save must interrupt immediately even if setup is awaiting a tool.
            const result=old?.result??(['save','suspend_recovery'].includes(command.command)
                ?dispatch(command):command.command==='start_candidate'
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
    if(saving){
        await outgoing;
        if(!socket.destroyed)await new Promise(resolve=>socket.end(resolve));
        process.exit(transportError?1:0);
    }
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

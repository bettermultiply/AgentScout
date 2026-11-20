/** Product Integration: Pi SDK hooks and session management; createInterposer
 * supplies the Interposer and AscProxy supplies ASC-Proxy communication. */
// This is a protocol boundary: Pi's SDK types are intentionally loaded from
// the selected checkout at runtime, so the Rust-side wire schema remains the
// compile-time contract for this small launcher.
// @ts-nocheck
import type { ToolCall, ToolResultMessage } from '../../../pi/packages/ai/src/types.ts';
import { IdentityWindow, positiveSequence } from '../../../pi/packages/pi-integration/src/identity-window.ts';
import { createRequire } from 'node:module';
import { resolve, dirname } from 'node:path';
import { evaluatePolicyArtifact } from '../../../pi/packages/pi-integration/src/local-policy.ts';
import { createInterposer } from '../core/index.ts';
import { SessionTree } from './session-tree.ts';
import { Notifications } from './notification.ts';
import { symlinkSync, renameSync, realpathSync, statSync, rmSync } from 'node:fs';
import { randomUUID } from 'node:crypto';
import {save as saveRecovery,load as loadRecovery,checksum} from './recovery.ts';
import {recoveryBinding,snapshotResources} from './recovery-binding.ts';
import { readFileSync } from 'node:fs';
import {AscProxy} from './asc-proxy.ts';
import {environmentTools} from './tools.ts';
import { Interaction } from './interaction.ts';

const root = process.env.ASCOUT_PI_ROOT;
if (!root) throw Error('ASCOUT_PI_ROOT is required');
const require = createRequire(resolve(root, 'package.json'));
const { createAgentSession: createNativeAgentSession, DefaultResourceLoader, SettingsManager, getAgentDir, SessionManager, ModelRuntime, resolveCliModel } = require(resolve(root, 'packages/coding-agent/src/index.ts'));
const nativeCommandSources = new WeakMap();
async function createAgentSession(options) {
    // Use a native inline extension to obtain the product's command catalogue.
    // Keep Pi's normal resource loader, including user extensions and resources.
    if (!DefaultResourceLoader) return createNativeAgentSession(options); // SDK test doubles
    const agentDir = options.agentDir ?? getAgentDir();
    const settingsManager = options.settingsManager ?? SettingsManager.create(options.cwd, agentDir);
    let extension;
    const resourceLoader = new DefaultResourceLoader({
        cwd: options.cwd, agentDir, settingsManager,
        extensionFactories: [api => { extension = api; }],
    });
    await resourceLoader.reload();
    const result = await createNativeAgentSession({...options, agentDir, settingsManager, resourceLoader});
    nativeCommandSources.set(result.session, () => extension.getCommands());
    return result;
}
const sessionFile = process.env.ASCOUT_SESSION_FILE;
if (!sessionFile) throw Error('ASCOUT_SESSION_FILE is required');
const nativeReference = JSON.parse(process.env.ASCOUT_OPEN_SESSION ?? 'null');
function openNativeSession(reference) {
    const file = realpathSync(reference.file);
    const info = statSync(file);
    if (!info.isFile() || info.size === 0) throw Error(`Invalid Pi session reference: ${file}`);
    const manager = SessionManager.open(file);
    const header = manager.getHeader();
    if (!header?.id || (reference.id && reference.id !== header.id) ||
        (reference.cwd && reference.cwd !== header.cwd)) {
        throw Error(`Pi session identity mismatch: ${file}`);
    }
    return manager;
}
let tree = new SessionTree(nativeReference ? openNativeSession(nativeReference) : SessionManager.create(process.cwd()));
const nativeTail = nativeReference ? tree.tail(tree.authority) : undefined;
tree.readOnlyUntilPrompt = Boolean(nativeReference);
function linkSession(manager = tree.manager) {
    const file = manager.getSessionFile();
    if (nativeReference) {
        try { if (realpathSync(sessionFile) === file) return; }
        catch (error) { if (error.code !== 'ENOENT') throw error; }
    }
    const temporary = `${sessionFile}.${randomUUID()}.link`;
    symlinkSync(file, temporary);
    try { renameSync(temporary, sessionFile); }
    finally { rmSync(temporary, {force:true}); }
}
if (!nativeReference && !process.env.ASCOUT_LOAD_SESSION) linkSession();
const { Type } = require('typebox');
const args = process.argv.slice(2);
const socketPath = args.shift();
const agentId = args.shift() ?? 'agent';
const taskId = process.env.ASCOUT_TASK_ID ?? '';
const environments=new Map<string,string>();
const ascProxy=new AscProxy();
const executionResults=new Map();
const toolHolds=new Map();
let nextAction = 1;
if (!socketPath) throw Error('usage: main.ts <socket> <agent> [--model provider/model] [--thinking level]');
let modelPattern, modelsPath, additionalModels;
let thinkingLevel = (nativeReference && tree.manager.buildSessionContext().thinkingLevel) || 'low';
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
let commandPrefix=0;
const commandAhead=new Set<number>();
const commandSequence=id=>{const match=/^command-([1-9][0-9]*)-/.exec(id);if(!match)return undefined;const n=Number(match[1]);return Number.isSafeInteger(n)?n:undefined};
function pruneCommands(){
    const floor=Math.max(0,commandPrefix-256);
    for(const [id,item] of commands)if(item.settled&&commandSequence(id)!==undefined&&commandSequence(id)<=floor)commands.delete(id);
}
const activeActions = new Map(), reportedActions = new Set();
const aborting = new Set();
const modelActions = new Map();
const modelForks = new Map();
const modelHolds = new Map();
const modelResultHolds = new Map();
const modelRestores = new Map();
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
let pauseRequested=false;
const parkedModels=new Set();
const pauseReleases=new Set<()=>void>();
const acceptancePauses = new Map();
const acceptanceInputs = new Map();
function releaseAcceptance(branch) {
    const pause = acceptancePauses.get(branch);
    acceptancePauses.delete(branch);
    pause?.resolve();
}
async function acceptanceBoundary(branch, signal) {
    while (acceptancePauses.has(branch)) {
        const pause = acceptancePauses.get(branch);
        await new Promise<void>((resolve, reject) => {
            const abort = () => reject(Error('Acceptance interrupted'));
            signal?.addEventListener('abort', abort, {once:true});
            pause.promise.then(resolve, reject).finally(() => signal?.removeEventListener('abort', abort));
            if (signal?.aborted) abort();
        });
    }
    if (signal?.aborted || cancelled.has(branch) || closing) throw Error('Branch execution interrupted');
}
async function pauseBoundary(signal){
    if(!pauseRequested)return;
    await new Promise<void>((resolve,reject)=>{
        const release=()=>{cleanup();resolve()};
        const abort=()=>{cleanup();reject(Error('Paused execution interrupted'))};
        const cleanup=()=>{pauseReleases.delete(release);signal?.removeEventListener('abort',abort)};
        pauseReleases.add(release);signal?.addEventListener('abort',abort,{once:true});
        if(signal?.aborted)abort();
    });
}
async function waitForPause(){
    while(pauseRequested){
        const stable=[...branchTasks.keys()].every(branch=>{
            const hooks=[...pendingHooks.values()].filter(h=>h.branch===branch);
            return !hooks.some(h=>h.started)&&(parkedModels.has(branch)||hooks.length>0);
        });
        if(stable)return;
        await new Promise(resolve=>setTimeout(resolve,5));
    }
    throw Error('Pause interrupted');
}
function releasePause(){pauseRequested=false;for(const release of [...pauseReleases])release();}
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
            if(checkpoints.get(id)?.result)continue;
            if(pendingHooks.get(id)?.started&&!runtimeTools.some(spec=>spec.name===part.name))continue;
            if(!suspendedHooks.has(id))suspendedHooks.set(id,{branch,toolCall:structuredClone(part),args:structuredClone(part.arguments),assistantMessage:structuredClone(assistant),admitted:checkpoints.has(id)});
        }
    }
}
const notifications = new Notifications();
const interaction = new Interaction(payload => report(payload));
const extensionBindings = new Map<string, Promise<void>>();
let commandCatalog;
let commandCatalogStarted = false;
function cacheCommands(session, branch) {
    if (branch !== authority || commandCatalogStarted) return;
    commandCatalogStarted = true;
    // bindExtensions includes session_start followed by resources_discover.
    // Publish prompts immediately; a session_start answer must not depend on
    // this catalog becoming available.
    void extensionBindings.get(branch).then(async () => {
        commandCatalog = nativeCommandSources.has(session) ? await nativeCommandSources.get(session)() : typeof session.getCommands === 'function' ? await session.getCommands() : [
            ...(session.extensionRunner?.getRegisteredCommands() ?? []).map(item => ({name:item.invocationName,description:item.description,source:'extension',sourceInfo:item.sourceInfo})),
            ...(session.promptTemplates ?? []).map(item => ({name:item.name,description:item.description,source:'prompt',sourceInfo:item.sourceInfo})),
            ...(session.resourceLoader?.getSkills().skills ?? []).map(item => ({name:`skill:${item.name}`,description:item.description,source:'skill',sourceInfo:item.sourceInfo})),
        ];
        await report({message:'commands_listed',branch_id:authority,request_id:'startup',commands:commandCatalog});
    }).catch(error => report({message:'branch_failed',branch_id:branch,reason:String(error)}));
}
function bindInteraction(session,branch) {
    interaction.resume(branch);
    sessions.set(branch,session);
    install(session,branch);
    // session_start extensions may ask the user. Never hold the command
    // queue while their UI promise is outstanding (including on candidates).
    const binding=session.bindExtensions({uiContext:interaction.context(branch)});
    extensionBindings.set(branch,binding);
    cacheCommands(session, branch);
    void binding.catch(error=>report({message:'branch_failed',branch_id:branch,reason:String(error)}));
}
async function stopSession(branch) {
    interaction.cancel(branch);
    await extensionBindings.get(branch)?.catch(()=>{});
    extensionBindings.delete(branch);
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
    releaseAcceptance(branch);
    acceptanceInputs.delete(branch);
    ascProxy.unbind(branch);
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
async function reportSessionHistory(branch, manager) {
    // Display inherited entries without reporting them as new execution facts.
    for (const entry of manager.getBranch()) {
        if (entry.type !== 'message') continue;
        reportedEntries.add(entry.id);
        await report({message:'session_history',branch_id:branch,entries:[entry.message]});
    }
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
let authority = tree.authority, closing = false;
await ascProxy.open(socketPath);
let transportError;
const report = payload => {
    if(payload.message==='action_started'){activeActions.set(key(payload.branch_id,payload.action_id),payload);reportedActions.add(key(payload.branch_id,payload.action_id));}
    if(payload.message==='action_finished')activeActions.delete(key(payload.branch_id,payload.action_id));
    payload={source:'agent_adapter',...payload};
    return sendWire({kind:'message',payload});
};
const sendWire = wire => ascProxy.send(wire);
const access = createInterposer({taskId,report,evaluate:evaluatePolicyArtifact});
const key = (branch, operation) => JSON.stringify([branch,operation]);
const text = value => typeof value==='string'?value:JSON.stringify(value);

type ToolResultContent = ToolResultMessage['content'];
type ToolResultBody = Pick<ToolResultMessage, 'content' | 'details' | 'usage' | 'nestedCalls' | 'durationMs'>
    & Partial<Pick<ToolResultMessage, 'isError' | 'timestamp'>>;

function isToolResultBlock(value: unknown): value is ToolResultContent[number] {
    if (!value || typeof value !== 'object') return false;
    const block = value as Record<string, unknown>;
    if (block.type === 'text') return typeof block.text === 'string';
    return block.type === 'image' && typeof block.mimeType === 'string' && typeof block.data === 'string';
}

function isToolResultBody(value: unknown): value is ToolResultBody {
    if (!value || typeof value !== 'object' || Array.isArray(value)) return false;
    const body = value as Record<string, unknown>;
    return (body.role === undefined || body.role === 'toolResult')
        && Array.isArray(body.content) && body.content.every(isToolResultBlock)
        && (body.isError === undefined || typeof body.isError === 'boolean');
}

function predictionResult(call: ToolCall, prediction: unknown): ToolResultMessage {
    const message: ToolResultMessage = {
        role: 'toolResult', toolCallId: call.id, toolName: call.name,
        content: [], isError: false, timestamp: Date.now(),
    };
    if (isToolResultBlock(prediction)) {
        message.content = [prediction];
    } else if (Array.isArray(prediction) && prediction.length && prediction.every(isToolResultBlock)) {
        message.content = prediction;
    } else if (isToolResultBody(prediction)) {
        message.content = prediction.content;
        message.isError = prediction.isError ?? false;
        if (typeof prediction.timestamp === 'number') message.timestamp = prediction.timestamp;
        if (typeof prediction.durationMs === 'number') message.durationMs = prediction.durationMs;
        if (prediction.details !== undefined) message.details = prediction.details;
        if (prediction.usage !== undefined) message.usage = prediction.usage;
        if (prediction.nestedCalls !== undefined) message.nestedCalls = prediction.nestedCalls;
    } else {
        message.content = [{type: 'text', text: text(prediction)}];
    }
    return message;
}
async function finishActions(branch,status) {
    for(const action of [...activeActions.values()])if(action.branch_id===branch && !(saving && suspendedHooks.has(key(branch,action.action_id))))
        await report({message:'action_finished',task_id:taskId,branch_id:branch,action_id:action.action_id,action_kind:action.action_kind,status,result:null,timestamp_ms:Date.now()});
}
function tools(branch) {
    const detect=bytes=>require(resolve(root,'packages/coding-agent/src/utils/mime.ts')).detectSupportedImageMimeType(bytes);
    const execution = {
        operation: async (...args) => {
            await acceptanceBoundary(branch, args[5]);
            return ascProxy.operation(...args);
        },
    };
    const builtins=environmentTools(execution,branch,detect,taskId);
    return [...builtins,...runtimeTools.map(spec=>({...spec,label:spec.name,parameters:Type.Unsafe(spec.parameters),execute:async(operation,args,signal,onUpdate)=>{
        await acceptanceBoundary(branch, signal);
        const value=await ascProxy.execute(branch,operation,spec.name,args,{task_id:taskId,tool_call_id:operation},signal,onUpdate);
        executionResults.set(key(branch,operation),value);
        return value.result?.content?value.result:{content:[{type:'text',text:text(value.result)}],details:undefined};
    }}))];
}
// FS execution must never fall back to host implementations selected through
// user settings or extensions. Only the isolated built-ins and Runtime tools
// have an execution plan in this integration.
const filesystemToolSelection={tools:['read','bash','edit','write',...runtimeTools.map(tool=>tool.name)]};
async function beforeModelInference(branch, modelValue, context, signal) {
    parkedModels.add(branch);
    await pauseBoundary(signal);
    await modelRestores.get(branch);
    await acceptanceBoundary(branch, signal);
    syncTranscript(branch);
    // Model inference does not pass the tool effect gate; admission and extent
    // remain owned by the generic access layer.
    if(saving||closing||cancelled.has(branch)||signal?.aborted)throw Error('Model inference cancelled before execution');
    const actionId=modelForks.get(branch)?.operation_id??`model-${nextAction++}`;
    const op={kind:'model_inference',name:'model',arguments:{model:`${modelValue.provider}/${modelValue.id}`},branch_id:branch,predicting:modelForks.has(branch),authority:branch===authority};
    op.has_predictor=await access.hasPredictor(branch);
    const hold=Promise.withResolvers();modelHolds.set(key(branch,actionId),hold);
    const disposition=await access.beforeAction({
        branch,
        actionId,
        actionKind:'model_inference',
        name:'model',
        arguments:{model:modelValue.provider+'/'+modelValue.id,message_count:context.messages.length},
        op,
        contractContext:{model:`${modelValue.provider}/${modelValue.id}`,message_count:context.messages.length,runtime_capabilities:runtimeTools},
        admit:true,
        admissionFacts:()=>{
            const saved=structuredClone(sessions.get(branch).messages),manager=tree.branches.get(branch).manager;
            const checkpoint={kind:'checkpoint',adapter:'pi',opaqueId:`${branch}:${actionId}:${saved.length}`};
            checkpoints.set(key(branch,actionId),{saved,checkpoint,treeTail:tree.assertCheckpoint(branch,saved),manager,model:true});
            return {checkpoint};
        },
        signal,
    });
    if(disposition.kind==='admit')await new Promise((resolve,reject)=>{
        const abort=()=>reject(Error('Model admission cancelled'));
        signal?.addEventListener('abort',abort,{once:true});
        hold.promise.then(resolve,reject).finally(()=>signal?.removeEventListener('abort',abort));
        if(signal?.aborted)abort();
    });
    modelHolds.delete(key(branch,actionId));
    if(closing||cancelled.has(branch)||signal?.aborted)throw Error('Model inference cancelled before execution');
    await pauseBoundary(signal);
    await acceptanceBoundary(branch, signal);
    for (const input of acceptanceInputs.get(branch) ?? []) {
        if (!context.messages.some(message => message.role === 'user' && message.content === input)) {
            context.messages.push({role:'user', content:input, timestamp:Date.now()});
        }
    }
    acceptanceInputs.delete(branch);
    parkedModels.delete(branch);
    const startedAt=Date.now();
    const queue=modelActions.get(branch)??[];
    const action={action_id:actionId,started_at_ms:startedAt,seen_message:false};
    queue.push(action);
    modelActions.set(branch,queue);
    await report({message:'action_started',task_id:taskId,branch_id:branch,action_id:actionId,action_kind:'model_inference',name:'model',arguments:{model:modelValue.provider+'/'+modelValue.id,message_count:context.messages.length},timestamp_ms:startedAt});
    return async status=>{
        if(status==='succeeded')return;
        // Pi settles the stream and runs this callback before the agent loop
        // emits the assistant message_end. Keep the action while its message
        // has started so message_end can still pair the usage. Only forget an
        // action that produced no message at all (the request threw before the
        // stream started), so it cannot be paired with a later message.
        // A provider can fail before its first stream event. Pi then emits a
        // synthetic assistant message after this callback; retain its identity
        // until that message arrives (or the turn exits without a message).
        action.terminalStatus=status??(signal?.aborted?'cancelled':'failed');
    };
}
function install(session,branch) {
    const before=session.agent.beforeToolCall, after=session.agent.afterToolCall;
    session.agent.beforeToolCall=async(context,signal)=>{
        if(cancelled.has(branch))throw Error('Tool cancelled');
        await modelRestores.get(branch);
        const resultHold=modelResultHolds.get(branch);
        if(resultHold)await new Promise((resolve,reject)=>{
            const abort=()=>reject(Error('Model continuation cancelled'));
            signal?.addEventListener('abort',abort,{once:true});
            resultHold.promise.then(resolve,reject).finally(()=>signal?.removeEventListener('abort',abort));
            if(signal?.aborted)abort();
        });
        const hookKey=key(branch,context.toolCall.id);
        pendingHooks.set(hookKey,{branch,toolCall:structuredClone(context.toolCall),args:structuredClone(context.args),assistantMessage:structuredClone(context.assistantMessage)});
        try {
        await pauseBoundary(signal);
        await acceptanceBoundary(branch, signal);
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
        const hold=Promise.withResolvers();toolHolds.set(hookKey,hold);
        const disposition=await access.beforeAction({
            branch,
            actionId:context.toolCall.id,
            actionKind:'tool_execution',
            name:context.toolCall.name,
            arguments:context.args,
            op:{kind:context.toolCall.name,name:context.toolCall.name,arguments:context.args,authority:branch===authority,has_predictor:await access.hasPredictor(branch)},
            contractContext:{arguments:context.args,runtime_capabilities:runtimeTools,isolated:!runtimeTools.some(t=>t.name===context.toolCall.name)},
            admit:()=>branch===authority || !runtimeTools.some(tool => tool.name === context.toolCall.name),
            admissionFacts:()=>({checkpoint:prepareCheckpoint()}),
            admitted:suspendedHooks.get(hookKey)?.admitted??false,
            signal,
        });
        if(disposition.kind==='admit')await new Promise((resolve,reject)=>{
            const abort=()=>reject(Error('Tool admission cancelled'));
            signal?.addEventListener('abort',abort,{once:true});
            hold.promise.then(resolve,reject).finally(()=>signal?.removeEventListener('abort',abort));
            if(signal?.aborted)abort();
        });
        toolHolds.delete(hookKey);
        if(signal?.aborted)throw Error('Tool cancelled');
        await pauseBoundary(signal);
        await acceptanceBoundary(branch, signal);
        const toolCheckpoint=prepareCheckpoint();
        const hook=pendingHooks.get(hookKey);if(hook)hook.started=true;
        await report({message:'action_started',task_id:taskId,branch_id:branch,action_id:context.toolCall.id,action_kind:'tool_execution',name:context.toolCall.name,arguments:context.args,checkpoint:toolCheckpoint,timestamp_ms:Date.now()});
        return previous;
        } catch(error) {
            toolHolds.delete(hookKey);
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
            const execution=executionResults.get(resultKey);executionResults.delete(resultKey);
            if(reportedActions.delete(key(branch,context.toolCall.id)))await report({message:'action_finished',task_id:taskId,branch_id:branch,action_id:context.toolCall.id,action_kind:'tool_execution',status:signal?.aborted?'cancelled':isError?'failed':'succeeded',result:execution?.result??{payload:content.length===1&&content[0].type==='text'?content[0].text:null,content,is_error:isError},evidence:execution?.evidence,timestamp_ms:Date.now()})
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
                if(action&&checkpoints.get(key(branch,action.action_id))?.model)modelResultHolds.set(branch,Promise.withResolvers());
                if(action)void report({message:'action_finished',task_id:taskId,branch_id:branch,action_id:action.action_id,action_kind:'model_inference',status:event.message.stopReason==='aborted'?'cancelled':event.message.stopReason==='error'?'failed':'succeeded',result:{tool_calls:structuredClone(event.message.content.filter(part=>part.type==='toolCall'))},evidence:{tool_calls:structuredClone(event.message.content.filter(part=>part.type==='toolCall'))},timestamp_ms:Date.now()});
                const fork=modelForks.get(branch);
                if(fork){modelForks.delete(branch);const restoring=session.setModel(fork.model).then(()=>session.setThinkingLevel(fork.thinkingLevel));modelRestores.set(branch,restoring);void restoring.catch(error=>report({message:'branch_failed',branch_id:branch,reason:String(error)}));}
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
    finally { parkedModels.delete(branch);syncTranscript(branch);await finishActions(branch,saving||cancelled.has(branch)||aborting.has(branch)?'cancelled':'failed'); }
    const last=[...session.messages].reverse().find(m=>m.role==='assistant');
    if(last?.stopReason==='error'&&!saving)await report({message:branch===authority?'turn_failed':'branch_failed',branch_id:branch,reason:last.errorMessage??'Model failed'});
    await report({message:'turn_completed',branch_id:branch});
    if(branch===authority)tree.settle();
}
// A turn may wait for a later control command (runtime result, Promote or
// Abort). Keep setup ordered, but never hold the command queue for the turn.
function executeTurn(session,branch,operation) {
    const task=run(session,branch,async()=>{await extensionBindings.get(branch);if(cancelled.has(branch)||closing) return;await operation()})
        .catch(error=>saving?undefined:report({message:'branch_failed',branch_id:branch,reason:String(error instanceof Error?error.message:error)}));
    tasks.add(task);
    const running=branchTasks.get(branch)??new Set();running.add(task);branchTasks.set(branch,running);
    void task.finally(()=>{running.delete(task);if(!running.size)branchTasks.delete(branch)}).catch(()=>ascProxy.destroy());
    void task.finally(()=>tasks.delete(task)).catch(()=>ascProxy.destroy());
}
async function handle(command) {
    const branch=command.branch_id;
    // Hold bookkeeping (local Policy, effect/pause/extent gates) belongs to the
    // generic access layer; each product only connects its own execution points.
    if(!['start_candidate','promote'].includes(command.command))await access.control(command);
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
                await waitForPause();
                captureHooks();
                await commandQueue;
                await Promise.allSettled([...setupTasks].filter(([branch])=>!candidateWaiters.get(branch)?.size).map(([,task])=>task));
                for(const [branch,session] of sessions){await notifications.drain(session);syncTranscript(branch)}
                const commandRecords=[];
                for(const [id,item] of commands){
                    const value=JSON.parse(item.serialized);
                    if(['save','load'].includes(value.command))continue;
                    commandRecords.push([id,{serialized:item.serialized,reply:!item.settled&&value.command==='start_candidate'?undefined:await item.result}]);
                }
                const snapshot=tree.snapshot(dirname(command.path),[...checkpoints.values()].map(c=>c.manager));
                const sourceFiles=new Map(snapshot.sources.map(source=>[source.originalFile,source.file]));
                saveRecovery(command.path,{version:1,taskId,tree:{...snapshot,originalEntries:snapshot.originalEntries.map(([id,entry])=>[id,{parentId:entry.parentId}]),sources:snapshot.sources.map(({entries,...reference})=>reference)},authority,nextAction,
                    resourceBinding:await recoveryBinding(root,snapshot.sources.map(source=>source.header.cwd)),
                    snapshotResources:snapshotResources(snapshot.sources),
                    executionConfig:{model:model??null,modelPattern:modelPattern??null,thinkingLevel,runtimeTools},
                    additionalModels:additionalModels??null,
                    access:access.snapshot(),generations:[...generations],results:[...results],acceptanceInputs:[...acceptanceInputs],
                    cancelled:[...cancelled],reportedEntries:[...reportedEntries],commands:commandRecords,
                    commandPrefix,commandAhead:[...commandAhead],
                    suspendedHooks:[...suspendedHooks],candidateSetups:[...candidateSetups],
                    interruptedBranches:[...interruptedBranches],
                    checkpoints:[...checkpoints].map(([id,c])=>[id,{...c,manager:sourceFiles.get(c.manager.getSessionFile())}])});
            }catch(reason){error=String(reason)}
            await report({message:'recovery_saved',path:command.path,error:error??null});
            if(error){saving=false;recoveryPaused=true;}
            return;
        }
        case 'load': {
            for(const [branch,environment] of Object.entries(command.environments))environments.set(branch,environment as string);
            for(const [branch,access] of Object.entries(command.execution_accesses??{}))ascProxy.bind(branch,access);
            // A failed load must not settle/delete the saved native sources.
            saving=true;
            let error;
            try {
                if(sessions.size)throw Error('Load requires an empty Agent adapter');
                const record=loadRecovery(command.path);
                for(const source of record.tree.sources){
                    // open() creates a session for a missing path: check the
                    // saved checksum first so missing references fail clearly.
                    if(checksum(readFileSync(source.file))!==source.checksum)throw Error(`Recovery source changed: ${source.file}`);
                    source.entries=SessionManager.open(source.file).getEntries();
                }
                const nativeEntries=new Map(record.tree.sources.flatMap(source=>source.entries.map(entry=>[entry.id,entry])));
                record.tree.originalEntries=record.tree.originalEntries.map(([id,control])=>{
                    const entry=nativeEntries.get(id);
                    if(!entry)throw Error(`Recovery Pi entry reference missing: ${id}`);
                    return [id,{...entry,parentId:control.parentId}];
                });
                if(record.commands.length>1024||(record.commandAhead?.length??0)>1024)throw Error('Recovery command identity window exceeds capacity');
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
                    const exact=entry?.type==='message'&&JSON.stringify(entry.message)===JSON.stringify(checkpoint.saved.at(-1));
                    const boundOperation=checkpoint.model
                        ? checkpoint.checkpoint?.opaqueId===`${owner}:${operation}:${checkpoint.saved.length}`
                        : entry?.message?.content?.some(part=>part.type==='toolCall'&&part.id===operation);
                    if(!exact||!boundOperation)throw Error('Recovery checkpoint context mismatch');
                }
                const restored=SessionTree.restore(record.tree,SessionManager,sessionFile);
                const managers=restored.recoveryManagers;
                for(const [,checkpoint] of record.checkpoints)if(!managers.has(checkpoint.manager))throw Error('Recovery checkpoint manager missing');
                tree=restored;authority=record.authority;nextAction=record.nextAction;
                access.restore(record.access);
                for(const [id,value] of record.generations)generations.set(id,value);
                for(const [id,value] of record.results)results.set(id,value);
                for(const [id,inputs] of record.acceptanceInputs??[])acceptanceInputs.set(id,inputs);
                for(const value of record.cancelled)cancelled.add(value);
                for(const value of record.reportedEntries)reportedEntries.add(value);
                commandPrefix=record.commandPrefix??0;
                for(const n of record.commandAhead??[])commandAhead.add(n);
                for(const [id,value] of record.commands)commands.set(id,{serialized:value.serialized,result:Promise.resolve(value.reply),settled:true});
                for(const [id,c] of record.checkpoints)checkpoints.set(id,{...c,manager:managers.get(c.manager)});
                for(const [id,hook] of record.suspendedHooks??[])suspendedHooks.set(id,hook);
                for(const [id,setup] of record.candidateSetups??[]){
                    const execution_access=command.execution_accesses?.[id];
                    if(!execution_access)throw Error('Recovered candidate execution access missing');
                    candidateSetups.set(id,{...setup,execution_access});
                }
                for(const branch of record.interruptedBranches??[])interruptedBranches.add(branch);
                for(const [branch,{manager,stopped}] of tree.branches){
                    if(stopped||candidateSetups.has(branch))continue;
                    const {session}=await createAgentSession({...filesystemToolSelection,cwd:manager.getHeader().cwd,sessionManager:manager,model,modelRuntime,thinkingLevel,customTools:tools(branch),beforeModelInference:(modelValue,context,signal)=>beforeModelInference(branch,modelValue,context,signal)});
                    bindInteraction(session,branch);
                }
                await reportSessionHistory(authority, tree.branches.get(authority)?.manager ?? tree.manager);
                linkSession(tree.manager);
                saving=false;
                recoveryPaused=true;
            }catch(reason){error=String(reason)}
            await report({message:'recovery_loaded',error:error??null});return;
        }
        case 'local_policy':
            return;
        case 'suspend_recovery': {
            pauseRequested=true;
            recoveryPaused=true;
            await waitForPause();
            captureHooks();
            await report({message:'recovery_suspended'});return;
        }
        case 'start_branch': {
            environments.set(branch,command.environment_id);
            ascProxy.bind(branch,command.execution_access);
            // The control plane explicitly starts this Branch as the authority;
            // a leftover authority marker in a reused session file must not
            // keep a previous run's winner in charge.
            if(command.role==='authority'){authority=branch;tree.adoptAuthority(branch)}
            const manager=tree.createBranch(branch,command.workspace,tree.manager,nativeReference ? nativeTail : tree.tail(branch)??tree.manager.getLeafId());
            const {session}=await createAgentSession({...filesystemToolSelection,cwd:command.workspace,sessionManager:manager,model,modelRuntime,thinkingLevel,customTools:tools(branch),beforeModelInference:(modelValue,context,signal)=>beforeModelInference(branch,modelValue,context,signal)});
            bindInteraction(session,branch);
            if(closing||cancelled.has(branch)){sessions.set(branch,session);await stopSession(branch);return}
            await report({message:'branch_created',branch_id:branch});
            if(nativeReference){
                // The control plane opens input on branch_started. Publish the
                // inherited conversation and restore the session link first.
                await reportSessionHistory(branch,manager);
                linkSession();
            }
            await report({message:'branch_started',branch_id:branch});
            await report({message:'branch_ready',branch_id:branch});
            if(command.prompt)executeTurn(session,branch,()=>session.prompt(command.prompt));
            return;
        }
        case 'continue':
        case 'prompt': {
            const continuing=command.command==='continue';
            if(!continuing)tree.readOnlyUntilPrompt=false;
            interaction.resume(branch);
            if(saving)throw Error('Execution is saved and waiting for exit');
            if(branch!==authority)throw Error('Only the authority accepts user input');
            const session=sessions.get(branch);if(!session)throw Error('Session missing');
            if(recoveryPaused){
                if(!continuing)return;
                recoveryPaused=false;
                const live=new Set(branchTasks.keys());
                for(const [id,hook] of suspendedHooks)if(live.has(hook.branch))suspendedHooks.delete(id);
                for(const owner of live)interruptedBranches.delete(owner);
                releasePause();
                // Live turns resume their existing hooks. Loaded turns rebuild
                // only saved hook positions from native messages and facts;
                // completed Runtime actions are never replayed.
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
                        if(owner===authority&&!continuing)await target.prompt(command.prompt,{streamingBehavior:'followUp'});else await target.continue();
                    });
                }
                for(const [owner,target] of sessions)if((owner!==authority||continuing)&&interruptedBranches.has(owner)&&!grouped.has(owner)&&!candidateSetups.has(owner)&&!cancelled.has(owner)){
                    const last=target.messages.at(-1);
                    // Pi excludes aborted assistant output from model context;
                    // keep its native record but continue from its prior input.
                    if(last?.role==='assistant'&&['aborted','error'].includes(last.stopReason))target.agent.state.messages=target.messages.slice(0,-1);
                    if(target.messages.at(-1)?.role!=='assistant')executeTurn(target,owner,()=>target.continue());
                }
                interruptedBranches.clear();
                if(grouped.has(branch)||continuing)return;
            }
            if(!continuing)executeTurn(session,branch,()=>session.prompt(command.prompt,{streamingBehavior:'followUp'}));return;
        }
        case 'start_candidate': {
            environments.set(branch,command.environment_id);
            ascProxy.bind(branch,command.execution_access);
            candidateSetups.set(branch,structuredClone(command));
            const parent=sessions.get(command.parent_branch_id),checkpoint=checkpoints.get(key(command.parent_branch_id,command.operation_id));
            if(!parent||!checkpoint||checkpoint.checkpoint.adapter!==command.checkpoint?.adapter||checkpoint.checkpoint.kind!==command.checkpoint?.kind||checkpoint.checkpoint.opaqueId!==command.checkpoint?.opaqueId)throw Error('Candidate requires its registered checkpoint');
            if(command.prediction?.predict==='branch_model'){
                const manager=tree.branches.get(branch)?.manager??tree.createBranch(branch,command.workspace,checkpoint.manager,checkpoint.treeTail);
                await access.control(command);
                const resolved=resolveCliModel({cliModel:command.prediction.model,cliThinking:parent.thinkingLevel,modelRuntime:parent.modelRuntime});
                if(resolved.error||!resolved.model)throw Error(resolved.error??'Unknown predictor model');
                const {session}=await createAgentSession({...filesystemToolSelection,cwd:command.workspace,sessionManager:manager,model:resolved.model,modelRuntime:parent.modelRuntime,thinkingLevel:parent.thinkingLevel,customTools:tools(branch),beforeModelInference:(modelValue,context,signal)=>beforeModelInference(branch,modelValue,context,signal)});
                bindInteraction(session,branch);
                modelForks.set(branch,{model:parent.model,thinkingLevel:parent.thinkingLevel,operation_id:command.operation_id});
                candidateSetups.delete(branch);
                await report({message:'branch_created',branch_id:branch});await report({message:'branch_started',branch_id:branch});await report({message:'branch_ready',branch_id:branch});
                if(!saving&&!recoveryPaused)executeTurn(session,branch,()=>session.continue());else interruptedBranches.add(branch);return;
            }
            const checkpointMessage=checkpoint.saved.findLast(message=>message.role==='assistant'&&message.content?.some(p=>p.type==='toolCall'&&p.id===command.operation_id&&p.name===command.tool_name));
            if(!checkpointMessage)throw Error('Invalid pending checkpoint');
            const manager=tree.branches.get(branch)?.manager??tree.createBranch(branch,command.workspace,checkpoint.manager,checkpoint.treeTail);
            if(saving)return;
            const paired: ToolResultMessage[]=[];
            for(const part of checkpointMessage.content){
                if(part.type!=='toolCall')continue;
                if(part.id===command.operation_id)paired.push(predictionResult(part,command.prediction));
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
            bindInteraction(session,branch);
            if(closing||cancelled.has(branch)){sessions.set(branch,session);await stopSession(branch);return}
            candidateSetups.delete(branch);
            await report({message:'branch_created',branch_id:branch});
            // Injected result is new candidate content, not copied from the Parent.
            for(let i=0;i<paired.length;i++)await report({message:'transcript',branch_id:branch,entry:paired[i],session_entry_id:pairedEntryIds[i]});
            await report({message:'branch_started',branch_id:branch});
            await report({message:'branch_ready',branch_id:branch});
            if(!saving&&!recoveryPaused)executeTurn(session,branch,()=>session.continue());else interruptedBranches.add(branch);return;
        }
        case 'pause':
        case 'resume':
            return;
        case 'resolve_pending': {
            if(command.outcome===undefined)return;
            if(command.outcome?.phase==='execute'){toolHolds.get(key(branch,command.operation_id))?.resolve();modelHolds.get(key(branch,command.operation_id))?.resolve();return;}
            const modelHold=modelHolds.get(key(branch,command.operation_id));
            if(modelHold){modelHold.resolve();return;}
            if(command.outcome?.phase==='continue'){modelResultHolds.get(branch)?.resolve();modelResultHolds.delete(branch);return;}
            const id=key(branch,command.operation_id),waiter=waiters.get(id);if(waiter){waiters.delete(id);waiter(command.outcome)}else results.set(id,command.outcome);
            return;
        }
        case 'prepare_acceptance': {
            const parent = command.authority_branch_id;
            if (parent === branch || [parent, branch].some(id => cancelled.has(id) || !sessions.has(id))) {
                throw Error('Acceptance sessions unavailable');
            }
            if (!command.target_suspended) {
                if ([parent, branch].some(id => acceptancePauses.has(id))) throw Error('Acceptance already pending');
                for (const id of [parent, branch]) acceptancePauses.set(id, Promise.withResolvers());
                return;
            }
            if ([parent, branch].some(id => !acceptancePauses.has(id))) throw Error('Acceptance pause missing');
            // The control plane has confirmed the target Environment is suspended.
            // A parked candidate turn can contain independent descendants.
            // Only an already-started inference owned by this session drains here.
            while ([...activeActions.values()].some(action => action.branch_id === branch && action.action_kind === 'model_inference')) {
                if (cancelled.has(branch) || cancelled.has(parent) || closing || !acceptancePauses.has(parent)) throw Error('Acceptance interrupted');
                await new Promise(resolve => setTimeout(resolve, 5));
            }
            return;
        }
        case 'cancel_acceptance':
            if (branch === authority && tree.accepted?.has(branch)) {
                await access.control({command:'promote',branch_id:branch});
            }
            releaseAcceptance(branch);
            return;
        case 'promote': {
            if(cancelled.has(branch)||!sessions.has(branch))throw Error('Branch unavailable');
            const parent = command.parent_branch_id ?? authority;
            if (command.part) {
                if (!sessions.has(parent) || !acceptancePauses.has(parent)) throw Error('Target acceptance pause missing');
                if (command.notification) {
                    const inputs = acceptanceInputs.get(parent) ?? [];
                    acceptanceInputs.set(parent, [...inputs, command.notification]);
                }
                return;
            }
            if (command.execution_access) {
                if (!acceptancePauses.has(branch) || !acceptancePauses.has(parent)) throw Error('Full acceptance pause missing');
                if (environments.get(parent) !== command.execution_access.environment_id) throw Error('Full acceptance target mismatch');
                ascProxy.unbind(parent);
                environments.delete(parent);
                ascProxy.bind(branch, command.execution_access);
                environments.set(branch, command.execution_access.environment_id);
                const inputs = acceptanceInputs.get(parent);
                if (inputs?.length) acceptanceInputs.set(branch, [...inputs, ...(acceptanceInputs.get(branch) ?? [])]);
                acceptanceInputs.delete(parent);
            } else {
                await access.control(command);
            }
            tree.accept(branch, parent, parent === authority);
            if (parent === authority) authority = branch;
            tree.settle();
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
        }
        case 'ui_prompt_answer':
            interaction.answer(branch,command.prompt_id,command.answer,command.cancelled,command.reason);return;
        case 'get_commands': {
            if(commandCatalog !== undefined) await report({message:'commands_listed',branch_id:authority,request_id:command.request_id,commands:commandCatalog});
            return;
        }
        case 'abort':
            pauseRequested=false;
            saving=false;
            recoveryPaused=false;
            interruptedBranches.delete(branch);
            for(const [id,hook] of suspendedHooks)if(hook.branch===branch)suspendedHooks.delete(id);
            interaction.cancel(branch);
            aborting.add(branch);
            reconnectControllers.get(branch)?.abort();
            try { const stopped=sessions.get(branch)?.abort();await ascProxy.cancel(branch);await stopped;await finishActions(branch,'cancelled');await report({message:'turn_completed',branch_id:branch}); }
            finally { aborting.delete(branch); }
            return;
        case 'discard': interaction.cancel(branch);cancelled.add(branch);reconnectControllers.get(branch)?.abort();candidateSetups.delete(branch);releaseCandidateWaiters(branch);await sessions.get(branch)?.abort();await ascProxy.cancel(branch);await finishActions(branch,'cancelled');tree.settle();return;
        case 'cleanup':
            interaction.cancel(branch);
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
    const operation=['abort','prepare_acceptance','cancel_acceptance','promote','resolve_pending','discard','cleanup'].includes(command.command)?command.command:undefined;
    let reason;
    const acknowledgement=status=>({message:'command_result',command_id:command.command_id,branch_id:command.branch_id,operation,status,generation:command.generation??0,...(command.operation_id?{operation_id:command.operation_id}:{})});
    try{
        if(saving&&!['abort','save','suspend_recovery','cancel_save','cancel_acceptance','resolve_pending','promote','discard','cleanup','start_candidate'].includes(command.command))throw Error('Save in progress; command not applied');
        const generation=generations.get(command.branch_id);
        if(generation!==undefined&&command.generation!==undefined&&generation!==command.generation)throw Error('Stale command generation');
        if(command.generation!==undefined)generations.set(command.branch_id,command.generation);
        if(operation&&command.command_id){await report(acknowledgement('accepted'));await report(acknowledgement('started'))}
        await handle(command);
    }catch(error){reason=String(error instanceof Error?error.message:error)}
    if(operation&&command.command_id)return {message:'command_result',command_id:command.command_id,branch_id:command.branch_id,operation,status:reason?'failed':'applied',generation:command.generation??0,...(command.operation_id?{operation_id:command.operation_id}:{}),...(reason?{reason}:{})};
    // A failed command-list query is not a Branch failure; the UI simply gets
    // no list for that request.
    if(reason&&command.command==='suspend_recovery'&&!pauseRequested)return;
    if(reason&&command.command!=='get_commands'&&!(saving&&command.command==='start_candidate'))return {message:'branch_failed',branch_id:command.branch_id,reason};
}
function startSetup(command){
    const task=dispatch(command);setupTasks.set(command.branch_id,task);
    void task.then(reply=>reply?report(reply):undefined).finally(()=>setupTasks.delete(command.branch_id));
}
await report({message:'agent_connected',agent_id:agentId,capabilities});
await report({message:'agent_ready',agent_id:agentId,session:{file:tree.manager.getSessionFile(),id:tree.manager.getHeader().id,cwd:tree.manager.getHeader().cwd}});
const tasks=new Set();
const deliveries=new IdentityWindow(positiveSequence);
let commandQueue: Promise<unknown>=Promise.resolve();
try{
    // Do not let the async iterator's clean EOF destroy the socket with a
    // synthetic AbortError while the writable half is still ending. Keep the
    // explicit half-close below as the only shutdown path.
    for await(const wire of ascProxy.messages()){
            if(!['command','delivery'].includes(wire.kind))throw Error('Unexpected message direction');
            const command=wire.kind==='delivery'?wire.payload.command:wire.payload;
            if(wire.kind==='delivery'){
                const id=String(wire.payload.id),encoded=JSON.stringify(command),previous=deliveries.get(id);
                if(previous!==undefined&&previous!==encoded)throw Error('delivery id reused with different command');
                if(previous===undefined)deliveries.accept(id,encoded);
                await sendWire({kind:'received',payload:{id:wire.payload.id}});
                if(previous!==undefined)continue;
            }
            // Session-start extensions may themselves await a prompt while
            // holding the command queue. Cancellation must release them first.
            const id=command.command_id;
            const old=id?commands.get(id):undefined;
            if(old&&old.serialized!==JSON.stringify(command)){await report({message:'branch_failed',branch_id:command.branch_id,reason:'Command id reused with different payload'});continue}
            pruneCommands();
            const sequence=id?commandSequence(id):undefined;
            if(!old&&id&&(sequence!==undefined&&(sequence<=commandPrefix||commandAhead.has(sequence))||commands.size>=1024)){
                await report({message:'branch_failed',branch_id:command.branch_id,reason:'Command identity expired or window full; command was not executed'});continue;
            }
            if(!old&&['abort','discard','cleanup'].includes(command.command))interaction.cancel(command.branch_id);
            // A candidate may wait for sibling tool results. It must not hold
            // up resolve_pending / cleanup on the transport's command queue.
            // Save must interrupt immediately even if setup is awaiting a tool.
            const result=old?.result??(['save','suspend_recovery','ui_prompt_answer','prepare_acceptance'].includes(command.command)
                ?dispatch(command):command.command==='start_candidate'
                ?commandQueue.then(()=>dispatch(command))
                :(commandQueue=commandQueue.then(()=>dispatch(command))));
            if(command.command==='start_candidate'){
                setupTasks.set(command.branch_id,result);
                void result.finally(()=>{if(setupTasks.get(command.branch_id)===result)setupTasks.delete(command.branch_id)}).catch(()=>{});
            }
            if(id&&!old){
                const item={serialized:JSON.stringify(command),result,settled:false};commands.set(id,item);
                if(sequence!==undefined){commandAhead.add(sequence);while(commandAhead.delete(commandPrefix+1))commandPrefix++}
                void result.finally(()=>{item.settled=true;pruneCommands()}).catch(()=>{});
            }
            const task=result.then(reply=>reply?report(reply):undefined);tasks.add(task);void task.finally(()=>tasks.delete(task)).catch(()=>ascProxy.destroy());
    }
}catch(error){
    transportError??=error;
}finally{
    if(saving){
        await ascProxy.close();
        process.exit(transportError||ascProxy.error?1:0);
    }
    closing=true;interaction.cancel();access.releaseAll();
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
    if(!transportError)await report({message:'agent_released',agent_id:agentId,session:{file:tree.manager.getSessionFile(),id:tree.manager.getHeader().id,cwd:tree.manager.getHeader().cwd}});
    await ascProxy.close();
    // An aborted local-policy evaluation can leave the evaluator child's stdio
    // handles open after it rejects; those handles would otherwise keep this
    // adapter alive even though every report is flushed and the socket is
    // closed. Shutdown is complete here, so terminate explicitly.
    if(transportError)console.error('Agent control connection failed:',transportError);
    process.exit(transportError||ascProxy.error?1:0);
}

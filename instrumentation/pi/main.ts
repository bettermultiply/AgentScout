// @ts-nocheck
import type { ToolCall, ToolResultMessage } from '../../../pi/packages/ai/src/types.ts';
import { createRequire } from 'node:module';
import { resolve, dirname } from 'node:path';
import * as telemetry from './telemetry.ts';
import { loadModelRuntime } from './model-runtime.ts';
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
const identityWindowPath = resolve(root, 'packages/pi-integration/src/identity-window.ts');
const localPolicyPath = resolve(root, 'packages/pi-integration/src/local-policy.ts');
let IdentityWindow, positiveSequence, evaluatePolicyArtifact, piModelComplete;
try {
    ({ IdentityWindow, positiveSequence } = require(identityWindowPath));
    ({ evaluatePolicyArtifact, piModelComplete } = require(localPolicyPath));
} catch (error) {
    const reason = error instanceof Error ? `: ${error.message}` : '';
    throw Error(`Failed to load Pi integration modules at ${identityWindowPath} and ${localPolicyPath}${reason}`, {cause: error});
}
const { createAgentSession: createNativeAgentSession, DefaultResourceLoader, SettingsManager, getAgentDir, SessionManager, ModelRuntime, resolveCliModel } = require(resolve(root, 'packages/coding-agent/src/index.ts'));
const nativeCommandSources = new WeakMap();
async function createAgentSession(options) {
    if (!DefaultResourceLoader) return createNativeAgentSession(options);
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
    ({ runtime: modelRuntime, additionalModels } = await loadModelRuntime(() => ModelRuntime.create(), modelsPath));
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
const toolResultHolds = new Map<string, Map<string, PromiseWithResolvers<void>>>();
const modelRestores = new Map();
const transcriptQueues = new Map();
const reportedEntries = new Set();
const candidateWaiters = new Map();
const candidateBindings = new Map();
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
function releaseToolResults(branch, operation?) {
    const holds = toolResultHolds.get(branch);
    if (!holds) return;
    if (operation !== undefined) {
        holds.get(operation)?.resolve();
        holds.delete(operation);
        if (holds.size) return;
    } else {
        for (const hold of holds.values()) hold.resolve();
    }
    toolResultHolds.delete(branch);
}
function holdToolResult(branch, operation) {
    const holds = toolResultHolds.get(branch) ?? new Map();
    if (!holds.has(operation)) holds.set(operation, Promise.withResolvers<void>());
    toolResultHolds.set(branch, holds);
}
async function waitForToolResults(branch, signal) {
    while (toolResultHolds.has(branch)) {
        const results = Promise.all([...toolResultHolds.get(branch).values()].map(hold => hold.promise));
        await new Promise<void>((resolve, reject) => {
            const cleanup = () => signal?.removeEventListener('abort', abort);
            const abort = () => {
                cleanup();
                reject(Error('Tool result arbitration interrupted'));
            };
            signal?.addEventListener('abort', abort, {once:true});
            void results.then(() => { cleanup(); resolve(); });
            if (signal?.aborted) abort();
        });
    }
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
    // session_start can await a UI answer delivered through the command queue.
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
    const stopped = session?.abort();
    releaseToolResults(branch);
    await stopped;
    await Promise.allSettled([...(branchTasks.get(branch)??[])]);
    // Pi's dispose() does not emit session_shutdown. Emit it while appends are valid.
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
    if(branch===authority&&pending.length)tree.settle();
    for(const payload of pending)void report(payload);
}
async function reportSessionHistory(branch, manager) {
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
    if(payload.message==='action_finished'){
        const id=key(payload.branch_id,payload.action_id);
        const started=activeActions.get(id);
        activeActions.delete(id);
        if(started)access.afterAction({branch:payload.branch_id,actionId:payload.action_id,
            actionKind:payload.action_kind,name:started.name,status:payload.status,
            isError:payload.status==='failed'||payload.result?.is_error===true});
    }
    payload={source:'agent_adapter',...payload};
    return sendWire({kind:'message',payload,trace_context:telemetry.observe(payload)});
};
const sendWire = wire => ascProxy.send(wire);
let policyModelRuntime;
const access = createInterposer({taskId,report,traceEvaluation:telemetry.evaluate,evaluate:(artifact, decision, data, signal) => telemetry.evaluate(decision, 'sandbox', async () => {
    const grant = artifact.policy_model_grant;
    if (!grant?.decisions.includes(decision)) return evaluatePolicyArtifact(artifact, decision, data, signal);
    const runtime = modelRuntime ?? await (policyModelRuntime ??= ModelRuntime.create());
    return evaluatePolicyArtifact(artifact, decision, data, signal, {
        decision, models: grant.models, requests: grant.max_requests, timeoutMs: grant.timeout_ms,
        complete: ((complete) => (...args) => telemetry.policyModel(() => complete(...args)))(piModelComplete({
            getModel: runtime.getModel.bind(runtime),
            complete: async (...args) => {
                const response = await runtime.complete(...args);
                telemetry.recordPolicyModelUsage(response.usage);
                return response;
            },
        })),
    });
})});
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
            return telemetry.toolContext(branch,args[1],()=>ascProxy.operation(...args));
        },
    };
    const builtins=environmentTools(execution,branch,detect,taskId);
    return [...builtins,...runtimeTools.map(spec=>({...spec,label:spec.name,parameters:Type.Unsafe(spec.parameters),execute:async(operation,args,signal,onUpdate)=>{
        await acceptanceBoundary(branch, signal);
        const value=await telemetry.toolContext(branch,operation,()=>ascProxy.execute(branch,operation,spec.name,args,{task_id:taskId,tool_call_id:operation},signal,onUpdate));
        executionResults.set(key(branch,operation),value);
        return value.result?.content?value.result:{content:[{type:'text',text:text(value.result)}],details:undefined};
    }}))];
}
// Host tool fallbacks would bypass filesystem isolation.
const filesystemToolSelection={tools:['read','bash','edit','write',...runtimeTools.map(tool=>tool.name)]};
async function beforeModelInference(branch, modelValue, context, signal) {
    parkedModels.add(branch);
    await pauseBoundary(signal);
    await modelRestores.get(branch);
    await waitForToolResults(branch, signal);
    await pauseBoundary(signal);
    await acceptanceBoundary(branch, signal);
    syncTranscript(branch);
    if(saving||closing||cancelled.has(branch)||signal?.aborted)throw Error('Model inference cancelled before execution');
    const actionId=modelForks.get(branch)?.operation_id??`model-${nextAction++}`;
    const op={kind:'model_inference',name:'model',produces_result:true,result_pending:true,arguments:{model:`${modelValue.provider}/${modelValue.id}`},branch_id:branch,predicting:modelForks.has(branch),authority:branch===authority};
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
        admit:!candidateBindings.has(branch),
        admissionFacts:()=>{
            const saved=Object.freeze(sessions.get(branch).messages.slice()),manager=tree.branches.get(branch).manager;
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
        // Stream completion precedes message_end, which still needs the action ID.
        // Pi can emit a synthetic assistant message even if no stream event arrived.
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
        const binding=candidateBindings.get(branch);
        if(binding) {
            await report({message:'environment_requested',branch_id:branch});
            await new Promise((resolve,reject)=>{
                const abort=()=>reject(Error('Candidate tool cancelled before environment binding'));
                signal?.addEventListener('abort',abort,{once:true});
                binding.promise.then(resolve,reject).finally(()=>signal?.removeEventListener('abort',abort));
                if(signal?.aborted)abort();
            });
            await pauseBoundary(signal);
            await acceptanceBoundary(branch, signal);
            if(cancelled.has(branch)||!ascProxy.has(branch))throw Error('Candidate execution unavailable');
        }
        const previous=await before?.(context,signal);
        if(previous?.block||signal?.aborted){pendingHooks.delete(key(branch,context.toolCall.id));return previous;}
        let saved, checkpoint;
        const prepareCheckpoint=()=>{
            if(checkpoint)return checkpoint;
            if(suspendedHooks.get(hookKey)?.admitted)return checkpoints.get(hookKey).checkpoint;
            syncTranscript(branch);
            const reconnect=suspendedHooks.get(hookKey);
            const index=reconnect?session.messages.findIndex(m=>m.role==='assistant'&&m.content?.some(p=>p.type==='toolCall'&&p.id===context.toolCall.id)):-1;
            saved=Object.freeze(session.messages.slice(0,index>=0?index+1:undefined));
            checkpoint={kind:'checkpoint',adapter:'pi',opaqueId:`${branch}:${context.toolCall.id}:${saved.length}`};
            const manager=tree.branches.get(branch).manager;
            const treeTail=reconnect?manager.getEntries().find(e=>e.type==='message'&&JSON.stringify(e.message)===JSON.stringify(saved.at(-1)))?.id:tree.assertCheckpoint(branch,saved);
            if(!treeTail)throw Error('Suspended tool checkpoint missing');
            checkpoints.set(key(branch,context.toolCall.id),{saved,checkpoint,treeTail,manager:tree.branches.get(branch).manager});
            return checkpoint;
        };
        const hold=Promise.withResolvers();toolHolds.set(hookKey,hold);
        const canStartLocally=branch===authority && !saving && !pauseRequested && !recoveryPaused
            && !acceptancePauses.size && !candidateBindings.size && !setupTasks.size
            && !toolResultHolds.size && !suspendedHooks.has(hookKey)
            && pendingHooks.size===1 && context.assistantMessage?.content?.filter(part=>part.type==='toolCall').length===1;
        const disposition=await access.beforeAction({
            branch,
            actionId:context.toolCall.id,
            actionKind:'tool_execution',
            name:context.toolCall.name,
            arguments:context.args,
            op:{kind:context.toolCall.name,name:context.toolCall.name,produces_result:true,result_pending:true,arguments:context.args??{},authority:branch===authority,has_predictor:await access.hasPredictor(branch)},
            contractContext:{arguments:context.args,runtime_capabilities:runtimeTools,isolated:!runtimeTools.some(t=>t.name===context.toolCall.name)},
            admit:()=>branch===authority || !runtimeTools.some(tool => tool.name === context.toolCall.name),
            admissionFacts:()=>{
                const checkpoint = prepareCheckpoint();
                checkpoints.get(hookKey).toolAdmitted = true;
                return {checkpoint};
            },
            fastStart:canStartLocally?{
                chanceId:`chance-local-${randomUUID().replaceAll('-','')}`,
                start(prediction){
                    const id=`candidate-${this.chanceId}`;
                    const record=checkpoints.get(hookKey);
                    record.fastStarted=true;
                    candidateBindings.set(id,Promise.withResolvers());
                    const task=handle({command:'start_candidate',branch_id:id,parent_branch_id:branch,
                        operation_id:context.toolCall.id,tool_name:context.toolCall.name,
                        workspace:session.sessionManager.getHeader().cwd,prediction,checkpoint:record.checkpoint,local:true})
                        .catch(error=>report({message:'branch_failed',branch_id:id,reason:String(error)}));
                    setupTasks.set(id,task);
                    void task.finally(()=>setupTasks.delete(id)).catch(()=>ascProxy.destroy());
                }
            }:undefined,
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
        const started=report({message:'action_started',task_id:taskId,branch_id:branch,action_id:context.toolCall.id,action_kind:'tool_execution',name:context.toolCall.name,arguments:context.args,checkpoint:toolCheckpoint,timestamp_ms:Date.now()});
        if(!checkpoints.get(hookKey)?.fastStarted)await started;
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
            const runtimeResult = execution?.result;
            const resultFields = runtimeResult && typeof runtimeResult === 'object' && !Array.isArray(runtimeResult) ? runtimeResult : {};
            const actionResult={payload:runtimeResult ?? (content.length===1&&content[0].type==='text'?content[0].text:null),
                ...resultFields,content,is_error:isError,tool_name:context.toolCall.name,arguments:context.args??{},
                details:previous?.details??context.result?.details??null};
            if (reportedActions.delete(resultKey)) {
                const status = signal?.aborted ? 'cancelled' : isError ? 'failed' : 'succeeded';
                if (status !== 'cancelled' && checkpoint.toolAdmitted && !checkpoint.speculationSkipped) {
                    holdToolResult(branch, context.toolCall.id);
                }
                await report({message:'action_finished',task_id:taskId,branch_id:branch,action_id:context.toolCall.id,action_kind:'tool_execution',status,result:actionResult,evidence:execution?.evidence,timestamp_ms:Date.now()});
            }
        }
        return previous;
    };
    session.subscribe(event=>{
        if(event.type==='message_start'&&event.message.role==='assistant'){
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
                if(action&&checkpoints.get(key(branch,action.action_id))?.model&&!checkpoints.get(key(branch,action.action_id))?.speculationSkipped)modelResultHolds.set(branch,{operation:action.action_id,...Promise.withResolvers()});
                if(action)void report({message:'action_finished',task_id:taskId,branch_id:branch,action_id:action.action_id,action_kind:'model_inference',status:event.message.stopReason==='aborted'?'cancelled':event.message.stopReason==='error'?'failed':'succeeded',result:{tool_calls:structuredClone(event.message.content.filter(part=>part.type==='toolCall'))},evidence:{tool_calls:structuredClone(event.message.content.filter(part=>part.type==='toolCall'))},timestamp_ms:Date.now()});
                const fork=modelForks.get(branch);
                if(fork){modelForks.delete(branch);const restoring=session.setModel(fork.model).then(()=>session.setThinkingLevel(fork.thinkingLevel));modelRestores.set(branch,restoring);void restoring.catch(error=>report({message:'branch_failed',branch_id:branch,reason:String(error)}));}
                const started_at_ms=Number.isFinite(event.message.timestamp)?event.message.timestamp:(action?.started_at_ms??Date.now());
                const duration_ms=Number.isFinite(event.message.durationMs)?event.message.durationMs:Math.max(0,Date.now()-started_at_ms);
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
// A turn can await a later command, so the queue only orders its setup.
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
    if (command.command === 'start_candidate') telemetry.startBranch(branch, 'candidate', command.parent_branch_id, command.local === true);
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
                    candidateBindings:[...candidateBindings.keys()],
                    interruptedBranches:[...interruptedBranches],
                    toolResultHolds:[...toolResultHolds].map(([branch, holds]) => [branch, [...holds.keys()]]),
                    checkpoints:[...checkpoints].map(([id,c])=>[id,{...c,manager:sourceFiles.get(c.manager.getSessionFile())}])});
            }catch(reason){error=String(reason)}
            await report({message:'recovery_saved',path:command.path,error:error??null});
            if(error){saving=false;recoveryPaused=true;}
            return;
        }
        case 'load': {
            for(const [branch,environment] of Object.entries(command.environments))environments.set(branch,environment as string);
            for(const [branch,access] of Object.entries(command.execution_accesses??{}))ascProxy.bind(branch,access);
            saving=true;
            let error;
            try {
                if(sessions.size)throw Error('Load requires an empty Agent adapter');
                const record=loadRecovery(command.path);
                for(const source of record.tree.sources){
                    // Verify the file first: Pi's open() creates a missing session.
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
                for (const [branch] of record.tree.branches) telemetry.recover(branch, authority);
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
                for(const id of record.candidateBindings??[]) {
                    if(!ascProxy.has(id))candidateBindings.set(id,Promise.withResolvers());
                }
                for(const [id,setup] of record.candidateSetups??[]){
                    const execution_access=command.execution_accesses?.[id];
                    if(!execution_access&&!setup.local)throw Error('Recovered candidate execution access missing');
                    candidateSetups.set(id,{...setup,execution_access});
                }
                for(const [branch, operations] of record.toolResultHolds??[]) {
                    for(const operation of operations) holdToolResult(branch, operation);
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
            if(command.role==='authority'){authority=branch;tree.adoptAuthority(branch)}
            const manager=tree.createBranch(branch,command.workspace,tree.manager,nativeReference ? nativeTail : tree.tail(branch)??tree.manager.getLeafId());
            const {session}=await createAgentSession({...filesystemToolSelection,cwd:command.workspace,sessionManager:manager,model,modelRuntime,thinkingLevel,customTools:tools(branch),beforeModelInference:(modelValue,context,signal)=>beforeModelInference(branch,modelValue,context,signal)});
            bindInteraction(session,branch);
            if(closing||cancelled.has(branch)){sessions.set(branch,session);await stopSession(branch);return}
            await report({message:'branch_created',branch_id:branch});
            if(nativeReference){
                // branch_started opens input, so restore history and its link first.
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
            if (command.command === 'prompt') telemetry.prompt(branch);
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
                // Rebuild only saved hook positions to avoid replaying Runtime actions.
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
                    // Pi excludes aborted assistant output from model context.
                    if(last?.role==='assistant'&&['aborted','error'].includes(last.stopReason))target.agent.state.messages=target.messages.slice(0,-1);
                    if(target.messages.at(-1)?.role!=='assistant')executeTurn(target,owner,()=>target.continue());
                }
                interruptedBranches.clear();
                if(grouped.has(branch)||continuing)return;
            }
            if(!continuing)executeTurn(session,branch,()=>session.prompt(command.prompt,{streamingBehavior:'followUp'}));return;
        }
        case 'start_candidate': {
            if(!command.local){
                environments.set(branch,command.environment_id);
                ascProxy.bind(branch,command.execution_access);
            }
            candidateSetups.set(branch,structuredClone(command));
            const parent=sessions.get(command.parent_branch_id),checkpoint=checkpoints.get(key(command.parent_branch_id,command.operation_id));
            if(!parent||!checkpoint||checkpoint.checkpoint.adapter!==command.checkpoint?.adapter||checkpoint.checkpoint.kind!==command.checkpoint?.kind||checkpoint.checkpoint.opaqueId!==command.checkpoint?.opaqueId)throw Error('Candidate requires its registered checkpoint');
            if(command.prediction?.predict==='branch_model'){
                const manager=tree.branches.get(branch)?.manager??tree.createBranch(branch,command.workspace,checkpoint.manager,checkpoint.treeTail,false);
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
            const manager=tree.branches.get(branch)?.manager??tree.createBranch(branch,command.workspace,checkpoint.manager,checkpoint.treeTail,false);
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
            await access.control(command);
            const {session}=await createAgentSession({...filesystemToolSelection,cwd:command.workspace,sessionManager:manager,model:parent.model,modelRuntime:parent.modelRuntime,thinkingLevel:parent.thinkingLevel,customTools:tools(branch),beforeModelInference:(modelValue,context,signal)=>beforeModelInference(branch,modelValue,context,signal)});
            bindInteraction(session,branch);
            if(closing||cancelled.has(branch)){sessions.set(branch,session);await stopSession(branch);return}
            candidateSetups.delete(branch);
            await report({message:'branch_created',branch_id:branch});
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
            if(command.outcome?.phase==='bind_environment'){
                environments.set(branch,command.outcome.environment_id);
                ascProxy.bind(branch,command.outcome.execution_access);
                candidateBindings.get(branch)?.resolve();
                candidateBindings.delete(branch);
                return;
            }
            if(command.outcome?.phase==='execute'){
                const actionKey=key(branch,command.operation_id);
                const checkpoint=checkpoints.get(actionKey);
                if(checkpoint&&command.outcome.speculation_skipped)checkpoint.speculationSkipped=true;
                toolHolds.get(actionKey)?.resolve();modelHolds.get(actionKey)?.resolve();return;
            }
            if (command.outcome?.phase === 'continue') {
                releaseToolResults(branch, command.operation_id);
                const modelResult = modelResultHolds.get(branch);
                if (modelResult?.operation === command.operation_id) {
                    modelResult.resolve();
                    modelResultHolds.delete(branch);
                }
                return;
            }
            const modelHold=modelHolds.get(key(branch,command.operation_id));
            if(modelHold){modelHold.resolve();return;}
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
            // A parked turn can have independent descendants. Drain only this session's inference.
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
            if(branch===authority&&ascProxy.has(branch)){
                candidateBindings.get(branch)?.resolve();
                candidateBindings.delete(branch);
            }
            return;
        case 'promote': {
            if(cancelled.has(branch)||!sessions.has(branch))throw Error('Branch unavailable');
            const parent = command.parent_branch_id ?? authority;
            if (command.part) {
                if (!sessions.has(parent) || !acceptancePauses.has(parent)) throw Error('Target acceptance pause missing');
                if (command.notification) {
                    try {
                        await notifications.accept(sessions.get(parent), command);
                        syncTranscript(parent);
                    } catch (error) {
                        await report({message:'adapter_capability_gap',branch_id:parent,capability:'notification',status:'failed',reason:`${command.command_id}: ${error}`});
                    }
                }
                return;
            }
            if (command.reuse_parent_environment) {
                if(parent!==authority||!candidateBindings.has(branch)||!environments.has(parent))throw Error('Parent environment handoff unavailable');
                ascProxy.transfer(parent,branch);
                candidateBindings.get(branch)?.resolve();
                candidateBindings.delete(branch);
                environments.set(branch,environments.get(parent));
                environments.delete(parent);
                const inputs=acceptanceInputs.get(parent);
                if(inputs?.length)acceptanceInputs.set(branch,[...inputs,...(acceptanceInputs.get(branch)??[])]);
                acceptanceInputs.delete(parent);
                await access.control(command);
            } else if (command.execution_access) {
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
            const replacesAuthority = parent === authority;
            tree.accept(branch, parent, replacesAuthority);
            if (replacesAuthority) {
                authority = branch;
                if (command.execution_access) await access.control(command);
            }
            telemetry.promote(branch, parent, replacesAuthority);
            tree.settle();
            if(command.notification!==undefined){
                try {
                    await notifications.accept(sessions.get(branch),command);
                    syncTranscript(branch);
                } catch(error) {
                    // Notification persistence cannot undo an applied Promote.
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
            try { const stopped=sessions.get(branch)?.abort();releaseToolResults(branch);await ascProxy.cancel(branch);await stopped;await finishActions(branch,'cancelled');await report({message:'turn_completed',branch_id:branch}); }
            finally { aborting.delete(branch); }
            return;
        case 'discard': {
            candidateBindings.get(branch)?.resolve();candidateBindings.delete(branch);
            interaction.cancel(branch);cancelled.add(branch);reconnectControllers.get(branch)?.abort();candidateSetups.delete(branch);releaseCandidateWaiters(branch);
            releaseToolResults(branch);
            await sessions.get(branch)?.abort();await ascProxy.cancel(branch);await finishActions(branch,'cancelled');tree.settle();return;
        }
        case 'cleanup':
            candidateBindings.get(branch)?.resolve();candidateBindings.delete(branch);
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
async function dispatch(command, incoming?: unknown){
    return telemetry.command(command.command, incoming, () => applyCommand(command));
}
async function applyCommand(command){
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
    // An iterator AbortError on clean EOF would interrupt the socket's pending writes.
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
            // Release extension prompts before queuing cleanup that may await binding.
            const id=command.command_id;
            const old=id?commands.get(id):undefined;
            if(old&&old.serialized!==JSON.stringify(command)){await report({message:'branch_failed',branch_id:command.branch_id,reason:'Command id reused with different payload'});continue}
            pruneCommands();
            const sequence=id?commandSequence(id):undefined;
            if(!old&&id&&(sequence!==undefined&&(sequence<=commandPrefix||commandAhead.has(sequence))||commands.size>=1024)){
                await report({message:'branch_failed',branch_id:command.branch_id,reason:'Command identity expired or window full; command was not executed'});continue;
            }
            if(!old&&['abort','discard','cleanup'].includes(command.command))interaction.cancel(command.branch_id);
            // Candidate setup can await a tool result that only a later command delivers.
            // Save must interrupt that wait immediately.
            const dispatchDelivery = () => dispatch(command, wire.payload.trace_context);
            const result=old?.result??(['save','suspend_recovery','ui_prompt_answer','prepare_acceptance'].includes(command.command)
                ?dispatchDelivery():command.command==='start_candidate'
                ?commandQueue.then(dispatchDelivery)
                :(commandQueue=commandQueue.then(dispatchDelivery)));
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
        await telemetry.shutdown(true);
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
    // A cancelled local-policy evaluator can leave child stdio open after the socket closes.
    if(transportError)console.error('Agent control connection failed:',transportError);
    await telemetry.shutdown(false);
    process.exit(transportError||ascProxy.error?1:0);
}

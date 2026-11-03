// Pi owns only its session. All branch decisions and process ownership are in Rust.
import { createInterface } from 'node:readline';
import { pathToFileURL } from 'node:url';
const root = process.env.ASCOUT_PI_SOURCE;
if (!root) throw new Error('ASCOUT_PI_SOURCE is required');
const load = (path: string) => import(pathToFileURL(`${root}/${path}`).href);
const { createAgentSession } = await load('packages/coding-agent/src/core/sdk.ts');
const { SessionManager } = await load('packages/coding-agent/src/core/session-manager.ts');
const { SettingsManager } = await load('packages/coding-agent/src/core/settings-manager.ts');
const { createExtensionRuntime } = await load('packages/coding-agent/src/core/extensions/index.ts');
const { getModel } = await load('packages/ai/src/compat.ts');
const Type = await load('node_modules/typebox/build/typebox.mjs');
const send = (data: unknown) => process.stdout.write(JSON.stringify(data) + '\n');
let session: any;
let pending: ((value: any) => void) | undefined;
let stop = false;
const branch_id = process.argv[2];
const resourceLoader = {
 getExtensions: () => ({extensions: [], errors: [], runtime: createExtensionRuntime()}),
 getSkills: () => ({skills: [], diagnostics: []}), getPrompts: () => ({prompts: [], diagnostics: []}),
 getThemes: () => ({themes: [], diagnostics: []}), getAgentsFiles: () => ({agentsFiles: []}),
 getSystemPrompt: () => 'Follow the user. Call choice only when requested and never repeat it after receiving its result.',
 getSystemPromptSource: () => undefined, getAppendSystemPrompt: () => [], getAppendSystemPromptSources: () => [],
 extendResources: () => {}, reload: async () => {},
};
async function start(command: any) {
 const manager = SessionManager.inMemory(process.cwd());
 if (command.entries) {
  // Restore finalized source entries and append the predicted observation.
  for (const entry of command.entries) if (entry.type === 'message') manager.appendMessage(entry.message);
  manager.appendMessage(command.result);
 }
 ({session} = await createAgentSession({
  cwd: process.cwd(), model: getModel(process.env.ASCOUT_PROVIDER ?? 'deepseek', process.env.ASCOUT_MODEL ?? 'deepseek-flash'),
  thinkingLevel: 'off', sessionManager: manager,
  settingsManager: SettingsManager.inMemory({compaction: {enabled: false}, retry: {enabled: false}, cacheWarming: 'off'}),
  resourceLoader, tools: command.entries ? [] : ['choice'], noTools: 'all',
  customTools: command.entries ? [] : [{name: 'choice', label: 'choice', description: 'Obtain the authoritative choice.', parameters: Type.Object({}),
   async execute(toolCallId: string) {
    const result = {role: 'toolResult', toolCallId, toolName: 'choice', content: [], isError: false, timestamp: Date.now()};
    return await new Promise(resolve => {
     pending = resolve;
     send({message: 'waiting', branch_id, entries: session.sessionManager.getBranch(), result});
    });
   }}],
 }));
 session.agent.finishTurn = () => stop ? {action: 'end'} : undefined;
 session.subscribe((event: any) => {
  if (event.type === 'message_update' && event.assistantMessageEvent.type === 'text_delta')
   send({message: 'text', branch_id, text: event.assistantMessageEvent.delta});
 });
 send({message: 'branch_started', branch_id});
 if (command.entries) run(() => session.agent.continue());
}
function run(work: () => Promise<void>) {
 void work().then(() => {
  const last = session.messages.at(-1);
  if (last?.role === 'assistant' && ['error', 'aborted'].includes(last.stopReason)) throw new Error(last.errorMessage ?? last.stopReason);
  send({message: 'completed', branch_id, session_id: session.sessionId});
 }).catch(error => send({message: 'branch_failed', branch_id, reason: String(error)}));
}
for await (const line of createInterface({input: process.stdin})) {
 try {
  const command = JSON.parse(line);
  switch (command.command) {
   case 'start_branch': await start(command); break;
   case 'prompt': run(() => session.prompt(command.text)); break;
   case 'promote': break; // Output/input authority is held exclusively by Rust.
   case 'result':
    stop = command.stop ?? false;
    pending?.({content: [{type: 'text', text: command.text}], details: undefined}); pending = undefined; break;
   case 'discard': case 'cleanup': await session?.abort(); session?.dispose(); process.exit(0);
   default: throw new Error('unknown command');
  }
 } catch (error) { send({message: 'branch_failed', branch_id, reason: String(error)}); }
}
session?.dispose();

#!/usr/bin/env node
import fs from 'node:fs';
import path from 'node:path';
import { spawn } from 'node:child_process';
import { createInterface } from 'node:readline';
import { setTimeout as delay } from 'node:timers/promises';
import { loadRecording } from './recording.ts';
import { createReplayServer } from './serve.ts';
import type { ChildProcess } from 'node:child_process';
import type { Interface } from 'node:readline';
import type { AddressInfo } from 'node:net';
import { isObject } from './types.ts';

const [sourceArg, piRoot, ...options] = process.argv.slice(2);
if (!piRoot) throw Error('usage: node tools/replay/run.ts SOURCE_RUN PI_ROOT [--policy NAME] --runtime CONFIG [--web 127.0.0.1:8080]');
const source = fs.realpathSync(sourceArg);
const insideSource = (file: string) => file === source || file.startsWith(source + path.sep);
if (options.some(x => ['--model','--models','--resume'].includes(x))) throw Error('Replay selects its local model and requires a new run');
if (options.includes('--policy-artifact')) throw Error('Policy artifacts are only used to resume their own session');
if (!options.includes('--policy')) throw Error('Supply --policy NAME for the new replay session');
const runtimeIndex = options.indexOf('--runtime');
if (runtimeIndex < 0 || !options[runtimeIndex + 1]) throw Error('Supply --runtime explicitly for the new environment');
const runtime: unknown = JSON.parse(fs.readFileSync(options[runtimeIndex + 1], 'utf8'));
if (!isObject(runtime) || typeof runtime.control_socket !== 'string' || !runtime.control_socket) throw Error('Runtime requires control_socket');
if (insideSource(path.join(fs.realpathSync(path.dirname(runtime.control_socket)), path.basename(runtime.control_socket)))) throw Error('Runtime socket must be outside the read-only source');
const { recording, inputs } = loadRecording(source);
if (!inputs.length) throw Error('No consumed native text user inputs');
let paused = false, failure: string | null = null, finished = false, closing = false, child: ChildProcess | undefined, childExited = false;
let nextInput = 0, waitingSequence: number | null = null;
function fail(reason: string) {
  if (!failure) console.error(`Replay stopped automatic input: ${reason}`);
  failure ??= reason;
}
const { server, report } = createReplayServer(recording, { onFailure: fail, reportPath: process.env.REPLAY_REPORT, requestLog: process.env.REPLAY_REQUEST_LOG });
await new Promise<void>((resolve, reject) => { server.once('error', reject); server.listen(0, '127.0.0.1', resolve); });
const temporary = fs.mkdtempSync('/tmp/opencode/ascout-models-');
const models = path.join(temporary, 'models.json');
fs.writeFileSync(models, JSON.stringify({providers:{'ascout-replay-local':{
  api:'openai-completions', apiKey:'local', baseUrl:`http://127.0.0.1:${(server.address() as AddressInfo).port}/v1`,
  models:[{id:'recorded',name:'Recorded replies',reasoning:false,input:['text'],contextWindow:1000000,maxTokens:100000}],
}}}));
if (!options.includes('--web')) options.push('--web','127.0.0.1:0');
let endpoint = '';
const { promise: endpointReady, resolve: resolveEndpoint } = Promise.withResolvers<void>();
const binary = process.env.ASCOUT_BIN || path.resolve('target/debug/ascout-control-plane');
async function request(body: { action: string; prompt: string }) {
  const response = await fetch(`${endpoint}/api/request`, {method:'POST', headers:{origin:endpoint,'content-type':'application/json'},body:JSON.stringify(body),signal:AbortSignal.timeout(5000)});
  if (!response.ok) throw Error(`AScout request ${response.status}: ${await response.text()}`);
}
async function readInputStatus() {
  const response = await fetch(`${endpoint}/api/input-status`, {signal:AbortSignal.timeout(5000)});
  if (!response.ok) throw Error(`Input status HTTP ${response.status}`);
  const status: unknown = await response.json();
  if (!isObject(status) || typeof status.sequence !== 'number' || typeof status.ready !== 'boolean' ||
      (status.error != null && typeof status.error !== 'string')) throw Error('Invalid AScout input status');
  return { sequence: status.sequence, ready: status.ready, error: status.error };
}
async function submitPrompt(sequence: number) {
  // No await between pause check and submitting the next request.
  waitingSequence=sequence;
  await request({action:'prompt',prompt:inputs[nextInput]});
  nextInput++;
}
async function advanceAutomaticInput() {
  const status = await readInputStatus();
  if (paused || closing || failure) return true;
  if (typeof status.error === 'string' && status.error) fail(status.error);
  else if (waitingSequence !== null) {
    if (status.sequence > waitingSequence && status.ready) waitingSequence=null;
  } else if (status.ready) {
    if (nextInput === inputs.length) finished=true;
    else await submitPrompt(status.sequence);
  }
  return false;
}
async function shutdown() {
  if (closing) return;
  closing = true;
  if (child && !childExited) {
    child.kill('SIGTERM');
  } else cleanup();
}
let lines: Interface | undefined;
function cleanup() {
  closing = true;
  lines?.close();
  process.stdin.pause();
  server.close(); server.closeAllConnections();
  fs.rmSync(temporary, {recursive:true,force:true});
}
for (const signal of ['SIGINT','SIGTERM']) process.on(signal, shutdown);
child = spawn(binary, [piRoot,...options,'--models',models,'--model','ascout-replay-local/recorded'], {stdio:['ignore','inherit','pipe']});
child.on('error', error => { fail(`AScout launch: ${error.message}`); childExited = true; resolveEndpoint(); cleanup(); process.exitCode=1; });
child.on('exit', (code, signal) => { childExited=true; resolveEndpoint(); cleanup(); if(code || signal) {console.error(`AScout exited: ${code ?? signal}`);process.exitCode=1;} });
let stderr = '';
child.stderr!.on('data', chunk => {
  process.stderr.write(chunk);
  stderr += chunk;
  const match = stderr.match(/AScout browser UI: (http:\/\/[^\s]+)/);
  if (match && !endpoint) {endpoint=match[1]; resolveEndpoint();}
  stderr = stderr.slice(-4096);
});
lines = createInterface({input:process.stdin});
lines.on('line', line => {
  switch (line.trim()) {
    case 'pause': paused=true; console.error('Replay input paused; in-flight turn continues.'); break;
    case 'resume': paused=false; console.error(failure ? `Replay remains stopped: ${failure}` : 'Replay input resumed.'); break;
    case 'status': console.error(JSON.stringify({paused,failure,finished,nextInput,inputs:inputs.length,...report()})); break;
    case 'quit': void shutdown(); break;
    default: console.error('Commands: pause | resume | status | quit');
  }
});
console.error('Replay commands: pause | resume | status | quit. Pause before manual input or model changes.');
await endpointReady;
while (!closing && !childExited) {
  try {
    if (!paused && !failure && !finished) {
      if (await advanceAutomaticInput()) continue;
    }
  } catch (error) { if (!closing) fail(error instanceof Error ? error.message : String(error)); }
  await delay(100);
}

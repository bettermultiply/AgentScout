#!/usr/bin/env node
/** External replay driver. AScout owns execution, persistence and shutdown. */
import fs from 'node:fs';
import path from 'node:path';
import { spawn } from 'node:child_process';
import { createInterface } from 'node:readline';
import { setTimeout as delay } from 'node:timers/promises';
import { loadRecording } from './recording.mjs';
import { createReplayServer } from './serve.mjs';

const [sourceArg, runArg, piRoot, ...options] = process.argv.slice(2);
if (!piRoot) throw Error('usage: node tools/replay/run.mjs SOURCE_RUN NEW_RUN PI_ROOT POLICY --runtime CONFIG [--web 127.0.0.1:8080]');
const source = fs.realpathSync(sourceArg), run = fs.realpathSync(runArg);
const insideSource = file => file === source || file.startsWith(source + path.sep);
if (insideSource(run)) throw Error('New run must be outside the read-only source');
if (options.some(x => ['--model','--models','--resume'].includes(x))) throw Error('Replay selects its local model and requires a new run');
if (options.includes('--policy-artifact')) throw Error('Replay uses the source Policy artifact');
if (fs.existsSync(path.join(source,'policy.json'))) options.push('--policy-artifact',path.join(source,'policy.json'));
else if (!options[0] || options[0].startsWith('--')) throw Error('Source Policy artifact missing; supply the original Policy YAML explicitly');
const runtimeIndex = options.indexOf('--runtime');
if (runtimeIndex < 0 || !options[runtimeIndex + 1]) throw Error('Supply --runtime explicitly for the new environment');
const runtime = JSON.parse(fs.readFileSync(options[runtimeIndex + 1], 'utf8'));
if (!runtime.control_socket) throw Error('Runtime requires control_socket');
if (insideSource(path.join(fs.realpathSync(path.dirname(runtime.control_socket)), path.basename(runtime.control_socket)))) throw Error('Runtime socket must be outside the read-only source');
const { recording, inputs } = loadRecording(source);
if (!inputs.length) throw Error('No consumed native text user inputs');
let paused = false, failure = null, finished = false, closing = false, child, childExited = false;
let nextInput = 0, waitingSequence = null;
function fail(reason) {
  if (!failure) console.error(`Replay stopped automatic input: ${reason}`);
  failure ??= reason;
}
const { server, report } = createReplayServer(recording, { onFailure: fail, reportPath: process.env.REPLAY_REPORT, requestLog: process.env.REPLAY_REQUEST_LOG });
await new Promise((resolve, reject) => { server.once('error', reject); server.listen(0, '127.0.0.1', resolve); });
const temporary = fs.mkdtempSync('/tmp/opencode/ascout-models-');
const models = path.join(temporary, 'models.json');
fs.writeFileSync(models, JSON.stringify({providers:{'ascout-replay-local':{
  api:'openai-completions', apiKey:'local', baseUrl:`http://127.0.0.1:${server.address().port}/v1`,
  models:[{id:'recorded',name:'Recorded replies',reasoning:false,input:['text'],contextWindow:1000000,maxTokens:100000}],
}}}));
let webIndex = options.indexOf('--web');
if (webIndex < 0) { options.push('--web','127.0.0.1:0'); webIndex = options.length - 2; }
let endpoint;
let resolveEndpoint;
const endpointReady = new Promise(resolve => { resolveEndpoint = resolve; });
const binary = process.env.ASCOUT_BIN || path.resolve('target/debug/ascout-control-plane');
async function request(body) {
  const response = await fetch(`${endpoint}/api/request`, {method:'POST', headers:{origin:endpoint,'content-type':'application/json'},body:JSON.stringify(body),signal:AbortSignal.timeout(5000)});
  if (!response.ok) throw Error(`AScout request ${response.status}: ${await response.text()}`);
}
async function shutdown() {
  if (closing) return;
  closing = true;
  if (child && !childExited) {
    // SIGTERM takes AScout's normal Shutdown/save path, including during startup.
    child.kill('SIGTERM');
  } else cleanup();
}
let lines;
function cleanup() {
  closing = true;
  lines?.close();
  process.stdin.pause();
  server.close(); server.closeAllConnections();
  fs.rmSync(temporary, {recursive:true,force:true});
}
for (const signal of ['SIGINT','SIGTERM']) process.on(signal, shutdown);
child = spawn(binary, [run,piRoot,...options,'--models',models,'--model','ascout-replay-local/recorded'], {stdio:['ignore','inherit','pipe']});
child.on('error', error => { fail(`AScout launch: ${error.message}`); childExited = true; resolveEndpoint(); cleanup(); process.exitCode=1; });
child.on('exit', (code, signal) => { childExited=true; resolveEndpoint(); cleanup(); if(code || signal) {console.error(`AScout exited: ${code ?? signal}`);process.exitCode=1;} });
let stderr = '';
child.stderr.on('data', chunk => {
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
      const response = await fetch(`${endpoint}/api/input-status`, {signal:AbortSignal.timeout(5000)});
      if (!response.ok) throw Error(`Input status HTTP ${response.status}`);
      const status = await response.json();
      if (paused || closing || failure) continue;
      if (status.error) fail(status.error);
      else if (waitingSequence !== null) {
        if (status.sequence > waitingSequence && status.ready) waitingSequence=null;
      } else if (status.ready) {
        if (nextInput === inputs.length) finished=true;
        else {
          // No await between pause check and submitting the next request.
          waitingSequence=status.sequence;
          await request({action:'prompt',prompt:inputs[nextInput].prompt});
          nextInput++;
        }
      }
    }
  } catch (error) { if (!closing) fail(error.message); }
  await delay(100);
}

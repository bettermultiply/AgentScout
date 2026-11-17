import {spawnControl, sessionPath} from './control-plane-fixture.mjs';
// Real control plane, Pi, bubblewrap and shell; local fixture only.
import fs from 'node:fs';
import http from 'node:http';
import {spawn,execFileSync} from 'node:child_process';
import {once} from 'node:events';
import assert from 'node:assert/strict';
import {test} from 'node:test';
import net from 'node:net';

test('filesystem main entry restores managed upper with current host lower and rejects changed package code',{timeout:120000},async t=>{
 const project=process.cwd(),root=fs.mkdtempSync('/tmp/opencode/recovery-filesystem-');console.log('evidence',root);
 for(const name of ['workspace','pi','policy','tmp','package/extensions'])fs.mkdirSync(root+'/'+name,{recursive:true});
 fs.writeFileSync(sessionPath(root, 'workspace/lower'),'saved lower');
 fs.writeFileSync(root+'/package/package.json',JSON.stringify({name:'fixture',pi:{extensions:['extensions/*.ts']}}));
 const extension=root+'/package/extensions/fixture.ts';fs.writeFileSync(extension,'export default ()=>{}');
 fs.writeFileSync(root+'/pi/settings.json',JSON.stringify({packages:[root+'/package']}));
 let requests=0;
 const fake=http.createServer((req,res)=>{let body='';req.on('data',b=>body+=b);req.on('end',()=>{
  fs.appendFileSync(root+'/requests.jsonl',body+'\n');requests++;
  res.writeHead(200,{'content-type':'text/event-stream'});
  if(requests===1||requests===3){
   const command=requests===1?'cat lower > value; printf first >> value':'cat lower value > continued; printf second >> value';
   res.write(`data: ${JSON.stringify({id:'fixture',object:'chat.completion.chunk',created:0,model:'fixture',choices:[{index:0,delta:{tool_calls:[{index:0,id:'shell'+requests,type:'function',function:{name:'bash',arguments:JSON.stringify({command})}}]}}]})}\n\n`);
   res.end(`data: ${JSON.stringify({id:'fixture',object:'chat.completion.chunk',created:0,model:'fixture',choices:[{index:0,delta:{},finish_reason:'tool_calls'}]})}\n\ndata: [DONE]\n\n`);
  }else res.write(`data: ${JSON.stringify({id:'fixture',object:'chat.completion.chunk',created:0,model:'fixture',choices:[{index:0,delta:{content:'waiting until aborted'}}]})}\n\n`);
 })});await new Promise(r=>fake.listen(0,'127.0.0.1',r));
  const port=await new Promise(r=>{const s=http.createServer();s.listen(0,'127.0.0.1',()=>{const p=s.address().port;s.close(()=>r(p))})});
  const runtimePort=await new Promise(r=>{const s=http.createServer();s.listen(0,'127.0.0.1',()=>{const p=s.address().port;s.close(()=>r(p))})});
 fs.writeFileSync(root+'/pi/models.json',JSON.stringify({providers:{recovery:{baseUrl:`http://127.0.0.1:${fake.address().port}/v1`,api:'openai-completions',apiKey:'fake',models:[{id:'fixture',reasoning:false,input:['text'],contextWindow:32768,maxTokens:256}]}}}));
 fs.copyFileSync(project+'/assets/policy/policy/policy.yaml',root+'/policy/policy.yaml');fs.copyFileSync(project+'/assets/policy/policy/decisions.py',root+'/policy/decisions.py');
 fs.writeFileSync(root+'/runtime.json',JSON.stringify({backend:'nle',control_address:`127.0.0.1:${runtimePort}`,adapter_instance:'fs',python:project+'/../.venv-nle/bin/python',bridge:project+'/examples/nle-runtime-bridge.py',guidebook:project+'/examples/nle/Guidebook.txt',seed:7,filesystem:{state_directory:root+'/fs-state',isolated_paths:[sessionPath(root, 'workspace')],passthrough_rw:[root]}}));
 const config=JSON.parse(fs.readFileSync(root+'/runtime.json'));
 config.backends=[{id:'first',type:'nle',config:{...config}},{id:'second',type:'nle',config:{...config,seed:8}}];
 config.policy={default:['first','second'],routes:[{operation:'invoke_tool',backends:['first'],output_backend:'first'}]};
 fs.writeFileSync(root+'/runtime.json',JSON.stringify(config));
 const children=[];t.after(()=>{for(const p of children)if(p.exitCode===null)p.kill('SIGKILL');fake.closeAllConnections();fake.close()});
 fs.writeFileSync(root+'/stderr.log','');
  const wait=async(fn,label)=>{const deadline=Date.now()+30000;while(Date.now()<deadline){if(await fn())return;await new Promise(r=>setTimeout(r,20))}try{fs.writeFileSync(root+'/timeout-state.json',JSON.stringify(await(await fetch(`http://127.0.0.1:${port}/api/observation/state-event-set`)).json(),null,2))}catch{}throw Error('Timeout '+label+' '+root)};
 const post=async body=>fetch(`http://127.0.0.1:${port}/api/request`,{method:'POST',headers:{origin:`http://127.0.0.1:${port}`,'content-type':'application/json'},body:JSON.stringify(body)});
  const loadedCount=()=>fs.existsSync(sessionPath(root, 'events.jsonl'))?fs.readFileSync(sessionPath(root, 'events.jsonl'),'utf8').split('\n').filter(line=>line&&JSON.parse(line).message==='recovery_loaded').length:0;
  const launch=resume=>{const previous=loadedCount();const p=spawnControl(project+'/target/debug/ascout-control-plane',[root,project+'/../pi',root+'/policy/policy.yaml','--web',`127.0.0.1:${port}`,'--model','recovery/fixture',...(resume?['--resume']:[])],{env:{...process.env,TMPDIR:root+'/tmp',ASCOUT_RUNTIME_CONFIG:root+'/runtime.json',PI_CODING_AGENT_DIR:root+'/pi'},stdio:['ignore','ignore','pipe']});p.recoveryCount=resume?previous:null;children.push(p);p.stderr.on('data',b=>fs.appendFileSync(root+'/stderr.log',b));return p};
  const ready=async p=>wait(async()=>{assert.equal(p.exitCode,null,fs.readFileSync(root+'/stderr.log','utf8'));try{return(await fetch(`http://127.0.0.1:${port}/api/observation/state-event-set`)).ok&&(p.recoveryCount===null||loadedCount()>p.recoveryCount)}catch{return false}},'ready');
  const stop=async p=>{
   const previous=fs.existsSync(sessionPath(root, 'recovery.json'))?fs.readFileSync(sessionPath(root, 'recovery.json'),'utf8'):null;
   const response=await post({action:'pause'});assert.equal(response.status,202);
   await wait(async()=>fs.existsSync(sessionPath(root, 'recovery.json'))&&fs.readFileSync(sessionPath(root, 'recovery.json'),'utf8')!==previous&&JSON.stringify(await(await fetch(`http://127.0.0.1:${port}/api/observation/state-event-set`)).json()).includes('Paused;'),'Pause published');
   assert.equal(p.exitCode,null,'Pause saves without exiting');
   const done=once(p,'exit');p.kill('SIGKILL');await done;assert.equal(p.signalCode,'SIGKILL');
   await wait(()=>!execFileSync('ps',['-eo','args'],{encoding:'utf8'}).split('\n').some(line=>line.includes(root)),'paused task processes exited');
  };
 let p=launch(false);await ready(p);await new Promise(r=>setTimeout(r,200));await stop(p);
 const baseline=fs.readFileSync(sessionPath(root, 'recovery.json'),'utf8');
  p=launch(true);await ready(p);await new Promise(r=>setTimeout(r,200));await post({action:'prompt',prompt:'write fixture'});assert.equal(requests,0,'ordinary input stays queued while paused');await post({action:'continue'});await wait(()=>requests===2,'shell completed, stream active');
 // The inner NLE save succeeds before the data-plane hits an unsavable socket.
 const socket=net.createServer();t.after(()=>socket.close());
 const currentBase=fs.readdirSync(root+'/fs-state').filter(n=>n.startsWith('restored-')).sort().at(-1);
  const findUpper=dir=>{for(const e of fs.readdirSync(dir,{withFileTypes:true})){const path=dir+'/'+e.name;if(e.isDirectory()){if(e.name==='upper')return path;const found=findUpper(path);if(found)return found}}};
  const base=findUpper(root+'/fs-state/'+currentBase);assert.ok(base);
 await new Promise(r=>socket.listen(root+'/s',r));fs.renameSync(root+'/s',base+'/s');
  await post({action:'pause'});
 await wait(()=>fs.readFileSync(sessionPath(root, 'events.jsonl'),'utf8').includes('Save failed'),'middle component save failure');
 assert.equal(fs.readFileSync(sessionPath(root, 'recovery.json'),'utf8'),baseline);assert.equal(p.exitCode,null);assert.equal(requests,2);
 await new Promise(r=>socket.close(r));
 fs.unlinkSync(base+'/s');
 await stop(p);
 const manifest=fs.readFileSync(sessionPath(root, 'recovery.json'),'utf8');const saved=JSON.parse(manifest);const generation=saved.files['runtime.json'].path;
 const digest=path=>execFileSync('sha256sum',[path],{encoding:'utf8'}).split(' ')[0];const original=digest(generation);
 const dataPlane=generation.replace(/\.json$/,'.components/1.filesystem');
 fs.writeFileSync(dataPlane+'/corrupt','injected load failure after NLE components');
 p=launch(true);await once(p,'exit');assert.equal(p.exitCode,1);assert.match(fs.readFileSync(root+'/stderr.log','utf8'),/Filesystem recovery content changed/);assert.equal(requests,2);
 assert.ok(!execFileSync('ps',['-eo','args'],{encoding:'utf8'}).split('\n').some(line=>line.includes(root)),'partial load resources exited');
 fs.unlinkSync(dataPlane+'/corrupt');
 assert.equal(fs.existsSync(sessionPath(root, 'workspace/value')),false,'shell changes are isolated');
 fs.writeFileSync(sessionPath(root, 'workspace/lower'),'mutated original lower');
 fs.writeFileSync(extension,'export default ()=>{return 1}');p=launch(true);await once(p,'exit');assert.equal(p.exitCode,1);assert.match(fs.readFileSync(root+'/stderr.log','utf8'),/resources changed/);assert.equal(requests,2);fs.writeFileSync(extension,'export default ()=>{}');
  p=launch(true);await ready(p);await new Promise(r=>setTimeout(r,200));assert.equal(requests,2,'no automatic inference');await post({action:'prompt',prompt:'continue fixture'});await post({action:'continue'});await wait(()=>requests===4,'continued shell');await stop(p);
 const find=(dir,name)=>{for(const e of fs.readdirSync(dir,{withFileTypes:true})){if(e.name==='work')continue;const path=dir+'/'+e.name;if(e.isDirectory()){const found=find(path,name);if(found)return found}else if(e.name===name)return path}};
  // Contract explicitly changed by the user on 2026-11-14: ordinary lower
  // uses current host data; managed upper and saved sources stay independent.
  const continued=find(root+'/fs-state','continued');assert.ok(continued);assert.equal(fs.readFileSync(continued,'utf8'),'mutated original lowersaved lowerfirst');assert.equal(digest(generation),original);
  fs.writeFileSync(sessionPath(root, 'recovery.json'),manifest);p=launch(true);await ready(p);await wait(()=>fs.readFileSync(sessionPath(root, 'events.jsonl'),'utf8').includes('recovery_loaded'),'old generation loaded');await new Promise(r=>setTimeout(r,200));assert.equal(requests,4);await stop(p);
  // Shutdown has its own terminate/materialize/cleanup path. It must finish
  // successfully without replacing the generation just validated above.
  const preserved=fs.readFileSync(sessionPath(root, 'recovery.json'),'utf8');
  p=launch(true);await ready(p);await new Promise(r=>setTimeout(r,200));
  const done=once(p,'exit');await post({action:'shutdown'});await done;
  assert.equal(p.exitCode,0,fs.readFileSync(root+'/stderr.log','utf8'));
  assert.equal(fs.readFileSync(sessionPath(root, 'recovery.json'),'utf8'),preserved,'Shutdown does not save');
  assert.ok(!execFileSync('ps',['-eo','args'],{encoding:'utf8'}).split('\n').some(line=>line.includes(root)),'Shutdown reaps task processes');
 console.log('PASS FS main entry',root);
});

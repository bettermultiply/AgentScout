// Real Pi + real NLE + local OpenAI fixture. No external model requests.
import fs from 'node:fs';
import http from 'node:http';
import {spawn,execFileSync} from 'node:child_process';
import {once} from 'node:events';
import assert from 'node:assert/strict';
import {test} from 'node:test';

for(const failureMode of ['none','retry','continue','multiple']) test(`active tools and candidates survive recovery: ${failureMode}`, {timeout:120000}, async t=>{
 const project=process.cwd(),root=fs.mkdtempSync('/tmp/opencode/recovery-active-');
 console.log('evidence',root);
 fs.mkdirSync(root+'/workspace');fs.mkdirSync(root+'/pi');fs.mkdirSync(root+'/policy');
 let requests=[];
 const fake=http.createServer((req,res)=>{let body='';req.on('data',b=>body+=b);req.on('end',()=>{
  const input=JSON.parse(body);requests.push(input);fs.appendFileSync(root+'/requests.jsonl',body+'\n');
  res.writeHead(200,{'content-type':'text/event-stream'});
  const toolCall=requests.length===1;
  const delta=toolCall?{tool_calls:[0,1].map(i=>({index:i,id:'op'+i,type:'function',function:{name:'nle_action',arguments:JSON.stringify({action:0})}}))}:{content:'candidate advanced after recovery'};
  res.write(`data: ${JSON.stringify({id:'fixture',object:'chat.completion.chunk',created:0,model:'fixture',choices:[{index:0,delta}]})}\n\n`);
  res.end(`data: ${JSON.stringify({id:'fixture',object:'chat.completion.chunk',created:0,model:'fixture',choices:[{index:0,delta:{},finish_reason:toolCall?'tool_calls':'stop'}]})}\n\ndata: [DONE]\n\n`);
 })});
 await new Promise(r=>fake.listen(0,'127.0.0.1',r));
 const port=await new Promise(resolve=>{const s=http.createServer();s.listen(0,'127.0.0.1',()=>{const p=s.address().port;s.close(()=>resolve(p))})});
 fs.writeFileSync(root+'/pi/models.json',JSON.stringify({providers:{recovery:{baseUrl:`http://127.0.0.1:${fake.address().port}/v1`,api:'openai-completions',apiKey:'fake',models:[{id:'fixture',reasoning:false,input:['text'],contextWindow:32768,maxTokens:256}]}}}));
 fs.writeFileSync(root+'/runtime.json',JSON.stringify({backend:'nle',control_socket:root+'/runtime.sock',adapter_instance:'nle',python:project+'/../.venv-nle/bin/python',bridge:project+'/examples/nle-runtime-bridge.py',guidebook:project+'/examples/nle/Guidebook.txt',seed:7,action_delay_seconds:2,env_id:'NetHackScore-v0'}));
 if(failureMode==='multiple') {
  const config=JSON.parse(fs.readFileSync(root+'/runtime.json'));
  fs.writeFileSync(root+'/runtime.json',JSON.stringify({control_socket:config.control_socket,adapter_instance:'multiple',backends:[{id:'first',type:'nle',config},{id:'second',type:'nle',config:{...config,seed:8}}],policy:{default:['first','second'],routes:[{operation:'invoke_tool',backends:['first'],output_backend:'first'}]}}));
 }
 let policy=fs.readFileSync(project+'/assets/policy/policy.yaml','utf8').replace('max_chances: 0','max_chances: 1').replace('candidates: ["example"]','enum_values: ["moved", "blocked"]\n      candidates: ["moved", "blocked"]');
 fs.writeFileSync(root+'/policy/policy.yaml',policy);
 fs.writeFileSync(root+'/policy/decisions.py',fs.readFileSync(project+'/assets/policy/decisions.py','utf8').replace('return [chance["chance_id"] for chance in data["frontier_view"][:policy_config["max_chances"]]]','return [chance["chance_id"] for chance in data["frontier_view"] if chance["chance_id"] == "chance-1"]').replace('def launch(data, *, policy_config, capabilities):\n    return True','def launch(data, *, policy_config, capabilities):\n    return all(c["status"] == "finished" for c in data["candidate_view"])').replace('if policy_config["value_type"] == "boolean"','if policy_config.get("value_type") == "boolean"').replace('elif policy_config["value_type"] == "enum"','elif policy_config.get("value_type") == "enum"').replace('{"intent": "Promote"}',failureMode==='continue'?'{"intent": "Promote", "notification": "recovery authoritative notification"}':'{"intent": "Promote"}'));
 const children=[];
 t.after(()=>{for(const p of children)if(p.exitCode===null)p.kill('SIGKILL');fake.closeAllConnections();fake.close()});
 const wait=async(fn,label)=>{const end=Date.now()+30000;while(Date.now()<end){if(await fn())return;await new Promise(r=>setTimeout(r,20))}throw Error('Timeout '+label+' '+root)};
 const events=()=>fs.existsSync(root+'/events.jsonl')?fs.readFileSync(root+'/events.jsonl','utf8').trim().split('\n').filter(Boolean).map(JSON.parse):[];
 const post=async body=>(await fetch(`http://127.0.0.1:${port}/api/request`,{method:'POST',headers:{origin:`http://127.0.0.1:${port}`,'content-type':'application/json'},body:JSON.stringify(body)})).status;
 const start=async resume=>{
  const p=spawn(project+'/target/debug/ascout-control-plane',[root,project+'/../pi',root+'/policy/policy.yaml','--web',`127.0.0.1:${port}`,'--model','recovery/fixture',...(resume?['--resume']:[])],{env:{...process.env,ASCOUT_RUNTIME_CONFIG:root+'/runtime.json',PI_CODING_AGENT_DIR:root+'/pi'},stdio:['ignore','pipe','pipe']});children.push(p);
  p.stderr.on('data',b=>fs.appendFileSync(root+'/stderr.log',b));
  await wait(async()=>{assert.equal(p.exitCode,null,fs.existsSync(root+'/stderr.log')?fs.readFileSync(root+'/stderr.log','utf8'):'');try{return(await fetch(`http://127.0.0.1:${port}/api/observation/state-event-set`)).ok}catch{return false}},'start');return p;
 };
 let p=await start(false);
 let baseline;
 if(['retry','continue'].includes(failureMode)) {
  const initialExit=once(p,'exit');await post({action:'shutdown'});await initialExit;assert.equal(p.exitCode,0);
  baseline=fs.readFileSync(root+'/recovery.json','utf8');p=await start(true);
  await wait(()=>events().some(e=>e.message==='recovery_loaded'),'baseline loaded');
 }
 await post({action:'prompt',prompt:'Start parallel fixture'});
 await wait(()=>events().filter(e=>e.message==='candidate_created').length===2,'two candidates');
 await wait(()=>events().some(e=>e.message==='action_started'&&e.action_id==='op1'),'active sibling tool');
 assert.equal(requests.length,1,'candidates should still be waiting for sibling result');
 if(['retry','continue'].includes(failureMode))fs.mkdirSync(root+'/recovery.tmp');
 const exited=once(p,'exit');await post({action:'shutdown'});
 if(['retry','continue'].includes(failureMode)) {
  await wait(async()=>JSON.stringify(await(await fetch(`http://127.0.0.1:${port}/api/observation/state-event-set`)).json()).includes('Save failed'),'injected publish failure');
  assert.equal(p.exitCode,null,'failed save remains paused and alive');
  assert.equal(fs.readFileSync(root+'/recovery.json','utf8'),baseline,'previous manifest remains intact');
  assert.equal(requests.length,1,'failure does not restart inference');
  fs.rmdirSync(root+'/recovery.tmp');
  if(failureMode==='continue') {
   await post({action:'prompt',prompt:'Continue after failed save'});
   await wait(()=>requests.length>=4,'all branches continue after failed save');
  }
  await post({action:'shutdown'});
 }
 await wait(()=>p.exitCode!==null||events().some(e=>e.message==='recovery_saved'&&e.error),'shutdown');
 const failure=events().find(e=>e.message==='recovery_saved'&&e.error);assert.equal(failure,undefined,JSON.stringify(failure));
 await exited;assert.equal(p.exitCode,0,fs.readFileSync(root+'/stderr.log','utf8'));
 assert.ok(!execFileSync('ps',['-eo','args'],{encoding:'utf8'}).split('\n').some(line=>line.includes(root)),'all task processes exited');
 const manifest=JSON.parse(fs.readFileSync(root+'/recovery.json')),saved=JSON.parse(fs.readFileSync(manifest.files['agent.json'].path));
 assert.equal(saved.candidateSetups.length,failureMode==='continue'?0:2,'persist sibling waiting relations');
 const identities=saved.tree.branches.map(([id])=>id);assert.equal(identities.length,failureMode==='continue'?1:3);
 const immutable=saved.tree.sources.map(source=>[source.file,fs.readFileSync(source.file,'utf8')]);
 if(failureMode==='continue')assert.equal(saved.tree.sources.flatMap(s=>s.entries).filter(e=>e.customType==='notification'&&e.content==='recovery authoritative notification').length>=1,true);
 const runtimeRecord=path=>{const value=JSON.parse(fs.readFileSync(path));if(value.kind==='runtime-components')return JSON.parse(fs.readFileSync(path.replace(/\.json$/,'.components/0.json')));return value};
 const beforeRuntime=runtimeRecord(manifest.files['runtime.json'].path);
 assert.equal(beforeRuntime.worlds[0].actions.length,failureMode==='continue'?2:1,'collect real actions before publishing');
 if(failureMode==='retry') {
  const modelFile=root+'/pi/models.json',original=fs.readFileSync(modelFile,'utf8');
  fs.writeFileSync(modelFile,original+'\n');
  const incompatible=spawn(project+'/target/debug/ascout-control-plane',[root,project+'/../pi',root+'/policy/policy.yaml','--resume','--web',`127.0.0.1:${port}`,'--model','recovery/fixture'],{env:{...process.env,ASCOUT_RUNTIME_CONFIG:root+'/runtime.json',PI_CODING_AGENT_DIR:root+'/pi'},stdio:['ignore','ignore','pipe']});children.push(incompatible);
  let rejection='';incompatible.stderr.on('data',b=>rejection+=b);await once(incompatible,'exit');
  assert.equal(incompatible.exitCode,1);assert.match(rejection,/configuration resources changed/);fs.writeFileSync(modelFile,original);
 }
 const count=requests.length;p=await start(true);await wait(()=>events().some(e=>e.message==='recovery_loaded'),'loaded');
 await new Promise(r=>setTimeout(r,200));assert.equal(requests.length,count,'restore waits for user input');
 await post({action:'prompt',prompt:'Continue after restart'});
 await wait(()=>requests.length>=count+(failureMode==='continue'?1:3),'branches advance');
 const texts=fs.readFileSync(root+'/session.jsonl','utf8');assert.match(texts,/candidate advanced after recovery/);
 // Candidate model requests must contain paired real sibling results, not abort errors.
 const candidateRequests=requests.slice(1,failureMode==='continue'?count:undefined).filter(r=>r.messages.some(m=>m.role==='tool'&&['moved','blocked'].includes(m.content)));
 assert.equal(candidateRequests.length,2);
 for(const request of candidateRequests){const results=request.messages.filter(m=>m.role==='tool');assert.equal(results.length,2);assert.ok(!JSON.stringify(results).includes('cancelled'));}
 const again=once(p,'exit');await post({action:'shutdown'});await wait(()=>p.exitCode!==null||events().some(e=>e.message==='recovery_saved'&&e.error),'second shutdown');await again;assert.equal(p.exitCode,0);
 const afterManifest=JSON.parse(fs.readFileSync(root+'/recovery.json'));
 if(failureMode==='continue') {
  const entries=fs.readFileSync(root+'/session.jsonl','utf8').trim().split('\n').map(JSON.parse);
  assert.equal(entries.filter(e=>e.customType==='notification'&&e.content==='recovery authoritative notification').length,1,'Promote notice persists exactly once across recovery');
 }
 const afterRuntime=runtimeRecord(afterManifest.files['runtime.json'].path);
 assert.equal(afterRuntime.worlds[0].actions.length,2,'each of the original actions executes exactly once');
 for(const [file,content] of immutable)assert.equal(fs.readFileSync(file,'utf8'),content,'saved sources remain immutable');
 assert.ok(!execFileSync('ps',['-eo','args'],{encoding:'utf8'}).split('\n').some(line=>line.includes(root)),'resumed task processes exited');
 if(baseline) {
  fs.writeFileSync(root+'/recovery.json',baseline);
  const oldCount=requests.length;p=await start(true);await new Promise(r=>setTimeout(r,100));assert.equal(requests.length,oldCount);
  const oldExit=once(p,'exit');await post({action:'shutdown'});await oldExit;assert.equal(p.exitCode,0,'previous generation independently restores');
 }
 console.log('PASS active/sibling recovery',root,identities);
});

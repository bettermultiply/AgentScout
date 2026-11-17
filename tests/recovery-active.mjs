// Real Pi + real NLE + local OpenAI fixture. No external model requests.
import fs from 'node:fs';
import http from 'node:http';
import {spawn,execFileSync} from 'node:child_process';
import {once} from 'node:events';
import assert from 'node:assert/strict';
import {test} from 'node:test';

for(const failureMode of ['none','retry','continue','multiple','filesystem']) test(`active tools and candidates survive recovery: ${failureMode}`, {timeout:120000}, async t=>{
 const project=process.cwd(),root=fs.mkdtempSync('/tmp/opencode/recovery-active-');
 console.log('evidence',root);
 fs.mkdirSync(root+'/workspace');fs.mkdirSync(root+'/pi');fs.mkdirSync(root+'/policy');
  let requests=[];
  let promotedWrite=false;
 const fake=http.createServer((req,res)=>{let body='';req.on('data',b=>body+=b);req.on('end',()=>{
  const input=JSON.parse(body);requests.push(input);fs.appendFileSync(root+'/requests.jsonl',body+'\n');
  res.writeHead(200,{'content-type':'text/event-stream'});
   const writeTool=failureMode==='filesystem'&&!promotedWrite&&input.messages.some(m=>m.role==='user'&&JSON.stringify(m.content).includes('verify promoted filesystem'));
   if(writeTool)promotedWrite=true;
   const toolCall=requests.length===1||writeTool;
   const delta=writeTool?{tool_calls:[{index:0,id:'promoted-write',type:'function',function:{name:'bash',arguments:JSON.stringify({command:'printf promoted > promoted-file'})}}]}:toolCall?{tool_calls:[0,1].map(i=>({index:i,id:'op'+i,type:'function',function:{name:'nle_action',arguments:JSON.stringify({action:0})}}))}:{content:'candidate advanced after recovery'};
  res.write(`data: ${JSON.stringify({id:'fixture',object:'chat.completion.chunk',created:0,model:'fixture',choices:[{index:0,delta}]})}\n\n`);
  res.end(`data: ${JSON.stringify({id:'fixture',object:'chat.completion.chunk',created:0,model:'fixture',choices:[{index:0,delta:{},finish_reason:toolCall?'tool_calls':'stop'}]})}\n\ndata: [DONE]\n\n`);
 })});
 await new Promise(r=>fake.listen(0,'127.0.0.1',r));
  const port=await new Promise(resolve=>{const s=http.createServer();s.listen(0,'127.0.0.1',()=>{const p=s.address().port;s.close(()=>resolve(p))})});
  const runtimePort=await new Promise(resolve=>{const s=http.createServer();s.listen(0,'127.0.0.1',()=>{const p=s.address().port;s.close(()=>resolve(p))})});
 fs.writeFileSync(root+'/pi/models.json',JSON.stringify({providers:{recovery:{baseUrl:`http://127.0.0.1:${fake.address().port}/v1`,api:'openai-completions',apiKey:'fake',models:[{id:'fixture',reasoning:false,input:['text'],contextWindow:32768,maxTokens:256}]}}}));
  fs.writeFileSync(root+'/runtime.json',JSON.stringify({backend:'nle',control_address:`127.0.0.1:${runtimePort}`,adapter_instance:'nle',python:project+'/../.venv-nle/bin/python',bridge:project+'/examples/nle-runtime-bridge.py',guidebook:project+'/examples/nle/Guidebook.txt',seed:7,action_delay_seconds:2,env_id:'NetHackScore-v0'}));
  if(failureMode==='filesystem') {
   fs.mkdirSync(root+'/tmp');
   const config=JSON.parse(fs.readFileSync(root+'/runtime.json'));
   config.filesystem={state_directory:root+'/fs-state',isolated_paths:[root+'/workspace'],passthrough_rw:[root]};
   fs.writeFileSync(root+'/runtime.json',JSON.stringify(config));
  }
 if(failureMode==='multiple') {
  const config=JSON.parse(fs.readFileSync(root+'/runtime.json'));
  fs.writeFileSync(root+'/runtime.json',JSON.stringify({control_address:config.control_address,adapter_instance:'multiple',backends:[{id:'first',type:'nle',config},{id:'second',type:'nle',config:{...config,seed:8}}],policy:{default:['first','second'],routes:[{operation:'invoke_tool',backends:['first'],output_backend:'first'}]}}));
 }
  let policy=fs.readFileSync(project+'/assets/policy/policy/policy.yaml','utf8').replace('max_chances: 0','max_chances: 1').replace('      scenarios: []\n    feasibility_pass:', '      scenarios: [{}]\n    feasibility_pass:').replace('  predict:\n    file: decisions.py\n    function: predict\n    policy_config:\n      scenarios: []','  predict:\n    file: decisions.py\n    function: predict\n    policy_config:\n      candidates: ["moved", "blocked"]');
  // This regression predicts tool outcomes, not model output. Select the
  // first tool Chance even when model admissions precede it.
  policy=policy.replace('scenarios: [{}]', 'scenarios: [{when: {kind: nle_action}}]');
  fs.writeFileSync(root+'/policy/policy.yaml',policy);
 const nleVerifier=fs.readFileSync(project+'/assets/policy/nle/decisions.py','utf8').split('def _actual_outcome(')[1].split('def dispose(')[0];
 const algorithms=fs.readFileSync(project+'/assets/policy/policy/decisions.py','utf8').replace(/def match_proposal\([\s\S]*?(?=def dispose\()/,'def _actual_outcome('+nleVerifier);
 fs.writeFileSync(root+'/policy/decisions.py',algorithms.replace('return [chance["chance_id"] for chance in data["frontier_view"][:policy_config["max_chances"]]]','return [chance["chance_id"] for chance in data["frontier_view"] if chance["chance_id"] == "chance-1"]').replace('def launch(data, *, policy_config, capabilities):\n    return True','def launch(data, *, policy_config, capabilities):\n    return all(c["status"] == "finished" for c in data["candidate_view"])').replace('if policy_config["value_type"] == "boolean"','if policy_config.get("value_type") == "boolean"').replace('elif policy_config["value_type"] == "enum"','elif policy_config.get("value_type") == "enum"').replace('{"intent": "Promote"}',failureMode==='continue'?'{"intent": "Promote", "notification": "recovery authoritative notification"}':'{"intent": "Promote"}'));
 const children=[];
 t.after(()=>{for(const p of children)if(p.exitCode===null)p.kill('SIGKILL');fake.closeAllConnections();fake.close()});
 const wait=async(fn,label)=>{const end=Date.now()+30000;while(Date.now()<end){if(await fn())return;await new Promise(r=>setTimeout(r,20))}throw Error('Timeout '+label+' '+root)};
  // The writer may be between writes of a JSON line. Only complete records
  // are observable; malformed complete records still fail this regression.
  const events=()=>fs.existsSync(root+'/events.jsonl')?fs.readFileSync(root+'/events.jsonl','utf8').split('\n').slice(0,-1).filter(Boolean).map(JSON.parse):[];
 const post=async body=>(await fetch(`http://127.0.0.1:${port}/api/request`,{method:'POST',headers:{origin:`http://127.0.0.1:${port}`,'content-type':'application/json'},body:JSON.stringify(body)})).status;
  const start=async resume=>{
   const loaded=events().filter(e=>e.message==='recovery_loaded').length;
   const p=spawn(project+'/target/debug/ascout-control-plane',[root,project+'/../pi',root+'/policy/policy.yaml','--web',`127.0.0.1:${port}`,'--model','recovery/fixture',...(resume?['--resume']:[])],{env:{...process.env,...(failureMode==='filesystem'?{TMPDIR:root+'/tmp'}:{}),ASCOUT_RUNTIME_CONFIG:root+'/runtime.json',PI_CODING_AGENT_DIR:root+'/pi'},stdio:['ignore','pipe','pipe']});children.push(p);
  p.stderr.on('data',b=>fs.appendFileSync(root+'/stderr.log',b));
   await wait(async()=>{assert.equal(p.exitCode,null,fs.existsSync(root+'/stderr.log')?fs.readFileSync(root+'/stderr.log','utf8'):'');try{return(await fetch(`http://127.0.0.1:${port}/api/observation/state-event-set`)).ok&&(!resume||events().filter(e=>e.message==='recovery_loaded').length>loaded)}catch{return false}},'start');return p;
  };
  // Pause publishes a generation without exiting. End only this fixture's
  // controller afterwards; adapter EOF reaps its owned processes. Shutdown
  // intentionally terminates/materializes the task and is not a save request.
  const endPaused=async p=>{
   assert.equal(p.exitCode,null,'Pause keeps the controller alive');
   const done=once(p,'exit');p.kill('SIGKILL');await done;assert.equal(p.signalCode,'SIGKILL');
   await wait(()=>!execFileSync('ps',['-eo','args'],{encoding:'utf8'}).split('\n').some(line=>line.includes(root)),'paused task processes exited');
  };
  const pause=async()=>{
   const previous=fs.existsSync(root+'/recovery.json')?fs.readFileSync(root+'/recovery.json','utf8'):null;
   assert.equal(await post({action:'pause'}),202);
    await wait(async()=>fs.existsSync(root+'/recovery.json')&&fs.readFileSync(root+'/recovery.json','utf8')!==previous&&JSON.stringify(await(await fetch(`http://127.0.0.1:${port}/api/observation/state-event-set`)).json()).includes('Paused;'),'Pause published');
  };
 let p=await start(false);
 let baseline;
 if(['retry','continue'].includes(failureMode)) {
   await pause();await endPaused(p);
  baseline=fs.readFileSync(root+'/recovery.json','utf8');p=await start(true);
  await wait(()=>events().some(e=>e.message==='recovery_loaded'),'baseline loaded');
 }
 await post({action:'prompt',prompt:'Start parallel fixture'});
 await wait(()=>events().filter(e=>e.message==='candidate_created').length===2,'two candidates');
  await wait(()=>events().some(e=>e.message==='action_started'&&e.action_id==='op1'),'active sibling tool');
  await wait(()=>events().some(e=>e.message==='runtime_event'&&e.event.payload.event==='operation_started'&&e.event.payload.operation?.operation==='invoke_tool'),'real Runtime action in flight');
 assert.equal(requests.length,1,'candidates should still be waiting for sibling result');
 if(['retry','continue'].includes(failureMode))fs.mkdirSync(root+'/recovery.tmp');
  if(['retry','continue'].includes(failureMode))await post({action:'pause'});else await pause();
 if(['retry','continue'].includes(failureMode)) {
  await wait(async()=>JSON.stringify(await(await fetch(`http://127.0.0.1:${port}/api/observation/state-event-set`)).json()).includes('Save failed'),'injected publish failure');
  assert.equal(p.exitCode,null,'failed save remains paused and alive');
  assert.equal(fs.readFileSync(root+'/recovery.json','utf8'),baseline,'previous manifest remains intact');
  assert.equal(requests.length,1,'failure does not restart inference');
  fs.rmdirSync(root+'/recovery.tmp');
  if(failureMode==='continue') {
    await post({action:'prompt',prompt:'/continue'});
   await wait(()=>requests.length>=4,'all branches continue after failed save');
  }
   await pause();
 }
  const failure=events().find(e=>e.message==='recovery_saved'&&e.error);assert.equal(failure,undefined,JSON.stringify(failure));
  await endPaused(p);
 assert.ok(!execFileSync('ps',['-eo','args'],{encoding:'utf8'}).split('\n').some(line=>line.includes(root)),'all task processes exited');
 const manifest=JSON.parse(fs.readFileSync(root+'/recovery.json')),saved=JSON.parse(fs.readFileSync(manifest.files['agent.json'].path));
 assert.equal(saved.candidateSetups.length,failureMode==='continue'?0:2,'persist sibling waiting relations');
 const identities=saved.tree.branches.map(([id])=>id);assert.equal(identities.length,failureMode==='continue'?1:3);
 const immutable=saved.tree.sources.map(source=>[source.file,fs.readFileSync(source.file,'utf8')]);
 if(failureMode==='continue')assert.equal(saved.tree.sources.flatMap(s=>s.entries).filter(e=>e.customType==='notification'&&e.content==='recovery authoritative notification').length>=1,true);
  const runtimeRecord=path=>{const value=JSON.parse(fs.readFileSync(path));if(value.kind==='runtime-components')return runtimeRecord(path.replace(/\.json$/,'.components/0.json'));return value};
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
   const completedBeforeInput=events().filter(e=>e.message==='task_completed').length;
   await post({action:'prompt',prompt:'/continue'});
   if(failureMode==='continue')await post({action:'prompt',prompt:'Next task after completed recovery'});
  await wait(()=>requests.length>=count+(failureMode==='continue'?1:3),'branches advance');
  assert.ok(!requests.some(r=>r.messages.some(m=>m.role==='user'&&JSON.stringify(m.content).includes('/continue'))),'continue is a control command, not model input');
 const texts=fs.readFileSync(root+'/session.jsonl','utf8');assert.match(texts,/candidate advanced after recovery/);
 // Candidate model requests must contain paired real sibling results, not abort errors.
 const candidateRequests=requests.slice(1,failureMode==='continue'?count:undefined).filter(r=>r.messages.some(m=>m.role==='tool'&&['moved','blocked'].includes(m.content)));
 assert.equal(candidateRequests.length,2);
  for(const request of candidateRequests){const results=request.messages.filter(m=>m.role==='tool');assert.equal(results.length,2);assert.ok(!JSON.stringify(results).includes('cancelled'));}
  if(failureMode==='filesystem') {
   await wait(()=>events().some(e=>e.message==='task_completed'&&e.branch_id!=='authority'),'promoted task materialized');
   const before=events().filter(e=>e.message==='task_completed').length;
   await post({action:'prompt',prompt:'verify promoted filesystem'});
   await wait(()=>events().filter(e=>e.message==='task_completed').length>before,'promoted file task');
   assert.equal(fs.readFileSync(root+'/workspace/promoted-file','utf8'),'promoted');
   const write=events().find(e=>e.message==='action_started'&&e.action_id==='promoted-write');
   assert.ok(write&&write.branch_id!=='authority','file tool belongs to promoted Branch');
  }
   // This path has submitted a new, finite task. Pause during finishing is
   // intentionally ignored, so wait for its publication receipt first.
   if(failureMode==='continue')await wait(()=>events().filter(e=>e.message==='task_completed').length>completedBeforeInput,'continued input task materialized');
   await pause();await endPaused(p);
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
   await pause();await endPaused(p);
 }
 console.log('PASS active/sibling recovery',root,identities);
});

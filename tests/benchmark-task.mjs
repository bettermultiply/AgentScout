// Real Terminal-Bench task, FS/Pi/control plane, and local scripted SSE only.
import fs from 'node:fs';
import http from 'node:http';
import {spawn, spawnSync} from 'node:child_process';
import assert from 'node:assert/strict';
import {test} from 'node:test';

for(const scenario of ['match','mismatch','authority_first','fast_failure']) {
test(`FS backend recovers original Terminal-Bench files with ${scenario}`,{timeout:120000},async t=>{
 const project=process.cwd(),fixture=project+'/tests/fixtures/recover-obfuscated-files';
  const root=fs.mkdtempSync('/tmp/opencode/benchmark-task-');console.log('evidence',root);
  for(const name of ['workspace','pi','policy','tmp','home'])fs.mkdirSync(root+'/'+name);
  fs.mkdirSync(root+'/home/.pi/agent',{recursive:true});
 fs.copyFileSync(fixture+'/environment/NOTE_FROM_ATTACKER.txt',root+'/workspace/NOTE_FROM_ATTACKER.txt');
 fs.cpSync(fixture+'/environment/setup_files',root+'/workspace/sensitive_data',{recursive:true});
 const prompt=fs.readFileSync(fixture+'/instruction.md','utf8');
  let requests=0;
 const commands=[
  'ls -la; cat NOTE_FROM_ATTACKER.txt',
   'set -e; mkdir -p recovered; for f in sensitive_data/*.b64_content; do n=$(basename "$f" .b64_content | base64 -d); base64 -d "$f" > "recovered/$n"; done',
  null,
 ];
 const fake=http.createServer((req,res)=>{let body='';req.on('data',b=>body+=b);req.on('end',()=>{
   fs.appendFileSync(root+'/requests.jsonl',body+'\n');const request=JSON.parse(body);requests++;
   const index=request.messages.filter(m=>m.role==='tool').length,command=commands[index];
   const fast=request.model==='fast';
   if(fast&&scenario==='fast_failure'){res.writeHead(500);res.end('simulated failure');return;}
   setTimeout(()=>{
  res.writeHead(200,{'content-type':'text/event-stream'});
   const selected=fast&&scenario==='mismatch'?'printf rejected > rejected-candidate':command;
   const delta=selected?{tool_calls:[{index:0,id:(fast?'fast':'shell')+index,type:'function',function:{name:'bash',arguments:JSON.stringify({command:selected})}}]}:{content:'done'};
  const chunk=(delta,finish_reason=null)=>({id:'fixture',object:'chat.completion.chunk',created:0,model:'fixture',choices:[{index:0,delta,finish_reason}]});
   res.write('data: '+JSON.stringify(chunk(delta))+'\n\n');res.end('data: '+JSON.stringify(chunk({},command?'tool_calls':'stop'))+'\n\ndata: [DONE]\n\n');
   },scenario==='authority_first'?(fast?1500:600):(fast?10:800));
 })});await new Promise(r=>fake.listen(0,'127.0.0.1',r));
 t.after(()=>{fake.closeAllConnections();fake.close()});
 const port=await new Promise(r=>{const s=http.createServer();s.listen(0,'127.0.0.1',()=>{const p=s.address().port;s.close(()=>r(p))})});
  fs.writeFileSync(root+'/pi/models.json',JSON.stringify({providers:{fixture:{baseUrl:`http://127.0.0.1:${fake.address().port}/v1`,api:'openai-completions',apiKey:'fake',models:['fixture','fast'].map(id=>({id,reasoning:false,input:['text'],contextWindow:32768,maxTokens:256}))}}}));
 for(const name of ['policy.yaml','decisions.py'])fs.copyFileSync(project+'/assets/policy/policy/'+name,root+'/policy/'+name);
  fs.writeFileSync(root+'/policy/policy.yaml',fs.readFileSync(root+'/policy/policy.yaml','utf8').replace('max_steps: 5','max_steps: 100').replace('max_chances: 0','max_chances: 1').replace('scenarios: []','scenarios: [{when: {kind: model_inference}}]').replace('scenarios: []','scenarios:\n        - when: {tool_name: model}\n          predict: branch_model\n          model: fixture/fast'));
  fs.writeFileSync(root+'/runtime.json',JSON.stringify({backend:'nle',control_socket:root+'/runtime.sock',adapter_instance:'fs',python:project+'/../.venv-nle/bin/python',bridge:project+'/examples/nle-runtime-bridge.py',guidebook:project+'/examples/nle/Guidebook.txt',seed:7,filesystem:{state_directory:root+'/fs-state',isolated_paths:[{path:root+'/workspace',target:'/app'}],passthrough_rw:[root]}}));
   const p=spawn(project+'/target/debug/ascout-control-plane',[root,project+'/../pi',root+'/policy/policy.yaml','--web',`127.0.0.1:${port}`,'--model','fixture/fixture','--models',root+'/pi/models.json'],{env:{...process.env,HOME:root+'/home',USERPROFILE:root+'/home',TMPDIR:root+'/tmp',ASCOUT_RUNTIME_CONFIG:root+'/runtime.json'},stdio:['ignore','ignore','pipe']});
  p.stderr.on('data',b=>fs.appendFileSync(root+'/stderr.log',b));
  t.after(()=>{if(p.exitCode===null)p.kill('SIGKILL')});
  const events=()=>fs.existsSync(root+'/events.jsonl')?fs.readFileSync(root+'/events.jsonl','utf8').split('\n').filter(Boolean).map(s=>JSON.parse(s)):[];
  const sseEvents=[];
  const sseAbort=new AbortController();
  let sseError=null;
  const sseDeadline=Date.now()+60000;
  let sseResponse;
  while(Date.now()<sseDeadline){
   assert.equal(p.exitCode,null,'process exited: '+root);
   try{
    sseResponse=await fetch(`http://127.0.0.1:${port}/api/observation/events`,{signal:sseAbort.signal});
    break;
   }catch(error){
    if(sseAbort.signal.aborted)throw error;
    if(Date.now()>=sseDeadline)throw Error(`SSE connection timeout: ${error.message} ${root}`);
    await new Promise(r=>setTimeout(r,50));
   }
  }
  assert.ok(sseResponse,'SSE connection timeout '+root);
  assert.ok(sseResponse.ok,'observation SSE accepted');
  const readSse=(async()=>{
   const decoder=new TextDecoder();let pending='';
   try{
    for await(const chunk of sseResponse.body){
     pending+=decoder.decode(chunk,{stream:true});
     const lines=pending.split('\n');pending=lines.pop()??'';
     for(const line of lines){
      if(!line.startsWith('data: '))continue;
      const value=line.slice(6);
      if(value==='[DONE]')continue;
      try{sseEvents.push(JSON.parse(value))}catch(error){throw Error(`invalid SSE event: ${error.message}`)}
     }
    }
   }catch(error){if(!sseAbort.signal.aborted)sseError=error}
  })();
   t.after(async()=>{
    sseAbort.abort();
    await readSse;
    fs.writeFileSync(root+'/observation.jsonl',sseEvents.map(e=>JSON.stringify(e)+'\n').join(''));
   });
  const waitForStatus=async(predicate,label)=>{
   while(Date.now()<sseDeadline){
    assert.equal(p.exitCode,null,'process exited: '+root);
    if(sseError)throw Error(`SSE failed while waiting for ${label}: ${sseError.message} ${root}`);
    if(sseEvents.some(predicate))return;
    await new Promise(r=>setTimeout(r,20));
   }
   throw Error(`Timeout waiting for ${label}; SSE evidence: ${JSON.stringify(sseEvents.slice(-10))} ${root}`);
  };
    await waitForStatus(e=>e.event==='run_status'&&e.ready===true,'run_status ready');
    while(!fs.existsSync(root+'/policy.json')&&Date.now()<sseDeadline){
     assert.equal(p.exitCode,null,'process exited before policy artifact: '+root);
     await new Promise(r=>setTimeout(r,20));
    }
    assert.ok(fs.existsSync(root+'/policy.json'),'policy artifact exists before prompt: '+root);
    const policyArtifact=JSON.parse(fs.readFileSync(root+'/policy.json','utf8'));
   assert.equal(policyArtifact.version,1,'normal YAML startup writes a compiled Policy artifact');
   assert.equal(typeof policyArtifact.digest,'string');
   assert.match(policyArtifact.definition,/branch_model/,'compiled Policy enables branch_model');
   assert.match(policyArtifact.definition,/fixture\/fast/,'compiled Policy selects the candidate model');
   const predictBinding=policyArtifact.decisions.predict;
   assert.ok(predictBinding,'compiled Policy contains predict binding');
    assert.ok(predictBinding.policy_config.scenarios.some(s=>s.when?.tool_name==='model'&&s.predict==='branch_model'&&s.model==='fixture/fast'),'artifact predict scenario selects branch_model fixture/fast for tool_name=model');
   // A tool command must run in the Branch overlay, never against the host
  // workspace. The host path may only change when materialization publishes
  // the Authority delta.
  let leakedBeforePublish=false;
  const watcher=setInterval(()=>{
   if(events().some(e=>e.message==='materialization_started')){clearInterval(watcher);return}
   if(fs.existsSync(root+'/workspace/recovered')){leakedBeforePublish=true;clearInterval(watcher)}
  },50);
  t.after(()=>clearInterval(watcher));
  const response=await fetch(`http://127.0.0.1:${port}/api/request`,{method:'POST',headers:{origin:`http://127.0.0.1:${port}`,'content-type':'application/json'},body:JSON.stringify({action:'prompt',prompt})});
  assert.ok(response.ok,'prompt accepted');
  await waitForStatus(e=>e.event==='run_status'&&e.running===true,'run_status running');
  await waitForStatus(e=>e.event==='run_status'&&e.ready===true&&e.running===false,'run_status ready after task');
  sseAbort.abort();
  while(Date.now()<sseDeadline&&!events().some(e=>e.message==='task_completed'))await new Promise(r=>setTimeout(r,20));
  const completed=events().find(e=>e.message==='task_completed');
  assert.ok(completed,`task_completed not written before deadline ${root}`);
   assert.equal(completed.status,'succeeded');
    const recorded=events();
    const observations=recorded.filter(e=>e.message==='observation').map(e=>e.observation);
    const candidates=recorded.filter(e=>e.message==='candidate_created');
    assert.ok(candidates.length>0,'a real Candidate was created: '+root);
   const modelRequests=fs.readFileSync(root+'/requests.jsonl','utf8').trim().split('\n').map(line=>JSON.parse(line));
   assert.ok(modelRequests.some(request=>request.model==='fixture'),'authority model was used');
   assert.ok(modelRequests.some(request=>request.model==='fast'),'candidate model was used');
    for(const candidate of candidates){
     const label=`${scenario} ${candidate.branch_id}: ${root}`;
     const started=recorded.find(e=>e.message==='action_started'&&e.branch_id===candidate.branch_id&&e.action_kind==='model_inference'&&e.arguments?.model==='fixture/fast');
     assert.ok(started,'candidate fast action_started '+label);
     const finished=recorded.find(e=>e.message==='action_finished'&&e.branch_id===candidate.branch_id&&e.action_id===started.action_id);
     assert.ok(finished,'candidate action_finished '+label);
     assert.ok(finished.timestamp_ms>=started.timestamp_ms,'candidate action timestamps are ordered '+label);
     // Pi can report cancellation Usage after clearing the action id. That
     // branch has exactly one model inference; require its sole Usage fact.
     const usages=observations.filter(e=>e.event==='model_usage'&&e.branch_id===candidate.branch_id&&e.model==='fast');
     const usage=usages.find(e=>e.action_id===started.action_id)??(scenario==='authority_first'&&usages.length===1&&usages[0].action_id===null?usages[0]:null);
     assert.ok(usage,'candidate fast model_usage '+label);
     assert.equal(typeof usage.total_tokens,'number','reported Usage (zero is valid for the local fixture) '+label);
     const disposition=recorded.find(e=>e.branch_id===candidate.branch_id&&e.disposition);
     assert.ok(disposition,'candidate disposition verdict '+label);
     assert.equal(disposition.policy_error,null,'disposition is not a policy error '+label);
     const proposals=observations.filter(e=>e.event==='proposal_added'&&e.chance_id===candidate.chance_id);
     const validations=observations.filter(e=>e.event==='proposal_validated'&&e.chance_id===candidate.chance_id);
     if(scenario==='match'||scenario==='mismatch'){
      assert.equal(finished.status,'succeeded','candidate model completed '+label);
      assert.equal(proposals.length,1,'real proposal_added (not candidate_created) '+label);
      assert.equal(validations.length,1,'real proposal_validated / Match completed '+label);
      assert.equal(validations[0].proposal_id,proposals[0].proposal_id,'Match validates the added Proposal '+label);
      assert.ok(validations[0].sequence>proposals[0].sequence,'Match follows ProposalAdded '+label);
      assert.equal(typeof validations[0].validation_id,'number','Match validation id '+label);
      assert.equal(typeof validations[0].validated_at_ms,'number','Match completion timestamp '+label);
      // The existing Match policy rejects an empty tool-call result, even
      // when both models finish with "done". Only matching tool calls promote.
      const compatible=scenario==='match'&&proposals[0].payload.tool_calls.length>0;
      assert.equal(disposition.verdict,compatible?'compatible':'incompatible','Match verdict '+label);
      assert.equal(disposition.disposition,compatible?'Promote':'Discard','completed candidate disposition '+label);
     }else{
      // No completed candidate result means no Proposal to Match. Do not
      // fabricate ProposalAdded/ProposalValidated for cancellation or failure.
      assert.equal(finished.status,scenario==='fast_failure'?'failed':'cancelled','candidate failure/cancellation fact '+label);
      assert.equal(proposals.length,0,'unfinished/failed candidate produces no Proposal '+label);
      assert.equal(validations.length,0,'no Match completion without a Proposal '+label);
      assert.equal(disposition.verdict,'incompatible','no completed candidate result '+label);
      assert.equal(disposition.disposition,'Discard','failed/late candidate is discarded '+label);
      const authorityFinished=recorded.find(e=>e.message==='action_finished'&&e.branch_id===candidate.parent&&e.action_id===started.action_id);
      assert.ok(authorityFinished,'authority completion for the same action '+label);
      if(scenario==='authority_first')assert.ok(authorityFinished.timestamp_ms<=finished.timestamp_ms,'authority completes before candidate cancellation '+label);
     }
    }
    if(scenario==='match')assert.ok(recorded.some(e=>e.disposition==='Promote'&&e.verdict==='compatible'),'normal Runtime promotion completed');
    else assert.ok(!recorded.some(e=>e.disposition==='Promote'),'nonmatching candidate cannot be promoted');
   assert.equal(fs.existsSync(root+'/workspace/rejected-candidate'),false,'discarded writes never materialize');
  clearInterval(watcher);
  assert.equal(leakedBeforePublish,false,'tool writes stayed in the Branch overlay until publication: '+root);
  const materialized=events().find(e=>e.message==='materialization_completed');
  assert.equal(materialized.filesystem.confirmation.published,true);
 const bodies=fs.readFileSync(root+'/requests.jsonl','utf8').trim().split('\n').map(line=>JSON.parse(line));
 assert.ok(bodies.some(body=>body.messages.some(message=>message.role==='user'&&(message.content===prompt||(Array.isArray(message.content)&&message.content.some(part=>part.text===prompt))))),'original instruction appears unchanged in model requests');
 const bwrap=spawnSync('which',['bwrap'],{encoding:'utf8'});
 assert.equal(bwrap.status,0,'bubblewrap must be installed');
 // A host without /app cannot create it under a read-only / bind. Recreate
 // that view with read-only top-level binds inside a disposable tmpfs root.
 const mounts=fs.existsSync('/app')?['--ro-bind','/','/']:['--tmpfs','/'];
 if(!fs.existsSync('/app')){
  for(const entry of fs.readdirSync('/').filter(name=>!['app','dev','proc'].includes(name))){
   const path='/'+entry;
   mounts.push(...(fs.lstatSync(path).isSymbolicLink()?['--symlink',fs.readlinkSync(path),path]:['--ro-bind',path,path]));
  }
 }
 mounts.push('--dev','/dev','--proc','/proc','--dir','/app','--bind',root+'/workspace','/app','--remount-ro','/');
  const verification=spawnSync(bwrap.stdout.trim(),[...mounts,'--','/home/betmul/BETMUL/AgentScout-suit/.terminal-bench/bin/python','-m','pytest',fixture+'/tests/test_outputs.py','-rA','-p','no:cacheprovider'],{encoding:'utf8',timeout:30000,env:{...process.env,TMPDIR:'/dev/shm',PYTHONDONTWRITEBYTECODE:'1'}});
 fs.writeFileSync(root+'/verification.stdout.log',verification.stdout??'');
 fs.writeFileSync(root+'/verification.stderr.log',verification.stderr??'');
 assert.equal(verification.status,0,'original benchmark assertions: '+verification.stdout+verification.stderr+(verification.error??''));
 console.log('PASS benchmark task',root);
});
}

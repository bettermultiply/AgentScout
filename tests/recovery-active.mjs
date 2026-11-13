// Real Pi + real NLE + local OpenAI fixture. No external model requests.
import fs from 'node:fs';
import http from 'node:http';
import {spawn,execFileSync} from 'node:child_process';
import {once} from 'node:events';
import assert from 'node:assert/strict';
import {test} from 'node:test';

test('active tool batch and sibling-blocked candidates survive process exit', {timeout:120000}, async t=>{
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
 let policy=fs.readFileSync(project+'/assets/policy/policy.yaml','utf8').replace('max_chances: 0','max_chances: 1').replace('candidates: ["example"]','enum_values: ["moved", "blocked"]\n      candidates: ["moved", "blocked"]');
 fs.writeFileSync(root+'/policy/policy.yaml',policy);
 fs.writeFileSync(root+'/policy/decisions.py',fs.readFileSync(project+'/assets/policy/decisions.py','utf8').replace('return [chance["chance_id"] for chance in data["frontier_view"][:policy_config["max_chances"]]]','return [chance["chance_id"] for chance in data["frontier_view"] if chance["chance_id"] == "chance-1"]').replace('def launch(data, *, policy_config, capabilities):\n    return True','def launch(data, *, policy_config, capabilities):\n    return all(c["status"] == "finished" for c in data["candidate_view"])').replace('if policy_config["value_type"] == "boolean"','if policy_config.get("value_type") == "boolean"').replace('elif policy_config["value_type"] == "enum"','elif policy_config.get("value_type") == "enum"'));
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
 let p=await start(false);await post({action:'prompt',prompt:'Start parallel fixture'});
 await wait(()=>events().filter(e=>e.message==='candidate_created').length===2,'two candidates');
 await wait(()=>events().some(e=>e.message==='action_started'&&e.action_id==='op1'),'active sibling tool');
 assert.equal(requests.length,1,'candidates should still be waiting for sibling result');
 const exited=once(p,'exit');await post({action:'shutdown'});
 await wait(()=>p.exitCode!==null||events().some(e=>e.message==='recovery_saved'&&e.error),'shutdown');
 const failure=events().find(e=>e.message==='recovery_saved'&&e.error);assert.equal(failure,undefined,JSON.stringify(failure));
 await exited;assert.equal(p.exitCode,0,fs.readFileSync(root+'/stderr.log','utf8'));
 assert.ok(!execFileSync('ps',['-eo','args'],{encoding:'utf8'}).split('\n').some(line=>line.includes(root)),'all task processes exited');
 const manifest=JSON.parse(fs.readFileSync(root+'/recovery.json')),saved=JSON.parse(fs.readFileSync(manifest.files['agent.json'].path));
 assert.equal(saved.candidateSetups.length,2,'persist sibling waiting relations');
 const identities=saved.tree.branches.map(([id])=>id);assert.equal(identities.length,3);
 const immutable=saved.tree.sources.map(source=>[source.file,fs.readFileSync(source.file,'utf8')]);
 const beforeRuntime=JSON.parse(fs.readFileSync(manifest.files['runtime.json'].path));
 assert.equal(beforeRuntime.worlds[0].actions.length,1,'collect in-flight real action before publishing');
 const count=requests.length;p=await start(true);await wait(()=>events().some(e=>e.message==='recovery_loaded'),'loaded');
 await new Promise(r=>setTimeout(r,200));assert.equal(requests.length,count,'restore waits for user input');
 await post({action:'prompt',prompt:'Continue after restart'});
 await wait(()=>requests.length>=count+3,'all three branches advance');
 const texts=fs.readFileSync(root+'/session.jsonl','utf8');assert.match(texts,/candidate advanced after recovery/);
 // Candidate model requests must contain paired real sibling results, not abort errors.
 const candidateRequests=requests.slice(count).filter(r=>r.messages.some(m=>m.role==='tool'&&['moved','blocked'].includes(m.content)));
 assert.equal(candidateRequests.length,2);
 for(const request of candidateRequests){const results=request.messages.filter(m=>m.role==='tool');assert.equal(results.length,2);assert.ok(!JSON.stringify(results).includes('cancelled'));}
 const again=once(p,'exit');await post({action:'shutdown'});await wait(()=>p.exitCode!==null||events().some(e=>e.message==='recovery_saved'&&e.error),'second shutdown');await again;assert.equal(p.exitCode,0);
 const afterManifest=JSON.parse(fs.readFileSync(root+'/recovery.json'));
 const afterRuntime=JSON.parse(fs.readFileSync(afterManifest.files['runtime.json'].path));
 assert.equal(afterRuntime.worlds[0].actions.length,2,'each of the original actions executes exactly once');
 for(const [file,content] of immutable)assert.equal(fs.readFileSync(file,'utf8'),content,'saved sources remain immutable');
 assert.ok(!execFileSync('ps',['-eo','args'],{encoding:'utf8'}).split('\n').some(line=>line.includes(root)),'resumed task processes exited');
 console.log('PASS active/sibling recovery',root,identities);
});

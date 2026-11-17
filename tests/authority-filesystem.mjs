// Real local FS/Pi/control plane, deterministic local SSE; never a paid model.
import fs from 'node:fs';
import http from 'node:http';
import {spawn} from 'node:child_process';
import {once} from 'node:events';
import assert from 'node:assert/strict';
import {test} from 'node:test';

test('Authority publishes after background writers, resets across tasks, cancels and retries failure',{timeout:120000},async t=>{
 const project=process.cwd(),root=fs.mkdtempSync('/tmp/opencode/authority-filesystem-');console.log('evidence',root);
 for(const name of ['workspace','pi','policy','tmp'])fs.mkdirSync(root+'/'+name);
 let requests=0;
  const commands=[`printf first > a; (echo $BASHPID > '${root}/background-pid'; while :; do printf x >> '${root}/background-heartbeat'; sleep 0.05; done) >/dev/null 2>&1 &`,null,
   {name:'write',args:{path:'b',content:'before'}},{name:'edit',args:{path:'b',oldText:'before',newText:'second'}},{name:'read',args:{path:'b'}},null,
   `printf cancelled > c; printf ready > '${root}/cancel-ready'; sleep 30`,null,`printf partial > partial; ln -sf new z`,null];
 const fake=http.createServer((req,res)=>{let body='';req.on('data',b=>body+=b);req.on('end',()=>{
  fs.appendFileSync(root+'/requests.jsonl',body+'\n');const index=requests++,command=commands[index];
  res.writeHead(200,{'content-type':'text/event-stream'});
   const delta=command?{tool_calls:[{index:0,id:'shell'+index,type:'function',function:{name:typeof command==='string'?'bash':command.name,arguments:JSON.stringify(typeof command==='string'?{command}:command.args)}}]}:{content:'done'};
  const chunk=(delta,finish_reason=null)=>({id:'fixture',object:'chat.completion.chunk',created:0,model:'fixture',choices:[{index:0,delta,finish_reason}]});
  res.write('data: '+JSON.stringify(chunk(delta))+'\n\n');res.end('data: '+JSON.stringify(chunk({},command?'tool_calls':'stop'))+'\n\ndata: [DONE]\n\n');
 })});await new Promise(r=>fake.listen(0,'127.0.0.1',r));
 const port=await new Promise(r=>{const s=http.createServer();s.listen(0,'127.0.0.1',()=>{const p=s.address().port;s.close(()=>r(p))})});
 fs.writeFileSync(root+'/pi/models.json',JSON.stringify({providers:{fixture:{baseUrl:`http://127.0.0.1:${fake.address().port}/v1`,api:'openai-completions',apiKey:'fake',models:[{id:'fixture',reasoning:false,input:['text'],contextWindow:32768,maxTokens:256}]}}}));
  for(const name of ['policy.yaml','decisions.py'])fs.copyFileSync(project+'/assets/policy/policy/'+name,root+'/policy/'+name);
  fs.writeFileSync(root+'/policy/policy.yaml',fs.readFileSync(root+'/policy/policy.yaml','utf8').replace('max_steps: 5','max_steps: 100'));
 fs.writeFileSync(root+'/runtime.json',JSON.stringify({backend:'nle',control_address:'127.0.0.1:18180',adapter_instance:'fs',python:project+'/../.venv-nle/bin/python',bridge:project+'/examples/nle-runtime-bridge.py',guidebook:project+'/examples/nle/Guidebook.txt',seed:7,filesystem:{state_directory:root+'/fs-state',isolated_paths:[root+'/workspace'],passthrough_rw:[root]}}));
 const p=spawn(project+'/target/debug/ascout-control-plane',[root,project+'/../pi',root+'/policy/policy.yaml','--web',`127.0.0.1:${port}`,'--model','fixture/fixture'],{env:{...process.env,TMPDIR:root+'/tmp',ASCOUT_RUNTIME_CONFIG:root+'/runtime.json',PI_CODING_AGENT_DIR:root+'/pi'},stdio:['ignore','ignore','pipe']});
 p.stderr.on('data',b=>fs.appendFileSync(root+'/stderr.log',b));
 t.after(()=>{if(p.exitCode===null)p.kill('SIGKILL');fake.closeAllConnections();fake.close()});
 const events=()=>fs.existsSync(root+'/events.jsonl')?fs.readFileSync(root+'/events.jsonl','utf8').split('\n').filter(Boolean).map(s=>JSON.parse(s)):[];
 const wait=async(fn,label)=>{const deadline=Date.now()+30000;while(Date.now()<deadline){assert.equal(p.exitCode,null,'process exited: '+root);if(await fn())return;await new Promise(r=>setTimeout(r,20))}throw Error('Timeout '+label+' '+root)};
 const post=body=>fetch(`http://127.0.0.1:${port}/api/request`,{method:'POST',headers:{origin:`http://127.0.0.1:${port}`,'content-type':'application/json'},body:JSON.stringify(body)});
 const completed=()=>events().filter(e=>e.message==='task_completed');
 await wait(()=>events().some(e=>e.message==='branch_ready'),'ready');
 await post({action:'prompt',prompt:'first'});
 await wait(()=>events().some(e=>e.message==='action_started'&&e.name==='bash'),'writer started');
 await new Promise(r=>setTimeout(r,200));
  assert.equal(fs.existsSync(root+'/workspace/a'),false,'background writer prevents publication');assert.equal(completed().length,0);
   // No release gate exists: only cancellation can end this resident writer.
    await wait(()=>events().some(e=>e.message==='materialization_started'),'waiting on background writer');
    await wait(()=>fs.existsSync(root+'/background-heartbeat'),'background writer alive');
    const liveHeartbeat=fs.statSync(root+'/background-heartbeat').size;
    await wait(()=>fs.statSync(root+'/background-heartbeat').size>liveHeartbeat,'writer continues without a release');
    assert.equal(completed().length,0,'an infinite writer is not naturally drained');
   const cancellationDeadline=Date.now()+3000;
   p.kill('SIGINT');
   while(completed().length===0&&Date.now()<cancellationDeadline)await new Promise(r=>setTimeout(r,20));
   const responded=completed().length===1;
    assert.ok(responded,'SIGINT must terminate a resident background writer during materialization');
 await wait(()=>completed().length===1,'first materialized');
   assert.equal(completed()[0].status,'cancelled');
   const heartbeat=fs.readFileSync(root+'/background-heartbeat','utf8');
   await new Promise(r=>setTimeout(r,150));
   assert.equal(fs.readFileSync(root+'/background-heartbeat','utf8'),heartbeat,'writer is stopped before task completion');
  assert.equal(events().find(e=>e.message==='materialization_completed').filesystem.confirmation.published,true,'publication receipt survives Runtime composition');
 fs.writeFileSync(root+'/workspace/a','host after first task');
 await post({action:'prompt',prompt:'second'});await wait(()=>completed().length===2,'second materialized');
 assert.equal(fs.readFileSync(root+'/workspace/a','utf8'),'host after first task','old increment must not replay');assert.equal(fs.readFileSync(root+'/workspace/b','utf8'),'second');
  assert.ok(fs.readFileSync(root+'/requests.jsonl','utf8').includes('second'),'read observes edited overlay content');
  await post({action:'prompt',prompt:'cancel'});await wait(()=>requests===7,'cancellation tool');
  await wait(()=>fs.existsSync(root+'/cancel-ready'),'cancellation write actually finished');await post({action:'interrupt'});await wait(()=>completed().length===3,'cancel materialized');
 assert.equal(completed()[2].status,'cancelled');assert.equal(fs.readFileSync(root+'/workspace/c','utf8'),'cancelled');
 // Cancellation skips the final inference from this turn.
  requests=8;
 fs.symlinkSync('old',root+'/workspace/z');
 await post({action:'prompt',prompt:'failure'});await wait(()=>events().some(e=>e.message==='materialization_failed'),'publication fails');
  assert.equal(completed().length,3,'failure cannot announce success');
  const failureState=await(await fetch(`http://127.0.0.1:${port}/api/observation/state-event-set`)).json();
  assert.match(JSON.stringify(failureState),/Materialization failed/,'publication failure is visible to the user');
  assert.equal(fs.readlinkSync(root+'/workspace/z'),'old');
  assert.equal(fs.readFileSync(root+'/workspace/partial','utf8'),'partial');
  fs.writeFileSync(root+'/workspace/partial','host after receipt');
 fs.unlinkSync(root+'/workspace/z');
 await post({action:'prompt',prompt:'retry publication'});await wait(()=>completed().length>=4,'retry succeeds');
  assert.equal(fs.readlinkSync(root+'/workspace/z'),'new');
  assert.equal(fs.readFileSync(root+'/workspace/partial','utf8'),'host after receipt','retry skips already published paths');
  await wait(()=>completed().length===5,'retry input task complete');
   commands[requests]=`printf sigint > signal; printf ready > '${root}/signal-ready'; sleep 30`;
  const before=requests;
  await post({action:'prompt',prompt:'signal cancellation'});await wait(()=>requests>before,'SIGINT tool');
   await wait(()=>fs.existsSync(root+'/signal-ready'),'SIGINT write actually finished');
  const signalCount=completed().length;p.kill('SIGINT');await wait(()=>completed().length>signalCount,'SIGINT retains session');
   assert.equal(fs.readFileSync(root+'/workspace/signal','utf8'),'sigint');
   assert.equal(completed().at(-1).status,'cancelled','SIGINT must not turn cancellation into success');
   await post({action:'prompt',prompt:'same session after SIGINT'});await wait(()=>completed().length===signalCount+2,'next task after SIGINT');
   const idleCount=completed().length;
   p.kill('SIGINT');await post({action:'prompt',prompt:'/continue'});
   await new Promise(r=>setTimeout(r,100));assert.equal(completed().length,idleCount,'idle controls do not create a task');
   await post({action:'prompt',prompt:'/pause'});
   await wait(()=>fs.existsSync(root+'/recovery.json'),'idle pause saved');
   p.kill('SIGINT');await wait(()=>completed().length===idleCount+1,'interrupt paused session');
   await post({action:'prompt',prompt:'after paused interrupt'});await wait(()=>completed().length===idleCount+2,'new input after paused interruption');
    // Keep the non-cancelled case: a live background writer may only publish
    // after its explicit gate opens, and Pause cannot start a competing save.
    commands[requests]=`printf waiting > natural; (while [ ! -e '${root}/release-natural' ]; do sleep 0.05; done; printf finished > natural-late) >/dev/null 2>&1 &`;
    const naturalCount=completed().length,startedCount=events().filter(e=>e.message==='materialization_started').length;
    await post({action:'prompt',prompt:'natural background completion'});
    await wait(()=>events().filter(e=>e.message==='materialization_started').length>startedCount,'natural writer waiting');
    await post({action:'pause'});await post({action:'prompt',prompt:'queued until publication'});
    await new Promise(r=>setTimeout(r,100));
    assert.equal(completed().length,naturalCount);assert.equal(fs.existsSync(root+'/workspace/natural'),false);
    assert.equal(events().filter(e=>e.message==='materialization_started').length,startedCount+1,'only one publication in flight');
    fs.writeFileSync(root+'/release-natural','release');
    await wait(()=>completed().length===naturalCount+2,'natural writer and deferred input finish');
    assert.equal(fs.readFileSync(root+'/workspace/natural-late','utf8'),'finished');
    // Shutdown must also stop a resident writer while publication is waiting.
    commands[requests]=`printf shutdown > shutdown-file; (while [ ! -e '${root}/release-shutdown' ]; do sleep 0.05; done; printf wrong > shutdown-late) >/dev/null 2>&1 &`;
    const shutdownStarted=events().filter(e=>e.message==='materialization_started').length;
    await post({action:'prompt',prompt:'shutdown while waiting'});
    await wait(()=>events().filter(e=>e.message==='materialization_started').length>shutdownStarted,'shutdown writer waiting');
    const done=once(p,'exit');p.kill('SIGTERM');await done;assert.equal(p.exitCode,0);
    assert.equal(completed().at(-1).status,'cancelled');assert.equal(fs.readFileSync(root+'/workspace/shutdown-file','utf8'),'shutdown');
    assert.equal(fs.existsSync(root+'/workspace/shutdown-late'),false);
 console.log('PASS Authority lifecycle',root);
});

import {spawnControl, sessionPath} from './control-plane-fixture.mjs';
// Real control plane -> Runtime -> Pi, saved failing Policy, loopback SSE only.
import {test} from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import http from 'node:http';
import {spawn} from 'node:child_process';
import {once} from 'node:events';
import {savedPolicy,policyDigest} from './fixtures/predictor-fact-policy.mjs';

for(const multistep of [false,true])test(multistep?'Promote during inference preserves its pending model admission and five real NLE steps':'saved NLE Policy permits first inference and real Runtime tool with a local model',{timeout:60000},async t=>{
    const project=process.cwd(),root=fs.mkdtempSync('/tmp/opencode/predictor-fact-e2e-');
    console.log('evidence',root);
    for(const dir of ['workspace','pi','policy'])fs.mkdirSync(`${root}/${dir}`);
    const saved=savedPolicy();
    fs.writeFileSync(root+'/policy/policy.yaml',saved.definition);
    for(const [path,source] of Object.entries(saved.files)){
        fs.mkdirSync(root+'/policy/'+path.split('/').slice(0,-1).join('/'),{recursive:true});
        fs.writeFileSync(root+'/policy/'+path,source);
    }
    let requests=0;
    const fake=http.createServer((req,res)=>{
        assert.equal(req.socket.remoteAddress,'127.0.0.1');
        let body='';req.on('data',b=>body+=b);req.on('end',()=>{
            fs.appendFileSync(root+'/requests.jsonl',body+'\n');
            const request=JSON.parse(body),number=++requests;
            const steps=request.messages.filter(m=>m.role==='tool').length;
            const first=multistep?steps<5:number===1;
            const delta=first?{tool_calls:[{index:0,id:steps===0?'first-tool':`step-${steps+1}-${number}`,type:'function',function:{name:'nle_action',arguments:'{"action":4}'}}]}:{content:'local fixture completed'};
            const chunk=(delta,finish_reason=null)=>({id:'fixture',object:'chat.completion.chunk',created:0,model:'fixture',choices:[{index:0,delta,finish_reason}]});
            const respond=()=>{res.writeHead(200,{'content-type':'text/event-stream'});
            res.write('data: '+JSON.stringify(chunk(delta))+'\n\n');
            res.end('data: '+JSON.stringify(chunk({},first?'tool_calls':'stop'))+'\n\ndata: [DONE]\n\n');};
            if(multistep)setTimeout(respond,1600);else respond();
        });
    });
    await new Promise(r=>fake.listen(0,'127.0.0.1',r));
    t.after(()=>{fake.closeAllConnections();fake.close()});
    async function freePort(){const server=http.createServer();await new Promise(r=>server.listen(0,'127.0.0.1',r));const port=server.address().port;await new Promise(r=>server.close(r));return port;}
    const port=await freePort(),runtimePort=await freePort();
    fs.writeFileSync(root+'/pi/models.json',JSON.stringify({providers:{fixture:{baseUrl:`http://127.0.0.1:${fake.address().port}/v1`,api:'openai-completions',apiKey:'local-fake',models:[{id:'fixture',reasoning:false,input:['text'],contextWindow:32768,maxTokens:256}]}}}));
    fs.writeFileSync(root+'/runtime.json',JSON.stringify({backend:'nle',control_address:`127.0.0.1:${runtimePort}`,adapter_instance:'predictor-fact',python:project+'/../.venv-nle/bin/python',bridge:project+'/examples/nle-runtime-bridge.py',guidebook:project+'/examples/nle/Guidebook.txt',seed:7,action_delay_seconds:1,env_id:'NetHackScore-v0'}));
    const argv=[root,project+'/../pi',root+'/policy/policy.yaml','--web',`127.0.0.1:${port}`,'--model','fixture/fixture','--thinking','off'];
    fs.writeFileSync(root+'/command.json',JSON.stringify({argv,model_endpoint:`http://127.0.0.1:${fake.address().port}/v1`,saved_policy_digest:saved.digest}));
    const p=spawnControl(project+'/target/debug/ascout-control-plane',argv,{env:{...process.env,ASCOUT_RUNTIME_CONFIG:root+'/runtime.json',PI_CODING_AGENT_DIR:root+'/pi'},stdio:['ignore','pipe','pipe']});
    for(const stream of [p.stdout,p.stderr])stream.on('data',b=>fs.appendFileSync(root+'/control.log',b));
    const post=body=>fetch(`http://127.0.0.1:${port}/api/request`,{method:'POST',headers:{origin:`http://127.0.0.1:${port}`,'content-type':'application/json'},body:JSON.stringify(body)});
    t.after(async()=>{if(p.exitCode===null){try{await post({action:'shutdown'})}catch{};await Promise.race([once(p,'exit'),new Promise(r=>{const timer=setTimeout(()=>{p.kill('SIGKILL');r()},5000);timer.unref()})])}});
    const events=()=>fs.existsSync(sessionPath(root, 'events.jsonl'))?fs.readFileSync(sessionPath(root, 'events.jsonl'),'utf8').split('\n').slice(0,-1).filter(Boolean).map(JSON.parse):[];
    async function wait(predicate,label){const end=Date.now()+30000;while(Date.now()<end){assert.equal(p.exitCode,null,'control plane exited: '+root);if(await predicate())return;await new Promise(r=>setTimeout(r,20))}assert.fail('timeout '+label+' '+root);}
    await wait(()=>events().some(e=>e.message==='branch_ready'),'Agent ready');
    const compiled=JSON.parse(fs.readFileSync(sessionPath(root, 'policy.json')));
    for(const key of ['definition','version','files','runtime','decisions'])assert.deepEqual(compiled[key],saved[key],`normal startup preserves saved Policy ${key}`);
    assert.equal(compiled.digest,policyDigest({...saved,python:compiled.python}),'digest differs only if the installed Python identity differs');
    if(compiled.python===saved.python)assert.equal(compiled.digest,saved.digest);
    assert.equal((await post({action:'prompt',prompt:'Execute one west nle_action, then finish.'})).status,202);
    await wait(()=>events().some(e=>e.message==='task_completed'),'task completion');
    const rows=events(),started=rows.filter(e=>e.message==='action_started'),finished=rows.filter(e=>e.message==='action_finished');
    assert.ok(started.some(e=>e.action_kind==='model_inference'&&e.branch_id==='authority'));
    assert.ok(finished.some(e=>e.action_kind==='model_inference'&&e.status==='succeeded'));
    const tool=finished.find(e=>e.action_id==='first-tool'&&e.action_kind==='tool_execution');
    assert.equal(tool?.status,'succeeded');
    assert.match(tool.result,/position=\(25,5\).*\nmap:/s,'tool received the real west-step public observation, not its predicted label');
    assert.equal(tool.evidence.is_error,false);
    assert.ok(rows.some(e=>e.message==='speculative_admission_request'&&e.action_kind==='model_inference'));
    assert.ok(rows.some(e=>e.message==='speculative_admission_request'&&e.action_kind==='tool_execution'));
    assert.equal(rows.find(e=>e.message==='task_completed').status,'succeeded');
    if(multistep){
        const promoted=rows.find(e=>e.message==='command_result'&&e.operation==='promote'&&e.status==='applied');
        assert.ok(promoted,'normal Policy must actually promote');
        assert.ok(rows.some(e=>e.message==='action_finished'&&e.action_kind==='model_inference'&&e.branch_id===promoted.branch_id&&e.t_ms>promoted.t_ms),'promotion precedes in-flight inference completion');
        const tools=finished.filter(e=>e.action_kind==='tool_execution'&&e.status==='succeeded');
        assert.equal(tools.length,5);
        assert.ok(tools.filter(e=>e.t_ms>promoted.t_ms).length>=2);
        const entries=fs.readFileSync(sessionPath(root, 'session.jsonl'),'utf8').trim().split('\n').map(JSON.parse);
        assert.ok(entries.some(e=>e.message?.role==='assistant'&&e.message.stopReason==='stop'&&e.message.content.some(p=>p.text==='local fixture completed')),'native session persists the complete final reply');
    }
    const done=once(p,'exit');assert.equal((await post({action:'shutdown'})).status,202);await done;
    assert.equal(p.exitCode,0);
    const result={outcome:'local fake-model E2E passed',multistep,requests,tool:tool.action_id,tool_count:finished.filter(e=>e.action_kind==='tool_execution'&&e.status==='succeeded').length,model_started:started.filter(e=>e.action_kind==='model_inference').length,admissions:rows.filter(e=>e.message==='speculative_admission_request').length,candidates:rows.filter(e=>e.message==='branch_started'&&e.branch_id!=='authority').length,exit_code:p.exitCode,real_model_e2e:'not rerun; original failure retained'};
    fs.writeFileSync(root+'/acceptance.json',JSON.stringify(result,null,2));console.log(JSON.stringify(result));
});

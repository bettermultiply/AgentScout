import {spawnControl, runPath} from './control-plane-fixture.mjs';
// Real control plane -> Runtime -> Pi, saved failing Policy, loopback SSE only.
import {test} from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import http from 'node:http';
import {spawn} from 'node:child_process';
import {once} from 'node:events';
import {savedPolicy,policyDigest} from './fixtures/predictor-fact-policy.mjs';

const nestedModes = ['nested-full-accept', 'nested-full-reject', 'nested-part-accept', 'nested-part-reject'];
for(const mode of ['single','advanced','full','part', ...(process.env.ASCOUT_TEST_ACCEPTANCE_FS==='1'?['selective']:[]), ...nestedModes])test(`NLE with a local model: ${mode}`,{timeout:60000},async t=>{
    const nested = nestedModes.includes(mode);
    const innerFull = mode.startsWith('nested-full');
    const outerAccepts = mode.endsWith('-accept');
    const multistep=mode!=='single';
    const selective=mode==='selective';
    const speculativeFirst=mode==='full'||mode==='part'||selective||nested;
    const project=process.cwd(),root=fs.mkdtempSync('/tmp/opencode/predictor-fact-e2e-');
    console.log('evidence',root);
    for(const dir of ['workspace','pi','policy'])fs.mkdirSync(`${root}/${dir}`);
    const saved=savedPolicy();
    fs.writeFileSync(root+'/policy/policy.yaml',saved.definition);
    for(const [path,original] of Object.entries(saved.files)){
        let source=original;
        if(speculativeFirst&&path==='decisions.py') {
            source=source.replace('def launch(data, *, policy_config, capabilities):\n    return True', 'def launch(data, *, policy_config, capabilities):\n    return False');
            source=source.replace('if evidence is None:\n        return {"verdict": "indeterminate"}', 'if not evidence:\n        return {"verdict": "compatible" if data["match_context"]["operation_id"] == "first-tool" and data["proposal"]["payload"] == "moved" else "incompatible"}');
            if(mode==='part')source=source.replace('{"intent": "Promote"}', '{"intent": "Promote", "full": False, "part": {"scopes": [], "notification": data["context"]["candidate_proposal"]}}');
            if (nested) {
                source = source.replace(/^def (opportunity_pass|predict|launch|match_proposal|dispose)\(/gm, 'def unused_$1(');
                source = source.replace('return {"mode": "block"}', 'return {"mode": "allow"}');
                source += `

def opportunity_pass(data, *, policy_config, capabilities):
    op = data['op']
    return op['has_predictor'] and (op['kind'] == 'nle_action' and op['authority'] or op['kind'] == 'read' and not op['authority'])

def predict(data, *, policy_config, capabilities):
    payload = 'nested guess' if data['chance_context']['tool_name'] == 'read' else 'moved'
    return {'proposals': [{'payload': payload}], 'generation': 'batch'}

def launch(data, *, policy_config, capabilities):
    return False

def match_proposal(data, *, policy_config, capabilities):
    if data['match_context']['operation_id'] == 'first-tool':
        if data['match_context']['candidate']['branch']['execution_position']['transcript_index'] < 7:
            return {'verdict': 'indeterminate'}
        return {'verdict': '${outerAccepts ? 'compatible' : 'incompatible'}'}
    if data['match_context']['candidate']['branch']['execution_position']['transcript_index'] < 9:
        return {'verdict': 'indeterminate'}
    return {'verdict': 'compatible'}

def dispose(data, *, policy_config, capabilities):
    if data['resolution'] != 'accepted':
        return {'actions': [{'intent': 'Discard'}]}
    if data['branch']['operation_id'] != 'first-tool' and ${innerFull ? 'False' : 'True'}:
        directory = next(r['result']['payload'].strip() for r in data['context']['candidate_results'] if r.get('action_id') == 'inner-write')
        return {'actions': [{'intent': 'Promote', 'full': False, 'part': {'results': [{'id': 'local', 'files': [directory + '/nested-report.txt'], 'notification': 'nested part delivered'}]}}]}
    return {'actions': [{'intent': 'Promote'}]}
`;
            }
            if(selective) {
                source=source.replace('return {"mode": "block"}', 'return {"mode": "allow"}');
                source=source.replace('    evidence = data["evidence"]', `    position = data["match_context"]["candidate"]["branch"]["execution_position"]\n    if position["transcript_index"] < 8:\n        return {"verdict": "indeterminate"}\n    evidence = data["evidence"]`);
                // Select the files from the successful candidate tool result.
                source=source.replace('{"intent": "Promote"}', `{"intent": "Promote", "full": False, "part": {"results": [\n            {"id": "report", "files": [next(r["result"]["payload"].strip() for r in data["context"]["candidate_results"] if r["action_id"] == "candidate-files") + "/report.txt"], "notification": "selected report"},\n            {"id": "conflict", "files": [next(r["result"]["payload"].strip() for r in data["context"]["candidate_results"] if r["action_id"] == "candidate-files") + "/conflict.txt"], "notification": "rejected conflict"}]}}`);
            }
        }
        fs.mkdirSync(root+'/policy/'+path.split('/').slice(0,-1).join('/'),{recursive:true});
        fs.writeFileSync(root+'/policy/'+path,source);
    }
    let requests=0, finishCandidateInference;
    const fake=http.createServer((req,res)=>{
        assert.equal(req.socket.remoteAddress,'127.0.0.1');
        let body='';req.on('data',b=>body+=b);req.on('end',()=>{
            fs.appendFileSync(root+'/requests.jsonl',body+'\n');
            const request=JSON.parse(body),number=++requests;
            const steps=request.messages.filter(m=>m.role==='tool').length;
            const toolCall=(id,name,args)=>({tool_calls:[{index:0,id,type:'function',function:{name,arguments:JSON.stringify(args)}}]});
            const toolMessages=request.messages.filter(m=>m.role==='tool');
            const predicted=toolMessages.some(message=>message.tool_call_id==='first-tool'&&message.content==='moved');
            let delta;
            if (nested) {
                delta = steps === 0 ? toolCall('first-tool','nle_action',{action:4})
                    : predicted && steps === 1 ? toolCall('inner-tool','read',{path:'input.txt'})
                    : toolMessages.some(message=>message.tool_call_id==='inner-tool'&&message.content==='nested guess') && steps === 2
                        ? toolCall('inner-write','bash',{command:"printf 'nested report' > nested-report.txt; pwd"})
                        : {content:'parent completed'};
            } else if(selective) {
                const shell=(id, command)=>toolCall(id,'bash',{command});
                if(steps===0) delta=shell('authority-files', `printf 'authority draft' > draft.txt; (while [ ! -e '${root}/stop-writer' ]; do if [ -e '${root}/candidate-wrote' ] && [ ! -e '${root}/conflict-ready' ]; then printf 'authority conflict' > conflict.txt; touch '${root}/conflict-ready'; fi; printf x >> '${root}/heartbeat'; sleep 0.02; done) >/dev/null 2>&1 &`);
                else if(steps===1) delta=toolCall('first-tool','nle_action',{action:4});
                else if(predicted&&steps===2) delta=shell('candidate-files', `printf 'report' > report.txt; printf 'candidate draft' > draft.txt; printf 'candidate conflict' > conflict.txt; touch '${root}/candidate-wrote'; while [ ! -e '${root}/conflict-ready' ]; do sleep 0.02; done; pwd`);
                else delta={content:'local fixture completed'};
            } else {
                const first=speculativeFirst?steps<2:multistep?steps<5:number===1;
                delta=first?toolCall(steps===0?'first-tool':`step-${steps+1}-${number}`,'nle_action',{action:4}):{content:'local fixture completed'};
            }
            const chunk=(delta,finish_reason=null)=>({id:'fixture',object:'chat.completion.chunk',created:0,model:'fixture',choices:[{index:0,delta,finish_reason}]});
            const respond=()=>{res.writeHead(200,{'content-type':'text/event-stream'});
            res.write('data: '+JSON.stringify(chunk(delta))+'\n\n');
            res.end('data: '+JSON.stringify(chunk({},delta.tool_calls?'tool_calls':'stop'))+'\n\ndata: [DONE]\n\n');};
            if (nested && steps === 1 && request.model !== 'fast') setTimeout(respond, 1200);
            else if(selective&&predicted&&steps===3)finishCandidateInference=respond;
            else if(multistep&&!speculativeFirst)setTimeout(respond,1600);else respond();
        });
    });
    await new Promise(r=>fake.listen(0,'127.0.0.1',r));
    t.after(()=>{if(selective)fs.writeFileSync(root+'/stop-writer','');fake.closeAllConnections();fake.close()});
    async function freePort(){const server=http.createServer();await new Promise(r=>server.listen(0,'127.0.0.1',r));const port=server.address().port;await new Promise(r=>server.close(r));return port;}
    const port=await freePort(),runtimePort=await freePort();
    fs.writeFileSync(root+'/pi/models.json',JSON.stringify({providers:{fixture:{baseUrl:`http://127.0.0.1:${fake.address().port}/v1`,api:'openai-completions',apiKey:'local-fake',models:[{id:'fixture',reasoning:false,input:['text'],contextWindow:32768,maxTokens:256}]}}}));
    const filesystem=(nested || process.env.ASCOUT_TEST_ACCEPTANCE_FS==='1')&&speculativeFirst
        ?{state_directory:root+'/fs-state',isolated_paths:[runPath(root,'workspace')],passthrough_rw:[root]}
        :undefined;
    fs.writeFileSync(root+'/runtime.json',JSON.stringify({backend:'nle',control_address:`127.0.0.1:${runtimePort}`,adapter_instance:'predictor-fact',python:project+'/../.venv-nle/bin/python',bridge:project+'/examples/nle-runtime-bridge.py',guidebook:project+'/examples/nle/Guidebook.txt',seed:7,action_delay_seconds:1,env_id:'NetHackScore-v0',filesystem}));
    const argv=[root,project+'/../pi',root+'/policy/policy.yaml','--web',`127.0.0.1:${port}`,'--model','fixture/fixture','--thinking','off'];
    fs.writeFileSync(root+'/command.json',JSON.stringify({argv,model_endpoint:`http://127.0.0.1:${fake.address().port}/v1`,saved_policy_digest:saved.digest}));
    const p=spawnControl(project+'/target/debug/ascout-control-plane',argv,{env:{...process.env,ASCOUT_RUNTIME_CONFIG:root+'/runtime.json',PI_CODING_AGENT_DIR:root+'/pi'},stdio:['ignore','pipe','pipe']});
    for(const stream of [p.stdout,p.stderr])stream.on('data',b=>fs.appendFileSync(root+'/control.log',b));
    const post=body=>fetch(`http://127.0.0.1:${port}/api/request`,{method:'POST',headers:{origin:`http://127.0.0.1:${port}`,'content-type':'application/json'},body:JSON.stringify(body)});
    t.after(async()=>{if(p.exitCode===null){try{await post({action:'shutdown'})}catch{};await Promise.race([once(p,'exit'),new Promise(r=>{const timer=setTimeout(()=>{p.kill('SIGKILL');r()},5000);timer.unref()})])}});
    const events=()=>fs.existsSync(runPath(root, 'events.jsonl'))?fs.readFileSync(runPath(root, 'events.jsonl'),'utf8').split('\n').slice(0,-1).filter(Boolean).map(JSON.parse):[];
    async function wait(predicate,label){const end=Date.now()+30000;while(Date.now()<end){assert.equal(p.exitCode,null,'control plane exited: '+root);if(await predicate())return;await new Promise(r=>setTimeout(r,20))}assert.fail('timeout '+label+' '+root);}
    await wait(()=>events().some(e=>e.message==='branch_ready'),'Agent ready');
    const compiled=JSON.parse(fs.readFileSync(runPath(root, 'policy.json')));
    if(!speculativeFirst) {
        for(const key of ['definition','version','files','runtime','decisions'])assert.deepEqual(compiled[key],saved[key],`normal startup preserves saved Policy ${key}`);
        assert.equal(compiled.digest,policyDigest({...saved,python:compiled.python}),'digest differs only if the installed Python identity differs');
        if(compiled.python===saved.python)assert.equal(compiled.digest,saved.digest);
    }
    if (nested) fs.writeFileSync(runPath(root,'workspace/input.txt'),'real nested outcome');
    assert.equal((await post({action:'prompt',prompt:'Execute one west nle_action, then finish.'})).status,202);
    if (nested) {
        await wait(()=>events().some(e=>e.operation==='promote'&&e.status==='applied'),'inner acceptance');
        const rows = events();
        const applied = rows.findIndex(e=>e.operation==='promote'&&e.status==='applied');
        const outer = rows.find(e=>e.message==='branch_started'&&e.branch_id!=='authority').branch_id;
        const inner = rows[applied].branch_id;
        const transfers = events=>events.filter(e=>e.event?.payload?.operation?.operation==='accept_environment').map(e=>e.event);
        const [local] = transfers(rows.slice(0, applied));
        assert.equal(transfers(rows.slice(0, applied)).length, 1);
        assert.ok(!rows.slice(0, applied).some(e=>e.observation?.event==='authority_changed'&&e.observation.branch_id!=='authority'));
        assert.ok(!fs.existsSync(runPath(root,'workspace/nested-report.txt')), 'inner transfer stays in the speculative environment');
        if (!innerFull) {
            await wait(()=>fs.readFileSync(root+'/requests.jsonl','utf8').includes('nested part delivered'),'part input delivered to B');
            const requests = fs.readFileSync(root+'/requests.jsonl','utf8').trim().split('\n').map(JSON.parse);
            assert.ok(requests.some(request=>request.messages.some(message=>message.tool_call_id==='first-tool'&&message.content==='moved') && request.messages.some(message=>JSON.stringify(message.content).includes('nested part delivered'))));
        }
        await wait(()=>events().some(e=>e.message==='task_completed'),'nested task completion');
        const done=once(p,'exit');
        assert.equal((await post({action:'shutdown'})).status,202);
        await done;
        assert.equal(p.exitCode,0);
        const finalTree=JSON.parse(fs.readFileSync(runPath(root,'final-tree.json'),'utf8'));
        const continuation=innerFull ? inner : outer;
        assert.equal(finalTree.authority_branch_id, outerAccepts ? continuation : 'authority');
        const allTransfers=transfers(events());
        assert.equal(allTransfers.length, outerAccepts ? 2 : 1);
        const binding = id => finalTree.branches[id]?.environment_id ?? events().find(e=>e.branch?.branch_id===id)?.branch.environment_id;
        const outerEnvironment=outerAccepts ? allTransfers[1].payload.operation.source_environment_id : binding(continuation);
        assert.equal(local.environment_id, outerEnvironment);
        assert.equal(local.payload.operation.source_environment_id, binding(innerFull ? outer : inner));
        const report = runPath(root,'workspace/nested-report.txt');
        if (outerAccepts) {
            assert.equal(finalTree.branches[continuation].environment_id, allTransfers[1].environment_id);
            assert.equal(fs.readFileSync(report,'utf8'), 'nested report');
        } else {
            assert.ok(!fs.existsSync(report), 'outer rejection discards locally accepted files');
        }
        return;
    }
    if(selective) {
        await wait(()=>finishCandidateInference,'candidate inference reached fake model');
        await wait(()=>events().some(e=>e.event?.payload?.operation?.operation==='suspend_environment'),'authority suspension');
        await wait(()=>events().filter(e=>e.operation==='prepare_acceptance'&&e.status==='started').length===2,'candidate drain started');
        const heartbeat=fs.statSync(root+'/heartbeat').size;
        await new Promise(r=>setTimeout(r,150));
        assert.equal(fs.statSync(root+'/heartbeat').size,heartbeat,'AuB writer frozen while inference is held');
        assert.equal(events().filter(e=>e.operation==='prepare_acceptance'&&e.status==='applied').length,1);
        assert.ok(!events().some(e=>e.event?.payload?.operation?.operation==='accept_environment'));
        finishCandidateInference();
        await wait(()=>events().some(e=>e.operation==='cancel_acceptance'&&e.status==='applied'),'acceptance release');
        await wait(()=>fs.statSync(root+'/heartbeat').size>heartbeat,'authority writer resumed');
        fs.writeFileSync(root+'/stop-writer','');
    }
    await wait(()=>events().some(e=>e.message==='task_completed'),'task completion');
    const rows=events(),started=rows.filter(e=>e.message==='action_started'),finished=rows.filter(e=>e.message==='action_finished');
    assert.ok(started.some(e=>e.action_kind==='model_inference'&&e.branch_id==='authority'));
    assert.ok(finished.some(e=>e.action_kind==='model_inference'&&e.status==='succeeded'));
    const tool=finished.find(e=>e.action_id==='first-tool'&&e.action_kind==='tool_execution');
    if(mode!=='full') {
        assert.equal(tool?.status,'succeeded');
        assert.match(tool.result,/position=\(25,5\).*\nmap:/s,'tool received the real west-step public observation, not its predicted label');
        assert.equal(tool.evidence.is_error,false);
    }
    assert.ok(rows.some(e=>e.message==='speculative_admission_request'&&e.action_kind==='model_inference'));
    assert.ok(rows.some(e=>e.message==='speculative_admission_request'&&e.action_kind==='tool_execution'));
    assert.equal(rows.find(e=>e.message==='task_completed').status,'succeeded');
    let acceptanceBinding;
    if(speculativeFirst) {
        const accepted=rows.find(e=>e.message==='command_result'&&e.operation==='promote'&&e.status==='applied');
        assert.ok(accepted,'Policy acceptance must reach the Agent');
        const receipt=rows.find(e=>e.disposition===(mode==='full'?'Promote':'Part'));
        assert.equal(receipt?.branch_id,accepted.branch_id);
        const transfer=rows.find(e=>e.event?.payload?.operation?.operation==='accept_environment')?.event;
        assert.ok(transfer,'acceptance uses the normal Runtime operation');
        acceptanceBinding={
            continuation:mode==='full'?accepted.branch_id:'authority',
            target:transfer.environment_id,
            source:transfer.payload.operation.source_environment_id,
        };
        const requests=fs.readFileSync(root+'/requests.jsonl','utf8');
        if(selective) {
            assert.match(requests,/Accepted result: selected report/);
            assert.doesNotMatch(requests,/rejected conflict/);
            assert.deepEqual(receipt.outcome,{part_results:{results:['report']}});
            for(const [file, content] of Object.entries({'report.txt':'report','draft.txt':'authority draft','conflict.txt':'authority conflict'})) {
                assert.equal(fs.readFileSync(runPath(root,`workspace/${file}`),'utf8'),content);
            }
        } else if(mode==='part') {
            assert.match(requests,/Accepted result: moved/);
            assert.ok(!rows.some(e=>e.observation?.event==='authority_changed'&&e.observation.branch_id===accepted.branch_id));
        } else {
            assert.ok(started.some(e=>e.branch_id===accepted.branch_id&&e.t_ms>=accepted.t_ms),'winner continues after commit');
        }
    } else if(multistep){
        const promoted=rows.find(e=>e.message==='command_result'&&e.operation==='promote'&&e.status==='applied');
        assert.equal(promoted,undefined,'later authority inference prevents full acceptance');
        assert.ok(rows.some(e=>e.operation==='prepare_acceptance'&&e.status==='applied'));
        const tools=finished.filter(e=>e.action_kind==='tool_execution'&&e.status==='succeeded');
        assert.equal(tools.length,5);
        const entries=fs.readFileSync(runPath(root, 'session.jsonl'),'utf8').trim().split('\n').map(JSON.parse);
        assert.ok(entries.some(e=>e.message?.role==='assistant'&&e.message.stopReason==='stop'&&e.message.content.some(p=>p.text==='local fixture completed')),'native session persists the complete final reply');
    }
    const done=once(p,'exit');assert.equal((await post({action:'shutdown'})).status,202);await done;
    assert.equal(p.exitCode,0);
    if(acceptanceBinding) {
        const finalTree=JSON.parse(fs.readFileSync(runPath(root,'final-tree.json'),'utf8'));
        assert.equal(finalTree.authority_branch_id,acceptanceBinding.continuation);
        assert.equal(finalTree.branches[acceptanceBinding.continuation].environment_id,acceptanceBinding.target);
        if(mode==='full') {
            assert.equal(finalTree.branches.authority.environment_id,acceptanceBinding.source);
        }
    }
    const result={outcome:'local fake-model E2E passed',multistep,requests,tool:tool?.action_id,tool_count:finished.filter(e=>e.action_kind==='tool_execution'&&e.status==='succeeded').length,model_started:started.filter(e=>e.action_kind==='model_inference').length,admissions:rows.filter(e=>e.message==='speculative_admission_request').length,candidates:rows.filter(e=>e.message==='branch_started'&&e.branch_id!=='authority').length,exit_code:p.exitCode,real_model_e2e:'not rerun; original failure retained'};
    fs.writeFileSync(root+'/acceptance.json',JSON.stringify(result,null,2));console.log(JSON.stringify(result));
});

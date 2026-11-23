import { test } from 'node:test';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, rmSync } from 'node:fs';
import { join, resolve } from 'node:path';

test('standalone real Pi resumes assembled history and completes a built-in write tool task using a local fake model',()=>{
    const pi=resolve('../pi');
    const script=`
        import assert from 'node:assert/strict';
        import http from 'node:http';
        import {writeFileSync,readFileSync,mkdirSync} from 'node:fs';
        import {createAgentSession,SessionManager,ModelRuntime,resolveCliModel} from '${pi}/packages/coding-agent/src/index.ts';
        import {SessionTree} from '${resolve('instrumentation/pi/session-tree.ts')}';
        const directory=process.argv[1],agentDir=directory+'/agent';mkdirSync(agentDir);
        const requests=[];
        const server=http.createServer((req,res)=>{let body='';req.on('data',b=>body+=b);req.on('end',()=>{
            const request=JSON.parse(body);requests.push(request);
            const done=request.messages.some(m=>m.role==='tool'&&m.tool_call_id==='finish-task');
            const delta=done?{content:'Original task completed.'}:{tool_calls:[{index:0,id:'finish-task',type:'function',function:{name:'write',arguments:JSON.stringify({path:'answer.txt',content:'verified task complete'})}}]};
            res.writeHead(200,{'content-type':'text/event-stream'});
            for(const [d,finish] of [[delta,null],[{},done?'stop':'tool_calls']])res.write('data: '+JSON.stringify({id:'local',object:'chat.completion.chunk',created:0,model:'fixture',choices:[{index:0,delta:d,finish_reason:finish}]})+'\\n\\n');
            res.end('data: [DONE]\\n\\n');
        })});
        await new Promise(r=>server.listen(0,'127.0.0.1',r));
        let session;
        try {
            writeFileSync(agentDir+'/models.json',JSON.stringify({providers:{fixture:{baseUrl:'http://127.0.0.1:'+server.address().port+'/v1',api:'openai-completions',apiKey:'dummy',models:[{id:'fixture',reasoning:false,input:['text'],contextWindow:8192,maxTokens:512}]}}}));
            mkdirSync(agentDir+'/extensions');
            writeFileSync(agentDir+'/extensions/lifecycle.ts',"export default pi=>{pi.on('session_shutdown',async()=>{await new Promise(r=>setImmediate(r));pi.appendEntry('async-shutdown-fixture',{complete:true})})}");
            const modelRuntime=await ModelRuntime.create({modelsPath:agentDir+'/models.json',authPath:agentDir+'/auth.json'});
            const {model}=resolveCliModel({cliModel:'fixture/fixture',cliThinking:'off',modelRuntime});assert.ok(model);
            const main=SessionManager.open(directory+'/session.jsonl');
            main.appendModelChange(model.provider,model.id);
            main.appendMessage({role:'user',content:'Inspect the requirement, then write answer.txt containing verified task complete.',timestamp:1});
            const usage={input:0,output:0,cacheRead:0,cacheWrite:0,totalTokens:0,cost:{input:0,output:0,cacheRead:0,cacheWrite:0,total:0}};
            const assistant=content=>({role:'assistant',api:model.api,provider:model.provider,model:model.id,usage,content,stopReason:'stop',timestamp:2});
            const call=main.appendMessage({...assistant([{type:'toolCall',id:'inspect',name:'read',arguments:{path:'requirement.txt'}}]),stopReason:'toolUse'});
            const tree=new SessionTree(main),parent=tree.createBranch('authority',directory,main,call),candidate=tree.createBranch('candidate',directory,parent,call);
            const result=text=>({role:'toolResult',toolCallId:'inspect',toolName:'read',content:[{type:'text',text}],isError:false,timestamp:3});
            const prediction=tree.append('candidate',result('predicted requirement'));
            tree.append('candidate',assistant([{type:'text',text:'Ready to write the answer.'}]));
            parent.appendMessage(result('confirmed requirement: verified task complete'));
            tree.inputs.set('candidate',{source:parent,checkpoint:call,pairedTail:prediction,operations:['inspect']});
            tree.accepted=new Set(['candidate']);tree.promote('candidate');tree.stop('authority');tree.stop('candidate');tree.settle();
            const manager=SessionManager.open(directory+'/session.jsonl');
            ({session}=await createAgentSession({cwd:directory,agentDir,sessionManager:manager,modelRuntime,model,thinkingLevel:'off',tools:['write']}));
            await session.prompt('Continue the original task and write its answer file.');
            assert.equal(readFileSync(directory+'/answer.txt','utf8'),'verified task complete');
            assert.equal(requests.length,2);
            assert.ok(JSON.stringify(requests[0].messages).includes('confirmed requirement'));
            assert.ok(!JSON.stringify(requests[0].messages).includes('predicted requirement'));
            assert.ok(session.messages.some(m=>m.role==='toolResult'&&m.toolCallId==='finish-task'&&!m.isError));
            assert.ok(SessionManager.open(directory+'/session.jsonl').buildSessionContext().messages.some(m=>m.role==='assistant'&&m.content.some(p=>p.text==='Original task completed.')));
            await session.abort();
            await session.extensionRunner.emit({type:'session_shutdown',reason:'quit'});
            await session.waitForIdle();
            assert.ok(SessionManager.open(directory+'/session.jsonl').getEntries().some(e=>e.customType==='async-shutdown-fixture'&&e.data.complete));
        } finally {await session?.abort();session?.dispose();server.closeAllConnections();await new Promise(r=>server.close(r))}
    `;
    const directory=mkdtempSync('/var/tmp/pi-standalone-');
    try {
        execFileSync(process.execPath,['--no-warnings','--experimental-strip-types','--import',`${pi}/packages/coding-agent/src/experimental/source-resolver.ts`,'--input-type=module','-e',script,directory],{encoding:'utf8',timeout:30000,env:{...process.env,PI_CODING_AGENT_DIR:join(directory,'agent'),JITI_FS_CACHE:'false'}});
    } finally {
        rmSync(directory, {recursive:true, force:true});
    }
});

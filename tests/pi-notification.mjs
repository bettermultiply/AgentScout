import { test } from 'node:test';
import { execFileSync } from 'node:child_process';
import { mkdtempSync } from 'node:fs';
import { resolve } from 'node:path';

test('native notification waits for the full tool batch, persists once, and never starts a turn',()=>{
    const pi=resolve('../pi'),directory=mkdtempSync('/tmp/opencode/pi-notification-');
    execFileSync(process.execPath,['--no-warnings','--experimental-strip-types','--import',`${pi}/packages/coding-agent/src/experimental/source-resolver.ts`,'--input-type=module','-e',`
        import assert from 'node:assert/strict';
        import {mkdirSync,writeFileSync} from 'node:fs';
        import http from 'node:http';
        import {createAgentSession,SessionManager,ModelRuntime,resolveCliModel} from '${pi}/packages/coding-agent/src/index.ts';
        import {Notifications} from '${resolve('instrumentation/pi/notification.ts')}';
        import {SessionTree} from '${resolve('instrumentation/pi/session-tree.ts')}';
        import {requestKey,messageBody} from '${resolve('tools/replay/normalize.ts')}';
        const directory=process.argv[1],agentDir=directory+'/pi';mkdirSync(agentDir);
        const requests=[];let received;const first=new Promise(r=>received=r);let release;
        const server=http.createServer((req,res)=>{let body='';req.on('data',b=>body+=b);req.on('end',async()=>{
            const request=JSON.parse(body);requests.push(request);
            if(requests.length===1){received();await new Promise(r=>release=r)}
            const done=requests.length!==1;
            const delta=done?{content:'done'}:{tool_calls:[0,1].map(i=>({index:i,id:'op'+i,type:'function',function:{name:'fixture',arguments:'{}'}}))};
            res.writeHead(200,{'content-type':'text/event-stream'});
            for(const [d,finish] of [[delta,null],[{},done?'stop':'tool_calls']])res.write('data: '+JSON.stringify({id:'local',object:'chat.completion.chunk',created:0,model:'fixture',choices:[{index:0,delta:d,finish_reason:finish}]})+'\\n\\n');
            res.end('data: [DONE]\\n\\n');
        })});
        await new Promise(r=>server.listen(0,'127.0.0.1',r));let session;
        try {
            writeFileSync(agentDir+'/models.json',JSON.stringify({providers:{fixture:{baseUrl:'http://127.0.0.1:'+server.address().port+'/v1',api:'openai-completions',apiKey:'dummy',models:[{id:'fixture',reasoning:false,input:['text'],contextWindow:8192,maxTokens:512}]}}}));
            const modelRuntime=await ModelRuntime.create({modelsPath:agentDir+'/models.json',authPath:agentDir+'/auth.json'});
            const {model}=resolveCliModel({cliModel:'fixture/fixture',cliThinking:'off',modelRuntime});
            const tree=new SessionTree(SessionManager.open(directory+'/session.jsonl'));
            const manager=tree.createBranch('candidate',directory);const notifications=new Notifications();
            const command={command_id:'promote-one',notification:'the authoritative result'};
            let calls=0;
            ({session}=await createAgentSession({cwd:directory,agentDir,sessionManager:manager,modelRuntime,model,thinkingLevel:'off',customTools:[{name:'fixture',label:'fixture',description:'fixture',parameters:{type:'object',properties:{}},async execute(){
                assert.equal(manager.getEntries().filter(e=>e.customType==='notification').length,0);
                calls++;return {content:[{type:'text',text:'tool '+calls}]};
            }}]}));
            await notifications.accept(session,{command_id:'default'});
            assert.equal(manager.getEntries().filter(e=>e.customType==='notification').length,0);
            const running=session.prompt('run both tools');await first;
            await notifications.accept(session,command);await notifications.accept(session,command);
            assert.equal(manager.getEntries().filter(e=>e.customType==='notification').length,0);
            assert.equal(notifications.pending.size,1);release();await running;
            assert.equal(calls,2);assert.equal(requests.length,2);
            assert.ok(!JSON.stringify(requests[0]).includes(command.notification));
            assert.ok(JSON.stringify(requests[1]).includes(command.notification));
            const batch=manager.getEntries().map(messageBody),index=batch.findIndex(m=>m.role==='custom');
            assert.equal(requestKey(batch.slice(0,index+1)),requestKey(requests[1].messages));
            await notifications.drain(session);assert.equal(notifications.pending.size,0);
            await notifications.accept(session,command);
            const entries=manager.getEntries(),notice=entries.find(e=>e.customType==='notification');
            assert.equal(entries.filter(e=>e.customType==='notification').length,1);
            assert.equal(notice.content,command.notification);
            assert.equal(entries.filter(e=>e.type==='message'&&e.message.role==='toolResult').length,2);
            assert.ok(entries.findIndex(e=>e.id===notice.id)>entries.findIndex(e=>e.type==='message'&&e.message.toolCallId==='op1'));
            await notifications.accept(session,{command_id:'idle',notification:'idle result'});
            assert.equal(requests.length,2);
            await notifications.drain(session);
            tree.promote('candidate');tree.settle();tree.settle();
            const reopened=SessionManager.open(directory+'/session.jsonl');
            assert.deepEqual(reopened.getEntry(notice.id),notice);
            assert.equal(messageBody(reopened.getEntry(notice.id)).content,command.notification);
            assert.equal(reopened.getEntries().filter(e=>e.customType==='notification').length,2);
            const persist=manager._persist;manager._persist=()=>{throw Error('injected write failure')};
            await assert.rejects(notifications.accept(session,{command_id:'write-failed',notification:'not yet saved'}),/injected/);
            assert.equal(notifications.pending.size,1);
            assert.ok(!manager.getEntries().some(e=>e.details?.command_id==='write-failed'));
            await assert.rejects(notifications.drain(session),/injected/);
            assert.equal(notifications.pending.size,1);
            manager._persist=persist;await notifications.drain(session);
            assert.equal(notifications.pending.size,0);
        } finally {await session?.abort();session?.dispose();server.closeAllConnections();await new Promise(r=>server.close(r))}
    `,directory],{encoding:'utf8',timeout:30000});
});

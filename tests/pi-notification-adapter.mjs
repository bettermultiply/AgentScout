import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, mkdir, writeFile, readFile, readdir } from 'node:fs/promises';
import { resolve, join } from 'node:path';
import { createServer } from 'node:net';
import { spawn } from 'node:child_process';

const pi=resolve('../pi');
// The real protocol launcher and native SessionManager run unchanged. This SDK
// double models only Pi's triggerTurn:false queue/abort/idle boundary, not an LLM.
// tests/pi-notification.mjs separately checks that boundary against native Pi.
for(const mode of ['cleanup','eof','abort','turn_end','default','write_cleanup','write_eof','idle_write_cleanup','idle_write_eof']) {
test(`notification through main.ts: ${mode}`,{timeout:15000},async t=>{
    const failed=mode.includes('write'),idle=mode==='default'||mode.startsWith('idle_');
    const root=await mkdtemp('/tmp/opencode/notification-adapter-');
    await mkdir(join(root,'packages/coding-agent/src'),{recursive:true});
    await mkdir(join(root,'node_modules/typebox'),{recursive:true});
    await writeFile(join(root,'package.json'),' {"type":"module"}');
    await writeFile(join(root,'node_modules/typebox/index.js'),'exports.Type={Unsafe:x=>x};');
    await writeFile(join(root,'packages/coding-agent/src/index.ts'),`
import {appendFileSync,existsSync} from 'node:fs';
import {SessionManager} from '${pi}/packages/coding-agent/src/core/session-manager.ts';
export {SessionManager};
export const ModelRuntime={},resolveCliModel=()=>({});
export async function createAgentSession(options){
 const manager=options.sessionManager,controller=new AbortController();let subscriber=()=>{},running,queued=[];
 const audit=value=>appendFileSync(${JSON.stringify(join(root,'audit.jsonl'))},JSON.stringify(value)+'\\n');
 const persist=manager._persist.bind(manager);
 manager._persist=entry=>{if(entry.type==='custom_message'&&existsSync(${JSON.stringify(join(root,'fail'))}))throw Error('injected notification write failure');return persist(entry)};
 const append=message=>{manager.appendCustomMessageEntry(message.customType,message.content,message.display,message.details);session.messages=manager.buildSessionContext().messages;subscriber({type:'message_end',message:{role:'custom',...message}})};
 const flush=()=>{const pending=queued;queued=[];for(const message of pending)append(message)};
 const session={agent:{},sessionManager:manager,messages:manager.buildSessionContext().messages,
 subscribe(fn){subscriber=fn},dispose(){audit('dispose')},async waitForIdle(){await running},
 async abort(){audit('abort');controller.abort();await running},
 async sendCustomMessage(message,options){audit({send:message,options});if(running)queued.push(message);else append(message)},
 async prompt(){audit('model');running=(async()=>{
  const done=await options.beforeModelInference({provider:'fixture',id:'fixture'},{messages:session.messages},controller.signal);
  const toolCall={id:'op',name:'runtime_tool'};
  const message={role:'assistant',content:[{type:'toolCall',...toolCall}],stopReason:'toolUse',model:'fixture',usage:{}};
  subscriber({type:'message_start',message});session.messages.push(message);manager.appendMessage(message);subscriber({type:'message_end',message});await done('succeeded');
  await session.agent.beforeToolCall({toolCall,args:{}},controller.signal);
  try {const result=await options.customTools[0].execute('op',{},controller.signal);await session.agent.afterToolCall({toolCall,result,isError:false});
   const message={role:'toolResult',toolCallId:'op',toolName:'runtime_tool',content:result.content};session.messages.push(message);manager.appendMessage(message);subscriber({type:'message_end',message});
  }catch(error){if(!controller.signal.aborted)throw error}
  flush();
 })();try{await running}finally{running=undefined}},
 };return {session};
}
`);
    const messages=[];let client,buffer=Buffer.alloc(0),stderr='';
    const server=createServer(socket=>{client=socket;socket.on('data',chunk=>{
        buffer=Buffer.concat([buffer,chunk]);
        while(buffer.length>=4&&buffer.length>=4+buffer.readUInt32BE(0)){
            const length=buffer.readUInt32BE(0);messages.push(JSON.parse(buffer.subarray(4,4+length)).payload);buffer=buffer.subarray(4+length);
        }
    })});
    await new Promise(r=>server.listen(join(root,'agent.sock'),r));
    const child=spawn(process.execPath,['--no-warnings','--experimental-strip-types','--import',`${pi}/packages/coding-agent/src/experimental/source-resolver.ts`,resolve('agent-access/pi/main.ts'),join(root,'agent.sock'),'agent'],{env:{...process.env,ASCOUT_PI_ROOT:root,ASCOUT_SESSION_FILE:join(root,'session.jsonl'),ASCOUT_RUNTIME_TOOLS:JSON.stringify([{name:'runtime_tool',description:'fixture',parameters:{type:'object'}}])},stdio:['ignore','pipe','pipe']});
    child.stderr.on('data',chunk=>stderr+=chunk);
    const exited=new Promise(r=>child.once('exit',(code,signal)=>r({code,signal})));
    t.after(async()=>{client?.destroy();if(child.exitCode===null){child.kill();await exited}await new Promise(r=>server.close(r))});
    async function wait(predicate){
        const deadline=Date.now()+5000;
        while(Date.now()<deadline){const found=messages.find(predicate);if(found)return found;if(child.exitCode!==null)break;await new Promise(r=>setTimeout(r,5))}
        assert.fail('missing protocol event: '+JSON.stringify(messages)+' '+stderr);
    }
    function send(payload){const body=Buffer.from(JSON.stringify({kind:'command',payload})),header=Buffer.alloc(4);header.writeUInt32BE(body.length);client.write(Buffer.concat([header,body]))}
    const result=(id,status)=>wait(m=>m.message==='command_result'&&m.command_id===id&&m.status===status);
    await wait(m=>m.message==='agent_ready');
    send({command:'start_branch',branch_id:'authority',role:'authority',workspace:root,prompt:idle?'':'run'});
    await wait(m=>m.message==='branch_ready');
    if(!idle)await wait(m=>m.message==='action_started'&&m.action_kind==='tool_execution');
    const command={command:'promote',branch_id:'authority',command_id:'promote-notice',generation:1,...(mode==='default'?{}:{notification:'real authoritative result'})};
    if(failed)await writeFile(join(root,'fail'),'fail');
    send(command);send(command);await result(command.command_id,'applied');
    // A barrier proves the retransmission has completed, not just its first ACK.
    send({command:'resolve_pending',branch_id:'authority',command_id:'barrier',generation:1});await result('barrier','applied');
    assert.equal(messages.filter(m=>m.message==='command_result'&&m.command_id===command.command_id&&m.status==='applied').length,2);
    let audit=(await readFile(join(root,'audit.jsonl'),'utf8').catch(()=>'' )).trim().split('\n').filter(Boolean).map(JSON.parse);
    assert.equal(audit.filter(e=>e==='abort').length,0,'normal Promote must not abort inference');
    assert.equal(audit.filter(e=>e==='model').length,idle?0:1);
    assert.equal(audit.filter(e=>e.send).length,mode==='default'?0:1,'full command retransmission sends once');
    if(mode!=='default')assert.equal(messages.filter(m=>m.message==='transcript'&&m.entry.role==='custom').length,0,'queued is not delivered');
    if(mode==='turn_end'){
        send({command:'resolve_pending',branch_id:'authority',command_id:'finish-tool',operation_id:'op',generation:1,outcome:'real'});await wait(m=>m.message==='turn_completed');
        send(command);send({command:'resolve_pending',branch_id:'authority',command_id:'after-delivery',generation:1});await result('after-delivery','applied');
        send({...command,notification:'changed payload'});send({command:'resolve_pending',branch_id:'authority',command_id:'after-conflict',generation:1});await result('after-conflict','applied');
        assert.ok(messages.some(m=>m.message==='branch_failed'&&m.reason==='Command id reused with different payload'));
    }
    if(mode==='abort'){send({command:'abort',branch_id:'authority'});send({command:'resolve_pending',branch_id:'authority',command_id:'after-abort',generation:1});await result('after-abort','applied')}
    if(!mode.endsWith('eof')){
        send({command:'cleanup',branch_id:'authority',command_id:'cleanup',generation:1});await result('cleanup',failed?'failed':'applied');
        if(failed)assert.ok(!messages.some(m=>['branch_released','agent_session_released'].includes(m.message)));
    }
    client.end();const exit=await exited;
    assert.deepEqual(exit,{code:failed?1:0,signal:null},stderr);
    const records=(await readFile(join(root,'session.jsonl'),'utf8')).trim().split('\n').map(JSON.parse);
    const notices=records.filter(e=>e.customType==='notification');
    assert.equal(notices.length,mode==='default'||failed?0:1);
    if(notices.length){assert.equal(notices[0].details.command_id,command.command_id);assert.equal(notices[0].content,command.notification);assert.equal(notices[0].display,false)}
    const transcripts=messages.filter(m=>m.message==='transcript'&&m.entry.role==='custom');
    assert.equal(transcripts.length,notices.length);
    if(notices.length)assert.equal(transcripts[0].session_entry_id,notices[0].id);
    const sources=await readdir(join(root,'sessions'));
    assert.equal(sources.filter(f=>f.endsWith('.jsonl')).length,failed?1:0);
    if(failed){
        assert.match(stderr,/injected notification write failure/);
        assert.ok(messages.some(m=>m.message==='agent_failed'));
        assert.ok(!messages.some(m=>m.message==='agent_released'));
        if(mode.startsWith('idle_'))assert.ok(messages.some(m=>m.message==='adapter_capability_gap'&&m.capability==='notification'&&m.status==='failed'));
    }
    audit=(await readFile(join(root,'audit.jsonl'),'utf8')).trim().split('\n').filter(Boolean).map(JSON.parse);
    assert.equal(audit.filter(e=>e==='model').length,idle?0:1,'delivery and shutdown do not request another model turn');
    for(const event of audit.filter(e=>e.send))assert.deepEqual(event.options,{triggerTurn:false});
});
}

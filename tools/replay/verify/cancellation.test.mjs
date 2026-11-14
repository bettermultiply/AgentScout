import assert from 'node:assert/strict';
import { test } from 'node:test';
import http from 'node:http';
import fs from 'node:fs';
import { setTimeout as delay } from 'node:timers/promises';
import { loadRecording } from '../recording.mjs';
import { createReplayServer } from '../serve.mjs';
import { requestKey } from '../normalize.mjs';

const messages = [{role:'user',content:'start'}], key = requestKey(messages);
const success = {key,reply:[{type:'text',text:'done'}]};
const aborted = {key,stopReason:'aborted'};
async function start(t, recording, options = {}) {
  const failures = [];
  const replay = createReplayServer(recording,{...options,onFailure:message=>failures.push(message)});
  await new Promise(resolve=>replay.server.listen(0,'127.0.0.1',resolve));
  t.after(async()=>{replay.server.closeAllConnections(); await new Promise(resolve=>replay.server.close(resolve));});
  function request(input = messages) {
    let answered = false;
    const req = http.request({host:'127.0.0.1',port:replay.server.address().port,path:'/v1/chat/completions',method:'POST',agent:false});
    const result = new Promise(resolve=>{
      req.on('response',res=>{answered=true;let body='';res.on('data',c=>body+=c);res.on('end',()=>resolve({status:res.statusCode,body}));});
      req.on('error',error=>resolve({error}));
    });
    req.end(JSON.stringify({messages:input}));
    return {req,result,get answered(){return answered;}};
  }
  return {...replay,failures,request};
}
async function until(check) {
  for(let i=0;i<100;i++){if(check())return;await delay(10);}
  assert.ok(check(),'condition did not settle');
}

test('UTF-8 request characters split across HTTP chunks retain their exact key',async t=>{
  const input=[{role:'user',content:'公开 observation（chars）'}];
  const r=await start(t,[{...success,key:requestKey(input)}]);
  const bytes=Buffer.from(JSON.stringify({messages:input})), split=bytes.indexOf(Buffer.from('（'))+1;
  const req=http.request({host:'127.0.0.1',port:r.server.address().port,path:'/v1/chat/completions',method:'POST',agent:false});
  const result=new Promise((resolve,reject)=>{req.on('error',reject);req.on('response',res=>{res.resume();res.on('end',()=>resolve(res.statusCode));});});
  req.write(bytes.subarray(0,split));await delay(30);req.end(bytes.subarray(split));
  assert.equal(await result,200);
  assert.equal(r.report().misses,0);
});

test('native aborted entries retain source order and context; errors are not cancellations',t=>{
  const root=fs.mkdtempSync('/tmp/opencode/replay-cancel-test-');
  t.after(()=>fs.rmSync(root,{recursive:true,force:true}));
  const entries=[{type:'message',id:'u',parentId:null,message:messages[0]},
    ...['stop','aborted','stop','error'].map((stopReason,i)=>({type:'message',id:`a${i}`,parentId:'u',message:{role:'assistant',stopReason,content:[{type:'text',text:'done'}],...(stopReason==='aborted'?{errorMessage:'aborted'}:{})}}))];
  fs.writeFileSync(`${root}/session.jsonl`,entries.map(JSON.stringify).join('\n'));
  fs.writeFileSync(`${root}/events.jsonl`,entries.map(e=>JSON.stringify({message:'transcript',session_entry_id:e.id,branch_id:'root'})).join('\n'));
  const loaded=loadRecording(root);
  assert.deepEqual(loaded.recording.map(r=>r.session_entry_id),['a0','a1','a2']);
  assert.deepEqual(loaded.recording.map(r=>r.key),[key,key,key]);
  assert.equal(loaded.recording[1].stopReason,'aborted');
  assert.equal(loaded.recording[1].reply,undefined);
  assert.equal(loaded.skippedError,1);
});

test('aborted match waits for real HTTP cancellation, then same-key success and exhaustion',async t=>{
  const r=await start(t,[aborted,success],{cancelTimeoutMs:300});
  const pending=r.request();
  await until(()=>r.report().hits===1);
  await delay(60);
  assert.equal(pending.answered,false,'cancelled source must not answer');
  pending.req.destroy();await pending.result;
  await until(()=>r.report().cancelled===1);
  await delay(320);
  assert.deepEqual(r.failures,[],'cancel timer must be cleared');
  const next=await r.request().result;
  assert.equal(next.status,200);assert.match(next.body,/done/);
  assert.equal((await r.request().result).status,400);
  assert.equal((await r.request([{role:'user',content:'missing'}]).result).status,400);
  assert.equal(r.report().exhausted,1);assert.equal(r.report().misses,1);
});

test('uncancelled aborted match has bounded explicit failure',async t=>{
  const r=await start(t,[aborted],{cancelTimeoutMs:100});
  const response=await r.request().result;
  assert.equal(response.status,504);assert.match(response.body,/cancel.*timed out/i);
  assert.equal(r.failures.length,1);
  assert.equal(r.report().cancel_timeouts,1);
  assert.equal(r.report().misses,0);
});

test('successful response waits once for 500ms; cancellation during delay clears work',async t=>{
  const r=await start(t,[success,success]);
  const began=performance.now(), pending=r.request();
  await until(()=>r.report().hits===1);
  await delay(150);assert.equal(pending.answered,false);
  const response=await pending.result;
  assert.ok(performance.now()-began>=500);assert.equal(response.status,200);
  const cancelled=r.request();await until(()=>r.report().hits===2);
  await delay(50);assert.equal(cancelled.answered,false);
  cancelled.req.destroy();await cancelled.result;
  await until(()=>r.report().cancelled===1);
  await delay(550);assert.deepEqual(r.failures,[]);
  assert.equal(r.report().pending,0);
});

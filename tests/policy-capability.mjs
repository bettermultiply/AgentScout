import {test} from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {execFileSync} from 'node:child_process';
import {evaluatePolicyArtifact,disposePolicyWorker} from '../../pi/packages/pi-integration/src/local-policy.ts';
const runner=readFileSync(new URL('../src/control_plane/policy/runner.py',import.meta.url),'utf8');
const python=execFileSync('/usr/bin/python3',['-c','import sys;print(sys.version,end="")'],{encoding:'utf8'});
function artifact(source){const manifest={python,files:{'policy.py':source},binding:{file:'policy.py',function:'run',policy_config:{}}};return {digest:'capability',decisions:{effect:`import os,json\nos.dup2(1,3)\nos.dup2(2,1)\nMANIFEST=${JSON.stringify(manifest) .replace(/:null/g,':None')}\n${runner}`}};}
const calls='def run(data, *, policy_config, capabilities):\n return capabilities.model_complete({"provider":"fake","model":"small","prompt":"Reply OK","max_tokens":4})';
test('actual runner performs authorized capability roundtrip and reuses process',async()=>{
 const a=artifact(calls);let count=0;
 const grant={decision:'effect',models:['fake/small'],requests:1,complete:async(r,signal)=>{assert.equal(r.prompt,'Reply OK');assert.equal(signal.aborted,false);count++;return {text:'OK'};}};
 try{for(let i=0;i<2;i++)assert.deepEqual(await evaluatePolicyArtifact(a,'effect',{},undefined,grant),{text:'OK'});assert.equal(count,2);}finally{await disposePolicyWorker();}
});
test('default deny, Decision mismatch, allowlist and quota reject before provider',async()=>{
 const a=artifact(calls);const grant={decision:'effect',models:['fake/small'],requests:1,complete:async()=>{throw Error('must not call')}};
 for(const g of [undefined,{...grant,decision:'extent'},{...grant,models:[]},{...grant,requests:0}])await assert.rejects(evaluatePolicyArtifact(a,'effect',{},undefined,g),/denied/);
});
test('stale request IDs and second calls beyond grant quota are rejected',async()=>{
 const forged='import sys,json\nfor line in sys.stdin.buffer:\n payload=json.dumps({"type":"result","request_id":"stale","result":True}).encode()\n sys.stdout.buffer.write(len(payload).to_bytes(4,"big")+payload)\n sys.stdout.buffer.flush()\n';
 await assert.rejects(evaluatePolicyArtifact({digest:'stale',decisions:{effect:forged}},'effect',{}),/request ID/);
 let calls=0;
 const twice=artifact('def run(data, *, policy_config, capabilities):\n request={"provider":"fake","model":"small","prompt":"OK","max_tokens":4}\n capabilities.model_complete(request)\n return capabilities.model_complete(request)');
 await assert.rejects(evaluatePolicyArtifact(twice,'effect',{},undefined,{decision:'effect',models:['fake/small'],requests:1,complete:async()=>{calls++;return {text:'OK'};}}),/denied/);
 assert.equal(calls,1);
});

test('cancellation aborts host provider and worker is rebuilt',async()=>{
 const controller=new AbortController();let aborted=false;
 const grant={decision:'effect',models:['fake/small'],requests:1,complete:async(_r,signal)=>new Promise(resolve=>{signal.addEventListener('abort',()=>{aborted=true;resolve({text:'late'});});controller.abort();})};
 await assert.rejects(evaluatePolicyArtifact(artifact(calls),'effect',{},controller.signal,grant),/cancelled/);
 assert.equal(aborted,true);
 assert.equal(await evaluatePolicyArtifact(artifact('def run(data, *, policy_config, capabilities):\n return True'),'effect',{}),true);
 await disposePolicyWorker();
});

test('actual runner reuses supervisor PID, isolates globals and denies host file/network/process access',async()=>{
 const source=`import os, socket, subprocess
counter=0
def run(data, *, policy_config, capabilities):
 global counter
 counter+=1
 denied=[]
 for operation in (lambda: open('/etc/passwd').read(), lambda: socket.socket(), lambda: subprocess.run(['/usr/bin/python3','-V'])):
  try:
   operation()
   denied.append(False)
  except OSError:
   denied.append(True)
 return {"pid": os.getpid(), "supervisor":os.getppid(), "counter":counter, "denied":denied}
`;
 const a=artifact(source);
 try{
 const first=await evaluatePolicyArtifact(a,'effect',{});
 const second=await evaluatePolicyArtifact(a,'effect',{});
 assert.equal(first.supervisor,second.supervisor);
 assert.notEqual(first.pid,second.pid);
 assert.equal(first.counter,1);assert.equal(second.counter,1);
 assert.deepEqual(first.denied,[true,true,true]);
 }finally{await disposePolicyWorker();}
});

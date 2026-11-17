import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { evaluatePolicyArtifact, disposePolicyWorker } from '../../pi/packages/pi-integration/src/local-policy.ts';

test('next grant cannot inherit mutated standard library or writable mount data', async () => {
 const python=execFileSync('/usr/bin/python3',['-c','import sys;print(sys.version,end="")'],{encoding:'utf8'});
 const source=`import os
import json
import builtins
def run(data, *, policy_config, capabilities):
 if data.get('premature'):
  payload=json.dumps({'type':'result','request_id':capabilities.request_id,'result':{'forged':True}}).encode()
  os.write(3,len(payload).to_bytes(4,'big')+payload)
  os.read(0,1024)
 clean = not hasattr(builtins, 'previous_grant') and not os.path.exists('/tmp/previous-grant') and not os.path.exists('/dev/shm/previous-grant')
 builtins.previous_grant = capabilities
 for path in ('/tmp/previous-grant', '/dev/shm/previous-grant'):
  open(path, 'w').write('secret')
 return {'clean':clean, 'supervisor':os.getppid()}
`;
 const manifest={python,files:{'policy.py':source},binding:{file:'policy.py',function:'run',policy_config:{}}};
 const artifact={digest:'isolation',decisions:{effect:`import os,json\nos.dup2(1,3)\nos.dup2(2,1)\nMANIFEST=${JSON.stringify(manifest)}\n${readFileSync(new URL('../src/control_plane/policy/runner.py',import.meta.url),'utf8')}`}};
 try {
  const first=await evaluatePolicyArtifact(artifact,'effect',{});
  const second=await evaluatePolicyArtifact(artifact,'effect',{});
  assert.equal(first.clean,true);
  assert.equal(second.clean,true);
  assert.equal(first.supervisor,second.supervisor);
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), 150);
  try {
   await assert.rejects(evaluatePolicyArtifact(artifact,'effect',{premature:true},controller.signal), /cancelled/);
  } finally { clearTimeout(timer); }
  assert.equal((await evaluatePolicyArtifact(artifact,'effect',{})).clean,true);
 } finally { await disposePolicyWorker(); }
});

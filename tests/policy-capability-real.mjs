// Explicit one-request smoke test. Never run as part of the automatic test suite.
import {readFileSync} from 'node:fs';
import {execFileSync} from 'node:child_process';
import {createModels} from '../../pi/packages/ai/src/models.ts';
import {deepseekProvider} from '../../pi/packages/ai/src/providers/deepseek.ts';
import {ReadOnlyAuthStorage} from '../../pi/packages/coding-agent/src/core/auth-storage.ts';
import {evaluatePolicyArtifact,disposePolicyWorker,piModelComplete} from '../../pi/packages/pi-integration/src/local-policy.ts';
const models=createModels({credentials:new ReadOnlyAuthStorage()});
models.setProvider(deepseekProvider());
const python=execFileSync('/usr/bin/python3',['-c','import sys;print(sys.version,end="")'],{encoding:'utf8'});
const manifest={python,files:{'policy.py':'def run(data, *, policy_config, capabilities):\n response = capabilities.model_complete({"provider":"deepseek","model":"deepseek-flash","prompt":"Reply with only OK.","max_tokens":16})\n return {"ok": isinstance(response.get("text"), str) and len(response["text"]) > 0}\n'},binding:{file:'policy.py',function:'run',policy_config:{}}};
const runner=readFileSync(new URL('../src/control_plane/policy/runner.py',import.meta.url),'utf8');
const script=`import os,json\nos.dup2(1,3)\nos.dup2(2,1)\nMANIFEST=json.loads(${JSON.stringify(JSON.stringify(manifest))})\n${runner}`;
let attempts=0;
try{
 const complete=piModelComplete(models);
 const result=await evaluatePolicyArtifact({digest:'single-request-smoke',decisions:{effect:script}},'effect',{},undefined,{decision:'effect',models:['deepseek/deepseek-flash'],requests:1,complete:async(r,s)=>{attempts++;return complete(r,s);}});
 if(result?.ok!==true)throw Error('Policy result validation failed');
 console.log(JSON.stringify({provider:'deepseek',model:'deepseek-flash',provider_invocations:attempts,policy_result:result}));
}catch{console.log(JSON.stringify({provider:'deepseek',model:'deepseek-flash',provider_invocations:attempts,status:'failed; no retry'}));process.exitCode=1;}
finally{await disposePolicyWorker();}

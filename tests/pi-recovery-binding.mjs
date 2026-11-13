import {test} from 'node:test';
import {execFileSync} from 'node:child_process';
import {mkdtempSync} from 'node:fs';
import {resolve} from 'node:path';

test('recovery binds auto-loaded resources and external rollback Git identity without resetting workspace',()=>{
 const directory=mkdtempSync('/tmp/opencode/recovery-binding-'),pi=resolve('../pi');
 execFileSync(process.execPath,['--experimental-strip-types','--input-type=module','-e',`
 import assert from 'node:assert/strict';
 import {mkdirSync,writeFileSync} from 'node:fs';
 import {execFileSync} from 'node:child_process';
 import {recoveryBinding,snapshotResources} from '${resolve('agent-access/pi/recovery-binding.ts')}';
 const root=process.argv[1],pi=process.argv[2];process.env.PI_CODING_AGENT_DIR=root+'/agent';
 mkdirSync(root+'/agent/extensions',{recursive:true});mkdirSync(root+'/workspace');
 writeFileSync(root+'/agent/extensions/example.ts','export default ()=>{}');
 const first=recoveryBinding(pi,[root+'/workspace']);
 assert.deepEqual(first,recoveryBinding(pi,[root+'/workspace']));
 writeFileSync(root+'/agent/extensions/example.ts','export default ()=>{return 1}');
 assert.notDeepEqual(first,recoveryBinding(pi,[root+'/workspace']));
 writeFileSync(root+'/agent/settings.json',JSON.stringify({packages:['unsupported']}));
 assert.throws(()=>recoveryBinding(pi,[root+'/workspace']),/package-managed/);
 const git=(...args)=>execFileSync('git',['-C',root+'/workspace',...args],{encoding:'utf8'}).trim();
 git('init','--quiet');writeFileSync(root+'/workspace/value','saved');git('add','value');
 const tree=git('write-tree');
 const commit=execFileSync('git',['-C',root+'/workspace','commit-tree',tree,'-m','snapshot'],{encoding:'utf8',env:{...process.env,GIT_AUTHOR_NAME:'fixture',GIT_AUTHOR_EMAIL:'fixture@example.invalid',GIT_COMMITTER_NAME:'fixture',GIT_COMMITTER_EMAIL:'fixture@example.invalid'}}).trim();
 const ref='refs/pi/rollback/fixture';git('update-ref',ref,commit);
 const sources=[{entries:[{type:'custom',customType:'rollback-snapshot',data:{repoRoot:root+'/workspace',ref,tree}}]}];
 const binding=snapshotResources(sources);assert.deepEqual(snapshotResources(sources,binding),binding);
 writeFileSync(root+'/workspace/value','continued');assert.deepEqual(snapshotResources(sources,binding),binding);
 git('update-ref','-d',ref);assert.throws(()=>snapshotResources(sources,binding));
 `,directory,pi],{encoding:'utf8'});
});

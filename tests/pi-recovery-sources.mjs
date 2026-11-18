import assert from 'node:assert/strict';
import {mkdtempSync} from 'node:fs';
import {execFileSync} from 'node:child_process';
import {resolve} from 'node:path';
import {test} from 'node:test';

test('native recovery sources remain independently restorable after original and restored execution',()=>{
    const directory=mkdtempSync('/tmp/opencode/pi-recovery-sources-'),pi=resolve('../pi');
    const script=`
        import assert from 'node:assert/strict';
        import {mkdirSync,readFileSync,unlinkSync} from 'node:fs';
        import {SessionManager} from '${pi}/packages/coding-agent/src/core/session-manager.ts';
        import {SessionTree} from '${resolve('instrumentation/pi/session-tree.ts')}';
        const directory=process.argv[1],main=SessionManager.open(directory+'/session.jsonl');
        main.appendMessage({role:'user',content:'before save',timestamp:1});
        const tree=new SessionTree(main),branch=tree.createBranch('authority',directory);
        branch.appendMessage({role:'assistant',content:[{type:'text',text:'partial'}],stopReason:'aborted',timestamp:2});
        tree.settle();mkdirSync(directory+'/generation');
        const snapshot=tree.snapshot(directory+'/generation');
        const saved=snapshot.sources.map(s=>readFileSync(s.file,'utf8'));
        branch.appendMessage({role:'user',content:'after failed publish',timestamp:3});tree.settle();
        const restored=SessionTree.restore(snapshot,SessionManager,directory+'/restored.jsonl');
        assert.equal(restored.context('authority').messages.at(-1).content[0].text,'partial');
        const active=restored.branches.get('authority').manager;
        active.appendMessage({role:'user',content:'continue restored',timestamp:4});restored.settle();
        assert.deepEqual(snapshot.sources.map(s=>readFileSync(s.file,'utf8')),saved);
        const again=SessionTree.restore(snapshot,SessionManager,directory+'/again.jsonl');
        assert.equal(again.context('authority').messages.at(-1).content[0].text,'partial');
        assert.notEqual(active.getSessionFile(),again.branches.get('authority').manager.getSessionFile());
        const invalid=structuredClone(snapshot);invalid.tails=[['authority','missing-entry']];
        assert.throws(()=>SessionTree.restore(invalid,SessionManager,directory+'/invalid.jsonl'),/tail missing/);
        const duplicate=structuredClone(snapshot);duplicate.sources.push(duplicate.sources[0]);
        assert.throws(()=>SessionTree.restore(duplicate,SessionManager,directory+'/duplicate.jsonl'),/Duplicate recovery Pi source/);
        unlinkSync(snapshot.sources[0].file);
        assert.throws(()=>SessionTree.restore(snapshot,SessionManager,directory+'/missing.jsonl'),/ENOENT/);
    `;
    execFileSync(process.execPath,['--no-warnings','--experimental-strip-types','--import',`${pi}/packages/coding-agent/src/experimental/source-resolver.ts`,'--input-type=module','-e',script,directory],{encoding:'utf8'});
});

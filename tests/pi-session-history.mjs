import assert from 'node:assert/strict';
import { mkdtempSync, existsSync, readFileSync, copyFileSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { resolve } from 'node:path';
import { test } from 'node:test';

test('native Pi history preserves compaction/custom/IDs, rewires only final history, retains live sources', () => {
    const directory=mkdtempSync('/tmp/opencode/pi-history-');
    const pi=resolve('../pi');
    const script=`
        import assert from 'node:assert/strict';
        import {existsSync} from 'node:fs';
        import {SessionManager} from '${pi}/packages/coding-agent/src/core/session-manager.ts';
        import {SessionTree} from '${resolve('instrumentation/pi/session-tree.ts')}';
        const directory=process.argv[1],file=directory+'/session.jsonl';
        const main=SessionManager.open(file),user=main.appendMessage({role:'user',content:'task',timestamp:1});
        const custom=main.appendCustomEntry('fixture',{targetId:user});
        const compact=main.appendCompaction('Earlier task summary',user,10);
        const assistant=main.appendMessage({role:'assistant',content:[{type:'toolCall',id:'op',name:'read',arguments:{}}],stopReason:'toolUse',timestamp:2});
        const tree=new SessionTree(main),parent=tree.createBranch('authority',directory,main,assistant);
        const candidate=tree.createBranch('candidate',directory,parent,assistant);
        for(const id of [user,custom,compact,assistant])assert.deepEqual(candidate.getEntry(id),parent.getEntry(id));
        const prediction=tree.append('candidate',{role:'toolResult',toolCallId:'op',toolName:'read',content:[{type:'text',text:'guess'}],timestamp:3});
        const next=tree.append('candidate',{role:'assistant',content:[{type:'text',text:'continue'}],stopReason:'stop',timestamp:4});
        const real=parent.appendMessage({role:'toolResult',toolCallId:'op',toolName:'read',content:[{type:'text',text:'real'}],timestamp:5});
        tree.inputs.set('candidate',{parent:'authority',operation:'op',prediction,pairedTail:prediction,operations:['op']});
        tree.accepted=new Set(['candidate']);tree.promote('candidate');tree.settle();
        let reopened=SessionManager.open(file);
        assert.equal(reopened.getEntry(next).parentId,real);
        assert.equal(candidate.getEntry(next).parentId,prediction);
        assert.ok(reopened.getEntry(prediction));assert.ok(reopened.getEntry(custom));assert.ok(reopened.getEntry(compact));
        assert.equal(reopened.buildSessionContext().messages.find(m=>m.role==='toolResult').content[0].text,'real');
        assert.ok(existsSync(candidate.getSessionFile()));
        tree.stop('authority');tree.settle();assert.ok(!existsSync(parent.getSessionFile()));assert.ok(existsSync(candidate.getSessionFile()));
        candidate.appendCustomEntry('fixture-final',{targetId:next});tree.settle();
        reopened=SessionManager.open(file);assert.equal(reopened.getEntries().at(-1).parentId,candidate.getLeafId());
        const bad=candidate.getEntry(next);bad.parentId='missing-parent';
        assert.throws(()=>tree.settle());assert.ok(existsSync(candidate.getSessionFile()));
        console.log(directory);
    `;
    execFileSync(process.execPath,['--no-warnings','--experimental-strip-types','--import',`${pi}/packages/coding-agent/src/experimental/source-resolver.ts`,'--input-type=module','-e',script,directory],{encoding:'utf8'});
    const entries=readFileSync(`${directory}/session.jsonl`,'utf8').trim().split('\n').map(JSON.parse);
    assert.ok(entries.some(e=>e.type==='compaction'));
    assert.ok(existsSync(`${directory}/session.jsonl`));
});

test('installed pi-code-rollback snapshot references survive assembly with their Git objects', () => {
    const directory=mkdtempSync('/tmp/opencode/pi-rollback-');
    copyFileSync('/home/betmul/.pi/agent/npm/node_modules/pi-code-rollback/index.ts',`${directory}/rollback.ts`);
    const pi=resolve('../pi');
    const script=`
        import assert from 'node:assert/strict';
        import {writeFileSync,readFileSync,existsSync} from 'node:fs';
        import {execFileSync} from 'node:child_process';
        import {SessionManager} from '${pi}/packages/coding-agent/src/core/session-manager.ts';
        import {SessionTree} from '${resolve('instrumentation/pi/session-tree.ts')}';
        import rollback,{__testing__} from '${directory}/rollback.ts';
        const directory=process.argv[1];
        execFileSync('git',['init','--quiet',directory]);
        writeFileSync(directory+'/task.txt','baseline');
        const main=SessionManager.open(directory+'/session.jsonl');
        const target=main.appendMessage({role:'user',content:'work',timestamp:1});
        const tree=new SessionTree(main),candidate=tree.createBranch('candidate',directory,main,target);
        const hooks=new Map(),commands=new Map(),warnings=[];
        rollback({on:(event,fn)=>hooks.set(event,fn),registerCommand:(name,command)=>commands.set(name,command),appendEntry:(type,data)=>candidate.appendCustomEntry(type,data)});
        const context={cwd:directory,sessionManager:candidate,hasUI:true,ui:{notify:message=>warnings.push(message)}};
        await hooks.get('agent_start')({},context);
        assert.deepEqual(warnings,[]);
        const snapshot=candidate.getEntries().find(e=>e.customType==='rollback-snapshot');
        assert.ok(snapshot);assert.equal(snapshot.data.targetId,target);assert.equal(snapshot.data.repoRoot,directory);
        const git=(...args)=>execFileSync('git',['-C',directory,...args],{encoding:'utf8'}).trim();
        assert.equal(git('rev-parse',snapshot.data.ref+'^{tree}'),snapshot.data.tree);
        tree.promote('candidate');tree.settle();
        const reopened=SessionManager.open(directory+'/session.jsonl');
        assert.deepEqual(reopened.getEntry(snapshot.id),JSON.parse(JSON.stringify(snapshot)));
        assert.ok(reopened.getBranch().some(e=>e.id===snapshot.data.targetId));
        assert.equal(git('show',snapshot.data.ref+':task.txt'),'baseline');
        // Re-open the extension against the standalone final Pi session, and
        // run its startup reference GC before exercising actual restoration.
        const after={...context,sessionManager:reopened};
        await hooks.get('session_start')({},after);
        assert.equal(git('rev-parse',snapshot.data.ref+'^{tree}'),snapshot.data.tree);
        assert.deepEqual(warnings,[]);
        // Restore only this disposable repository, never the user repository.
        writeFileSync(directory+'/task.txt','changed after snapshot');
        writeFileSync(directory+'/new.txt','new file');
        await __testing__.restoreSnapshot(directory,reopened.getEntry(snapshot.id).data.ref);
        assert.equal(readFileSync(directory+'/task.txt','utf8'),'baseline');
        assert.equal(existsSync(directory+'/new.txt'),false);
    `;
    execFileSync(process.execPath,['--no-warnings','--experimental-strip-types','--import',`${pi}/packages/coding-agent/src/experimental/source-resolver.ts`,'--input-type=module','-e',script,directory],{encoding:'utf8'});
});

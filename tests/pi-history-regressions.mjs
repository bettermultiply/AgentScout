import { test } from 'node:test';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, rmSync } from 'node:fs';
import { join, resolve } from 'node:path';

const pi=resolve('../pi');
function check(script) {
    const directory=mkdtempSync('/var/tmp/pi-history-review-');
    try {
        execFileSync(process.execPath,['--no-warnings','--experimental-strip-types','--import',`${pi}/packages/coding-agent/src/experimental/source-resolver.ts`,'--input-type=module','-e',`
        import assert from 'node:assert/strict';
        import {SessionManager} from '${pi}/packages/coding-agent/src/core/session-manager.ts';
        import {SessionTree} from '${resolve('instrumentation/pi/session-tree.ts')}';
        const directory=process.argv[1], main=SessionManager.open(directory+'/session.jsonl');
        const call=main.appendMessage({role:'assistant',content:[{type:'toolCall',id:'op',name:'read',arguments:{}}],stopReason:'toolUse',timestamp:1});
        const tree=new SessionTree(main),parent=tree.createBranch('authority',directory,main,call);
        const result=(text,id='op')=>({role:'toolResult',toolCallId:id,toolName:'read',content:[{type:'text',text}],timestamp:2});
        ${script}
        `,directory],{encoding:'utf8',env:{...process.env,PI_CODING_AGENT_DIR:join(directory,'agent'),JITI_FS_CACHE:'false'}});
    } finally {
        rmSync(directory, {recursive:true, force:true});
    }
}

test('real result is on source checkpoint path, never a later off-path prediction with reused call ID',()=>check(`
    const real=parent.appendMessage(result('real'));
    parent.branch(call);const wrong=parent.appendMessage(result('off-path guess'));parent.branch(real);
    const candidate=tree.createBranch('candidate',directory,parent,call);
    const prediction=tree.append('candidate',result('guess'));
    const next=tree.append('candidate',{role:'assistant',content:[{type:'text',text:'continue'}],stopReason:'stop'});
    tree.inputs.set('candidate',{source:parent,checkpoint:call,pairedTail:prediction,operations:['op']});
    tree.accepted=new Set(['candidate']);tree.promote('candidate');tree.settle();
    assert.equal(tree.manager.getEntry(next).parentId,real);
    assert.equal(candidate.getEntry(next).parentId,prediction);
`));

test('rewiring selects candidate continuation rather than first indexed child of paired input',()=>check(`
    const candidate=tree.createBranch('candidate',directory,parent,call);
    const prediction=tree.append('candidate',result('guess'));
    const sibling=candidate.appendCustomEntry('off-path',{targetId:prediction});candidate.branch(prediction);
    const next=candidate.appendCustomEntry('continuation',{targetId:prediction});
    const real=parent.appendMessage(result('real'));
    tree.inputs.set('candidate',{source:parent,checkpoint:call,pairedTail:prediction,operations:['op']});
    tree.accepted=new Set(['candidate']);tree.promote('candidate');tree.settle();
    assert.equal(tree.manager.getEntry(next).parentId,real);
    assert.equal(tree.manager.getEntry(sibling).parentId,prediction);
`));

test('new branch opens independently at checkpoint even when copied index ends on another path',()=>check(`
    const off=parent.appendCustomEntry('off-path',{});
    const candidate=tree.createBranch('candidate',directory,parent,call);
    const reopened=SessionManager.open(candidate.getSessionFile());
    assert.deepEqual(reopened.buildSessionContext(),candidate.buildSessionContext());
    assert.ok(!reopened.getBranch().some(e=>e.id===off));
`));

test('parallel result set is bounded by checkpoint and stops before a later call with reused IDs',()=>check(`
    const candidate=tree.createBranch('candidate',directory,parent,call);
    tree.append('candidate',result('guess op'));
    const pairedTail=tree.append('candidate',result('sibling real','other'));
    const next=candidate.appendCustomEntry('after-parallel',{});
    parent.appendMessage(result('real op'));
    const realTail=parent.appendMessage(result('real other','other'));
    parent.appendMessage({role:'assistant',content:[{type:'toolCall',id:'op',name:'read',arguments:{}}],stopReason:'toolUse'});
    parent.appendMessage(result('later op'));
    tree.inputs.set('candidate',{source:parent,checkpoint:call,pairedTail,operations:['op','other']});
    tree.accepted=new Set(['candidate']);tree.promote('candidate');tree.settle();
    assert.equal(tree.manager.getEntry(next).parentId,realTail);
    tree.stop('authority');tree.settle();candidate.appendCustomEntry('later',{});tree.settle();
    assert.equal(tree.manager.getEntry(next).parentId,realTail);
`));

test('nested accepted paths rewire each exact checkpoint, retaining source managers and original checkpoints',()=>check(`
    const candidate=tree.createBranch('candidate',directory,parent,call);
    const prediction=tree.append('candidate',result('guess'));
    const childCall=tree.append('candidate',{role:'assistant',content:[{type:'toolCall',id:'nested',name:'read',arguments:{}}],stopReason:'toolUse'});
    const child=tree.createBranch('child',directory,candidate,childCall);
    const childPrediction=tree.append('child',result('nested guess','nested'));
    const next=child.appendCustomEntry('nested-continuation',{});
    const real=parent.appendMessage(result('real'));
    const nestedReal=candidate.appendMessage(result('nested real','nested'));
    tree.inputs.set('candidate',{source:parent,checkpoint:call,pairedTail:prediction,operations:['op']});
    tree.inputs.set('child',{source:candidate,checkpoint:childCall,pairedTail:childPrediction,operations:['nested']});
    tree.accepted=new Set(['candidate','child']);tree.promote('child');tree.settle();
    assert.equal(tree.manager.getEntry(childCall).parentId,real);
    assert.equal(tree.manager.getEntry(next).parentId,nestedReal);
    assert.equal(candidate.getEntry(childCall).parentId,prediction);
    assert.deepEqual(candidate.getEntry(call),parent.getEntry(call));
    tree.stop('authority');tree.stop('candidate');tree.settle();tree.settle();
    assert.equal(tree.manager.getEntry(childCall).parentId,real);
    assert.equal(tree.manager.getEntry(next).parentId,nestedReal);
`));

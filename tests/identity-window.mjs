import {test} from 'node:test';
import assert from 'node:assert/strict';
import {IdentityWindow,positiveSequence} from '../../pi/packages/pi-integration/src/identity-window.ts';

test('identity retirement follows contiguous acceptance, not arrival high watermark',()=>{
    const window=new IdentityWindow(positiveSequence);
    window.accept('2','second');
    window.accept('1','first');
    let executions=2;
    for(let id=3;id<=2000;id++){window.accept(String(id),String(id));executions++}
    const before=executions;
    assert.throws(()=>{window.accept('1','changed');executions++},/expired/);
    assert.equal(executions,before);
    assert.equal(window.get('1'),undefined);
    assert.equal(window.get('2000'),'2000');
});

test('unsequenced identities fail closed at a finite connection capacity',()=>{
    const window=new IdentityWindow(()=>undefined);
    for(let id=0;id<1024;id++)window.accept(String(id),id);
    assert.throws(()=>window.accept('new',0),/window full/);
    assert.equal(window.get('0'),0);
});

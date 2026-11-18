// Invoked only by the exact no-model Firecracker test. Independent Pi adapters,
// AscProxy, receiver, Runtime and guest; no SDK tool implementations.
import assert from 'node:assert/strict';
import {mkdtempSync,symlinkSync,rmSync} from 'node:fs';
import {createRequire} from 'node:module';
import {AscProxy} from '../../instrumentation/pi/asc-proxy.ts';
import {environmentTools} from '../../instrumentation/pi/tools.ts';
const require=createRequire(import.meta.url);
const {detectSupportedImageMimeType}=require('../../../pi/packages/coding-agent/src/utils/mime.ts');
const proxy=new AscProxy();
const workspace='/root',relative=`ascout-pi-vm-${process.pid}`,cwd=`${workspace}/${relative}`;
assert.notEqual(process.cwd(),workspace);
proxy.bind('pi-tools',JSON.parse(process.env.ASCOUT_TEST_ACCESS));
const tools=Object.fromEntries(environmentTools(proxy,'pi-tools',detectSupportedImageMimeType,'no-model').map(t=>[t.name,t]));
let id=0;
const run=(name,args,signal,update)=>tools[name].execute(`call-${++id}`,args,signal,update);
const text=result=>result.content.filter(c=>c.type==='text').map(c=>c.text).join('\n');
const generic=(operation,args)=>proxy.operation('pi-tools',`generic-${++id}`,operation,args,{});
const nested=`${relative}/pi-boundary/nested/file`;
assert.match(text(await run('write',{path:nested,content:'alpha\nbeta\n'})),/Successfully wrote/);
assert.equal(Buffer.from((await generic('read_file',{path:`${workspace}/${nested}`})).value.bytes).toString(),'alpha\nbeta\n');
assert.equal(text(await run('bash',{command:'pwd'})),`${workspace}\n`);
// The host alias intentionally targets a different file than the guest alias.
const host=mkdtempSync('/tmp/opencode/pi-vm-alias-');
try {
    symlinkSync('/root/host-only-target',`${host}/alias`);
    await run('bash',{command:`mkdir -p ${host}; printf 'alpha\nbeta\n' > ${host}/target; ln -s ${host}/target ${host}/alias`});
    await Promise.all([
        run('edit',{path:`${host}/target`,edits:[{oldText:'alpha',newText:'ALPHA'}]}),
        run('edit',{path:`${host}/alias`,edits:[{oldText:'beta',newText:'BETA'}]}),
    ]);
    assert.equal(text(await run('read',{path:`${host}/target`})),'ALPHA\nBETA\n');
} finally {rmSync(host,{recursive:true,force:true})}
assert.equal(text(await run('read',{path:`${cwd}/pi-boundary/nested/file`})),'alpha\nbeta\n');
assert.match(text(await run('edit',{path:nested,edits:[{oldText:'beta',newText:'gamma'}]})),/Successfully replaced/);
assert.equal(text(await run('read',{path:nested,offset:2,limit:1})),'gamma\n\n[Showing lines 2-2. Use offset=3 to continue.]');
await assert.rejects(run('edit',{path:nested,edits:[{oldText:'missing',newText:'x'}]}),/Could not find|not find/);
assert.equal(text(await run('read',{path:nested})),'alpha\ngamma\n');
await assert.rejects(run('read',{path:`${relative}/pi-boundary/missing`}),/No such file|ENOENT/);
const updates=[];
const failed=await run('bash',{command:`printf early; sleep 0.05; printf late >&2; exit 7`},undefined,v=>updates.push(v));
const output=text(failed);
assert.match(output,/earlylate/);assert.match(output,/code 7/);assert.ok(updates.some(v=>text(v).includes('early')));
assert.equal(failed.isError,true);assert.equal(failed.details.exitCode,7);
await assert.rejects(run('bash',{command:`sleep 30`,timeout:0.1}),/timed out|timeout/);
const controller=new AbortController();
await assert.rejects(run('bash',{command:`echo ready; sleep 30; echo leaked > ${cwd}/pi-boundary/cancelled`},controller.signal,v=>{if(text(v).includes('ready'))controller.abort()}),/cancel|abort|interrupted/i);
assert.equal(text(await run('bash',{command:`test ! -e ${cwd}/pi-boundary/cancelled; printf reused`})),'reused');
// Diagnostic, not a claimed guarantee: the intermediate parent exits before
// proc sampling, so setsid's writer can be adopted by the guest subreaper.
const detached=new AbortController();
await assert.rejects(run('bash',{command:`setsid sh -c '(sleep 0.4; echo leaked > ${cwd}/pi-boundary/detached) </dev/null >/dev/null 2>&1 & exit 0' </dev/null >/dev/null 2>&1 & sleep 0.1; echo ready; sleep 30`},detached.signal,v=>{if(text(v).includes('ready'))detached.abort()}),/cancel|abort|interrupted/i);
const detachedCancellationEscape=text(await run('bash',{command:`sleep 0.5; if test -f ${cwd}/pi-boundary/detached; then printf escaped; else printf stopped; fi`}))==='escaped';
console.log(JSON.stringify({diagnostic:'individual-cancel detached descendant ownership',detachedCancellationEscape}));
await run('bash',{command:`setsid sh -c 'sleep 0.4; echo leaked > ${cwd}/pi-boundary/drain-leak' </dev/null >/dev/null 2>&1 &`});
await proxy.cancel('pi-tools');
assert.equal(text(await run('bash',{command:`sleep 0.5; if test -f ${cwd}/pi-boundary/drain-leak; then printf leaked; else printf drained; fi`})),'drained','BaseRuntime must forward backend-required writer drain');
// Direct generic calls use the existing guest workspace without Pi path conversion.
await generic('write_file',{path:'ascout-relative-stage',bytes:Buffer.from([97,98])});
assert.deepEqual(Array.from((await generic('read_file',{path:'ascout-relative-stage'})).value.bytes),[97,98]);
await generic('replace_text',{path:'ascout-relative-stage',replacements:[{old_text:'ab',new_text:'cd'}]});
assert.ok((await generic('list_directory',{path:'.'})).value.entries.some(e=>e.name==='ascout-relative-stage'));
assert.equal((await generic('exec',{command:'test "$PWD" = /root && test -f ascout-relative-stage'})).exit_code,0);
assert.equal((await generic('exec',{cwd:'/tmp',command:'test "$PWD" = /tmp'})).exit_code,0);
await run('bash',{command:`printf 'one\\ntwo\\n' > ${cwd}/sparse; truncate -s 1G ${cwd}/sparse`});
assert.match(text(await run('read',{path:`${relative}/sparse`,offset:2,limit:1})),/^two/);
const png='iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aGNcAAAAASUVORK5CYII=';
await generic('write_file',{path:`${cwd}/image`,bytes:Buffer.from(png,'base64')});
assert.equal((await run('read',{path:`${relative}/image`,offset:2,limit:0})).content[1].data,png);
// FIFO open is a barrier proving the first VM holds the mutation lock.
proxy.bind('queue-second',JSON.parse(process.env.ASCOUT_TEST_SECOND_ACCESS));
const queuePath='/root/ascout-queue-stage';
await generic('exec',{command:`rm -f ${queuePath} ${queuePath}-alias ${queuePath}-gate; mkfifo ${queuePath} ${queuePath}-gate; ln -s ${queuePath} ${queuePath}-alias`});
const held=generic('replace_text',{path:queuePath,replacements:[{old_text:'old',new_text:'new'}]});
let readyResolve;const ready=new Promise(resolve=>readyResolve=resolve);
const writer=proxy.operation('pi-tools',`writer-${++id}`,'exec',{command:`exec 3>${queuePath}; printf ready; read release < ${queuePath}-gate; printf old >&3`},{},undefined,event=>{if(event.bytes&&Buffer.from(event.bytes).toString().includes('ready'))readyResolve()});
await ready;
const queued=generic('write_file',{path:`${queuePath}-alias`,bytes:Buffer.from('final')});
try {
    assert.equal((await proxy.operation('queue-second',`independent-${++id}`,'write_file',{path:queuePath,bytes:Buffer.from('independent')},{})).value.bytes_written,11);
} finally {await generic('exec',{command:`printf 'release\\n' > ${queuePath}-gate`})}
await writer;await held;await queued;
assert.equal(Buffer.from((await generic('read_file',{path:queuePath})).value.bytes).toString(),'final');
// Raw bytes cross the former 32 MiB file limit in both directions.
const path=`${cwd}/pi-boundary/nested/file`;
const count=33*1024*1024+17;
const original=Buffer.alloc(count,255);
original.set([0,128,254,10]);
assert.equal((await generic('write_file',{path,bytes:original})).value.bytes_written,count);
assert.deepEqual((await generic('read_file',{path})).value.bytes,original);
const stdout=[],stderr=[];
const executed=await proxy.operation('pi-tools',`binary-exec-${++id}`,'exec',{
    command:`cat; printf '\\377\\000' >&2`,input:original.subarray(0,150001),
},{},undefined,event=>(event.stream==='stdout'?stdout:stderr).push(event.bytes));
assert.equal(executed.exit_code,0);
assert.deepEqual(Buffer.concat(stdout),original.subarray(0,150001));
assert.deepEqual(Buffer.concat(stderr),Buffer.from([255,0]));
const largeText='λ'.repeat(750000);
await run('write',{path,content:'one two'});
await run('edit',{path,edits:[{oldText:'one',newText:largeText},{oldText:largeText,newText:'last'}]});
assert.equal(text(await run('read',{path})),'last two');
await run('write',{path,content:'one two'});
await assert.rejects(run('edit',{path,edits:[{oldText:'one',newText:'changed'},{oldText:'absent',newText:'fail'}]}),/exact|not find/);
assert.equal(text(await run('read',{path})),'one two');
await run('edit',{path,edits:[{oldText:'one',newText:'first'},{oldText:'first',newText:'last'}]});
assert.equal(text(await run('read',{path})),'last two');
console.log(JSON.stringify({verified:'independent Pi adapters + AscProxy + Runtime + Firecracker',writeBytes:count,readBytes:count,updates:updates.length}));

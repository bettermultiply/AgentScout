// Exercise the real isolated operations through a minimal local SDK, no model.
import fs from 'node:fs';
import {join} from 'node:path';
import assert from 'node:assert/strict';
import {test} from 'node:test';
import {filesystemTools} from '../instrumentation/pi/filesystem-tools.ts';

function fixture(readDefinition) {
    const root=fs.mkdtempSync('/tmp/opencode/pi-filesystem-tools-');
    for(const name of ['plans','workspace','view'])fs.mkdirSync(join(root,name));
    const cwd=join(root,'workspace');
    fs.writeFileSync(join(cwd,'large'),'host must remain unchanged');
    fs.writeFileSync(join(root,'plans','branch.json'),JSON.stringify({program:'bwrap',args:[
        '--die-with-parent','--unshare-user','--unshare-pid','--as-pid-1',
        '--ro-bind','/','/','--proc','/proc','--dev','/dev',
        '--bind',join(root,'view'),cwd,
        '--tmpfs',join(root,'plans'),'--remount-ro',join(root,'plans'),
    ]}));
    const sdk={createToolDefinition(name,cwd,options) {
        const ops=options[name].operations;
        if(name==='read'&&readDefinition)return readDefinition(cwd,options.read);
        return {name,async execute(_id,args,signal) {
            const path=join(cwd,'large');
            if(name==='write')return ops.writeFile(path,args.content);
            if(name==='edit') {
                await ops.access(path);
                const content=(await ops.readFile(path)).toString();
                return ops.writeFile(path,content.replace(args.oldText,args.newText));
            }
            if(name==='read')return ops.readFile(path);
            return ops.exec(args.command,cwd,{signal,onData:args.onData,input:args.input,timeout:args.timeout});
        }};
    }};
    const tools=Object.fromEntries(filesystemTools(join(root,'plans'),'branch',cwd,sdk).map(tool=>[tool.name,tool]));
    console.log('evidence',root);
    return {root,cwd,tools};
}

test('write and edit carry large UTF-8 content without argv limits or host writes',async t=>{
    const {root,cwd,tools}=fixture();
    // Already over Linux's per-argument limit after base64, including quotes,
    // newlines and non-ASCII text to also catch transport/encoding corruption.
    const content='汉字\n\'"\\'.repeat(20000)+'unique ending';
    for(const name of ['write','edit'])await t.test(name,async()=>{
        fs.writeFileSync(join(root,'view','large'),content);
        const expected=name==='write'?content:content.replace('unique ending','edited ending');
        await tools[name].execute(name,{content,oldText:'unique ending',newText:'edited ending'});
        assert.equal(fs.readFileSync(join(root,'view','large'),'utf8'),expected);
        assert.equal((await tools.read.execute('read',{})).toString(),expected);
        assert.equal(fs.readFileSync(join(cwd,'large'),'utf8'),'host must remain unchanged');
        assert.deepEqual(fs.readdirSync(root).sort(),['plans','view','workspace']);
    });
});

test('read image detection and binary bytes come from its Branch, not host',async()=>{
    const {root,cwd,tools}=fixture((cwd,{operations:ops})=>({name:'read',async execute(_id,{path}) {
        const absolute=join(cwd,path);
        await ops.access(absolute);
        const mime=await ops.detectImageMimeType?.(absolute);
        const bytes=await ops.readFile(absolute);
        return mime?{type:'image',mimeType:mime,data:bytes.toString('base64')}:{type:'text',text:bytes.toString('utf8')};
    }}));
    const png=Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aGNcAAAAASUVORK5CYII=','base64');
    fs.writeFileSync(join(cwd,'picture'),'host is text');
    fs.writeFileSync(join(root,'view','picture'),png);
    assert.deepEqual(await tools.read.execute('image',{path:'picture'}),{type:'image',mimeType:'image/png',data:png.toString('base64')});
    fs.writeFileSync(join(cwd,'picture'),png);
    fs.writeFileSync(join(root,'view','picture'),'branch is text');
    assert.deepEqual(await tools.read.execute('text',{path:'picture'}),{type:'text',text:'branch is text'});
});

test('piped input preserves cancellation, timeout, failed-write queue and descendant waiting',{timeout:10000},async()=>{
    const {root,tools}=fixture();
    const input=Buffer.alloc(4*1024*1024,120);
    const controller=new AbortController();
    await assert.rejects(tools.bash.execute('cancel',{
        command:'setsid /bin/bash -c \'sleep 0.3; printf leaked > cancelled-child\' </dev/null >/dev/null 2>&1 & printf ready; sleep 30',input,
        onData:()=>controller.abort(),
    },controller.signal),/Tool cancelled/);
    assert.equal((await tools.bash.execute('after-cancel',{
        command:'sleep 0.4; test ! -e cancelled-child',
    })).exitCode,0,'cancellation must also end descendants in other process groups');
    await assert.rejects(tools.bash.execute('timeout',{
        command:'sleep 30',input,timeout:0.1,
    }),/Tool timed out/);
    fs.mkdirSync(join(root,'view','large'));
    await assert.rejects(tools.write.execute('fail',{content:input.toString()}),/EPIPE|ECONNRESET|IsADirectoryError/);
    fs.rmdirSync(join(root,'view','large'));
    await tools.write.execute('retry',{content:'queue recovered'});
    assert.equal(fs.readFileSync(join(root,'view','large'),'utf8'),'queue recovered');
    // An early reader close must be handled, and the queued next operation must
    // observe even descendants which close stdio and leave the process group.
    let finished=false;
    const early=tools.bash.execute('early',{
        command:'setsid /bin/bash -c \'sleep 0.2; printf done > late\' </dev/null >/dev/null 2>&1 &',input,
    }).then(()=>{finished=true},error=>{assert.match(error.code,/EPIPE|ECONNRESET/);finished=true});
    const read=tools.bash.execute('after',{command:'test "$(cat late)" = done'});
    await early;
    assert.equal(finished,true);
    assert.equal((await read).exitCode,0);
    assert.equal(fs.readFileSync(join(root,'view','late'),'utf8'),'done');
});

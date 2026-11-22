// Complete operations against the real namespace holder; no SDK tools or model.
import fs from 'node:fs';
import {join} from 'node:path';
import assert from 'node:assert/strict';
import {test} from 'node:test';
import {environmentTools} from '../instrumentation/pi/tools.ts';
import {wrapToolDefinition} from '../../pi/packages/coding-agent/src/core/tools/tool-definition-wrapper.ts';
import {spawn,execFileSync} from 'node:child_process';
import {connect} from 'node:net';
import {executionMessages, operationBody, writeExecution} from '../instrumentation/pi/execution-wire.ts';
import {createRequire} from 'node:module';
const require=createRequire(import.meta.url);
const {detectSupportedImageMimeType}=require('../../pi/packages/coding-agent/src/utils/mime.ts');
async function fixture(t,workspace){
 const root=fs.mkdtempSync('/tmp/ascout-tools-');
 for(const name of ['plans','workspace','view'])fs.mkdirSync(join(root,name));
 const cwd=workspace??join(root,'workspace'),socket=join(root,'plans','cmd.sock');
 const stage=join(root,'mount-root');
 fs.mkdirSync(stage);
 const holder=spawn(new URL('../target/debug/ascout-fs-holder',import.meta.url).pathname,[],{stdio:['pipe','pipe','pipe']});
 let errors='';holder.stderr.on('data',b=>errors+=b);
 t.after(async()=>{
  if(holder.exitCode===null&&holder.signalCode===null){
   const exited=new Promise(r=>holder.once('exit',r));holder.kill();await exited;
  }
  fs.rmSync(root,{recursive:true,force:true});
 });
 holder.stdin.end(JSON.stringify({parent:process.pid,stage,socket:'/dev/ascout-holder/cmd.sock',chdir:cwd,entries:[
  {RoBind:{source:'/',target:'/'}},{Proc:{target:'/proc'}},{Dev:{target:'/dev'}},
  {Bind:{source:join(root,'view'),target:cwd}},{Bind:{source:join(root,'plans'),target:'/dev/ascout-holder'}},
 ]}));
 let ready='';for await(const chunk of holder.stdout)ready+=chunk;
 assert.ok(ready.trim(),errors);
 assert.ok(JSON.parse(ready)['child-pid']>1);
 const request=async(value,signal,output)=>{
  const {metadata,parts}=operationBody(value.op,value);
  const client=connect(socket);
  const stop=()=>client.end();
  try {
   await writeExecution(client,metadata,parts);
   signal?.addEventListener('abort',stop,{once:true});
   if(signal?.aborted)stop();
   for await(const {metadata:event,body} of executionMessages(client)){
    if(event.error)throw Error(event.error);
    if(event.body_kind==='file_bytes')event.value.bytes=body;
    if(event.body_kind==='json_value')event.value=JSON.parse(body.toString());
    if('value'in event||'exit_code'in event)return event;
    output?.({...event,bytes:body});
   }
   throw Error('Tool cancelled');
  } finally {signal?.removeEventListener('abort',stop);client.destroy()}
 };
 const proxy={operation:(_b,_i,op,args,_ctx,signal,output)=>request({op,bound_cwd:cwd,...args},signal,output)};
 const tools=Object.fromEntries(environmentTools(proxy,'branch',detectSupportedImageMimeType,'task').map(tool=>[tool.name,tool]));
 return {root,cwd,tools,request,socket};
}
test('small target line and byte ranges work on a sparse file above the old whole-file cap',async t=>{
 const {root,cwd,request,tools}=await fixture(t);
 const target=join(root,'view','sparse');
 fs.writeFileSync(target,'one\ntwo\n');fs.truncateSync(target,1024*1024*1024);
 const line=await request({op:'read_file',bound_cwd:cwd,path:'sparse',line_range:{offset:2,limit:1}});
 assert.equal(Buffer.from(line.value.bytes).toString(),'two');
 assert.deepEqual(line.value.actual_range,{offset:2,limit:1});assert.equal(line.value.has_more,true);
 const bytes=await request({op:'read_file',bound_cwd:cwd,path:'sparse',byte_range:{offset:0,limit:3}});
 assert.equal(Buffer.from(bytes.value.bytes).toString(),'one');assert.equal(bytes.value.has_more,true);
 const proc=await request({op:'read_file',path:'/proc/self/status',byte_range:{limit:5}});
 assert.equal(Buffer.from(proc.value.bytes).toString(),'Name:');
 assert.match((await tools.read.execute('sparse',{path:'sparse',offset:2,limit:1})).content[0].text,/^two/);
 fs.writeFileSync(target,'ok\n'+'x'.repeat(100000));
 const bounded=await tools.read.execute('bounded',{path:'sparse'});
 assert.match(bounded.content[0].text,/^ok/);assert.equal(bounded.details.truncated,true);
 assert.match((await tools.read.execute('long',{path:'sparse',offset:2})).content[0].text,/exceeds 50 KiB/);
 await assert.rejects(tools.read.execute('outside',{path:'sparse',offset:4}),/beyond end/);
 for(const content of ['', 'a', 'a\n', 'a\nb', 'a\nb\n', '\n\n']){
  fs.writeFileSync(target,content);const lines=content.split('\n');
  for(let offset=1;offset<=lines.length;offset++)for(let limit=0;limit<=lines.length+1;limit++){
   const result=await request({op:'read_file',bound_cwd:cwd,path:'sparse',line_range:{offset,limit}});
   assert.equal(Buffer.from(result.value.bytes).toString(),lines.slice(offset-1,offset-1+limit).join('\n'));
   assert.equal(result.value.has_more,offset-1+limit<lines.length);
  }
 }
});
test('complete operations preserve target paths, large bodies and atomic ordered edits',async t=>{
 const {root,cwd,tools,request}=await fixture(t),target=join(root,'view','large');
 fs.writeFileSync(join(cwd,'large'),'host unchanged');
 const content='汉字\n'.repeat(160000)+'unique ending';
 await tools.write.execute('write',{path:'large',content});
 assert.equal(fs.readFileSync(target,'utf8'),content);
 await tools.edit.execute('edit',{path:'large',edits:[{oldText:'unique ending',newText:'first'},{oldText:'first',newText:'last'}]});
 const expected=content.replace('unique ending','last');assert.equal(fs.readFileSync(target,'utf8'),expected);
 await assert.rejects(tools.edit.execute('mismatch',{path:'large',edits:[{oldText:'last',newText:'changed'},{oldText:'absent',newText:'fail'}]}),/exact/);
 assert.equal(fs.readFileSync(target,'utf8'),expected);
 await assert.rejects(tools.edit.execute('ambiguous',{path:'large',edits:[{oldText:'汉字',newText:'fail'}]}),/ambiguous/);
 assert.equal(fs.readFileSync(target,'utf8'),expected);
 await assert.rejects(request({op:'write_file',path:join(cwd,'large'),bytes:[256]}),/must be bytes/);
 assert.equal(fs.readFileSync(target,'utf8'),expected);
 assert.equal(fs.readFileSync(join(cwd,'large'),'utf8'),'host unchanged');
 const read=await request({op:'read_file',path:join(cwd,'large'),byte_range:{offset:0,limit:6}});
 assert.equal(Buffer.from(read.value.bytes).toString(),'汉字');assert.equal(read.value.has_more,true);
 const lines=await request({op:'read_file',path:join(cwd,'large'),line_range:{offset:2,limit:2}});
 assert.equal(Buffer.from(lines.value.bytes).toString(),'汉字\n汉字');
 assert.equal((await request({op:'list_directory',path:cwd})).value.entries[0].name,'large');
});
test('aliases serialize inside the target view despite conflicting host symlinks',async t=>{
 const {root,cwd,tools}=await fixture(t);
 fs.writeFileSync(join(root,'view','target'),'alpha\nbeta\n');
 fs.symlinkSync(join(cwd,'target'),join(root,'view','alias'));fs.symlinkSync('/root/host-only-target',join(cwd,'alias'));
 await Promise.all([tools.edit.execute('target',{path:'target',edits:[{oldText:'alpha',newText:'ALPHA'}]}),tools.edit.execute('alias',{path:'alias',edits:[{oldText:'beta',newText:'BETA'}]})]);
 assert.equal(fs.readFileSync(join(root,'view','target'),'utf8'),'ALPHA\nBETA\n');
 assert.equal(fs.readlinkSync(join(cwd,'alias')),'/root/host-only-target');
});
test('independent holders do not share canonical-path queues; aliases wait for the held mutation',async t=>{
 const first=await fixture(t),second=await fixture(t,first.cwd);
 const fifo=join(first.root,'view','target');execFileSync('mkfifo',[fifo]);
 fs.symlinkSync(join(first.cwd,'target'),join(first.root,'view','alias'));
 const editing=first.request({op:'replace_text',bound_cwd:first.cwd,path:'target',replacements:[{old_text:'old',new_text:'new'}]});
 // Opening the writer completes only once the mutation owns its lock and opens the FIFO reader.
 const writer=await fs.promises.open(fifo,'w');
 const alias=first.request({op:'write_file',bound_cwd:first.cwd,path:'alias',bytes:Buffer.from('final')});
 try {
  const independent=await second.request({op:'write_file',bound_cwd:first.cwd,path:'target',bytes:Buffer.from('ok')});
  assert.equal(independent.value.bytes_written,2);
 } finally {await writer.write('old');await writer.close()}
 await editing;await alias;
 assert.equal(fs.readFileSync(join(first.root,'view','target'),'utf8'),'final');
 assert.equal(fs.readFileSync(join(second.root,'view','target'),'utf8'),'ok');
});
test('Pi formatting reads images and line ranges from the environment',async t=>{
 const {root,cwd,tools}=await fixture(t);
 const png=Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aGNcAAAAASUVORK5CYII=','base64');
 fs.writeFileSync(join(cwd,'picture'),'host text');fs.writeFileSync(join(root,'view','picture'),png);
 assert.deepEqual((await tools.read.execute('image',{path:'picture'})).content[1],{type:'image',mimeType:'image/png',data:png.toString('base64')});
 fs.writeFileSync(join(root,'view','picture'),'one\ntwo\nthree');
 assert.match((await tools.read.execute('lines',{path:'picture',offset:2,limit:1})).content[0].text,/^two/);
});
test('Pi paths and implicit bash use the holder workspace and preserve target symlink traversal',async t=>{
 const {root,cwd,request,tools}=await fixture(t);
 assert.notEqual(process.cwd(),cwd);
 const view=join(root,'view');
 fs.mkdirSync(join(view,'actual','child'),{recursive:true});
 fs.symlinkSync(join(cwd,'actual','child'),join(view,'alias'));
 await tools.write.execute('write',{path:'note.txt',content:'relative'});
 assert.equal(fs.readFileSync(join(view,'note.txt'),'utf8'),'relative');
 const absolute=`${cwd}/alias/../note.txt`;
 await tools.write.execute('absolute',{path:absolute,content:'absolute'});
 await tools.edit.execute('edit',{path:'alias/../note.txt',edits:[{oldText:'absolute',newText:'edited'}]});
 assert.equal((await tools.read.execute('read',{path:absolute})).content[0].text,'edited');
 assert.equal(fs.readFileSync(join(view,'actual','note.txt'),'utf8'),'edited');
 assert.equal((await tools.bash.execute('pwd',{command:'pwd'})).content[0].text,`${cwd}\n`);
 assert.equal((await request({op:'exec',bound_cwd:cwd,cwd:'/tmp',command:'test "$PWD" = /tmp'})).exit_code,0);
});

test('exec streams, reports nonzero exits and supports timeout and process-group cancellation',async t=>{
 const {tools,root}=await fixture(t);let streamed='';
 const bash=wrapToolDefinition(tools.bash);
 const result=await bash.execute('exit',{command:'printf x >> runs; printf out; printf err >&2; exit 7'},undefined,v=>streamed=v.content[0].text);
 assert.equal(result.details.exitCode,7);assert.match(streamed,/out/);assert.match(streamed,/err/);
 assert.match(result.content[0].text,/out/);assert.match(result.content[0].text,/err/);assert.match(result.content[0].text,/code 7/);
 assert.equal(result.isError,true);
 assert.equal(fs.readFileSync(join(root,'view','runs'),'utf8'),'x');
 await assert.rejects(tools.bash.execute('timeout',{command:'sleep 30',timeout:0.1}),/timed out/i);
 const controller=new AbortController();
 await assert.rejects(tools.bash.execute('cancel',{command:'printf ready; sleep 30'},controller.signal,()=>controller.abort()),/cancelled/);
 const success=await bash.execute('next',{command:'printf alive'});
 assert.equal(success.details.exitCode,0);assert.equal(success.content[0].text,'alive');assert.notEqual(success.isError,true);
});

test('holder rejects truncated declared bodies before replacing the target',async t=>{
 const {socket,root,cwd}=await fixture(t);
 const path=join(root,'view','original');
 fs.writeFileSync(path,'keep');
 const client=connect({path:socket,allowHalfOpen:true});
 try {
  client.write(JSON.stringify({op:'write_file',bound_cwd:cwd,path:'original',body_length:6})+'\n');
  client.write(Buffer.from([0,0]));
  client.end(Buffer.from([0,6,255,0,10]));
  for await(const {metadata} of executionMessages(client)){
   assert.match(metadata.error,/incomplete body frame/);
   assert.equal(fs.readFileSync(path,'utf8'),'keep');
   return;
  }
  assert.fail('holder closed without rejecting the body');
 } finally {client.destroy()}
});

import {test} from 'node:test';
import assert from 'node:assert/strict';
import {createServer} from 'node:net';
import {mkdtemp,rm} from 'node:fs/promises';
import {AscProxy} from '../instrumentation/pi/asc-proxy.ts';
import {environmentTools} from '../instrumentation/pi/tools.ts';

import {executionMessages, writeExecution} from '../instrumentation/pi/execution-wire.ts';

async function receive(socket, callback){
    for await(const {metadata,body} of executionMessages(socket)) {
        await callback(metadata,body);
        return;
    }
}

test('one complete tool operation retains identity and terminal cancellation',async t=>{
    const root=await mkdtemp('/tmp/opencode/asc-proxy-'),requests=[],sockets=new Set();
    let writeAccepted;
    const accepted=new Promise(resolve=>writeAccepted=resolve);
    let writer;
    const server=createServer(socket=>{
        sockets.add(socket);socket.on('close',()=>sockets.delete(socket));
        receive(socket,(request,body)=>{
            requests.push(request);
            if(request.operation==='write_file'){
                assert.deepEqual(body,Buffer.from('data'));
                assert.ok(!Object.hasOwn(request.arguments,'bytes'));
            }
            if(request.command==='execute') {
                socket.write('{"event":"accepted"}\n');
                if(request.operation==='write_file'){writer=socket;writeAccepted();}
                else assert.fail('unexpected operation');
            }else if(request.command==='cancel') {
                const terminal='{"event":"terminal","status":"cancelled","reason":"writer stopped"}\n';
                writer.end(terminal);socket.end(terminal);
            }else socket.end('{"event":"confirmed"}\n');
        });
    });
    await new Promise(resolve=>server.listen(root+'/command.sock',resolve));
    t.after(async()=>{for(const socket of sockets)socket.destroy();await new Promise(resolve=>server.close(resolve));await rm(root,{recursive:true,force:true})});
    const proxy=new AscProxy();proxy.bind('branch',{transport:'unix-json-lines',address:root+'/command.sock',environment_id:'env'});
    const tool=environmentTools(proxy,'branch',()=>null,'task').find(tool=>tool.name==='write');
    const controller=new AbortController();
    const running=tool.execute('original-call',{path:'file',content:'data'},controller.signal);
    const rejected=assert.rejects(running,/writer stopped/);
    await accepted;controller.abort();await rejected;
    const executions=requests.filter(request=>request.command==='execute');
    assert.deepEqual(executions.map(request=>request.operation),['write_file']);
    assert.equal(executions[0].arguments.path,'file');
    assert.deepEqual(executions.map(request=>request.request_id),['req-1']);
    assert.ok(executions.every(request=>request.context.tool_call_id==='original-call'&&request.context.task_id==='task'));
    assert.equal(requests.find(request=>request.command==='cancel').operation_id,executions[0].operation_id);
    assert.equal(requests.filter(request=>request.command==='confirm').length,1);
});

test('immediate cancellation waits for acceptance and backend terminal before confirming',async t=>{
    const root=await mkdtemp('/tmp/opencode/asc-proxy-'),path=root+'/command.sock';
    const events=[];let execution,finish;
    const terminal=new Promise(resolve=>{finish=resolve});
    const server=createServer(socket=>{
        receive(socket,request=>{
            events.push(request.command);
            assert.equal(request.environment_id,'environment');assert.equal('branch_id' in request,false);assert.equal(request.operation_id,'operation');
            if(request.command==='execute'){
                execution=socket;
                setImmediate(()=>socket.write('{"event":"accepted"}\n'));
            }else if(request.command==='cancel'){
                assert.ok(execution);
                finish(()=>{
                    const result='{"event":"terminal","status":"cancelled","reason":"backend stopped"}\n';
                    execution.end(result);socket.end(result);
                });
            }else socket.end('{"event":"confirmed"}\n');
        });
    });
    await new Promise(resolve=>server.listen(path,resolve));
    t.after(async()=>{await new Promise(resolve=>server.close(resolve));await rm(root,{recursive:true,force:true})});
    const proxy=new AscProxy();proxy.bind('branch',{transport:'unix-json-lines',address:path,environment_id:'environment'});
    const controller=new AbortController();let settled=false;
    const result=proxy.execute('branch','operation','tool',{}, {},controller.signal);
    const rejected=assert.rejects(result,/backend stopped/).then(()=>{settled=true});
    controller.abort();
    const stop=await terminal;
    assert.equal(settled,false);
    assert.deepEqual(events,['execute','cancel']);
    stop();await rejected;
    assert.deepEqual(events,['execute','cancel','confirm']);
});

test('control framing and execution output use independent ASC-Proxy connections',async t=>{
    const root=await mkdtemp('/tmp/opencode/asc-proxy-');
    const sockets=new Set(),servers=[];
    t.after(async()=>{for(const socket of sockets)socket.destroy();await Promise.all(servers.map(server=>new Promise(resolve=>server.close(resolve))));await rm(root,{recursive:true,force:true})});
    const listen=async(name,handler)=>{const server=createServer(socket=>{sockets.add(socket);socket.on('close',()=>sockets.delete(socket));handler(socket)});servers.push(server);await new Promise(resolve=>server.listen(root+'/'+name,resolve));return root+'/'+name};
    let received;
    const control=await listen('control.sock',socket=>{
        socket.once('data',bytes=>{received=JSON.parse(bytes.subarray(4).toString());const value=Buffer.from(JSON.stringify({kind:'command',value:'continue'})),header=Buffer.alloc(4);header.writeUInt32BE(value.length);socket.write(header.subarray(0,2));socket.end(Buffer.concat([header.subarray(2),value]));});
    });
    let cancelled=false;
    const execution=await listen('execution.sock',socket=>receive(socket,async request=>{
        if(request.command==='execute'){
            const stdout=Buffer.from('{"result":"done"}');
            await writeExecution(socket,{event:'accepted'});
            await writeExecution(socket,{event:'output',value:'partial'});
            await writeExecution(socket,{event:'terminal',status:'succeeded',body_kind:'business_output',stdout_length:stdout.length,exit_code:0},[stdout]);
            socket.end();
        }
        else if(request.command==='cancel_environment'){cancelled=true;socket.end('{"event":"terminal","status":"cancelled"}\n')}
        else socket.end('{"event":"confirmed"}\n');
    }));
    const proxy=new AscProxy();await proxy.open(control);
    proxy.bind('branch',{transport:'unix-json-lines',address:execution,environment_id:'environment'});
    await proxy.send({kind:'message',value:'ready'});
    const messages=[];for await(const value of proxy.messages())messages.push(value);
    assert.deepEqual(received,{kind:'message',value:'ready'});assert.deepEqual(messages,[{kind:'command',value:'continue'}]);
    const output=[];assert.deepEqual(await proxy.execute('branch','operation','tool',{}, {},undefined,value=>output.push(value)),{result:'done'});
    assert.deepEqual(output,['partial']);await proxy.cancel('branch');assert.equal(cancelled,true);await proxy.close();
});

test('branch drain follows an active-operation cancel rejection',async t=>{
    const root=await mkdtemp('/tmp/opencode/asc-proxy-'),commands=[];let active;
    const server=createServer(socket=>receive(socket,request=>{
        commands.push(request.command);
        if(request.command==='execute'){active=socket;socket.write('{"event":"accepted"}\n');}
        else if(request.command==='cancel'){active.end('{"event":"rejected","reason":"cancelled during admission"}\n');socket.end('{"event":"rejected","reason":"operation already removed"}\n');}
        else if(request.command==='cancel_environment')socket.end('{"event":"terminal","status":"cancelled","generation":1}\n');
        else socket.end('{"event":"terminal","status":"cancelled"}\n');
    }));
    await new Promise(resolve=>server.listen(root+'/execution.sock',resolve));
    t.after(async()=>{await new Promise(resolve=>server.close(resolve));await rm(root,{recursive:true,force:true})});
    const proxy=new AscProxy();proxy.bind('branch',{transport:'unix-json-lines',address:root+'/execution.sock',environment_id:'environment'});
    const execution=proxy.execute('branch','operation','tool',{},{});
    const rejected=assert.rejects(execution,/cancelled during admission/);
    await proxy.cancel('branch');
    await rejected;
    assert.deepEqual(commands,['execute','cancel','cancel_environment']);
});

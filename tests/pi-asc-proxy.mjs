import {test} from 'node:test';
import assert from 'node:assert/strict';
import {createServer} from 'node:net';
import {mkdtemp,rm} from 'node:fs/promises';
import {AscProxy} from '../instrumentation/pi/asc-proxy.ts';

test('immediate cancellation waits for acceptance and backend terminal before confirming',async t=>{
    const root=await mkdtemp('/tmp/opencode/asc-proxy-'),path=root+'/command.sock';
    const events=[];let execution,finish;
    const terminal=new Promise(resolve=>{finish=resolve});
    const server=createServer(socket=>{
        let buffer='';socket.on('data',chunk=>{
            buffer+=chunk;const end=buffer.indexOf('\n');if(end<0)return;
            const request=JSON.parse(buffer.slice(0,end));buffer=buffer.slice(end+1);
            events.push(request.command);
            assert.equal(request.branch_id,'branch');assert.equal(request.operation_id,'operation');
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
    const execution=await listen('execution.sock',socket=>socket.once('data',bytes=>{
        const request=JSON.parse(bytes);
        if(request.command==='execute')socket.end('{"event":"accepted"}\n{"event":"output","value":"partial"}\n{"event":"terminal","status":"succeeded","output":{"stdout":"{\\"result\\":\\"done\\"}"}}\n');
        else if(request.command==='cancel_branch'){cancelled=true;socket.end('{"event":"terminal","status":"cancelled"}\n')}
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
    const server=createServer(socket=>socket.once('data',bytes=>{
        const request=JSON.parse(bytes.toString());commands.push(request.command);
        if(request.command==='execute'){active=socket;socket.write('{"event":"accepted"}\n');}
        else if(request.command==='cancel'){active.end('{"event":"rejected","reason":"cancelled during admission"}\n');socket.end('{"event":"rejected","reason":"operation already removed"}\n');}
        else if(request.command==='cancel_branch')socket.end('{"event":"terminal","status":"cancelled","generation":1}\n');
        else socket.end('{"event":"terminal","status":"cancelled"}\n');
    }));
    await new Promise(resolve=>server.listen(root+'/execution.sock',resolve));
    t.after(async()=>{await new Promise(resolve=>server.close(resolve));await rm(root,{recursive:true,force:true})});
    const proxy=new AscProxy();proxy.bind('branch',{transport:'unix-json-lines',address:root+'/execution.sock',environment_id:'environment'});
    const execution=proxy.execute('branch','operation','tool',{},{});
    const rejected=assert.rejects(execution,/cancelled during admission/);
    await proxy.cancel('branch');
    await rejected;
    assert.deepEqual(commands,['execute','cancel','cancel_branch']);
});

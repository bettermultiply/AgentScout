import assert from 'node:assert/strict';
import {spawn} from 'node:child_process';
import {once} from 'node:events';
import {mkdtemp, readFile, readdir, rm, writeFile} from 'node:fs/promises';
import {createServer} from 'node:net';
import {setTimeout as delay} from 'node:timers/promises';
import {test} from 'node:test';
import {fileURLToPath} from 'node:url';
import {AscProxy} from '../instrumentation/pi/asc-proxy.ts';
import {environmentTools} from '../instrumentation/pi/tools.ts';

const fixture = fileURLToPath(new URL('./fixtures/executable-backend.py', import.meta.url));
const binary = process.env.ASCOUT_RUNTIME_ADAPTER ?? fileURLToPath(
    new URL('../target/debug/ascout-runtime-adapter', import.meta.url),
);

async function start(t) {
    const root = await mkdtemp('/tmp/asc-program-cancel-');
    const server = createServer();
    await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
    await writeFile(`${root}/config.json`, JSON.stringify({
        control_address: `127.0.0.1:${server.address().port}`,
        adapter_instance: 'executable-cancellation-test',
        backends: [{id:'fixture', type:'executable', executable:fixture,
            interfaces:['environment'], capabilities:{branch_environment:false}, config:{root}}],
    }));
    const connection = once(server, 'connection');
    const child = spawn(binary, ['--config', `${root}/config.json`], {stdio:['ignore', 'ignore', 'pipe']});
    const exited = once(child, 'exit');
    let stderr = '';
    child.stderr.setEncoding('utf8').on('data', chunk => { stderr += chunk; });
    let socket;
    t.after(async () => {
        socket?.destroy();
        server.close();
        if (child.exitCode === null) {
            await Promise.race([exited, delay(2000, undefined, {ref:false})]);
            child.kill();
        }
        if (!process.env.ASCOUT_KEEP_EXECUTABLE_TRACE) await rm(root, {recursive:true, force:true});
        else t.diagnostic(`trace: ${root}/trace.jsonl`);
    });
    [socket] = await Promise.race([connection, exited.then(() => assert.fail(stderr))]);
    let bytes = Buffer.alloc(0);
    const pending = new Map();
    socket.on('data', chunk => {
        bytes = Buffer.concat([bytes, chunk]);
        while (bytes.length >= 4 && bytes.length >= 4 + bytes.readUInt32BE(0)) {
            const end = 4 + bytes.readUInt32BE(0);
            const message = JSON.parse(bytes.subarray(4, end));
            bytes = bytes.subarray(end);
            if (message.message === 'result') pending.get(message.request_id).resolve(message);
        }
    });
    let sequence = 0;
    async function call(operation, environment_id = '') {
        const request_id = `request-${++sequence}`;
        const result = Promise.withResolvers();
        pending.set(request_id, result);
        const body = Buffer.from(JSON.stringify({message:'request', request_id, environment_id, operation:{operation}}));
        const length = Buffer.alloc(4);
        length.writeUInt32BE(body.length);
        socket.write(Buffer.concat([length, body]));
        const reply = await result.promise;
        assert.equal(reply.status, 'completed', JSON.stringify(reply));
        return reply;
    }
    const a = await call('establish_environment');
    const b = await call('establish_environment');
    const proxy = new AscProxy();
    proxy.bind('a', a.execution_access);
    proxy.bind('b', b.execution_access);
    const trace = async () => (await readFile(`${root}/trace.jsonl`, 'utf8')).trim().split('\n').map(JSON.parse);
    async function wait(event, environment, operation_id) {
        for (let attempt = 0; attempt < 400; attempt++) {
            const match = (await trace()).find(row => row.event === event &&
                row.environment === environment && (operation_id === undefined || row.operation_id === operation_id));
            if (match) return match;
            await delay(10);
        }
        assert.fail(`missing ${event} in ${environment} for ${operation_id}`);
    }
    const close = () => socket.end();
    async function done() {
        assert.deepEqual(await exited, [0, null]);
        return stderr;
    }
    return {root, a:a.environment_id, b:b.environment_id, proxy, call, trace, wait, close, done};
}

test('executable tool cancellation preserves the shared instance, frames, and cleanup', {timeout:20000}, async t => {
    const {root, a, b, proxy, call, trace, wait, close, done} = await start(t);
    const identity = 'same-operation';
    const pid = (await proxy.execute('b', identity, 'fixture_tool', {}, {})).result.pid;
    const controller = new AbortController();
    const output = [];
    let settled = false;
    const cancelled = assert.rejects(proxy.execute('a', identity, '', {command:'hold'}, {},
        controller.signal, event => output.push(event.bytes), 'exec'), /fixture writer stopped/)
        .then(() => { settled = true; });
    await wait('output-partial', a, identity);
    controller.abort();
    await wait('cancel', a, identity);
    assert.equal(settled, false);
    assert.ok((await readdir(`${root}/${a}`)).includes('writer'));
    await writeFile(`${root}/finish-stop`, '');
    await cancelled;
    assert.deepEqual(output, [Buffer.from([0, 255, 128, 10, 0])]);
    assert.deepEqual(await readdir(`${root}/${a}`), []);
    const ordered = (await trace()).filter(row => row.environment === a && row.operation_id === identity);
    assert.deepEqual(ordered.map(row => row.event),
        ['environment_operation','started','output-partial','cancel','stopped','terminal']);
    assert.ok(ordered.every(row => row.pid === pid));

    const after = await proxy.execute('b', 'after-a', 'fixture_tool', {check:'b'}, {});
    assert.deepEqual(after.result, {pid, environment:b, arguments:{check:'b'}});
    await proxy.cancel('a');
    const tools = Object.fromEntries(environmentTools(proxy, 'a', () => null, 'test-task').map(tool => [tool.name, tool]));
    await tools.write.execute('write', {path:'text', content:'still available'});
    assert.match((await tools.read.execute('read', {path:'text'})).content[0].text, /still available/);
    await rm(`${root}/${a}/text`);

    const upload = new AbortController();
    const input = Buffer.from(Array.from({length:256}, (_, i) => i));
    const large = Buffer.concat(Array(8192).fill(input));
    const uploading = assert.rejects(proxy.execute('a', 'upload', '', {path:'slow-upload', bytes:large}, {},
        upload.signal, undefined, 'write_file'), /fixture writer stopped/);
    await wait('upload-partial', a, 'upload');
    upload.abort();
    await delay(150);
    await writeFile(`${root}/finish-upload`, '');
    await uploading;
    const uploadEvents = (await trace()).filter(row => row.operation_id === 'upload').map(row => row.event);
    assert.ok(uploadEvents.indexOf('upload-partial') < uploadEvents.indexOf('upload-complete'));
    assert.ok(uploadEvents.indexOf('upload-complete') < uploadEvents.indexOf('cancel'));
    assert.ok(uploadEvents.indexOf('cancel') < uploadEvents.indexOf('stopped'));
    assert.equal(uploadEvents.filter(event => event === 'terminal').length, 1);

    const finishing = new AbortController();
    const current = proxy.execute('a', 'current', '', {command:'finish'}, {}, finishing.signal, undefined, 'exec');
    await wait('started', a, 'current');
    const queued = new AbortController();
    const queuedResult = assert.rejects(proxy.execute('b', 'queued', 'fixture_tool', {hold:true}, {}, queued.signal), /interrupt/);
    queued.abort();
    finishing.abort();
    await wait('cancel', a, 'current');
    await writeFile(`${root}/finish-current`, '');
    assert.equal((await current).exit_code, 7);
    await queuedResult;
    assert.equal((await trace()).some(row => row.operation_id === 'queued'), false);

    await proxy.operation('b', 'binary-write', 'write_file', {path:'binary', bytes:input}, {});
    assert.deepEqual((await proxy.operation('b', 'binary-read', 'read_file', {path:'binary'}, {})).value.bytes, input);
    await rm(`${root}/${b}/binary`);
    await call('terminate_environment', b);
    await call('cleanup_environment', b);

    const named = new AbortController();
    const namedCancelled = assert.rejects(proxy.execute('a', 'named-hold', 'fixture_tool', {hold:true}, {}, named.signal),
        /fixture writer stopped/);
    await wait('started', a, 'named-hold');
    named.abort();
    await namedCancelled;
    assert.equal((await proxy.execute('a', 'after-named', 'fixture_tool', {}, {})).result.pid, pid);

    const shutdownHold = assert.rejects(proxy.execute('a', 'shutdown-hold', 'fixture_tool', {hold:true}, {}),
        /fixture writer stopped|socket closed|connection closed/i);
    await wait('started', a, 'shutdown-hold');
    close();
    await shutdownHold;
    await wait('cancel', a, 'shutdown-hold');
    assert.doesNotMatch(await done(), /shutdown incomplete/);
    const final = await trace();
    assert.deepEqual(final.slice(-4).map(row => row.event), ['terminal','terminate','cleanup','shutdown']);
    assert.ok(final.every(row => row.pid === pid));
    assert.deepEqual((await readdir(root)).sort(), ['config.json','finish-current','finish-stop','finish-upload','trace.jsonl']);
    assert.throws(() => process.kill(pid, 0), {code:'ESRCH'});
});

test('executable process failure during cancel reports transport failure', {timeout:10000}, async t => {
    const {a, proxy, wait, close, done} = await start(t);
    const controller = new AbortController();
    const failure = assert.rejects(proxy.execute('a', 'fault', '', {command:'fault'}, {},
        controller.signal, undefined, 'exec'), /unexpected end of file/i);
    await wait('started', a, 'fault');
    controller.abort();
    await wait('cancel', a, 'fault');
    await failure;
    close();
    assert.match(await done(), /shutdown incomplete/);
});

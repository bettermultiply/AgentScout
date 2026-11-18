#!/usr/bin/env python3
"""Small executable backend for the ASC-Proxy cancellation regression."""
import json
import os
from pathlib import Path
import struct
import sys
from threading import Event, Thread
import time

source, sink = sys.stdin.buffer, sys.stdout.buffer
methods = ('establish operate branch promote suspend resume terminate cleanup shutdown '
           'environment_operation invoke_tool cancel').split()


def send(meta, body=None):
    if body is not None:
        meta = {**meta, 'body_length': len(body)}
    sink.write(json.dumps(meta).encode() + b'\n')
    if body is not None:
        for offset in range(0, len(body), 65536):
            chunk = body[offset:offset + 65536]
            sink.write(struct.pack('>I', len(chunk)) + chunk)
        sink.write(bytes(4))
    sink.flush()


def trace(event, request):
    with (root / 'trace.jsonl').open('a') as log:
        print(json.dumps(dict(event=event, pid=os.getpid(),
                              environment=request.get('environment'),
                              operation_id=request.get('operation_id'))), file=log)


def gate(name):
    deadline = time.monotonic() + 8
    while not (root / name).exists():
        if time.monotonic() >= deadline:
            raise TimeoutError(name)
        time.sleep(0.01)


def read_body(request):
    if 'body_length' not in request:
        return None
    body = bytearray()
    while (size := struct.unpack('>I', source.read(4))[0]):
        assert size <= 65536
        chunk = source.read(size)
        assert len(chunk) == size
        body.extend(chunk)
        if len(body) == size and request['arguments'].get('path') == 'slow-upload':
            trace('upload-partial', request)
            gate('finish-upload')
    assert len(body) == request['body_length']
    if request['arguments'].get('path') == 'slow-upload':
        assert body == bytes(range(256)) * 8192
        trace('upload-complete', request)
    return bytes(body)


def key(request):
    return request['environment'], request['operation_id']


def run_tool(request, body, stopped):
    trace('started', request)
    args = request['arguments']
    path = args.get('path')
    command = args.get('command')
    environment = root / request['environment']
    response = dict(ok=True)
    result_body = None
    if command in ('hold', 'finish', 'fault') or path == 'slow-upload' or args.get('hold'):
        if command == 'hold':
            # Stop midway through an output chunk until the reader receives cancel.
            sink.write(b'{"stream":"stdout","body_length":5}\n' + struct.pack('>I', 5) + b'\x00\xff')
            sink.flush()
            trace('output-partial', request)
        elif command == 'finish':
            send(dict(stream='stdout'), b'ready')
            gate('finish-current')
        if command != 'finish':
            writer = environment / 'writer'
            with writer.open('wb') as resource:
                assert stopped.wait(8), 'cancel was not delivered to the active worker'
                if command == 'fault':
                    os._exit(3)
                resource.write(b'stopped')
                gate('finish-stop')
            writer.unlink()
            trace('stopped', request)
            if command == 'hold':
                sink.write(b'\x80\n\x00' + bytes(4))
            response = dict(ok=False, error='fixture writer stopped', error_category='cancelled')
        else:
            response['exit_code'] = 7
    elif request['op'] == 'invoke_tool':
        value = dict(result=dict(pid=os.getpid(), environment=request['environment'], arguments=args))
        response['result'] = dict(exit_code=0, stdout=json.dumps(value), stderr='')
    elif request['operation'] == 'write_file':
        (environment / path).write_bytes(body)
        response['value'] = dict(bytes_written=len(body))
    elif request['operation'] == 'read_file':
        result_body = (environment / path).read_bytes()
        response.update(body_kind='file_bytes', value={'actual_range': {'offset': 1, 'limit': 1}})
    else:
        raise ValueError(request)
    trace('terminal', request)
    send(response, result_body)


active = None
for raw in source:
    request = json.loads(raw)
    op = request['op']
    if op != 'hello':
        trace(op, request)
    if op == 'cancel':
        if active is not None and key(request) == active[0]:
            active[1].set()
        continue  # No separate response, even if the worker has already finished.
    if active is not None:
        active[2].join()
        active = None
    body = read_body(request)
    if op in ('environment_operation', 'invoke_tool'):
        stopped = Event()
        worker = Thread(target=run_tool, args=(request, body, stopped), daemon=True)
        active = key(request), stopped, worker
        worker.start()
        continue
    result = None
    if op == 'hello':
        root = Path(request['config']['root'])
        result = dict(version=1, methods=methods, capabilities=dict(branch_environment=False))
    elif op == 'establish':
        (root / request['environment']).mkdir()
    elif op == 'cleanup':
        (root / request['environment']).rmdir()
    elif op == 'operate':
        result = dict(exit_code=0, stdout='', stderr='')
    send(dict(ok=True, result=result))
    if op == 'shutdown':
        break

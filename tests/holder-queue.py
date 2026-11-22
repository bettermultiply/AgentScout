"""Verify canonical-path serialization through the native holder socket."""
import json
import os
from pathlib import Path
import select
import socket
import stat
import struct
import subprocess
import tempfile
import time

BINARY = Path(__file__).resolve().parents[1] / 'target/debug/ascout-fs-holder'


def request(address, operation, body):
    connection = socket.socket(socket.AF_UNIX)
    connection.settimeout(5)
    connection.connect(str(address))
    operation['body_length'] = len(body)
    connection.sendall(json.dumps(operation).encode() + b'\n')
    connection.sendall(struct.pack('>I', len(body)) + body + bytes(4))
    return connection


def result(connection):
    with connection.makefile('rb') as stream:
        return json.loads(stream.readline())


with tempfile.TemporaryDirectory() as directory:
    root = Path(directory)
    address, target, alias = (root / name for name in ('cmd.sock', 'target', 'alias'))
    os.mkfifo(target)
    alias.symlink_to(target)
    with subprocess.Popen([BINARY, address]) as holder:
        try:
            deadline = time.monotonic() + 5
            while not address.exists():
                assert holder.poll() is None and time.monotonic() < deadline
                time.sleep(0.005)
            with request(address, {'op': 'replace_text', 'path': str(target),
                                   'replacements': [{'old_length': 8, 'new_length': 5}]}, b'originalfirst') as first:
                while True:
                    try:
                        writer = os.open(target, os.O_WRONLY | os.O_NONBLOCK)
                        break
                    except OSError:
                        assert time.monotonic() < deadline, 'first mutation never opened the FIFO'
                        time.sleep(0.005)
                try:
                    with request(address, {'op': 'write_file', 'path': str(alias)}, b'final') as second:
                        assert not select.select([second], [], [], 0.1)[0], 'alias overtook held mutation'
                        assert stat.S_ISFIFO(target.stat().st_mode)
                        os.write(writer, b'original')
                        os.close(writer)
                        writer = None
                        assert result(first)['value']['changed'] is True
                        assert result(second)['value']['bytes_written'] == 5
                finally:
                    if writer is not None:
                        os.close(writer)
                assert target.read_bytes() == b'final'
                assert alias.is_symlink()
        finally:
            holder.terminate()
print('native holder canonical aliases serialize across the FIFO barrier')

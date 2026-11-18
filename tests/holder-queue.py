"""Exercise the holder's actual mutation function with controlled lock events."""
import ast
import io
import shutil
import os
from pathlib import Path
import tempfile
import threading
from concurrent.futures import ThreadPoolExecutor
from types import SimpleNamespace

source = Path(__file__).parents[1] / 'src/runtime/process/holder.py'
nodes = ast.parse(source.read_text()).body
namespace = {'os': os, 'tempfile': tempfile, 'FRAME_SIZE': 65536, 'shutil': shutil, 'send_event': lambda *_: True,
             'mutation_registration': threading.Lock(), 'mutation_locks': {}}
exec(compile(ast.Module(body=[n for n in nodes if isinstance(n, ast.FunctionDef) and n.name in ('atomic_write', 'file_operation')], type_ignores=[]), str(source), 'exec'), namespace)
held, attempted, release = threading.Event(), threading.Event(), threading.Event()
class ObservedLock:
    def __init__(self):
        self.lock = threading.Lock()
    def __enter__(self):
        if self.lock.locked():
            attempted.set()
        self.lock.acquire()
    def __exit__(self, *_):
        self.lock.release()
namespace['threading'] = SimpleNamespace(Lock=ObservedLock)
original = namespace['atomic_write']
def write(path, content):
    if content.getvalue() == b'first':
        held.set()
        assert release.wait(10), 'release event missing'
    original(path, content)
namespace['atomic_write'] = write
with tempfile.TemporaryDirectory() as directory, ThreadPoolExecutor(2) as pool:
    path = Path(directory)/'target'
    alias = Path(directory)/'alias'
    path.write_bytes(b'original')
    alias.symlink_to(path)
    operation = namespace['file_operation']
    first = pool.submit(operation, None, 'write_file', {'path': str(path)}, io.BytesIO(b'first'), 5)
    assert held.wait(10)
    second = pool.submit(operation, None, 'write_file', {'path': str(alias)}, io.BytesIO(b'final'), 5)
    try:
        assert attempted.wait(10), 'alias did not reach the same canonical lock'
        assert not second.done()
        assert path.read_bytes() == b'original'
    finally:
        release.set()
    first.result()
    second.result()
    assert path.read_bytes() == b'final'
print('holder canonical aliases serialize: controlled lock acquisition and release events')

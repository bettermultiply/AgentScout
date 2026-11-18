"""Minimal namespace holder.

The process only keeps its branch mount namespace alive and executes tool
commands inside it.  It is not an Agent and it owns no transcript or policy.
Every connection is handled concurrently; a command runs in its own session so
an aborted command can be terminated without killing the holder or background
work started by an earlier command.
"""

import ctypes
import json
import os
import select
import shutil
import signal
import socket
import subprocess
import sys
import threading
import time
import tempfile
import struct

SOCKET = sys.argv[1]
# Adopt detached tool descendants even when the launcher does not create a
# private PID namespace. Never treat other processes in /proc as owned.
if ctypes.CDLL(None, use_errno=True).prctl(36, 1, 0, 0, 0) != 0:
    raise OSError(ctypes.get_errno(), "cannot adopt tool descendants")
POLL_SECONDS = 0.01
# After the foreground exits, keep forwarding briefly so bytes still buffered
# in a pipe held open by a background descendant are not lost. A background
# process that writes without end must still not delay the tool result.
FLUSH_SECONDS = 0.5

try:
    os.unlink(SOCKET)
except FileNotFoundError:
    pass

listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
listener.bind(SOCKET)
os.chmod(SOCKET, 0o600)
listener.listen(64)
writer_lock = threading.Lock()
mutation_registration = threading.Lock()
mutation_locks = {}


HEADER_LIMIT = 1024 * 1024
FRAME_SIZE = 64 * 1024


def receive_request(reader, spool):
    header = reader.readline(HEADER_LIMIT + 1)
    if len(header) > HEADER_LIMIT or not header.endswith(b'\n'):
        raise ValueError('oversized or incomplete message header')
    request = json.loads(header)
    if not isinstance(request, dict) or 'framed' in request:
        raise ValueError('invalid request header')
    if 'body_length' not in request:
        return request, None
    length = request.pop('body_length')
    if type(length) is not int or length < 0 or length > 2**64 - 1:
        raise ValueError('invalid body length')
    remaining = length
    while True:
        raw = reader.read(4)
        if len(raw) != 4:
            raise ValueError('incomplete body frame')
        size = struct.unpack('>I', raw)[0]
        if size == 0:
            if remaining:
                raise ValueError('incomplete declared body')
            spool.seek(0)
            return request, length
        if size > FRAME_SIZE or size > remaining:
            raise ValueError('invalid body frame length')
        chunk = reader.read(size)
        if len(chunk) != size:
            raise ValueError('incomplete body frame')
        spool.write(chunk)
        remaining -= size


def validate_body(operation, request, length):
    if 'bytes' in request or 'input' in request:
        raise ValueError('expected a separate binary body')
    if operation in ('write_file', 'replace_text') and length is None:
        raise ValueError('operation requires a body')
    if length is not None and operation not in ('write_file', 'replace_text', 'exec'):
        raise ValueError('unexpected operation body')
    if operation == 'replace_text':
        edits = request.get('replacements')
        if not isinstance(edits, list) or not edits:
            raise ValueError('replacements must not be empty')
        total = 0
        for edit in edits:
            if not isinstance(edit, dict) or set(edit) - {'old_length', 'new_length', 'match'}:
                raise ValueError('invalid replacement metadata')
            old, new = edit.get('old_length'), edit.get('new_length')
            if type(old) is not int or old <= 0 or type(new) is not int or new < 0:
                raise ValueError('invalid replacement lengths')
            if edit.get('match', 'exactly_one') not in ('all', 'exactly_one'):
                raise ValueError('invalid replacement match')
            total += old + new
        if total != length:
            raise ValueError('invalid replacement body lengths')


def atomic_write(path, content):
    fd, temporary = tempfile.mkstemp(dir=os.path.dirname(path))
    try:
        with os.fdopen(fd, 'wb') as target:
            if os.path.exists(path):
                os.fchmod(target.fileno(), os.stat(path).st_mode & 0o7777)
            if isinstance(content, bytes):
                target.write(content)
            else:
                shutil.copyfileobj(content, target, FRAME_SIZE)
            target.flush()
            os.fsync(target.fileno())
        os.replace(temporary, path)
    finally:
        if os.path.exists(temporary):
            os.unlink(temporary)


MAX_LINE_BYTES = 32 * 1024 * 1024


def file_operation(connection, operation, request, spool, length):
    path = os.path.join(request.get('bound_cwd') or '/', request['path'])
    if operation == 'list_directory':
        with os.scandir(path) as entries:
            entries = sorted([{'name': e.name, 'type': 'symlink' if e.is_symlink() else 'directory' if e.is_dir() else 'file' if e.is_file() else 'other'} for e in entries], key=lambda e: e['name'])
        with tempfile.TemporaryFile(dir=os.path.dirname(SOCKET)) as output:
            for part in json.JSONEncoder().iterencode({'entries': entries}):
                output.write(part.encode())
            size = output.tell()
            output.seek(0)
            send_file(connection, {'value': {}, 'body_kind': 'json_value'}, output, size)
        return
    if operation == 'read_file':
        if 'byte_range' in request and 'line_range' in request:
            raise ValueError('byte_range and line_range are mutually exclusive')
        with open(path, 'rb') as source:
            prefix = source.read(12)
            source.seek(0)
            if request.get('binary_fallback') and (prefix.startswith((b'\x89PNG', b'\xff\xd8\xff', b'GIF8', b'BM')) or (prefix.startswith(b'RIFF') and prefix[8:12] == b'WEBP')):
                size = os.fstat(source.fileno()).st_size
                return send_file(connection, {'value': {'representation': 'binary'}, 'body_kind': 'file_bytes'}, source, size)
            if 'line_range' in request:
                max_bytes = min(request.get('max_bytes', MAX_LINE_BYTES), MAX_LINE_BYTES)
                bounds = request['line_range']
                offset, limit = bounds['offset'], bounds['limit']
                if type(offset) is not int or offset < 1 or type(limit) is not int or limit < 0:
                    raise ValueError('invalid line range')
                line, selected, content = 1, 0, bytearray()
                ended_with_newline, reached_eof = False, False
                completed_bytes, truncated = 0, False
                while not (line >= offset and selected >= limit):
                    chunk = source.readline(65536)
                    if not chunk:
                        reached_eof = True
                        if line < offset:
                            raise ValueError('Offset is beyond end of file')
                        if line >= offset and selected < limit:
                            selected += 1
                        break
                    ended_with_newline = chunk.endswith(b'\n')
                    if line >= offset:
                        if len(content) + len(chunk) > max_bytes:
                            del content[completed_bytes:]
                            truncated = True
                            break
                        content.extend(chunk)
                    if ended_with_newline:
                        if line >= offset:
                            selected += 1
                            completed_bytes = len(content)
                        line += 1
                if not reached_eof and content.endswith(b'\n'):
                    content.pop()
                content.decode('utf-8')
                value = {'actual_range': {'offset': offset, 'limit': selected}, 'has_more': not reached_eof, 'truncated': truncated}
                return send_event(connection, {'value': value, 'body_kind': 'file_bytes'}, content)
            bounds = request.get('byte_range') or {}
            offset, limit = bounds.get('offset', 0), bounds.get('limit')
            if type(offset) is not int or offset < 0 or (limit is not None and (type(limit) is not int or limit < 0)):
                raise ValueError('invalid byte range')
            source.seek(offset)
            file_size = os.fstat(source.fileno()).st_size
            available = max(0, file_size - offset)
            if file_size == 0 and limit:
                # Pseudo-files such as /proc entries need an actual read to
                # determine the size of a requested range.
                with tempfile.TemporaryFile(dir=os.path.dirname(SOCKET)) as output:
                    remaining = limit
                    while remaining:
                        chunk = source.read(min(remaining, FRAME_SIZE))
                        if not chunk:
                            break
                        output.write(chunk)
                        remaining -= len(chunk)
                    size = limit - remaining
                    output.seek(0)
                    value = {'actual_range': {'offset': offset, 'limit': size}, 'has_more': False, 'truncated': False}
                    return send_file(connection, {'value': value, 'body_kind': 'file_bytes'}, output, size)
            size = available if limit is None else min(limit, available)
            value = {'actual_range': {'offset': offset, 'limit': size}, 'has_more': size < available, 'truncated': False}
            return send_file(connection, {'value': value, 'body_kind': 'file_bytes'}, source, size)
    path = os.path.realpath(path)
    with mutation_registration:
        lock, users = mutation_locks.get(path, (threading.Lock(), 0))
        mutation_locks[path] = (lock, users+1)
    try:
        with lock:
            if operation == 'write_file':
                if request.get('create_parents'):
                    os.makedirs(os.path.dirname(path), exist_ok=True)
                atomic_write(path, spool)
                value = {'bytes_written': length}
            else:
                value = replace_text(path, request['replacements'], spool)
        return send_event(connection, {'value': value})
    finally:
        with mutation_registration:
            _, users = mutation_locks[path]
            if users == 1:
                del mutation_locks[path]
            else:
                mutation_locks[path] = (lock, users-1)


def replace_text(path, replacements, spool):
    with open(path, 'rb') as source:
        original = source.read().decode('utf-8')
    text, counts = original, []
    for replacement in replacements:
        old = spool.read(replacement['old_length']).decode('utf-8')
        new = spool.read(replacement['new_length']).decode('utf-8')
        count = text.count(old)
        if count == 0:
            raise ValueError('Could not find exact old_text')
        if replacement.get('match', 'exactly_one') == 'exactly_one' and count != 1:
            raise ValueError('old_text is ambiguous; expected exactly one occurrence')
        text = text.replace(old, new)
        counts.append(count)
    changed = text != original
    content = text.encode() if changed else b''
    if changed:
        atomic_write(path, content)
    return {'counts': counts, 'changed': changed, 'bytes_written': len(content)}


def send_header(connection, payload, length=None):
    if length is not None:
        payload = dict(payload, body_length=length)
    header = (json.dumps(payload) + "\n").encode()
    if len(header) > HEADER_LIMIT:
        raise ValueError('oversized message header')
    connection.sendall(header)


def send_chunk(connection, chunk):
    connection.sendall(struct.pack('>I', len(chunk)))
    if chunk:
        connection.sendall(chunk)


def send_file(connection, payload, source, remaining):
    send_header(connection, payload, remaining)
    while remaining:
        chunk = source.read(min(remaining, FRAME_SIZE))
        if not chunk:
            # A concurrent truncate must close the incomplete response, since
            # the peer is still expecting a content frame.
            connection.shutdown(socket.SHUT_WR)
            return
        send_chunk(connection, chunk)
        remaining -= len(chunk)
    send_chunk(connection, b'')


def send_event(connection, payload, content=None):
    try:
        send_header(connection, payload, None if content is None else len(content))
        if content is not None:
            view = memoryview(content)
            for offset in range(0, len(view), FRAME_SIZE):
                send_chunk(connection, view[offset:offset + FRAME_SIZE])
            send_chunk(connection, b'')
        return True
    except OSError:
        return False


def drain(source, connection, state, field):
    """Forward one stream until every byte the foreground process wrote is out.

    The foreground process shares its stdout/stderr pipes with any background
    descendant it leaves behind.  Once the foreground exits, all of its bytes
    are already buffered in the pipe, so a non-blocking poll can deliver them
    deterministically instead of sleeping for an arbitrary moment.  The thread
    then stays on the pipe (discarding later background output) so a descendant
    never receives SIGPIPE because the holder closed its end.
    """
    fd = source.fileno()
    os.set_blocking(fd, False)
    flushed = False
    foreground_at = None

    def forward(chunk):
        with state["lock"]:
            # After the final result the client is gone; keep draining so a
            # background process that inherited the pipe is not blocked by a
            # closed reader.
            if state["finished"]:
                return True
            if send_event(connection, {"stream": field}, chunk):
                return True
            state["finished"] = True
            state["foreground_done"] = True
            return False

    def flush_available(limit):
        # Read everything already buffered (bounded, in case a background
        # descendant keeps the pipe continuously full) and forward it. This is
        # the pass that guarantees no foreground bytes are lost when the
        # foreground exits while a slow client still blocks a send.
        remaining = limit
        while remaining > 0:
            try:
                chunk = os.read(fd, min(65536, remaining))
            except BlockingIOError:
                return True
            except OSError:
                return True
            if not chunk:
                return False
            remaining -= len(chunk)
            if not forward(chunk):
                return False
        return True

    try:
        while True:
            if state["foreground_done"]:
                if foreground_at is None:
                    foreground_at = time.monotonic()
                elif time.monotonic() - foreground_at >= FLUSH_SECONDS:
                    # A background descendant keeps the pipe busy without end.
                    # Flush what is already buffered, then return the result:
                    # the foreground output is ahead of any background output.
                    flush_available(1 << 20)
                    flushed = True
                    state[field + "_flushed"].set()
                    break
            try:
                ready = select.select([source], [], [], POLL_SECONDS)[0]
            except (OSError, ValueError):
                break
            if not ready:
                if state["foreground_done"]:
                    # Every byte the foreground process wrote is already in the
                    # pipe when it exits; an empty poll means this stream is
                    # fully delivered.
                    flushed = True
                    state[field + "_flushed"].set()
                    break
                continue
            try:
                chunk = os.read(fd, 65536)
            except BlockingIOError:
                continue
            except OSError:
                break
            if not chunk:
                break
            if not forward(chunk):
                flushed = True
                state[field + "_flushed"].set()
                break
        if flushed:
            # Discard later output from background descendants silently. Keep
            # the read end open (no SIGPIPE) without polling the kernel.
            os.set_blocking(fd, True)
            while True:
                try:
                    chunk = os.read(fd, 65536)
                except OSError:
                    break
                if not chunk:
                    break
    finally:
        if not flushed:
            state[field + "_flushed"].set()
        try:
            source.close()
        except OSError:
            pass


def terminate(process):
    try:
        os.killpg(process.pid, signal.SIGKILL)
    except (ProcessLookupError, PermissionError):
        pass


def client_closed(connection):
    try:
        # A zero timeout leaves the socket blocking, so this never races with a
        # concurrent sendall from the drain threads.
        ready = select.select([connection], [], [], 0)[0]
    except (OSError, ValueError):
        return True
    if not ready:
        return False
    try:
        data = connection.recv(1, socket.MSG_PEEK)
        return data == b""
    except BlockingIOError:
        return False
    except OSError:
        return True


def owned_writers():
    processes = {}
    for name in os.listdir('/proc'):
        if not name.isdigit() or int(name) == os.getpid():
            continue
        try:
            with open('/proc/' + name + '/stat') as source:
                status = source.read().rsplit(')', 1)[1].split()
            processes[int(name)] = (int(status[1]), status[0])
        except (OSError, IndexError):
            pass
    owned = {os.getpid()}
    while True:
        added = {pid for pid, (parent, _) in processes.items() if parent in owned} - owned
        if not added:
            break
        owned.update(added)
    return [pid for pid in owned if pid != os.getpid() and processes[pid][1] != 'Z']


def serve(connection):
    state = {
        "finished": False,
        "foreground_done": False,
        "lock": threading.Lock(),
        "stdout_flushed": threading.Event(),
        "stderr_flushed": threading.Event(),
    }
    process = None
    spool = None
    try:
        connection.settimeout(30)
        with connection.makefile('rb') as reader:
            # Only the target holder owns this upload file. It is never named
            # in the target workspace and cannot survive a failed request.
            spool = tempfile.TemporaryFile(dir=os.path.dirname(SOCKET))
            request, length = receive_request(reader, spool)
        operation = request.get('op')
        validate_body(operation, request, length)
        if request.get("op") == "drain":
            # Only descendants owned by this holder can be writers. Detached
            # descendants are adopted via subreaper; unrelated PIDs are ignored.
            while True:
                # Do not let the passive drain overtake cancellation's receipt
                # and stop this holder before its cancelling client is answered.
                with writer_lock:
                    children = owned_writers()
                    if request.get('cancel'):
                        while children:
                            for pid in children:
                                try:
                                    os.kill(pid, signal.SIGKILL)
                                except ProcessLookupError:
                                    pass
                            time.sleep(POLL_SECONDS)
                            children = owned_writers()
                    if not children:
                        send_event(connection, {"exit_code": 0})
                        return
                time.sleep(POLL_SECONDS)
            return
        if operation in ("read_file", "write_file", "replace_text", "list_directory"):
            file_operation(connection, operation, request, spool, length)
            return
        if operation != "exec":
            raise RuntimeError("unknown holder operation")
        environment = dict(os.environ)
        environment.update(request.get("env") or {})
        process = subprocess.Popen(
            ["/bin/bash", "-c", request["command"]],
            cwd=os.path.join(request.get('bound_cwd') or '/', request.get('cwd') or '.'),
            env=environment,
            stdin=spool if length is not None else subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            start_new_session=True,
            close_fds=True,
        )
        spool.close()
        for source, field in ((process.stdout, "stdout"), (process.stderr, "stderr")):
            threading.Thread(
                target=drain, args=(source, connection, state, field), daemon=True
            ).start()
        timeout = request.get("timeout")
        deadline = (
            time.monotonic() + timeout
            if isinstance(timeout, (int, float)) and timeout > 0
            else None
        )
        failure = None
        while process.poll() is None:
            if deadline is not None and time.monotonic() >= deadline:
                failure = "Tool timed out"
                terminate(process)
                break
            if client_closed(connection):
                failure = "Tool cancelled"
                terminate(process)
                break
            time.sleep(POLL_SECONDS)
        process.wait()
        # The foreground process can no longer write. Deliver every byte it
        # already produced before returning the result; a background descendant
        # keeps its pipes open and must not delay or truncate the result.
        state["foreground_done"] = True
        state["stdout_flushed"].wait()
        state["stderr_flushed"].wait()
        with state["lock"]:
            state["finished"] = True
            if failure is None:
                send_event(connection, {"exit_code": process.returncode})
            else:
                send_event(connection, {"error": failure, "error_category": "timed_out" if failure == "Tool timed out" else "cancelled"})
    except Exception as error:  # noqa: BLE001 - report every holder failure
        if process is not None and process.poll() is None:
            terminate(process)
            try:
                process.wait(timeout=1)
            except subprocess.TimeoutExpired:
                pass
        state["foreground_done"] = True
        with state["lock"]:
            state["finished"] = True
            category = 'not_found' if isinstance(error, FileNotFoundError) else 'permission_denied' if isinstance(error, PermissionError) else 'invalid_input' if isinstance(error, (ValueError, TypeError, KeyError, UnicodeError)) else 'io_error'
            send_event(connection, {"error": str(error), "error_category": category})
    finally:
        try:
            if spool is not None:
                spool.close()
            connection.close()
        except OSError:
            pass


while True:
    client, _ = listener.accept()
    threading.Thread(target=serve, args=(client,), daemon=True).start()

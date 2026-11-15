"""Minimal namespace holder.

The process only keeps its branch mount namespace alive and executes tool
commands inside it.  It is not an Agent and it owns no transcript or policy.
Every connection is handled concurrently; a command runs in its own session so
an aborted command can be terminated without killing the holder or background
work started by an earlier command.
"""

import base64
import json
import os
import select
import signal
import socket
import subprocess
import sys
import threading
import time

SOCKET = sys.argv[1]
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


def send_event(connection, payload):
    try:
        connection.sendall((json.dumps(payload) + "\n").encode())
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
            if send_event(connection, {field: base64.b64encode(chunk).decode("ascii")}):
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


def serve(connection):
    state = {
        "finished": False,
        "foreground_done": False,
        "lock": threading.Lock(),
        "stdout_flushed": threading.Event(),
        "stderr_flushed": threading.Event(),
    }
    process = None
    try:
        reader = connection.makefile("rb")
        line = reader.readline()
        if not line:
            return
        request = json.loads(line)
        if request.get("op") != "exec":
            raise RuntimeError("unknown holder operation")
        environment = dict(os.environ)
        environment.update(request.get("env") or {})
        raw_input = request.get("input")
        process = subprocess.Popen(
            ["/bin/bash", "-c", request["command"]],
            cwd=request.get("cwd") or "/",
            env=environment,
            stdin=subprocess.PIPE if raw_input is not None else subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            start_new_session=True,
            close_fds=True,
        )
        if raw_input is not None:

            def feed():
                try:
                    process.stdin.write(base64.b64decode(raw_input))
                    process.stdin.close()
                except (BrokenPipeError, OSError):
                    pass

            threading.Thread(target=feed, daemon=True).start()
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
                send_event(connection, {"error": failure})
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
            send_event(connection, {"error": str(error)})
    finally:
        try:
            connection.close()
        except OSError:
            pass


while True:
    client, _ = listener.accept()
    threading.Thread(target=serve, args=(client,), daemon=True).start()

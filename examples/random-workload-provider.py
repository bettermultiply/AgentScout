"""Local scripted model interface for the random workload, NOT a real LLM.

Use the temporary Pi provider configuration described in reference-workload.md.
Only the model interface is scripted: Pi executes all bash commands normally.
"""

import json
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

COMMAND = "sleep 6; python3 -c 'import secrets; print(secrets.choice([\"example\", \"true\", \"false\"]), end=\"\")'"
LOCK = threading.Lock()
STARTS = {}
COUNTS = {}


class Handler(BaseHTTPRequestHandler):
    def do_POST(self):
        data = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        messages = data["messages"]
        user = max(i for i, message in enumerate(messages) if message["role"] == "user")
        results = [message for message in messages[user:] if message["role"] == "tool"]
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.end_headers()

        def chunk(delta, finish=None):
            payload = {"id": "random-workload", "object": "chat.completion.chunk", "created": 1,
                       "model": "boundary-check", "choices": [{"index": 0, "delta": delta, "finish_reason": finish}]}
            self.wfile.write(("data: " + json.dumps(payload) + "\n\n").encode())
            self.wfile.flush()

        try:
            chunk({"role": "assistant"})
            if not results:
                operation = "random-" + str(time.time_ns())
                with LOCK:
                    STARTS[operation] = time.monotonic()
                    COUNTS[operation] = 0
                command = COMMAND
            elif len(results) == 1:
                operation = results[0]["tool_call_id"]
                with LOCK:
                    elapsed = time.monotonic() - STARTS[operation]
                    COUNTS[operation] += 1
                    index = COUNTS[operation]
                # The example delay separates the two speculative model requests
                # from Parent's request. This is a demonstration, not routing logic.
                if elapsed < 6:
                    value = results[0]["content"]
                    assert value in ("example", "true", "false"), value
                    command = f"sleep 1; printf 'Predicted{index}: {value}'"
                else:
                    command = "sleep 1; printf 'spec fail'"
                operation = "continuation-" + str(time.time_ns())
            else:
                chunk({"content": str(results[-1]["content"])})
                chunk({}, "stop")
                self.wfile.write(b"data: [DONE]\n\n")
                self.wfile.flush()
                return
            print(json.dumps({"operation": operation, "command": command}), flush=True)
            chunk({"tool_calls": [{"index": 0, "id": operation, "type": "function",
                                   "function": {"name": "bash", "arguments": json.dumps({"command": command})}}]})
            chunk({}, "tool_calls")
            self.wfile.write(b"data: [DONE]\n\n")
            self.wfile.flush()
        except (BrokenPipeError, ConnectionResetError):
            pass


if __name__ == "__main__":
    ThreadingHTTPServer(("127.0.0.1", 18764), Handler).serve_forever()

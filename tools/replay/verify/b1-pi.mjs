#!/usr/bin/env node
import assert from "node:assert/strict";
import fs from "node:fs";
import net from "node:net";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { createRequire } from "node:module";
import { spawn } from "node:child_process";
import { requestKey } from "../normalize.mjs";

// The parent needs only Node. The child loads the existing Pi source/dependencies.
const pi = process.env.PI_ROOT || "/home/betmul/BETMUL/AgentScout-suit/pi";
const expectedCalls = [
  { type: "toolCall", id: "call_alpha", name: "alpha", arguments: { x: 1, nested: { z: "甲", a: true } } },
  { type: "toolCall", id: "call_beta", name: "beta", arguments: { values: [2, "乙"], y: false } },
];
const user = { role: "user", content: "Run alpha and beta in parallel.", timestamp: 0 };
const toolResults = expectedCalls.map((call, i) => ({ role: "toolResult", toolCallId: call.id, toolName: call.name, content: [{ type: "text", text: i ? "result beta" : "result alpha" }], isError: false, timestamp: 0 }));

if (process.argv[2] === "--pi-child") {
  const root = process.argv[3];
  const require = createRequire(path.join(pi, "package.json"));
  const { ModelRuntime } = await import(pathToFileURL(path.join(pi, "packages/coding-agent/src/core/model-runtime.ts")));
  const { resolveCliModel } = await import(pathToFileURL(path.join(pi, "packages/coding-agent/src/core/model-resolver.ts")));
  const { stream } = require(path.join(pi, "packages/ai/src/api/openai-completions.ts"));
  const runtime = await ModelRuntime.create({ modelsPath: path.join(root, "models.json"), authPath: path.join(root, "auth.json") });
  const resolved = resolveCliModel({ cliModel: "replay/fixture", cliThinking: "off", modelRuntime: runtime });
  assert.ok(!resolved.error && resolved.model, resolved.error);
  assert.equal(resolved.model.api, "openai-completions");
  const tools = expectedCalls.map(c => ({ name: c.name, description: c.name, parameters: { type: "object", properties: {} } }));
  const first = await stream(resolved.model, { messages: [user], tools }, { apiKey: "dummy", maxRetries: 0 }).result();
  assert.equal(first.stopReason, "toolUse", first.errorMessage);
  assert.deepEqual(first.content.filter(p => p.type === "toolCall"), expectedCalls, "Pi must parse two separate calls with exact id/name/arguments");
  console.log("B1-PI calls PASS: two separate exact id/name/arguments, text between calls");
  const second = await stream(resolved.model, { messages: [user, first, ...toolResults], tools }, { apiKey: "dummy", maxRetries: 0 }).result();
  assert.equal(second.stopReason, "stop", second.errorMessage);
  assert.deepEqual(second.content, [{ type: "text", text: "parallel continuation hit" }]);
  fs.writeFileSync(path.join(root, "parsed.json"), JSON.stringify({ first, second }));
  console.log("B1-PI continuation PASS: parsed assistant + both parallel tool results hit next recording");
} else {
  const here = path.dirname(fileURLToPath(import.meta.url));
  fs.mkdirSync("/tmp/opencode", { recursive: true });
  const root = fs.mkdtempSync("/tmp/opencode/ascout-replay-b1-pi-");
  const children = [], ports = [];
  const deadline = async (promise, ms, label) => {
    let timer;
    try { return await Promise.race([promise, new Promise((_, reject) => { timer = setTimeout(() => reject(Error(label)), ms); })]); }
    finally { clearTimeout(timer); }
  };
  const start = (args, options = {}) => {
    const child = spawn(process.execPath, args, { stdio: ["ignore", "pipe", "pipe"], ...options });
    child.output = ""; child.errors = "";
    child.stdout.on("data", b => child.output += b);
    child.stderr.on("data", b => child.errors += b);
    child.done = new Promise((resolve, reject) => { child.once("error", reject); child.once("exit", code => resolve(code)); });
    // Keep startup errors observed even when the caller is still waiting for stdout.
    child.done.catch(() => {});
    children.push(child); return child;
  };
  const stop = async child => {
    if (child.exitCode !== null || child.signalCode !== null) return;
    child.kill("SIGTERM");
    try { await deadline(child.done, 5000, "SIGTERM timeout"); }
    catch { child.kill("SIGKILL"); await deadline(child.done, 5000, "SIGKILL timeout"); }
  };
  const rows = name => fs.readFileSync(path.join(root, name), "utf8").trim().split(/\n/).filter(Boolean).map(JSON.parse);
  const request = async (port, messages) => {
    const response = await fetch(`http://127.0.0.1:${port}/v1/chat/completions`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ model: "fixture", messages, stream: true }), signal: AbortSignal.timeout(5000) });
    await response.text(); return response.status;
  };
  const startServer = async (recording, suffix) => {
    const server = start([process.env.REPLAY_VERIFY_SERVE || path.join(here, "../serve.mjs"), path.join(root, recording), "0"], { env: { ...process.env, REPLAY_REPORT: path.join(root, `${suffix}-report.json`), REPLAY_REQUEST_LOG: path.join(root, `${suffix}-requests.jsonl`) } });
    const ready = new Promise((resolve, reject) => {
      server.stdout.on("data", () => { const match = server.output.match(/endpoint http:\/\/127\.0\.0\.1:(\d+)\/v1/); if (match) resolve(Number(match[1])); });
      server.done.then(code => reject(Error(`serve exited ${code}: ${server.errors}`)), reject);
    });
    const port = await deadline(ready, 10000, "serve startup timeout"); ports.push(port); return { server, port };
  };
  const failures = [];
  try {
    const stamp = "2026-11-12T00:00:00.000Z";
    const usage = { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, totalTokens: 0, cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 } };
    const assistant = { role: "assistant", api: "openai-completions", provider: "replay", model: "fixture", usage, stopReason: "toolUse", timestamp: 0, content: [expectedCalls[0], { type: "text", text: "between calls" }, expectedCalls[1]] };
    const entry = (type, id, parentId, more = {}) => ({ type, id, parentId, timestamp: stamp, ...more });
    const session = [
      { type: "session", version: 3, id: "fixture-session", timestamp: stamp, cwd: root },
      entry("message", "system", null, { message: { role: "system", content: "local fixture", timestamp: 0 } }),
      entry("model_change", "model", "system", { provider: "replay", modelId: "fixture" }),
      entry("thinking_level_change", "thinking", "model", { thinkingLevel: "off" }),
      entry("message", "user", "thinking", { message: user }),
      // An interleaved sibling user must not replace the parent-chain context.
      entry("message", "sibling", "thinking", { message: { role: "user", content: "wrong sibling", timestamp: 0 } }),
      entry("custom", "authority", "user", { customType: "authority", data: { branch_id: "authority", parentId: "user" } }),
      entry("message", "calls", "authority", { message: assistant }),
      entry("message", "tool-a", "calls", { message: toolResults[0] }),
      entry("custom", "state", "tool-a", { customType: "authority", data: { branch_id: "authority", parentId: "tool-a" } }),
      entry("message", "tool-b", "state", { message: toolResults[1] }),
      entry("message", "continuation", "tool-b", { message: { ...assistant, stopReason: "stop", content: [{ type: "text", text: "parallel continuation hit" }] } }),
      entry("custom", "tip", "continuation", { customType: "authority", data: { branch_id: "authority", parentId: "continuation" } }),
    ];
    fs.writeFileSync(path.join(root, "session.jsonl"), session.map(JSON.stringify).join("\n") + "\n");
    fs.writeFileSync(path.join(root, "events.jsonl"), ["calls", "continuation"].map(session_entry_id => JSON.stringify({ message: "transcript", session_entry_id, branch_id: "authority" })).join("\n") + "\n");
    const exporter = start([path.join(here, "../export.mjs"), root]);
    assert.equal(await deadline(exporter.done, 10000, "export timeout"), 0, exporter.errors);
    const recording = JSON.parse(fs.readFileSync(path.join(root, "recording.json")));
    assert.equal(recording.length, 2); assert.deepEqual(recording.map(r => r.branch_id), ["authority", "authority"]);
    assert.deepEqual(recording[0].reply, assistant.content);
    assert.equal(recording[0].key, requestKey([user]));
    assert.equal(recording[1].key, requestKey([user, assistant, ...toolResults]));
    const { server, port } = await startServer("recording.json", "pi");
    fs.writeFileSync(path.join(root, "models.json"), JSON.stringify({ providers: { replay: { baseUrl: `http://127.0.0.1:${port}/v1`, api: "openai-completions", apiKey: "dummy", models: [{ id: "fixture", reasoning: false, input: ["text"], contextWindow: 8192, maxTokens: 256 }] } } }));
    const probe = start(["--import", path.join(pi, "packages/coding-agent/src/experimental/source-resolver.ts"), fileURLToPath(import.meta.url), "--pi-child", root], { cwd: pi, env: { ...process.env, PI_CODING_AGENT_DIR: root } });
    const code = await deadline(probe.done, 30000, "Pi probe timeout");
    process.stdout.write(probe.output);
    if (code !== 0) failures.push(`Pi probe exited ${code}:\n${probe.errors}`);
    else {
      const requests = rows("pi-requests.jsonl");
      assert.equal(requests.length, 2);
      assert.deepEqual(requests.map(r => r.key), recording.map(r => r.key), "real Pi HTTP keys must equal exported keys");
      assert.equal(requests[1].messages.filter(m => m.role === "tool").length, 2);
      const report = JSON.parse(fs.readFileSync(path.join(root, "pi-report.json")));
      assert.equal(report.hits, 2); assert.equal(report.unconsumed, 0);
      console.log(`B1-PI HTTP keys PASS: ${requests.map(r => r.key).join(", ")}`);
    }
    await stop(server);

    // Separate HTTP checks isolate report accounting, including keys never requested
    // and remaining rows for a partially consumed key. Pi exclusively drives the above loop.
    try {
      const contexts = Object.fromEntries(["unrequested", "partial", "exhausted", "unknown"].map(s => [s, [{ role: "user", content: s }]]));
      const keys = Object.fromEntries(Object.entries(contexts).map(([s, ms]) => [s, requestKey(ms)]));
      const reportRecording = ["unrequested", "unrequested", "partial", "partial", "partial", "exhausted"].map(s => ({ key: keys[s], reply: [{ type: "text", text: s }] }));
      fs.writeFileSync(path.join(root, "accounting.json"), JSON.stringify(reportRecording));
      const accounting = await startServer("accounting.json", "accounting");
      for (const [name, status] of [["partial", 200], ["exhausted", 200], ["exhausted", 400], ["exhausted", 400], ["unknown", 400], ["unknown", 400], ["unknown", 400]]) assert.equal(await request(accounting.port, contexts[name]), status);
      await stop(accounting.server);
      const report = JSON.parse(fs.readFileSync(path.join(root, "accounting-report.json")));
      assert.deepEqual(report.missing_keys, { [keys.exhausted]: 2, [keys.unknown]: 3 }, "missing_keys must count only failed requests, including exhaustion");
      assert.deepEqual(report.unconsumed_keys, { [keys.unrequested]: 2, [keys.partial]: 2 });
      assert.deepEqual(Object.fromEntries(["total_requests", "hits", "misses", "exhausted", "consumed", "unconsumed"].map(k => [k, report[k]])), { total_requests: 7, hits: 2, misses: 3, exhausted: 2, consumed: 2, unconsumed: 4 });
      assert.deepEqual(report.requests, { [keys.partial]: 1, [keys.exhausted]: 3, [keys.unknown]: 3 });
      assert.equal(Object.values(report.unconsumed_keys).reduce((a, b) => a + b, 0), report.unconsumed);
      console.log("B1-PI report PASS: missing_keys={exhausted:2,unknown:3}; unconsumed_keys={unrequested:2,partial:2}; totals=7/2/3/2/2/4");
    } catch (error) { failures.push(error.stack); }
    assert.deepEqual(failures, [], failures.join("\n"));
    console.log("B1-PI PASS: exported parent-chain fixture → real Pi stream → parallel continuation; separate report accounting");
  } finally {
    await Promise.all(children.map(stop));
    assert.ok(children.every(c => c.exitCode !== null || c.signalCode !== null), "child process survived cleanup");
    for (const port of ports) {
      await new Promise((resolve, reject) => {
        const socket = net.connect({ host: "127.0.0.1", port }); socket.setTimeout(1000);
        socket.once("error", error => { socket.destroy(); error.code === "ECONNREFUSED" ? resolve() : reject(error); });
        socket.once("connect", () => { socket.destroy(); reject(Error(`port ${port} survived cleanup`)); });
        socket.once("timeout", () => { socket.destroy(); reject(Error(`port ${port} cleanup check timeout`)); });
      });
    }
    console.log(`B1-PI cleanup PASS: ${children.length} child processes exited; ports ${ports.join(",")} closed; artifacts ${root}`);
  }
}

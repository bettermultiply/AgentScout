#!/usr/bin/env node
import http from "node:http";
import fs from "node:fs";
import crypto from "node:crypto";
import { pathToFileURL } from "node:url";
import { loadRecording } from "./recording.mjs";
import { requestKey } from "./normalize.mjs";

export function createReplayServer(recording, { reportPath, requestLog, onFailure = () => {}, cancelTimeoutMs = 30000 } = {}) {
if (!Number.isFinite(cancelTimeoutMs) || cancelTimeoutMs <= 0) throw Error("cancelTimeoutMs must be positive and finite");
const used = new Array(recording.length).fill(false); const counts = new Map();
let hits = 0, misses = 0, exhausted = 0, pending = 0, cancelled = 0, cancelTimeouts = 0;
function report() {
  const missing = {};
  for (const [key, n] of counts) if (!recording.some(r => r.key === key)) missing[key] = n;
  for (const [key, n] of counts) {
    const available = recording.filter(r => r.key === key).length;
    if (available > 0 && n > available) missing[key] = n - available;
  }
  const unconsumed = {};
  recording.forEach((r, i) => {
    if (!used[i]) unconsumed[r.key] = (unconsumed[r.key] || 0) + 1;
  });
   const status = { total_requests: hits + misses + exhausted, hits, misses, exhausted, pending, cancelled, cancel_timeouts: cancelTimeouts, consumed: used.filter(Boolean).length, unconsumed: used.filter(x => !x).length, missing_keys: missing, unconsumed_keys: unconsumed, requests: Object.fromEntries(counts) };
  if (reportPath) fs.writeFileSync(reportPath, JSON.stringify(status, null, 2) + "\n");
  return status;
}
const server = http.createServer((req, res) => {
  if (req.method !== "POST" || !req.url?.endsWith("/chat/completions")) { res.writeHead(404); return res.end(); }
  req.setEncoding("utf8");
  let body = ""; req.on("data", c => body += c); req.on("end", () => {
    let json; try { json = JSON.parse(body); } catch { return fail(res, "invalid JSON"); }
    const key = requestKey(json.messages);
    if (requestLog) fs.appendFileSync(requestLog, JSON.stringify({key,messages:json.messages}) + "\n");
    const index = recording.findIndex((r, i) => r.key === key && !used[i]);
    counts.set(key, (counts.get(key) || 0) + 1);
    if (index < 0) {
      if (recording.some(r => r.key === key)) exhausted++; else misses++;
      report();
      return fail(res, `missing or exhausted replay key ${key}`);
    }
    hits++;
    used[index] = true; const item = recording[index], reply = item.reply || [];
    pending++;
    let timer, settled = false;
    const settle = () => {
      if (settled) return false;
      settled = true; clearTimeout(timer); pending--;
      res.off("close", close);
      return true;
    };
    const close = () => { if (settle()) { cancelled++; report(); } };
    res.once("close", close);
    if (res.destroyed) { close(); return; }
    report();
    timer = setTimeout(() => {
    if (!settle()) return;
    if (res.destroyed) { cancelled++; report(); return; }
    if (item.stopReason === "aborted") {
      cancelTimeouts++; report();
      return fail(res, `replay cancellation timed out for key ${key} after ${cancelTimeoutMs}ms`, 504, "replay_cancel_timeout");
    }
    report();
    res.writeHead(200, {"content-type":"text/event-stream", "cache-control":"no-cache", connection:"keep-alive"});
    const id = "replay-" + crypto.randomBytes(6).toString("hex");
    let toolIndex = 0;
    for (const p of reply) {
      const delta = p.type === "text" ? {content:p.text} : {tool_calls:[{index:toolIndex++,id:p.id,type:"function",function:{name:p.name,arguments:JSON.stringify(p.arguments ?? {})}}]};
      res.write(`data: ${JSON.stringify({id,object:"chat.completion.chunk",created:0,model:json.model||"replay/fixture",choices:[{index:0,delta}]})}\n\n`);
    }
    const tool = reply.some(p => p.type === "toolCall");
     res.write(`data: ${JSON.stringify({id,object:"chat.completion.chunk",created:0,model:json.model||"replay/fixture",choices:[{index:0,delta:{},finish_reason:tool?"tool_calls":"stop"}]})}\n\ndata: [DONE]\n\n`); res.end();
    }, item.stopReason === "aborted" ? cancelTimeoutMs : 500);
  });
});
function fail(res, message, status = 400, type = "replay_miss") { onFailure(message); if (res.destroyed) return; res.writeHead(status, {"content-type":"application/json"}); res.end(JSON.stringify({error:{type,message}})); }
return { server, report };
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const source = process.argv[2];
  if (!source) throw Error("usage: node tools/replay/serve.mjs SOURCE_RUN_OR_RECORDING [port]");
  const native = fs.statSync(source).isDirectory();
  const recording = native ? loadRecording(source).recording : JSON.parse(fs.readFileSync(source, "utf8"));
  const {server, report} = createReplayServer(recording, {
    reportPath: process.env.REPLAY_REPORT || (native ? undefined : source.replace(/recording\.json$/, "report.json")),
    requestLog: process.env.REPLAY_REQUEST_LOG,
  });
  process.on("exit", report);
  for (const signal of ["SIGTERM", "SIGINT"]) process.on(signal, () => { report(); server.close(); server.closeAllConnections(); });
  server.listen(Number(process.argv[3] || process.env.PORT || 0), "127.0.0.1", () => console.log(`replay endpoint http://127.0.0.1:${server.address().port}/v1`));
}

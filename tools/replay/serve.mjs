#!/usr/bin/env node
import http from "node:http";
import fs from "node:fs";
import crypto from "node:crypto";
import { requestKey } from "./normalize.mjs";

const recordingPath = process.argv[2];
if (!recordingPath) { console.error("usage: node tools/replay/serve.mjs recording.json [port]"); process.exit(2); }
const recording = JSON.parse(fs.readFileSync(recordingPath, "utf8"));
const used = new Array(recording.length).fill(false); const counts = new Map();
let hits = 0, misses = 0, exhausted = 0;
const reportPath = process.env.REPLAY_REPORT || recordingPath.replace(/recording\.json$/, "report.json");
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
  fs.writeFileSync(reportPath, JSON.stringify({ total_requests: hits + misses + exhausted, hits, misses, exhausted, consumed: used.filter(Boolean).length, unconsumed: used.filter(x => !x).length, missing_keys: missing, unconsumed_keys: unconsumed, requests: Object.fromEntries(counts) }, null, 2) + "\n");
}
process.on("exit", report); process.on("SIGTERM", () => { report(); process.exit(0); });
const port = Number(process.argv[3] || process.env.PORT || 0);
const server = http.createServer((req, res) => {
  if (req.method !== "POST" || !req.url?.endsWith("/chat/completions")) { res.writeHead(404); return res.end(); }
  let body = ""; req.on("data", c => body += c); req.on("end", () => {
    let json; try { json = JSON.parse(body); } catch { return fail(res, "invalid JSON"); }
    const key = requestKey(json.messages);
    if (process.env.REPLAY_REQUEST_LOG) fs.appendFileSync(process.env.REPLAY_REQUEST_LOG, JSON.stringify({key,messages:json.messages}) + "\n");
    const index = recording.findIndex((r, i) => r.key === key && !used[i]);
    counts.set(key, (counts.get(key) || 0) + 1);
    if (index < 0) {
      if (recording.some(r => r.key === key)) exhausted++; else misses++;
      report();
      return fail(res, `missing or exhausted replay key ${key}`);
    }
    hits++;
    used[index] = true; const reply = recording[index].reply || [];
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
  });
});
function fail(res, message) { res.writeHead(400, {"content-type":"application/json"}); res.end(JSON.stringify({error:{type:"replay_miss",message}})); }
server.listen(port, "127.0.0.1", () => console.log(`replay endpoint http://127.0.0.1:${server.address().port}/v1`));

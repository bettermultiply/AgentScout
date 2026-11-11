#!/usr/bin/env node
import fs from "node:fs";
import path from "node:path";
import { messageBody, requestKey, replyParts } from "./normalize.mjs";

const run = process.argv[2];
if (!run) { console.error("usage: node tools/replay/export.mjs RUN_DIR [OUTPUT]"); process.exit(2); }
const read = name => fs.readFileSync(path.join(run, name), "utf8").split(/\r?\n/).filter(Boolean).map(JSON.parse);
const session = read("session.jsonl");
let events = [];
try { events = read("events.jsonl"); } catch { /* event file is optional for branch lookup */ }
const branchByEntry = new Map();
for (const e of events) {
  const x = e.transcript ?? (e.message === "transcript" ? e : e);
  const id = x.session_entry_id ?? x.sessionEntryId;
  const branch = x.branch_id ?? x.branchId;
  if (id && branch) branchByEntry.set(id, branch);
}
const out = []; let missing = 0; let skippedAborted = 0; let skippedError = 0;
const byId = new Map(session.map((line) => [line.id ?? line.session_entry_id ?? line.sessionEntryId, line]));
const linearContext = [];
function contextFor(line) {
  if (!(line?.parentId ?? line?.parent_id)) return linearContext.slice();
  const chain = []; let current = line?.parentId ?? line?.parent_id;
  const seen = new Set();
  while (current && !seen.has(current)) {
    seen.add(current); const parent = byId.get(current); if (!parent) break;
    const message = messageBody(parent); if (message?.role) chain.push(message);
    current = parent.parentId ?? parent.parent_id;
  }
  return chain.reverse();
}
for (const line of session) {
  const m = messageBody(line);
  if (!m?.role) continue;
  if (m.role === "assistant") {
    const stopReason = m.stopReason ?? m.stop_reason;
    const errorMessage = m.errorMessage ?? m.error_message;
    if (stopReason === "aborted") { skippedAborted++; continue; }
    if (stopReason === "error" || errorMessage != null) { skippedError++; continue; }
    const entry = line.session_entry_id ?? line.sessionEntryId ?? line.id ?? line.entry_id;
    const branch = entry && branchByEntry.get(entry);
    if (!branch) missing++;
    out.push({ key: requestKey(contextFor(line)), reply: replyParts(m), branch_id: branch ?? null, session_entry_id: entry ?? null });
  }
  linearContext.push(m);
}
const destination = process.argv[3] || path.join(run, "recording.json");
fs.writeFileSync(destination, JSON.stringify(out, null, 2) + "\n");
if (missing) console.error(`warning: ${missing} assistant entries had no transcript branch_id`);
console.error(`skipped assistant records: aborted=${skippedAborted}, error=${skippedError}, total=${skippedAborted + skippedError}`);

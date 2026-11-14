/** Read native session/events without writing to the source. */
import fs from "node:fs";
import path from "node:path";
import { requestKey, replyParts, sessionContext } from "./normalize.mjs";

export function loadRecording(run) {
const read = name => fs.readFileSync(path.join(run, name), "utf8").split(/\r?\n/).filter(Boolean).map(JSON.parse);
const session = read("session.jsonl"), events = read("events.jsonl");
const entries = session.filter(e => e.type !== "session");
const byId = new Map(entries.map(e => [e.id, structuredClone(e)]));
if (byId.size !== entries.length || byId.has(undefined)) throw Error("Missing or duplicate native session ID");
const owner = new Map(), transcripts = new Map(), candidates = new Map(), chances = new Map(), promoted = new Set();
// File order is authoritative. t_ms restarts after recovery.
for (const e of events) {
  if (e.observation?.event === "chance_created") chances.set(e.observation.chance_id, e.observation);
  if (e.message === "candidate_created") {
    if (candidates.has(e.branch_id)) throw Error(`Duplicate Candidate ${e.branch_id}`);
    candidates.set(e.branch_id, e);
  }
  if (e.operation === "promote" && e.status === "applied") promoted.add(e.branch_id);
  if (e.message !== "transcript") continue;
  const id = e.session_entry_id, branch = e.branch_id;
  if (!id || !branch || !byId.has(id)) throw Error(`Invalid transcript reference ${id}`);
  if (owner.has(id)) {
    if (owner.get(id) !== branch) throw Error(`Ambiguous transcript owner ${id}`);
    continue;
  }
  owner.set(id, branch);
  const list = transcripts.get(branch) ?? []; list.push(id); transcripts.set(branch, list);
}
function chain(id) {
  const result = [], seen = new Set();
  while (id) {
    if (seen.has(id)) throw Error(`Cyclic native history at ${id}`);
    seen.add(id);
    const e = byId.get(id); if (!e) throw Error(`Missing native parent ${id}`);
    result.unshift(e); id = e.parentId;
  }
  return result;
}
// Injected batches retain their original links. Restore only the first edge
// after the batch; it can belong to a custom/compaction/edit entry, not a reply.
for (const [branch, c] of candidates) {
  const ids = transcripts.get(branch) ?? [];
  if (!ids.length) continue;
  const chance = chances.get(c.chance_id);
  if (!chance || chance.parent_branch_id !== c.parent) throw Error(`Missing Chance relation for ${branch}`);
  const first = byId.get(ids[0]);
  if (first.type !== "message" || first.message.role !== "toolResult") throw Error(`Missing injected input for ${branch}`);
  const checkpoint = byId.get(first.parentId), calls = checkpoint?.message?.content?.filter(p => p.type === "toolCall");
  if (checkpoint?.message?.role !== "assistant" || owner.get(checkpoint.id) !== c.parent || !calls?.some(p => p.id === chance.operation_id)) throw Error(`Invalid native checkpoint for ${branch}`);
  if (ids.length < calls.length) throw Error(`Incomplete injected batch for ${branch}`);
  let tail = checkpoint.id;
  for (let i = 0; i < calls.length; i++) {
    const input = byId.get(ids[i]);
    if (input.parentId !== tail || input.type !== "message" || input.message.role !== "toolResult" || input.message.toolCallId !== calls[i].id) throw Error(`Invalid injected batch for ${branch}`);
    tail = input.id;
  }
  if (ids.length === calls.length) continue;
  const continuation = chain(ids[calls.length]);
  if (continuation.some(e => e.id === tail)) continue;
  if (!promoted.has(branch)) throw Error(`Unexplained detached continuation for ${branch}`);
  const ancestors = new Set(chain(checkpoint.id).map(e => e.id));
  let index = continuation.length - 1;
  while (index >= 0 && !ancestors.has(continuation[index].id) && owner.get(continuation[index].id) !== c.parent) {
    const e = continuation[index], branchOwner = owner.get(e.id);
    if (branchOwner && branchOwner !== branch) throw Error(`Ambiguous continuation ${e.id} for ${branch}`);
    if (e.type === "custom_message" && e.customType === "notification" && !events.some(x => x.operation === "promote" && x.branch_id === branch && x.command_id === e.details?.command_id && x.status === "applied")) throw Error(`Unlinked notification ${e.id}`);
    index--;
  }
  const source = continuation[index], moved = continuation[index + 1];
  if (!moved || source?.message?.role !== "toolResult" || owner.get(source.id) !== c.parent) throw Error(`Cannot identify Promote edge for ${branch}`);
  const sourcePath = chain(source.id), start = sourcePath.findIndex(e => e.id === checkpoint.id);
  const results = sourcePath.slice(start + 1).filter(e => e.type === "message");
  const operations = new Set(results.map(e => e.message.toolCallId));
  if (start < 0 || results.length !== calls.length || operations.size !== calls.length || results.some(e => e.message.role !== "toolResult") || calls.some(c => !operations.has(c.id))) throw Error(`Ambiguous Promote source batch for ${branch}`);
  moved.parentId = tail;
}
const out = []; let aborted = 0, skippedError = 0;
for (const line of entries) {
  const m = line.type === "message" ? line.message : null;
  if (m?.role !== "assistant") continue;
   const cancelled = m.stopReason === "aborted";
   if (!cancelled && (m.stopReason === "error" || m.errorMessage != null)) { skippedError++; continue; }
  const branch = owner.get(line.id);
  if (!branch) throw Error(`Missing transcript branch for assistant ${line.id}`);
  const context = sessionContext(chain(byId.get(line.id).parentId));
   if (cancelled) aborted++;
   out.push({ key: requestKey(context), ...(cancelled ? {stopReason:"aborted"} : {reply:replyParts(m)}), branch_id: branch, session_entry_id: line.id });
}
const inputs=[],consumed=new Set();
for(const event of events) {
  if(event.message!=='transcript'||consumed.has(event.session_entry_id))continue;
  const entry=byId.get(event.session_entry_id);
  if(entry?.type!=='message'||entry.message.role!=='user')continue;
  consumed.add(entry.id);
  const content=entry.message.content;
  if(typeof content!=='string'&&(!Array.isArray(content)||content.some(p=>p.type!=='text')))throw Error(`User input ${entry.id} requires unsupported non-text input`);
  inputs.push({id:entry.id,prompt:typeof content==='string'?content:content.map(p=>p.text).join('\n')});
}
return {recording:out,inputs,aborted,skippedError};
}

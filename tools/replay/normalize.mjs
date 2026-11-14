#!/usr/bin/env node
/** Dependency-free request normalization for external replay tools. */
import crypto from "node:crypto";

export function textOfContent(content) {
  if (typeof content === "string") return content;
  if (!Array.isArray(content)) return "";
  const text = content.filter((b) => b && b.type === "text").map((b) => b.text ?? "").join("\n");
  if (text) return text;
  if (content.some((b) => b && (b.type === "image" || b.type === "input_image"))) return "(see attached image)";
  return "(no tool output)";
}

export function stable(value) {
  if (Array.isArray(value)) return `[${value.map(stable).join(",")}]`;
  if (value && typeof value === "object") return `{${Object.keys(value).sort().map(k => `${JSON.stringify(k)}:${stable(value[k])}`).join(",")}}`;
  return JSON.stringify(value);
}

function argsOf(call) {
  const a = call?.function?.arguments ?? call?.arguments ?? {};
  if (typeof a !== "string") return a || {};
  try { return JSON.parse(a); } catch { return a; }
}
function callsOf(message) {
  if (Array.isArray(message?.tool_calls)) return message.tool_calls;
  return (message?.content ?? []).filter?.(p => p?.type === "toolCall") || [];
}

/** Matches the final context Pi sends to openai-completions. */
export function requestKey(messages) {
  const ms = (Array.isArray(messages) ? messages : []).map(m => m?.role === "custom" ? {...m, role:"user"} : m);
  let ai = -1;
  for (let i = ms.length - 1; i >= 0; i--) if (ms[i]?.role === "assistant") { ai = i; break; }
  const tail = ai >= 0 ? ms.slice(ai + 1) : [];
  const tools = tail.filter(m => m?.role === "tool" || m?.role === "toolResult");
  let material;
  if (tools.length) {
    const prior = ai >= 0 ? callsOf(ms[ai]) : [];
    const byId = new Map(prior.map(c => [c.id ?? c.toolCallId, c]));
    material = tools.map(t => {
      const id = t.tool_call_id ?? t.toolCallId;
      const call = byId.get(id) || {};
      return { name: call.function?.name ?? call.name ?? t.toolName ?? "", arguments: argsOf(call), result: textOfContent(t.content) };
    });
  } else {
    const user = [...ms].reverse().find(m => m?.role === "user");
    material = textOfContent(user?.content ?? "");
  }
  // Pi converts custom messages to user content before provider serialization.
  // Include that context after tool results.
  const users = tail.filter(m => m?.role === "user");
  if (tools.length && users.length) material = { material, users: users.map(m => textOfContent(m.content)) };
  return crypto.createHash("sha256").update(stable(material)).digest("hex");
}

export function replyParts(message) {
  const parts = [
    ...(Array.isArray(message?.content) ? message.content : (typeof message?.content === "string" ? [{type:"text", text:message.content}] : [])),
    ...(message?.tool_calls ?? []).map(c => ({type:"toolCall", id:c.id, name:c.function?.name, arguments:argsOf(c)})),
  ];
  return parts.filter(p => p?.type === "text" || p?.type === "toolCall").map(p => p.type === "toolCall" ? { type: "toolCall", id: p.id, name: p.name, arguments: p.arguments ?? {} } : { type: "text", text: p.text ?? "" });
}

/** Read a current native Pi message entry, never custom state data. */
export function messageBody(entry) {
  if (entry.type === "custom_message") return { role: "custom", customType: entry.customType, content: entry.content ?? [], display: entry.display, details: entry.details, timestamp: Date.parse(entry.timestamp) };
  return entry.type === "message" ? entry.message : {};
}

/** Current Pi buildSessionProjection + convertToLlm semantics on one native path. */
export function sessionContext(path) {
  const compaction = path.findLastIndex(e => e.type === "compaction");
  let selected = path;
  if (compaction >= 0) {
    const c = path[compaction], kept = path.findIndex(e => e.id === c.firstKeptEntryId);
    if (c.firstKeptEntryId && (kept < 0 || kept > compaction)) throw Error(`Missing compaction retained entry ${c.firstKeptEntryId}`);
    selected = [c, ...(kept >= 0 ? path.slice(kept, compaction).filter(e => e.message?.role !== "system") : []), ...path.slice(compaction + 1)];
  }
  const edits = new Map(selected.filter(e => e.type === "context_edit").map(e => [e.targetId, e]));
  const messages = [];
  for (const [i, e] of selected.entries()) {
    const edit = edits.get(e.id);
    if (edit?.replacement === null) continue;
    let projected = [];
    if (e.type === "message") projected = [e.message];
    if (e.type === "custom_message") projected = [{ role: "user", content: e.content ?? [] }];
    if (e.type === "branch_summary" && e.summary) projected = [{ role: "user", content: `The following is a summary of a branch that this conversation came back from:\n\n<summary>\n${e.summary}</summary>` }];
    if (e.type === "compaction" && i === 0) projected = [...(e.systemMessage ? [e.systemMessage] : []), { role: "user", content: `The conversation history before this point was compacted into the following summary:\n\n<summary>\n${e.summary}\n</summary>` }];
    for (let m of projected) {
      if (!["system", "user", "assistant", "toolResult"].includes(m.role)) throw Error(`Unsupported persisted message role ${m.role}`);
      if (edit && ["message", "custom_message"].includes(e.type) && ["user", "assistant", "toolResult", "custom"].includes(m.role)) m = { ...m, content: edit.replacement.content };
      if (m.role === "assistant" && ["aborted", "error"].includes(m.stopReason)) continue;
      if (m.role === "toolResult") {
        const previous = messages.findLastIndex(x => x.role === "toolResult" && x.toolCallId === m.toolCallId);
        const assistant = messages.findLastIndex(x => x.role === "assistant");
        if (previous > assistant) messages.splice(previous, 1);
      }
      messages.push(m);
    }
  }
  return messages;
}

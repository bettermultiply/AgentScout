#!/usr/bin/env node
/** Shared, dependency-free replay request normalization. */
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
  return (message?.toolCalls ?? message?.content ?? [])
    .filter?.(p => p && (p.type === "toolCall" || p.type === "tool_use")) || [];
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
      return { name: call.function?.name ?? call.name ?? t.name ?? "", arguments: argsOf(call), result: textOfContent(t.content ?? t.result ?? t.data) };
    });
  } else {
    const user = [...ms].reverse().find(m => m?.role === "user");
    material = textOfContent(user?.content ?? user?.text ?? "");
  }
  // Pi converts custom messages to user content before provider serialization.
  // Include that context after tool results while keeping legacy keys stable.
  const users = tail.filter(m => m?.role === "user");
  if (tools.length && users.length) material = { material, users: users.map(m => textOfContent(m.content)) };
  return crypto.createHash("sha256").update(stable(material)).digest("hex");
}

export function replyParts(message) {
  const parts = [
    ...(Array.isArray(message?.content) ? message.content : (typeof message?.content === "string" ? [{type:"text", text:message.content}] : [])),
    ...(message?.tool_calls ?? []).map(c => ({type:"toolCall", id:c.id, name:c.function?.name, arguments:argsOf(c)})),
  ];
  return parts.filter(p => p?.type === "text" || p?.type === "toolCall" || p?.type === "tool_use").map(p => p.type === "tool_use" ? {
    type: "toolCall", id: p.id, name: p.name, arguments: p.input ?? p.arguments ?? {}
  } : p.type === "toolCall" ? { type: "toolCall", id: p.id, name: p.name, arguments: p.arguments ?? {} } : { type: "text", text: p.text ?? "" });
}

export function messageBody(line) {
  if (line?.type === "custom_message") return {
    role: "custom", customType: line.customType, content: line.content,
    display: line.display, details: line.details, timestamp: Date.parse(line.timestamp),
  };
  return line?.message ?? line?.data?.message ?? line;
}

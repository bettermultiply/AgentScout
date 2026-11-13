#!/usr/bin/env node
// Regression for the reproduced authority-session export failure. No model calls.
import assert from "node:assert/strict";
import fs from "node:fs";
import { spawnSync } from "node:child_process";
import { requestKey } from "../normalize.mjs";

const root = fs.mkdtempSync("/tmp/opencode/replay-native-");
const entry = (type, id, parentId, more) => ({ type, id, parentId, timestamp: "2026-11-13T00:00:00Z", ...more });
const message = (id, parentId, message) => entry("message", id, parentId, { message });
const call = (id, name) => ({ type: "toolCall", id, name, arguments: { x: 1 } });
const assistant = content => ({ role: "assistant", content, stopReason: "stop" });
const result = (toolCallId, text) => ({ role: "toolResult", toolCallId, content: [{ type: "text", text }] });
const calls = assistant([call("op-a", "alpha"), call("op-b", "beta")]);
const next = assistant([call("op-c", "gamma")]);
const session = [
  { type: "session", version: 3, id: "header" },
  message("u", null, { role: "user", content: "start" }),
  message("a", "u", calls),
  // Actual parallel results finish in reverse tool-call order.
  message("r-b", "a", result("op-b", "actual-b")),
  message("r-a", "r-b", result("op-a", "actual-a")),
  message("p-a", "a", result("op-a", "predicted-a")),
  message("p-b", "p-a", result("op-b", "sibling-b")),
  // Promote rewired this plain custom entry, not the assistant.
  entry("custom", "state", "r-a", { customType: "local-state", data: {} }),
  entry("compaction", "compact", "state", { summary: "kept summary", firstKeptEntryId: "a", tokensBefore: 12 }),
  entry("context_edit", "edit", "compact", { targetId: "p-a", replacement: { content: "edited-prediction" } }),
  entry("custom_message", "note", "edit", { customType: "notification", content: "accepted", display: false, details: { command_id: "cmd-1" } }),
  message("next", "note", next),
  message("real-c", "next", result("op-c", "actual-c")),
  message("pred-c", "next", result("op-c", "predicted-c")),
  message("done", "real-c", assistant([{ type: "text", text: "done" }])),
  entry("custom", "tip", "done", { customType: "authority", data: { branch_id: "nested", parentId: "done" } }),
];
const transcript = (session_entry_id, branch_id, t_ms) => ({ message: "transcript", session_entry_id, branch_id, t_ms });
const chance = (chance_id, parent_branch_id, operation_id) => ({ observation: { event: "chance_created", chance_id, parent_branch_id, operation_id } });
const candidate = (branch_id, parent, chance_id) => ({ message: "candidate_created", branch_id, parent, chance_id });
const promote = (branch_id, command_id) => ({ operation: "promote", branch_id, command_id, status: "applied" });
const events = [
  transcript("u", "root", 1), transcript("a", "root", 2),
  chance("chance-a", "root", "op-a"), candidate("child", "root", "chance-a"),
  transcript("r-b", "root", 3), transcript("r-a", "root", 4),
  transcript("p-a", "child", 0), transcript("p-b", "child", 0),
  transcript("note", "child", 1), transcript("next", "child", 2), promote("child", "cmd-1"),
  chance("chance-b", "child", "op-c"), candidate("nested", "child", "chance-b"),
  transcript("real-c", "child", 3), transcript("pred-c", "nested", 0),
  transcript("done", "nested", 1), promote("nested", "cmd-2"),
];
const write = () => {
  fs.writeFileSync(`${root}/session.jsonl`, session.map(JSON.stringify).join("\n") + "\n");
  fs.writeFileSync(`${root}/events.jsonl`, events.map(JSON.stringify).join("\n") + "\n");
};
write();
const before = ["session.jsonl", "events.jsonl"].map(f => fs.readFileSync(`${root}/${f}`, "utf8"));
let exported = spawnSync(process.execPath, ["tools/replay/export.mjs", root], { encoding: "utf8" });
assert.equal(exported.status, 0, exported.stderr);
const out = JSON.parse(fs.readFileSync(`${root}/recording.json`));
assert.equal(out.length, 3);
assert.equal(out[1].key, requestKey([calls, result("op-a", "edited-prediction"), result("op-b", "sibling-b"), { role: "user", content: "accepted" }]));
assert.equal(out[2].key, requestKey([next, result("op-c", "predicted-c")]));
assert.deepEqual(["session.jsonl", "events.jsonl"].map(f => fs.readFileSync(`${root}/${f}`, "utf8")), before, "source files mutated");
// Missing and contradictory native references must fail, never guess a key.
events.push(transcript("next", "other", 0)); write();
exported = spawnSync(process.execPath, ["tools/replay/export.mjs", root], { encoding: "utf8" });
assert.notEqual(exported.status, 0); assert.match(exported.stderr, /Ambiguous transcript owner/);
console.log(`native export PASS: parallel reverse completion, nested Promote, custom first edge, compaction/edit/notification, reset clocks, immutable sources, ambiguity error; artifacts ${root}`);

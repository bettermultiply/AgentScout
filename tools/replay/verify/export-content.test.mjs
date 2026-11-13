import assert from "node:assert/strict";
import {test} from "node:test";
import {exportContent} from "./export-content.mjs";

const record = (key, text) => ({key, branch_id: "candidate", reply: [{type: "text", text}]});
test("independent completion order does not change replay queues", () => {
  const a = record("a", "first"), b = record("b", "other"), a2 = record("a", "second");
  assert.deepEqual(exportContent([a, b, a2]), exportContent([b, a, a2]));
});
test("same-key order, multiplicity, branch, key and complete reply remain strict", () => {
  const a = record("a", "first"), a2 = record("a", "second");
  const baseline = exportContent([a, a2]);
  for (const changed of [[a2, a], [a], [a, a2, a2], [a, {...a2, branch_id: "other"}], [a, {...a2, key: "b"}], [a, record("a", "changed")]]) {
    assert.notDeepEqual(exportContent(changed), baseline);
  }
});

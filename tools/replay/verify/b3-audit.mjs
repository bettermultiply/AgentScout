import assert from "node:assert/strict";
import fs from "node:fs";

const rows = path => fs.readFileSync(path,"utf8").split(/\r?\n/).filter(Boolean).map(JSON.parse);
export function audit(dir) {
  const events=rows(dir+"/events.jsonl"), obs=events.map(e=>e.observation||e), session=rows(dir+"/session.jsonl");
  const chanceIds=[...new Set(obs.filter(e=>e.event==="chance_stage_changed").map(e=>e.chance_id))];
  assert.equal(chanceIds.length,1,"current B3 fixture must have exactly one Chance");
  const perChance=chanceIds.map(chance_id=>{
    const proposals=obs.filter(e=>e.event==="proposal_added"&&e.chance_id===chance_id);
    const candidates=events.filter(e=>e.message==="candidate_created"&&e.chance_id===chance_id);
    assert.equal(proposals.length,2); assert.equal(candidates.length,2);
    assert.deepEqual(proposals.map(e=>e.payload).sort(),["blocked","moved"]);
    const ids=new Set(candidates.map(e=>e.branch_id)); assert.equal(ids.size,2);
    const dispositions=events.filter(e=>e.disposition&&ids.has(e.branch_id));
    assert.equal(dispositions.filter(e=>e.disposition==="Promote").length,1);
    // A winner may close the Chance before the sibling is individually validated.
    // Require the losing Candidate's applied discard below, not an optional verdict log.
    return {chance_id,proposals,candidates,dispositions};
  });
  const winner=perChance[0].dispositions.find(e=>e.disposition==="Promote").branch_id;
  const loser=perChance[0].candidates.find(e=>e.branch_id!==winner).branch_id;
  const promoted=events.filter(e=>e.operation==="promote"&&e.status==="applied").map(e=>e.branch_id);
  const discarded=events.filter(e=>e.operation==="discard"&&e.status==="applied").map(e=>e.branch_id);
  assert.deepEqual(promoted,[winner]); assert.deepEqual([...discarded].sort(),["authority",loser].sort());
  const authority=session.filter(e=>e.type==="custom"&&e.customType==="authority").at(-1);
  assert.equal(authority?.data.branch_id,winner);
  assert.equal(obs.filter(e=>e.event==="authority_changed").at(-1)?.branch_id,winner);
  const byId=new Map(session.filter(e=>e.id).map(e=>[e.id,e])), chain=[], seen=new Set();
  let id=authority.data.parentId;
  while(id) {
    assert.ok(!seen.has(id),"session parent cycle"); seen.add(id);
    const entry=byId.get(id); assert.ok(entry,`missing session parent ${id}`);
    chain.unshift(entry); id=entry.parentId;
  }
  const roleText=chain.filter(e=>e.message?.role).map(e=>({role:e.message.role,text:typeof e.message.content==="string"?e.message.content:(e.message.content||[]).filter(p=>p.type==="text").map(p=>p.text).join("\n")}));
  assert.deepEqual(roleText.map(e=>e.role),["system","user","assistant","toolResult","assistant"]);
  assert.equal(roleText[1].text,"Use nle_action once, then stop.");
  assert.equal(roleText[3].text,"moved"); assert.equal(roleText[4].text,"scripted complete");
  const nle=obs.filter(e=>e.event==="nle_map_action").map(e=>({action:e.action,action_id:e.action_id,branch_id:e.branch_id,before:e.before,after:e.after}));
  assert.equal(nle.length,1); assert.equal(nle[0].action,1);
  const recording=JSON.parse(fs.readFileSync(dir+"/recording.json")); assert.equal(recording.length,4);
  assert.ok(recording.some(e=>e.branch_id===loser),"losing Candidate recording absent");
  return {perChance,winner,loser,promoted,discarded,authority_tip:authority.data.parentId,chain_ids:chain.map(e=>e.id),roleText,nle};
}
export function checkReport(report,expected) {
  for(const [key,value] of Object.entries(expected)) assert.equal(report[key],value,`report.${key}`);
  assert.equal(Object.values(report.requests).reduce((a,b)=>a+b,0),report.total_requests);
}

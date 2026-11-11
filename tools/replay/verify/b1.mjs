#!/usr/bin/env node
import assert from "node:assert/strict";
import fs from "node:fs";
import http from "node:http";
import { spawn } from "node:child_process";
import { requestKey } from "../normalize.mjs";

const root = "/tmp/ascout-replay/b1"; fs.rmSync(root,{recursive:true,force:true}); fs.mkdirSync(root,{recursive:true});
const messages = [
  {role:"user",content:"move"},
  {role:"assistant",content:[{type:"toolCall",id:"old-id",name:"nle_action",arguments:{action:1}}]},
  {role:"tool",toolCallId:"old-id",content:[{type:"text",text:"prediction"}]},
  {role:"assistant",content:[{type:"text",text:"accepted"}]},
];
const parallel = [
  {role:"user",content:"parallel"},
  {role:"assistant",tool_calls:[{id:"a",function:{name:"x",arguments:'{"b":2,"a":1}'}}]},
  {role:"tool",tool_call_id:"a",content:"one"},
  {role:"tool",tool_call_id:"b",content:[{type:"image",source:{}}]},
];
assert.equal(requestKey(messages), requestKey(JSON.parse(JSON.stringify(messages)).map(x=>x)));
assert.notEqual(requestKey(messages), requestKey(parallel));
const run = root + "/run"; fs.mkdirSync(run); fs.writeFileSync(run+"/session.jsonl", [
  JSON.stringify({id:"u",message:messages[0]}), JSON.stringify({id:"a",message:messages[1],session_entry_id:"e1"}),
  JSON.stringify({id:"t",message:messages[2]}), JSON.stringify({id:"a2",message:messages[3],session_entry_id:"e2"}),
  JSON.stringify({id:"discard-user",message:{role:"user",content:"discard branch"}}),
  JSON.stringify({id:"discard",parentId:"discard-user",message:{role:"assistant",content:[{type:"text",text:"discarded"}]},session_entry_id:"e3"}),
  JSON.stringify({id:"unused-user",message:{role:"user",content:"unused branch"}}),
  JSON.stringify({id:"unused",parentId:"unused-user",message:{role:"assistant",content:[{type:"text",text:"never requested"}]},session_entry_id:"e4"}),
  JSON.stringify({id:"aborted",message:{role:"assistant",stopReason:"aborted",content:[{type:"text",text:"must not export"}]},session_entry_id:"e5"}),
  JSON.stringify({id:"error",message:{role:"assistant",errorMessage:"fixture failure",content:[{type:"text",text:"must not export"}]},session_entry_id:"e6"}),
].join("\n")+"\n");
fs.writeFileSync(run+"/events.jsonl", [
  JSON.stringify({message:"transcript",session_entry_id:"e1",branch_id:"authority"}), JSON.stringify({message:"transcript",session_entry_id:"e2",branch_id:"authority"}), JSON.stringify({message:"transcript",session_entry_id:"e3",branch_id:"discard"})
].join("\n")+"\n");
let exportStderr="";
await new Promise((resolve,reject)=>{const p=spawn(process.execPath,["tools/replay/export.mjs",run],{stdio:["ignore","ignore","pipe"]});p.stderr.on("data",b=>exportStderr+=b);p.on("exit",c=>c?reject(new Error("export")):resolve());});
const recording=JSON.parse(fs.readFileSync(run+"/recording.json")); assert.equal(recording.length,4); assert.equal(recording[2].branch_id,"discard");
assert.ok(!recording.some(r=>JSON.stringify(r).includes("must not export")), "aborted/error assistant was exported");
assert.match(exportStderr,/aborted=1/); assert.match(exportStderr,/error=1/); assert.match(exportStderr,/total=2/);
const port=19081; const p=spawn(process.execPath,["tools/replay/serve.mjs",run+"/recording.json",port],{stdio:["ignore","pipe","inherit"]});
try {
await new Promise((resolve,reject)=>{p.stdout.once("data",resolve);p.once("exit",()=>reject(new Error("serve exited")));});
function post(body){return new Promise((resolve,reject)=>{const r=http.request({hostname:"127.0.0.1",port,path:"/v1/chat/completions",method:"POST",headers:{"content-type":"application/json"}},x=>{let b="";x.on("data",c=>b+=c);x.on("end",()=>resolve({status:x.statusCode,body:b}));});r.on("error",reject);r.end(JSON.stringify(body));});}
const req={model:"fixture",messages:messages.slice(0,2)}; const req2={model:"fixture",messages:messages.slice(0,3)};
assert.equal((await post(req)).status,200); assert.equal((await post(req)).status,400);
assert.equal((await post(req2)).status,200);
const miss=await post({model:"fixture",messages:[{role:"user",content:"never recorded"}]}); assert.equal(miss.status,400);
p.kill("SIGTERM"); await new Promise(r=>p.once("exit",r)); const report=JSON.parse(fs.readFileSync(run+"/report.json")); assert.equal(report.unconsumed,2); assert.deepEqual(report.unconsumed_keys,{[recording[2].key]:1,[recording[3].key]:1}); assert.deepEqual(report.missing_keys,{[recording[0].key]:1,[requestKey([{role:"user",content:"never recorded"}])]:1});
console.log("B1 PASS: text/tool/parallel normalization, ordered consumption, miss/exhaustion 400, report, discarded branch export");
} finally {
  if(p.exitCode===null&&p.signalCode===null) {
    p.kill("SIGTERM");
    await new Promise((resolve,reject)=>{p.once("exit",resolve);p.once("error",reject);});
  }
}

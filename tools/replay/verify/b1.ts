#!/usr/bin/env node
import assert from "node:assert/strict";
import fs from "node:fs";
import http from "node:http";
import { spawn } from "node:child_process";
import { requestKey } from "../normalize.ts";
import type { RecordingItem } from '../types.ts';

const root = fs.mkdtempSync('/tmp/opencode/replay-b1-');
const messages = [
  {role:"user",content:"move"},
  {role:"assistant",content:[{type:"toolCall",id:"old-id",name:"nle_action",arguments:{action:1}}]},
  {role:"tool",toolCallId:"old-id",content:[{type:"text",text:"prediction"}]},
  {role:"assistant",content:[{type:"text",text:"accepted"}]},
];
const run = root + "/run"; fs.mkdirSync(run); fs.writeFileSync(run+"/session.jsonl", [
  JSON.stringify({type:"message",id:"u",parentId:null,message:messages[0]}), JSON.stringify({type:"message",id:"a",parentId:"u",message:messages[1]}),
  JSON.stringify({type:"message",id:"t",parentId:"a",message:{...messages[2],role:"toolResult"}}), JSON.stringify({type:"message",id:"a2",parentId:"t",message:messages[3]}),
  JSON.stringify({type:"message",id:"discard-user",parentId:null,message:{role:"user",content:"discard branch"}}),
  JSON.stringify({type:"message",id:"discard",parentId:"discard-user",message:{role:"assistant",content:[{type:"text",text:"discarded"}]}}),
  JSON.stringify({type:"message",id:"unused-user",parentId:null,message:{role:"user",content:"unused branch"}}),
  JSON.stringify({type:"message",id:"unused",parentId:"unused-user",message:{role:"assistant",content:[{type:"text",text:"never requested"}]}}),
  JSON.stringify({type:"message",id:"cancel-user",parentId:null,message:{role:"user",content:"cancel branch"}}),
  JSON.stringify({type:"message",id:"aborted",parentId:"cancel-user",message:{role:"assistant",stopReason:"aborted",content:[{type:"text",text:"must not export"}]}}),
  JSON.stringify({type:"message",id:"error",parentId:"u",message:{role:"assistant",errorMessage:"fixture failure",content:[{type:"text",text:"must not export"}]}}),
].join("\n")+"\n");
fs.writeFileSync(run+"/events.jsonl", [
  JSON.stringify({message:"transcript",session_entry_id:"aborted",branch_id:"cancel"}),
  JSON.stringify({message:"transcript",session_entry_id:"a",branch_id:"authority"}), JSON.stringify({message:"transcript",session_entry_id:"a2",branch_id:"authority"}), JSON.stringify({message:"transcript",session_entry_id:"discard",branch_id:"discard"}), JSON.stringify({message:"transcript",session_entry_id:"unused",branch_id:"unused"})
].join("\n")+"\n");
let exportStderr="";
await new Promise<void>((resolve,reject)=>{const p=spawn(process.execPath,["tools/replay/export.ts",run],{stdio:["ignore","ignore","pipe"]});p.stderr.on("data",b=>exportStderr+=b);p.on("exit",c=>c?reject(new Error("export")):resolve());});
const recording: RecordingItem[]=JSON.parse(fs.readFileSync(run+"/recording.json", 'utf8')); assert.equal(recording.length,5); assert.equal(recording[2].branch_id,"discard");
assert.ok(!recording.some(r=>JSON.stringify(r).includes("must not export")), "aborted/error assistant was exported");
assert.match(exportStderr,/aborted=1/); assert.match(exportStderr,/error=1/); assert.equal(recording[4].stopReason,"aborted");
let port: number;
const p=spawn(process.execPath,["tools/replay/serve.ts",run+"/recording.json","0"],{stdio:["ignore","pipe","inherit"]});
try {
port = await new Promise<number>((resolve,reject)=>{
  let output = '';
  p.stdout.on('data', chunk => {
    output += chunk;
    const match = output.match(/endpoint http:\/\/127\.0\.0\.1:(\d+)\/v1/);
    if (match) resolve(Number(match[1]));
  });
  p.once('exit',()=>reject(new Error('serve exited')));
});
function post(body: unknown){return new Promise<{status: number | undefined; body: string}>((resolve,reject)=>{const r=http.request({hostname:"127.0.0.1",port,path:"/v1/chat/completions",method:"POST",headers:{"content-type":"application/json"}},x=>{let b="";x.on("data",c=>b+=c);x.on("end",()=>resolve({status:x.statusCode,body:b}));});r.on("error",reject);r.end(JSON.stringify(body));});}
const req={model:"fixture",messages:messages.slice(0,2)}; const req2={model:"fixture",messages:messages.slice(0,3)};
assert.equal((await post(req)).status,200); assert.equal((await post(req)).status,400);
assert.equal((await post(req2)).status,200);
const miss=await post({model:"fixture",messages:[{role:"user",content:"never recorded"}]}); assert.equal(miss.status,400);
p.kill("SIGTERM"); await new Promise(r=>p.once("exit",r)); const report=JSON.parse(fs.readFileSync(run+"/report.json", 'utf8')); assert.equal(report.unconsumed,3); assert.deepEqual(report.unconsumed_keys,{[recording[2].key]:1,[recording[3].key]:1,[recording[4].key]:1}); assert.deepEqual(report.missing_keys,{[recording[0].key]:1,[requestKey([{role:"user",content:"never recorded"}])]:1});
console.log("B1 PASS: text/tool normalization, ordered consumption, miss/exhaustion 400, report, discarded branch export");
} finally {
  if(p.exitCode===null&&p.signalCode===null) {
    p.kill("SIGTERM");
    await new Promise((resolve,reject)=>{p.once("exit",resolve);p.once("error",reject);});
  }
}

#!/usr/bin/env node
/* B3 harness: a deterministic scripted model, never a replay endpoint. */
import fs from "node:fs";
import http from "node:http";
import {execFileSync, spawn as nodeSpawn} from "node:child_process";
import {once} from "node:events";
import assert from "node:assert/strict";

const children = [], servers = [];
const spawn = (...args) => { const child = nodeSpawn(...args); children.push(child); return child; };
const deadline = async (promise, ms, label) => {
  let timer;
  try { return await Promise.race([promise,new Promise((_,reject)=>{timer=setTimeout(()=>reject(Error(label)),ms);})]); }
  finally { clearTimeout(timer); }
};

const root = "/tmp/ascout-replay/b3"; fs.rmSync(root,{recursive:true,force:true}); fs.mkdirSync(root,{recursive:true});
const waitExit = child => child.exitCode !== null || child.signalCode !== null ? Promise.resolve(child.exitCode) : once(child,"exit").then(([code]) => code);
const terminate = async child => {
  if (!child) return null;
  if (child.exitCode === null && child.signalCode === null) child.kill("SIGTERM");
  try { return await deadline(waitExit(child),15000,"SIGTERM timeout"); }
  catch { child.kill("SIGKILL"); return deadline(waitExit(child),5000,"SIGKILL timeout"); }
};
const closeServer = server => new Promise((resolve, reject) => {
  if (!server || !server.listening) return resolve();
  server.close(error => error ? reject(error) : resolve());
});
const piDir = root+"/pi"; fs.mkdirSync(piDir); fs.writeFileSync(piDir+"/models.json", JSON.stringify({providers:{replay:{baseUrl:"http://127.0.0.1:18082/v1",api:"openai-completions",apiKey:"dummy",models:[{id:"fixture",reasoning:false,input:["text"],contextWindow:8192,maxTokens:256}]}}}));
const runtime = root+"/runtime.json"; fs.writeFileSync(runtime, JSON.stringify({backend:"nle",control_socket:root+"/nle.sock",adapter_instance:"nle",python:"/home/betmul/BETMUL/AgentScout-suit/nle-313/bin/python",bridge:process.cwd()+"/examples/nle-runtime-bridge.py",guidebook:process.cwd()+"/examples/nle/Guidebook.txt",seed:7,action_delay_seconds:0,env_id:"NetHackScore-v0"}));

let modelRequests=0; const modelLog=[];
const fake = http.createServer((req,res)=>{let body="";req.on("data",c=>body+=c);req.on("end",()=>{modelRequests++; let j={};try{j=JSON.parse(body)}catch{}
  modelLog.push(j.messages||[]);
  const hasToolResult=(j.messages||[]).some(m=>m.role==="tool"||m.role==="toolResult");
  const call=hasToolResult?null:{id:"fixture-call",type:"function",function:{name:"nle_action",arguments:JSON.stringify({action:1})}};
  res.writeHead(200,{"content-type":"text/event-stream"}); const id="scripted"; const d=call?{tool_calls:[call]}:{content:"scripted complete"};
  res.write(`data: ${JSON.stringify({id,object:"chat.completion.chunk",created:0,model:"fixture",choices:[{index:0,delta:d}]})}\n\n`);
  res.write(`data: ${JSON.stringify({id,object:"chat.completion.chunk",created:0,model:"fixture",choices:[{index:0,delta:{},finish_reason:call?"tool_calls":"stop"}]})}\n\ndata: [DONE]\n\n`);res.end();});});
servers.push(fake);
try {
await new Promise((resolve,reject)=>{fake.once("error",reject);fake.listen(18082,"127.0.0.1",resolve);});
const bin=root+"/ascout-control-plane";
await new Promise((resolve,reject)=>{const p=spawn("cargo",["build","--bin","ascout-control-plane"],{cwd:process.cwd(),stdio:"inherit"});p.on("error",reject);p.on("exit",c=>c?reject(Error("cargo build failed")):resolve());});
fs.symlinkSync(process.cwd()+"/target/debug/ascout-control-plane",bin);
const run=root+"/run"; fs.mkdirSync(run); const policy=root+"/b3-policy.yaml";
fs.copyFileSync(process.cwd()+"/assets/policy/decisions.py", root+"/decisions.py");
fs.writeFileSync(policy, fs.readFileSync(process.cwd()+"/tools/replay/verify/b3-policy.yaml","utf8"));
const child=spawn(bin,[run,process.env.PI_ROOT||"/home/betmul/BETMUL/AgentScout-suit/pi",policy,"--web","127.0.0.1:18083","--model","replay/fixture","--thinking","low"],{cwd:process.cwd(),env:{...process.env,ASCOUT_RUNTIME_CONFIG:runtime,PI_CODING_AGENT_DIR:piDir},stdio:["ignore","pipe","pipe"]});
let controlOut="", controlErr=""; child.stdout.on("data",b=>controlOut+=b); child.stderr.on("data",b=>controlErr+=b);
const postTo=(port,path,body)=>new Promise((resolve,reject)=>{const r=http.request({hostname:"127.0.0.1",port,path,method:"POST",headers:{host:`127.0.0.1:${port}`,origin:`http://127.0.0.1:${port}`,"content-type":"application/json","content-length":Buffer.byteLength(body)}},x=>{let b="";x.on("data",c=>b+=c);x.on("end",()=>resolve({status:x.statusCode,body:b}));});r.on("error",reject);r.end(body);});
const post=(path,body)=>postTo(18083,path,body);
const events=()=>{try{return fs.readFileSync(run+"/events.jsonl","utf8").split(/\r?\n/).filter(Boolean).map(JSON.parse)}catch{return []}};
const summary=()=>{const es=events(); const obs=es.map(e=>e.observation||e); const proposals=new Set(es.flatMap(e=>[...JSON.stringify(e).matchAll(/proposal-[A-Za-z0-9_-]+/g)].map(m=>m[0]))); return {chances:new Set(obs.filter(e=>e.event==="chance_stage_changed"&&e.chance_id).map(e=>e.chance_id)).size,proposals:proposals.size,candidates:new Set(obs.filter(e=>e.message==="branch_created"&&e.branch_id!=="authority").map(e=>e.branch_id)).size,promote:es.filter(e=>e.disposition==="Promote").length,discard:es.filter(e=>e.operation==="discard"&&e.status==="applied").length,raw:es};};
try { for(let i=0;i<100;i++){try{const r=await post("/api/request",JSON.stringify({action:"prompt",prompt:"Use nle_action once, then stop."}));if(r.status===202)break;}catch{await new Promise(r=>setTimeout(r,100));}}
  let state; for(let i=0;i<600;i++){state=summary(); if(state.candidates>=2 && state.promote>=1 && state.discard>=1)break; await new Promise(r=>setTimeout(r,200));}
  if(!(state.candidates>=2 && state.promote>=1 && state.discard>=1)) { console.error(controlErr); throw new Error(`B3 did not reach winner state: ${JSON.stringify({candidates:state.candidates,promote:state.promote,discard:state.discard,chances:state.chances})}`); }
   await post("/api/request",JSON.stringify({action:"shutdown"})); await deadline(waitExit(child),15000,"first control shutdown timeout");
 } finally {
   await terminate(child);
   await closeServer(fake);
 }
const ex=spawn(process.execPath,["tools/replay/export.mjs",run],{cwd:process.cwd(),stdio:"inherit"}); assert.equal(await waitExit(ex),0);
const firstSummary=summary();
const recording=run+"/recording.json", replayRecording=root+"/replay-recording.json"; fs.copyFileSync(recording,replayRecording);
const replayPi=root+"/replay-pi"; fs.mkdirSync(replayPi); fs.writeFileSync(replayPi+"/models.json",JSON.stringify({providers:{replay:{baseUrl:"http://127.0.0.1:18085/v1",api:"openai-completions",apiKey:"dummy",models:[{id:"fixture",reasoning:false,input:["text"],contextWindow:8192,maxTokens:256}]}}}));
const replay=spawn(process.execPath,["tools/replay/serve.mjs",replayRecording,"18085"],{cwd:process.cwd(),env:{...process.env,REPLAY_REQUEST_LOG:root+"/replay-requests.jsonl"},stdio:["ignore","pipe","pipe"]}); await deadline(new Promise((resolve,reject)=>{replay.stdout.once("data",resolve); replay.once("error",reject); replay.once("exit",()=>reject(Error("replay endpoint exited")));}),10000,"replay startup timeout");
const run2=root+"/run-replay"; fs.mkdirSync(run2); const child2=spawn(bin,[run2,process.env.PI_ROOT||"/home/betmul/BETMUL/AgentScout-suit/pi",policy,"--web","127.0.0.1:18086","--model","replay/fixture","--thinking","low"],{cwd:process.cwd(),env:{...process.env,ASCOUT_RUNTIME_CONFIG:runtime,PI_CODING_AGENT_DIR:replayPi},stdio:["ignore","pipe","pipe"]});
let replayErr=""; child2.stderr.on("data",b=>replayErr+=b);
const events2=()=>{try{return fs.readFileSync(run2+"/events.jsonl","utf8").split(/\r?\n/).filter(Boolean).map(JSON.parse)}catch{return []}};
const summary2=()=>{const es=events2(),obs=es.map(e=>e.observation||e),proposals=new Set(es.flatMap(e=>[...JSON.stringify(e).matchAll(/proposal-[A-Za-z0-9_-]+/g)].map(m=>m[0])));return {chances:new Set(obs.filter(e=>e.event==="chance_stage_changed"&&e.chance_id).map(e=>e.chance_id)).size,proposals:proposals.size,candidates:new Set(obs.filter(e=>e.message==="branch_created"&&e.branch_id!=="authority").map(e=>e.branch_id)).size,promote:es.filter(e=>e.disposition==="Promote").length,discard:es.filter(e=>e.operation==="discard"&&e.status==="applied").length};};
const post2=(path,body)=>postTo(18086,path,body);
try { for(let i=0;i<150;i++){try{if((await post2("/api/request",JSON.stringify({action:"prompt",prompt:"Use nle_action once, then stop."}))).status===202)break;}catch{await new Promise(r=>setTimeout(r,100));}}
  for(let i=0;i<300;i++){const s=summary2();if(s.candidates>=2&&s.promote>=1&&s.discard>=1)break;if(child2.exitCode!==null)break;await new Promise(r=>setTimeout(r,200));}
   await post2("/api/request",JSON.stringify({action:"shutdown"})); await deadline(waitExit(child2),15000,"replay control shutdown timeout");
 } finally {
   await terminate(child2);
 }
const ex2=spawn(process.execPath,["tools/replay/export.mjs",run2],{cwd:process.cwd(),stdio:"inherit"}); assert.equal(await waitExit(ex2),0);
const {audit, checkReport} = await import("./b3-audit.mjs");
const firstAudit=audit(run), secondAudit=audit(run2);
const roleText=dir=>dir===run ? firstAudit.roleText : secondAudit.roleText;
const same=JSON.stringify(roleText(run))===JSON.stringify(roleText(run2));
assert.deepEqual(firstAudit.roleText,secondAudit.roleText,"full authority parent-chain role/text mismatch");
assert.deepEqual(firstAudit.nle,secondAudit.nle,"real NLE observation mismatch");
assert.equal(child.exitCode,0); assert.equal(child2.exitCode,0);
const normalReport=JSON.parse(fs.readFileSync(root+"/replay-report.json"));
checkReport(normalReport,{total_requests:4,hits:4,misses:0,exhausted:0,consumed:4,unconsumed:0});
assert.deepEqual(normalReport.missing_keys,{});
fs.writeFileSync(root+"/normal-replay-report.json",JSON.stringify(normalReport,null,2));
const replayCall=body=>{for(let i=0;i<50;i++){} return postTo(18085,"/v1/chat/completions",body)};
const miss=await replayCall(JSON.stringify({model:"fixture",messages:[{role:"user",content:"intentionally missing"}]}));
const exhausted=await postTo(18085,"/v1/chat/completions",JSON.stringify({model:"fixture",messages:modelLog[0]}));
const bad=root+"/bad-recording.json"; const badRows=JSON.parse(fs.readFileSync(recording)); badRows[0].key="deliberately-changed"; fs.writeFileSync(bad,JSON.stringify(badRows));
const badServer=spawn(process.execPath,["tools/replay/serve.mjs",bad,"18087"],{cwd:process.cwd(),stdio:["ignore","pipe","pipe"]}); await deadline(new Promise((resolve,reject)=>{badServer.stdout.once("data",resolve);badServer.once("error",reject);badServer.once("exit",()=>reject(Error("bad endpoint exited")));}),10000,"bad endpoint startup timeout"); const altered=await postTo(18087,"/v1/chat/completions",JSON.stringify({model:"fixture",messages:modelLog[0]})); await terminate(badServer);
await terminate(replay);
assert.equal(miss.status,400); assert.equal(exhausted.status,400); assert.equal(altered.status,400);
const finalReport=JSON.parse(fs.readFileSync(root+"/replay-report.json")), badReport=JSON.parse(fs.readFileSync(root+"/bad-report.json"));
checkReport(finalReport,{total_requests:6,hits:4,misses:1,exhausted:1,consumed:4,unconsumed:0});
checkReport(badReport,{total_requests:1,hits:0,misses:1,exhausted:0,consumed:0,unconsumed:4});
fs.writeFileSync(root+"/acceptance.json",JSON.stringify({firstAudit,secondAudit,normalReport,finalReport,badReport},null,2));
const psText=execFileSync("ps",["-eo","pid=,args="],{encoding:"utf8"});
const psMatches=psText.split("\n").filter(line=>/(ascout-control-plane|ascout-runtime-adapter|nle-runtime-bridge|tools\/replay\/(serve|verify\/b3)\.mjs)/.test(line)).filter(line=>!line.trim().startsWith(String(process.pid)+" ")).filter(line=>!(/\b(bash|timeout)\b/.test(line)));
if (psMatches.length) throw new Error(`B3 process cleanup failed:\n${psMatches.join("\n")}`);
console.log(JSON.stringify({run,run2,modelRequests,recording:true,firstSummary:{...firstSummary,raw:undefined},secondSummary:summary2(),same_role_text:same,replay:{missing_status:miss.status,exhausted_status:exhausted.status,altered_status:altered.status},control_exit:child.exitCode,replay_control_exit:child2.exitCode,ps_checked:true,ps_matches:psMatches},null,2));
} finally {
  const cleanup=await Promise.allSettled(children.map(terminate));
  for(const server of servers) { server.closeAllConnections(); await closeServer(server); }
  assert.equal(cleanup.filter(r=>r.status==="rejected").length,0,"process cleanup failures");
  const remaining=execFileSync("ps",["-eo","pid=,comm=,args="],{encoding:"utf8"}).split("\n").filter(l=>/(ascout-control-plane|ascout-runtime-adapter|nle-runtime-bridge|tools\/replay\/(serve|verify\/b3)\.mjs)/.test(l)).filter(l=>!l.trim().startsWith(process.pid+" ")&&!/\b(bash|timeout)\s/.test(l));
  assert.deepEqual(remaining,[],"B3 process cleanup failed");
  const ports=execFileSync("ss",["-ltnH"],{encoding:"utf8"}).split("\n").filter(l=>/:(18082|18083|18085|18086|18087)\b/.test(l));
  assert.deepEqual(ports,[],"B3 ports still occupied");
}

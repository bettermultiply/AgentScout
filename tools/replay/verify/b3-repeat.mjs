#!/usr/bin/env node
/* Replay one B3 recording five times; this never records model output again. */
import fs from "node:fs";
import {spawn} from "node:child_process";
import {once} from "node:events";
import {execFileSync} from "node:child_process";
import assert from "node:assert/strict";
import {exportContent} from "./export-content.mjs";

const source = process.argv[2];
assert.ok(source, "usage: node tools/replay/verify/b3-repeat.mjs EXISTING_RUN_DIR");
assert.ok(fs.existsSync(`${source}/session.jsonl`) && fs.existsSync(`${source}/events.jsonl`), "existing native run required");
const project = process.cwd(), root = fs.mkdtempSync("/tmp/opencode/ascout-replay-b3-repeat-");
const rounds = Number(process.env.REPLAY_ROUNDS || 5);
assert.ok(Number.isInteger(rounds) && rounds > 0);
const depth = Number(process.env.DEEP_ROUNDS || 1);
assert.ok([1,3,4,10].includes(depth), "supported fixture depths: 1, 3, 4, 10");
const sourceRoot = `${source}/..`;
const children=[];
const wait = child => child.exitCode !== null || child.signalCode !== null ? Promise.resolve(child.exitCode) : once(child,"exit").then(([code])=>code);
const deadline = async (promise, ms, label) => { let timer; try { return await Promise.race([promise,new Promise((_,reject)=>{timer=setTimeout(()=>reject(Error(label)),ms);})]); } finally { clearTimeout(timer); } };
const stop = async child => { if (!child || child.exitCode !== null || child.signalCode !== null) return; child.kill("SIGTERM"); try { await deadline(wait(child),15000,"stop timeout"); } catch { child.kill("SIGKILL"); await deadline(wait(child),5000,"kill timeout"); } };
const ready = child => new Promise((resolve,reject)=>{ let text="", err=""; const on=b=>{text+=b; const m=text.match(/endpoint http:\/\/127\.0\.0\.1:(\d+)\/v1/); if(m) resolve(Number(m[1]));}; child.stdout.on("data",on); child.stderr.on("data",b=>err+=b); child.once("error",reject); child.once("exit",c=>reject(Error(`serve exited ${c}: ${text}${err}`))); });
const rows = dir => fs.readFileSync(`${dir}/events.jsonl`,"utf8").split(/\r?\n/).filter(Boolean).map(JSON.parse);
const audit = dir => {
  const es=rows(dir), obs=es.map(e=>e.observation||e), session=fs.readFileSync(`${dir}/session.jsonl`,"utf8").split(/\r?\n/).filter(Boolean).map(JSON.parse);
  const chances=[...new Set(obs.filter(e=>e.event==="chance_stage_changed"&&e.chance_id).map(e=>e.chance_id))];
  const proposals=obs.filter(e=>e.event==="proposal_added");
  const candidates=es.filter(e=>e.message==="candidate_created");
  const promote=es.filter(e=>e.disposition==="Promote");
  const discard=es.filter(e=>e.operation==="discard"&&e.status==="applied");
  const authority=session.filter(e=>e.type==="custom"&&e.customType==="authority").at(-1);
  const byId=new Map(session.filter(e=>e.id).map(e=>[e.id,e])), chain=[]; let id=authority?.data?.parentId;
  while(id){const e=byId.get(id); if(!e) break; chain.unshift(e); id=e.parentId;}
  const roleText=chain.filter(e=>e.message?.role).map(e=>({role:e.message.role,text:typeof e.message.content==="string"?e.message.content:(e.message.content||[]).filter(p=>p.type==="text").map(p=>p.text).join("\n")}));
  const nle=obs.filter(e=>e.event==="nle_map_action").map(e=>({action:e.action,action_id:e.action_id,branch_id:e.branch_id,before:e.before,after:e.after}));
  const structure=obs.filter(e=>e.event==="chance_created").map(e=>({chance_id:e.chance_id,parent_branch_id:e.parent_branch_id,parent_chance_id:e.parent_chance_id}));
  const expected=Array.from({length:depth},(_,i)=>({chance_id:`chance-${i+1}`,parent_branch_id:i===0?"authority":`candidate-${2*i}`,parent_chance_id:i===0?null:`chance-${i}`}));
  assert.deepEqual(structure,expected,"Chance parent structure");
  assert.equal(proposals.length,depth*2); assert.equal(candidates.length,depth*2); assert.equal(promote.length,depth); assert.equal(discard.length,depth*2);
  const facts=[];
  for(let i=0;i<depth;i++) {
    const chance_id=`chance-${i+1}`, winner=`candidate-${2*i+2}`, loser=`candidate-${2*i+3}`, parent=i===0?"authority":`candidate-${2*i}`;
    assert.equal(proposals.filter(e=>e.chance_id===chance_id).length,2);
    assert.deepEqual(candidates.filter(e=>e.chance_id===chance_id).map(e=>e.branch_id),[winner,loser]);
    assert.equal(es.filter(e=>e.operation==="promote"&&e.status==="applied"&&e.branch_id===winner).length,1);
    for(const branch_id of [parent,loser]) assert.equal(discard.filter(e=>e.branch_id===branch_id).length,1);
  }
  for(const e of proposals) facts.push({kind:"proposal",chance_id:e.chance_id,proposal_id:e.proposal_id,payload:e.payload});
  for(const e of es) if(e.message==="candidate_created"||(["promote","discard"].includes(e.operation)&&e.status==="applied")) facts.push({kind:e.message||e.operation,chance_id:e.chance_id||null,branch_id:e.branch_id});
  assert.equal(nle.length,depth);
  return {structure,facts:facts.sort((a,b)=>JSON.stringify(a).localeCompare(JSON.stringify(b))),chances:chances.length,proposal_count:proposals.length,candidate_count:candidates.length,promote_count:promote.length,discard_count:discard.length,roleText,nle};
};
const post=(port,path,body)=>fetch(`http://127.0.0.1:${port}${path}`,{method:"POST",headers:{"content-type":"application/json",origin:`http://127.0.0.1:${port}`},body}).then(async r=>({status:r.status,body:await r.text()}));
let server;
try {
  const recording=`${root}/recording.json`, baseline=audit(source), results=[];
  const exporter=spawn(process.execPath,["tools/replay/export.mjs",source,recording],{cwd:project,stdio:"inherit"}); children.push(exporter); assert.equal(await wait(exporter),0,"export existing run failed");
  const build=spawn("cargo",["build","--bin","ascout-control-plane"],{cwd:project,stdio:"inherit"}); children.push(build); assert.equal(await wait(build),0,"cargo build failed");
  for(let i=1;i<=rounds;i++) {
    const dir=`${root}/run-${i}`; fs.mkdirSync(dir,{recursive:true});
    const policy=`${dir}/policy.yaml`, runtime=`${dir}/runtime.json`;
    fs.copyFileSync(`${sourceRoot}/b3-policy.yaml`,policy);
    fs.copyFileSync(`${sourceRoot}/decisions.py`,`${dir}/decisions.py`);
    fs.writeFileSync(runtime,JSON.stringify({...JSON.parse(fs.readFileSync(`${sourceRoot}/runtime.json`)),control_socket:`${dir}/nle.sock`}));
    fs.writeFileSync(policy,fs.readFileSync(policy,"utf8").replace(/runtime: .*/,`runtime: ${runtime}`));
    server=spawn(process.execPath,["tools/replay/serve.mjs",source,"0"],{cwd:project,env:{...process.env,REPLAY_REPORT:`${dir}/report.json`,REPLAY_REQUEST_LOG:`${dir}/requests.jsonl`},stdio:["ignore","pipe","pipe"]}); children.push(server); const modelPort=await ready(server);
    const piDir=`${dir}/pi`; fs.mkdirSync(piDir); fs.writeFileSync(`${piDir}/models.json`,JSON.stringify({providers:{replay:{baseUrl:`http://127.0.0.1:${modelPort}/v1`,api:"openai-completions",apiKey:"dummy",models:[{id:"fixture",reasoning:false,input:["text"],contextWindow:8192,maxTokens:256}]}}}));
    const controlPort=19090+i, bin="target/debug/ascout-control-plane";
    const control=spawn(bin,[dir,process.env.PI_ROOT||"/home/betmul/BETMUL/AgentScout-suit/pi",policy,"--web",`127.0.0.1:${controlPort}`,"--model","replay/fixture","--thinking","low"],{cwd:project,env:{...process.env,ASCOUT_RUNTIME_CONFIG:runtime,PI_CODING_AGENT_DIR:piDir},stdio:["ignore","ignore","pipe"]}); children.push(control); let controlErr=""; control.stderr.on("data",b=>{controlErr+=b;fs.appendFileSync(`${dir}/control.stderr.log`,b);});
    const prompt=depth===1?"Use nle_action once, then stop.":`Use nle_action exactly ${depth} times, then stop.`;
    let accepted=false, lastStatus=null, lastBody=""; for(let t=0;t<300;t++){try{const response=await post(controlPort,"/api/request",JSON.stringify({action:"prompt",prompt})); lastStatus=response.status; lastBody=response.body; if(response.status===202){accepted=true;break;}}catch(error){lastBody=String(error);} if(control.exitCode!==null) break; await new Promise(r=>setTimeout(r,100));} assert.ok(accepted,`run ${i} prompt not accepted (status=${lastStatus}, body=${lastBody}): ${controlErr}`);
    for(let t=0;t<600;t++){if(fs.existsSync(`${dir}/events.jsonl`)){const es=rows(dir); if(es.filter(e=>e.operation==="discard"&&e.status==="applied").length===depth*2) break;} if(control.exitCode!==null)break; await new Promise(r=>setTimeout(r,100));}
    assert.equal(control.exitCode,null,`control exited early: ${controlErr}`);
    await post(controlPort,"/api/request",JSON.stringify({action:"shutdown"})); assert.equal(await wait(control),0,`run ${i} control exit: ${controlErr}`); await stop(server); server=null;
    const actual=audit(dir); assert.deepEqual(actual,baseline,`run ${i} differs from recorded B3`);
    const reexport=spawn(process.execPath,["tools/replay/export.mjs",dir,`${dir}/recording.json`],{cwd:project,stdio:"inherit"}); children.push(reexport); assert.equal(await wait(reexport),0,"replay export failed");
    const content=file=>exportContent(JSON.parse(fs.readFileSync(file)));
    assert.deepEqual(content(`${dir}/recording.json`),content(recording),"replay exported inputs/replies differ");
    const report=JSON.parse(fs.readFileSync(`${dir}/report.json`)), count=depth*3+1; assert.equal(JSON.parse(fs.readFileSync(recording)).length,count); assert.equal(report.total_requests,count); assert.equal(report.hits,count); assert.equal(report.consumed,count); assert.equal(report.misses,0); assert.equal(report.exhausted,0); assert.equal(report.unconsumed,0); assert.deepEqual(report.missing_keys,{}); assert.deepEqual(report.unconsumed_keys,{});
    results.push({run:i,actual,report:{hits:report.hits,misses:report.misses,exhausted:report.exhausted,consumed:report.consumed,unconsumed:report.unconsumed}});
  }
  const ps=execFileSync("ps",["-eo","pid=,comm=,args="],{encoding:"utf8"}).split("\n").filter(l=>children.some(c=>c.pid===Number(l.trim().split(/\s+/)[0]))); assert.deepEqual(ps,[],`processes remain:\n${ps.join("\n")}`);
  const ports=execFileSync("ss",["-ltnH"],{encoding:"utf8"}).split("\n").filter(l=>Array.from({length:rounds},(_,i)=>19091+i).some(p=>new RegExp(`:${p}\\b`).test(l))); assert.deepEqual(ports,[]);
  fs.writeFileSync(`${root}/repeat-report.json`,JSON.stringify({recording,baseline,results,ps_checked:true,ports_checked:true},null,2)); console.log(JSON.stringify({recording,runs:results,ps_checked:true,ports_checked:true},null,2));
} finally {
  const cleanup=await Promise.allSettled(children.map(stop));
  assert.ok(cleanup.every(x=>x.status==="fulfilled"),"child cleanup failed");
  assert.ok(children.every(c=>c.exitCode!==null||c.signalCode!==null),"child survived cleanup");
  const processes=execFileSync("ps",["-eo","pid=,args="],{encoding:"utf8"}).split("\n").filter(l=>l.includes(root));
  assert.deepEqual(processes,[],"run-owned descendant survived cleanup");
  const ports=execFileSync("ss",["-ltnH"],{encoding:"utf8"}).split("\n").filter(l=>Array.from({length:rounds},(_,i)=>19091+i).some(p=>new RegExp(`:${p}\\b`).test(l)));
  assert.deepEqual(ports,[],"control ports survived cleanup");
  fs.writeFileSync(`${root}/cleanup.json`,JSON.stringify({children:children.map(c=>({pid:c.pid,exit:c.exitCode,signal:c.signalCode})),processes,ports,clean:true},null,2));
  console.log(`artifacts ${root}; child cleanup complete`);
}

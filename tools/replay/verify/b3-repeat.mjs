#!/usr/bin/env node
/* Replay one B3 recording five times; this never records model output again. */
import fs from "node:fs";
import {spawn} from "node:child_process";
import {once} from "node:events";
import {execFileSync} from "node:child_process";
import assert from "node:assert/strict";

const root = "/tmp/opencode/ascout-replay-b3-repeat";
fs.rmSync(root,{recursive:true,force:true}); fs.mkdirSync(root,{recursive:true});
const project = process.cwd(), source = "/tmp/ascout-replay/b3";
const children=[];
const wait = child => child.exitCode !== null ? Promise.resolve(child.exitCode) : once(child,"exit").then(([code])=>code);
const stop = async child => { if (!child || child.exitCode !== null) return; child.kill("SIGTERM"); try { await Promise.race([wait(child),new Promise((_,r)=>setTimeout(()=>r(Error("stop timeout")),15000))]); } catch { child.kill("SIGKILL"); await wait(child); } };
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
  return {chances:chances.length,proposal_count:proposals.length,candidate_count:candidates.length,promote_count:promote.length,discard_count:discard.length,roleText,nle};
};
const post=(port,path,body)=>fetch(`http://127.0.0.1:${port}${path}`,{method:"POST",headers:{"content-type":"application/json",origin:`http://127.0.0.1:${port}`},body}).then(async r=>({status:r.status,body:await r.text()}));
let server;
try {
  if(!fs.existsSync(`${source}/recording.json`)) {
    const b3=spawn(process.execPath,["tools/replay/verify/b3.mjs"],{cwd:project,stdio:"inherit"}); children.push(b3); assert.equal(await wait(b3),0,"B3 recording bootstrap failed");
  }
  const recording=`${source}/run/recording.json`, baseline=audit(`${source}/run`), results=[];
  for(let i=1;i<=5;i++) {
    const dir=`${root}/run-${i}`; fs.mkdirSync(dir,{recursive:true});
    const policy=`${dir}/policy.yaml`, runtime=`${dir}/runtime.json`;
    fs.copyFileSync(`${project}/tools/replay/verify/b3-policy.yaml`,policy);
    fs.copyFileSync(`${project}/assets/policy/decisions.py`,`${dir}/decisions.py`);
    fs.writeFileSync(runtime,JSON.stringify({...JSON.parse(fs.readFileSync(`${source}/runtime.json`)),control_socket:`${dir}/nle.sock`}));
    fs.writeFileSync(policy,fs.readFileSync(policy,"utf8").replace(/runtime: .*/,`runtime: ${runtime}`));
    server=spawn(process.execPath,["tools/replay/serve.mjs",recording,"0"],{cwd:project,env:{...process.env,REPLAY_REPORT:`${dir}/report.json`},stdio:["ignore","pipe","pipe"]}); children.push(server); const modelPort=await ready(server);
    const piDir=`${dir}/pi`; fs.mkdirSync(piDir); fs.writeFileSync(`${piDir}/models.json`,JSON.stringify({providers:{replay:{baseUrl:`http://127.0.0.1:${modelPort}/v1`,api:"openai-completions",apiKey:"dummy",models:[{id:"fixture",reasoning:false,input:["text"],contextWindow:8192,maxTokens:256}]}}}));
    const controlPort=19090+i, bin="target/debug/ascout-control-plane";
    const control=spawn(bin,[dir,process.env.PI_ROOT||"/home/betmul/BETMUL/AgentScout-suit/pi",policy,"--web",`127.0.0.1:${controlPort}`,"--model","replay/fixture","--thinking","low"],{cwd:project,env:{...process.env,ASCOUT_RUNTIME_CONFIG:runtime,PI_CODING_AGENT_DIR:piDir},stdio:["ignore","ignore","pipe"]}); children.push(control); let controlErr=""; control.stderr.on("data",b=>controlErr+=b);
    let accepted=false, lastStatus=null, lastBody=""; for(let t=0;t<300;t++){try{const response=await post(controlPort,"/api/request",JSON.stringify({action:"prompt",prompt:"Use nle_action once, then stop."})); lastStatus=response.status; lastBody=response.body; if(response.status===202){accepted=true;break;}}catch(error){lastBody=String(error);} if(control.exitCode!==null) break; await new Promise(r=>setTimeout(r,100));} assert.ok(accepted,`run ${i} prompt not accepted (status=${lastStatus}, body=${lastBody}): ${controlErr}`);
    for(let t=0;t<300;t++){if(fs.existsSync(`${dir}/session.jsonl`)){const a=audit(dir); if(a.candidate_count>=2&&a.promote_count>=1&&a.discard_count>=1) break;} await new Promise(r=>setTimeout(r,100));}
    await post(controlPort,"/api/request",JSON.stringify({action:"shutdown"})); assert.equal(await wait(control),0,`run ${i} control exit`); await stop(server); server=null;
    const actual=audit(dir); assert.deepEqual(actual,baseline,`run ${i} differs from recorded B3`);
    const report=JSON.parse(fs.readFileSync(`${dir}/report.json`)); assert.equal(report.hits,4); assert.equal(report.misses,0); assert.equal(report.exhausted,0); assert.equal(report.unconsumed,0);
    results.push({run:i,actual,report:{hits:report.hits,misses:report.misses,exhausted:report.exhausted,consumed:report.consumed,unconsumed:report.unconsumed}});
  }
  const ps=execFileSync("ps",["-eo","pid=,comm=,args="],{encoding:"utf8"}).split("\n").filter(l=>/(ascout-control-plane|ascout-runtime-adapter|nle-runtime-bridge|tools\/replay\/(serve|verify\/b3(-repeat)?)\.mjs)/.test(l)).filter(l=>!l.trim().startsWith(String(process.pid)+" ")).filter(l=>!(/\b(bash|timeout)\b/.test(l))); assert.deepEqual(ps,[],`processes remain:\n${ps.join("\n")}`);
  const ports=execFileSync("ss",["-ltnH"],{encoding:"utf8"}).split("\n").filter(l=>/:(1909[1-5])\b/.test(l)); assert.deepEqual(ports,[]);
  fs.writeFileSync(`${root}/repeat-report.json`,JSON.stringify({recording,baseline,results,ps_checked:true,ports_checked:true},null,2)); console.log(JSON.stringify({recording,runs:results,ps_checked:true,ports_checked:true},null,2));
} finally { await Promise.allSettled(children.map(stop)); }

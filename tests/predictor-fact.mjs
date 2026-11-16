// Regression for the saved 2026-11-16 first-request failure. No model calls.
import {test} from 'node:test';
import assert from 'node:assert/strict';
import {savedPolicy,runner} from './fixtures/predictor-fact-policy.mjs';
import {createAgentAccess} from '../agent-access/core/index.ts';
import {evaluatePolicyArtifact} from '../../pi/packages/ascout-agent-access/src/local-policy.ts';

// Frozen real Policy, with provenance paths sanitized; no temporary evidence input.
const saved=savedPolicy();
const artifact={digest:saved.digest,decisions:Object.fromEntries(Object.entries(saved.decisions).map(([decision,binding])=>{
    const manifest={version:saved.version,python:saved.python,files:saved.files,binding,decision,digest:saved.digest};
    return [decision,`import os,json\nos.dup2(1,3)\nos.dup2(2,1)\nMANIFEST = json.loads(${JSON.stringify(JSON.stringify(manifest))})\n${runner}`];
}))};

for(const kind of ['model_inference','nle_action'])test(`saved Policy receives predictor fact before ${kind} admission`,async()=>{
    const decisions=[],reports=[];
    const access=createAgentAccess({taskId:'regression',report:async m=>reports.push(m),evaluate:async(a,d,data,signal)=>{
        decisions.push(d);return evaluatePolicyArtifact(a,d,data,signal);
    }});
    access.setPolicy('authority',artifact);
    const op={kind,arguments:kind==='nle_action'?{action:0}:{model:'fixture/fixture'}};
    op.has_predictor=await access.hasPredictor('authority');
    assert.equal(op.has_predictor,true);
    assert.deepEqual(decisions,[],'predictor availability must not execute any Decision');
    assert.equal((await access.beforeAction({branch:'authority',actionId:kind,actionKind:kind==='model_inference'?kind:'tool_execution',name:kind,op,admit:true})).kind,'admit');
    assert.deepEqual(decisions,['admit.opportunity_pass','admit.feasibility_pass','extent']);
    assert.equal(reports.filter(m=>m.message==='speculative_admission_request').length,1);
    const proposals=await access.predict('authority',{chance_context:{},process_context:{arguments:op.arguments},assumption_set:[],capability_context:{}});
    assert.deepEqual(proposals,{proposals:[],generation:'batch'},'availability is not a promise of a nonempty prediction');
});

test('predictor availability follows delivery, inheritance, restore and cleanup; admission remains Policy-owned',async()=>{
    const calls=[];
    const access=createAgentAccess({taskId:'facts',report:async()=>{},evaluate:async(_a,d)=>{calls.push(d);return false}});
    assert.equal(await access.hasPredictor('authority'),false);
    access.setPolicy('authority',artifact);
    assert.equal(await access.hasPredictor('authority'),true);
    await access.control({command:'start_candidate',branch_id:'candidate',parent_branch_id:'authority'});
    assert.equal(await access.hasPredictor('candidate'),true);
    const restored=createAgentAccess({taskId:'restore',report:async()=>{},evaluate:async()=>{throw Error('must not evaluate')}});
    restored.restore(access.snapshot());
    assert.equal(await restored.hasPredictor('candidate'),true);
    assert.equal((await access.beforeAction({branch:'authority',actionId:'deny',actionKind:'model_inference',name:'model',op:{kind:'model_inference',has_predictor:true},admit:true})).kind,'allow');
    assert.deepEqual(calls,['admit.opportunity_pass']);
    await access.control({command:'cleanup',branch_id:'candidate'});
    assert.equal(await access.hasPredictor('candidate'),false);
    access.setPolicy('no-predict',{digest:'manual',decisions:{'admit.opportunity_pass':'unused'}});
    assert.equal(await access.hasPredictor('no-predict'),false);
});

test('Decision exceptions still propagate after predictor availability is established',async()=>{
    const access=createAgentAccess({taskId:'failure',report:async()=>{},evaluate:async()=>{throw Error('deliberate Policy failure')}});
    access.setPolicy('authority',artifact);
    const op={kind:'model_inference',has_predictor:await access.hasPredictor('authority')};
    await assert.rejects(access.beforeAction({branch:'authority',actionId:'failed',actionKind:'model_inference',name:'model',op,admit:true}),/deliberate Policy failure/);
});

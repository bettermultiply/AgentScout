import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { test } from 'node:test';
import { resolve } from 'node:path';

const policy = resolve('assets/policy/coding/decisions.py');
function call(functionName, data, config = {}) {
    const script = `import importlib.util, json\ns=importlib.util.spec_from_file_location('decisions', ${JSON.stringify(policy)})\nm=importlib.util.module_from_spec(s); s.loader.exec_module(m)\nprint(json.dumps(m.${functionName}(json.loads(${JSON.stringify(JSON.stringify(data))}), policy_config=json.loads(${JSON.stringify(JSON.stringify(config))}), capabilities={})))`;
    return JSON.parse(execFileSync('python3', ['-c', script], {encoding:'utf8'}));
}

test('policy ablation fixture covers model continuation and authority launch', () => {
    const model = {kind:'model_inference', authority:true, predicting:false};
    assert.equal(call('opportunity_pass', {op:model}, {fast_model_speculation:false}), false);
    assert.equal(call('opportunity_pass', {op:model}, {fast_model_speculation:true}), true);
    assert.equal(call('opportunity_pass', {op:{...model, authority:false, predicting:true}}, {fast_model_speculation:true}), true);
    assert.equal(call('launch', {chance_context:{arguments:{command:'python -m pytest -q'}}}, {skip_authority_after_independent_verification:true}), false);
    assert.equal(call('launch', {chance_context:{arguments:{command:'python -m pytest -q'}}}, {skip_authority_after_independent_verification:false}), true);
    const guidance = call('predict', {chance_context:{kind:'model_inference', arguments:{}}}, {
        fast_model_speculation:true, candidate_model:'deepseek/deepseek-flash',
    }).candidate_guidance;
    assert.match(guidance, /Batch independent symbol, call-site, and test lookups/);
    assert.deepEqual(call('predict', {chance_context:{kind:'model_inference', arguments:{}}}, {
        fast_model_speculation:true, candidate_model:'deepseek/deepseek-flash', candidate_thinking:'high',
        max_candidate_model_steps:1,
    }), {predict:'branch_model', model:'deepseek/deepseek-flash', thinking:'high', max_candidate_model_steps:1, candidate_guidance:guidance});
    assert.deepEqual(call('predict', {chance_context:{kind:'model_inference', arguments:{}}}, {
        fast_model_speculation:true, candidate_model:'deepseek/deepseek-flash', candidate_thinking:'high',
        max_candidate_model_steps:2,
    }), {predict:'branch_model', model:'deepseek/deepseek-flash', thinking:'high', max_candidate_model_steps:2, candidate_guidance:guidance});
    assert.equal(call('predict', {chance_context:{kind:'model_inference', arguments:{}}}, {
        fast_model_speculation:true, candidate_model:'deepseek/deepseek-flash', max_candidate_model_steps:0,
    }).abstain, true);
});


test('partial disposition transfers only independent results', () => {
    const context = {match:{details:{safe_result_ids:['bash-1']}}};
    assert.deepEqual(call('dispose', {resolution:'accepted', context}, {partial_accept_reuse:true, partial_accept_discard:true}), {
        actions:[{intent:'Reuse', results:[{id:'bash-1', notification:'Independent candidate result reused.'}]}, {intent:'Discard'}],
    });
    assert.deepEqual(call('dispose', {resolution:'accepted', context:{match:{details:{}}}}, {partial_accept_reuse:true}), {
        actions:[{intent:'Promote', notification:'Real verification command finished matching prediction:\n'}],
    });
    assert.deepEqual(call('dispose', {resolution:'rejected', context}, {partial_accept_salvage:true}), {
        actions:[{intent:'Salvage', results:[{id:'bash-1', notification:'Independent candidate result transferred.'}]}, {intent:'Discard'}],
    });
});

test('unverified workspace salvage is held for authority and cannot promote failed evidence', () => {
    const config = {require_authority_review:true, partial_accept_salvage:true,
        partial_accept_unverified_workspace:true};
    const candidate = {branch:{status:{blocked:true}, execution_state:{turn_active:false}},
        speculation_kind:'model_inference'};
    const edit = {action_id:'edit-1', tool_name:'edit', status:'succeeded', arguments:{path:'source.py', edits:[]}};
    const failedTest = {action_id:'test-1', tool_name:'bash', status:'failed',
        arguments:{command:'python -m pytest -q'}, result:{content:'1 failed'}};
    const match = call('match_proposal', {match_context:{candidate, candidate_results:[edit, failedTest]}, proposal:{}, evidence:{}}, config);
    assert.equal(match.verdict, 'incompatible');
    assert.equal(match.details.salvage_workspace, true);
    assert.equal(match.details.unverified_partial_draft, true);
    const disposition = call('dispose', {resolution:'rejected', context:{match}}, config);
    assert.deepEqual(disposition.actions.map(action => action.intent), ['Salvage', 'Discard']);
    assert.equal(disposition.actions[0].scopes[0], 'workspace');
    assert.match(disposition.actions[0].notification, /unverified partial draft/i);
    assert.match(disposition.actions[0].notification, /verify the entire public request/i);
    assert.equal(disposition.actions.some(action => action.intent === 'Promote'), false);
});


test('independent verification match accepts a successful local bash result', () => {
    const result = {action_id:'bash-1', tool_name:'bash', status:'succeeded', arguments:{command:'python -m pytest -q'}, result:{content:'2 passed'}};
    assert.deepEqual(call('match_proposal', {match_context:{candidate_results:[result]}, proposal:{}, evidence:{}}, {skip_authority_after_independent_verification:true}), {
        verdict:'compatible', score:1, details:{independent_verification:true, safe_result_ids:['bash-1']},
    });
});

test('verification evidence requires a standalone successful test and no later mutation', () => {
    const passing = id => ({action_id:id, tool_name:'bash', status:'succeeded',
        arguments:{command:'python -m pytest -q'}, result:{content:'1 passed'}});
    const piped = {...passing('piped'), arguments:{command:'python -m pytest -q | tail -1'}, result:{content:'1 passed'}};
    assert.equal(call('match_proposal', {match_context:{candidate_results:[piped]}, proposal:{}, evidence:{}},
        {skip_authority_after_independent_verification:true}).verdict, 'indeterminate');
    const edit = {action_id:'edit-1', tool_name:'edit', status:'succeeded', arguments:{path:'source.py', edits:[]}};
    assert.equal(call('match_proposal', {match_context:{candidate_results:[passing('test-1'), edit]}, proposal:{}, evidence:{}},
        {skip_authority_after_independent_verification:true}).verdict, 'indeterminate');
    assert.equal(call('match_proposal', {match_context:{candidate_results:[edit, passing('test-2')]}, proposal:{}, evidence:{}},
        {skip_authority_after_independent_verification:true}).verdict, 'compatible');
});

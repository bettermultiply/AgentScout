import fs from 'node:fs';
import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';

export const runner=fs.readFileSync(new URL('../../src/control_plane/policy/runner.py',import.meta.url),'utf8');
// Match CompiledPolicy::content_digest: Decision enum order, Binding struct
// field order, and recursively sorted serde_json::Value/BTreeMap keys.
const decisions=['admit.opportunity_pass','admit.feasibility_pass','select','rebalance','extent','predict','launch','effect','match','dispose'];
const sorted=value=>Array.isArray(value)?value.map(sorted):value&&typeof value==='object'?Object.fromEntries(Object.keys(value).sort().map(k=>[k,sorted(value[k])])):value;
export function policyDigest(policy) {
    const bindings=Object.fromEntries(decisions.map(d=>{
        const b=policy.decisions[d];
        return [d,{file:b.file,function:b.function,policy_config:sorted(b.policy_config),inputs:b.inputs}];
    }));
    return createHash('sha256').update(JSON.stringify([policy.version,policy.python,sorted(policy.files),policy.runtime,bindings,runner])).digest('hex');
}
export function savedPolicy() {
    const saved=JSON.parse(fs.readFileSync(process.env.ASCOUT_PREDICTOR_POLICY??new URL('./predictor-fact-policy.json',import.meta.url),'utf8'));
    assert.equal(policyDigest(saved),saved.digest,'frozen Policy source, bindings and runner must match the retained executable digest');
    return saved;
}

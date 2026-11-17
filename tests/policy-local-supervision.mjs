import { test } from 'node:test';
import assert from 'node:assert/strict';
import { evaluatePolicyArtifact, disposePolicyWorker } from '../../pi/packages/pi-integration/src/local-policy.ts';

test('repeated requests reuse one process and explicit disposal rebuilds it', async () => {
  const script = 'import os,sys,json\ncount=0\nfor line in sys.stdin.buffer:\n count+=1\n payload=json.dumps({"type":"result","request_id":json.loads(line)["request_id"],"result":{"count":count,"data":json.loads(line)["data"]}}).encode()\n sys.stdout.buffer.write(len(payload).to_bytes(4,"big")+payload)\n sys.stdout.buffer.flush()\n';
  const artifact = {digest:'reuse',decisions:{effect:script}};
  const first = await evaluatePolicyArtifact(artifact, 'effect', 1);
  const second = await evaluatePolicyArtifact(artifact, 'effect', 2);
  assert.equal(first.count, 1);
  assert.equal(second.count, 2);
  assert.equal(second.data, 2);
  await disposePolicyWorker();
  const rebuilt = await evaluatePolicyArtifact(artifact, 'effect', 3);
  assert.equal(rebuilt.data, 3);
  assert.equal(rebuilt.count, 1);
  await disposePolicyWorker();
});

test('an already cancelled evaluation is rejected before running Policy code', async () => {
  const controller = new AbortController();
  controller.abort();
  await assert.rejects(evaluatePolicyArtifact({digest:'cancelled',decisions:{effect:'raise RuntimeError("must not run")'}}, 'effect', {}, controller.signal), /cancelled/);
});

test('oversized Policy output is rejected while the process is still alive', async () => {
  const script = 'import os,time\nos.write(1,b"x" * (1024 * 1024 + 5))\ntime.sleep(30)';
  await assert.rejects(evaluatePolicyArtifact({digest:'oversized',decisions:{effect:script}}, 'effect', {}), /size limit/);
});

test('running evaluation cancellation kills and reaps the process', async () => {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), 100);
  try {
    await assert.rejects(evaluatePolicyArtifact({digest:'running',decisions:{effect:'import time\ntime.sleep(30)'}}, 'effect', {}, controller.signal), /cancelled/);
  } finally { clearTimeout(timer); }
});

test('a nonterminating evaluation reaches its wall deadline', async () => {
  await assert.rejects(evaluatePolicyArtifact({digest:'deadline',decisions:{effect:'import time\ntime.sleep(30)'}}, 'effect', {}), /deadline exceeded/);
});

test('oversized input is rejected before starting a process', async () => {
  await assert.rejects(evaluatePolicyArtifact({digest:'input',decisions:{effect:'raise RuntimeError("must not run")'}}, 'effect', 'x'.repeat(1024*1024)), /input exceeds size limit/);
});

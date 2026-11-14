import assert from 'node:assert/strict';
import { test } from 'node:test';
import type { AddressInfo } from 'node:net';
import { createReplayServer } from '../serve.ts';
import { requestKey, replyParts } from '../normalize.ts';
import type { Message } from '../types.ts';

const toolCalls = [{ id: 'fixture-call', type: 'function', function: { name: 'nle_action', arguments: '{"action":1}' } }];
// Pi's OpenAI provider serializes a native toolCall-only assistant this way.
const assistant = { role: 'assistant', content: null, tool_calls: toolCalls };
const user = { role: 'user', content: 'Use nle_action once, then stop.' };
const result = { role: 'tool', tool_call_id: 'fixture-call', content: 'moved' };
const native: Message[] = [user, { role: 'assistant', content: [{ type: 'toolCall', id: 'fixture-call', name: 'nle_action', arguments: { action: 1 } }] }, { role: 'toolResult', toolCallId: 'fixture-call', toolName: 'nle_action', content: [{ type: 'text', text: 'moved' }] }];

test('HTTP replay accepts Pi assistant content:null tool calls and matches native context', async t => {
  const failures: string[] = [];
  const replay = createReplayServer([{ key: requestKey(native), reply: [{ type: 'text', text: 'continuation hit' }] }], { onFailure: message => failures.push(message) });
  await new Promise<void>(resolve => replay.server.listen(0, '127.0.0.1', resolve));
  t.after(async () => { replay.server.closeAllConnections(); await new Promise(resolve => replay.server.close(resolve)); });
  const url = `http://127.0.0.1:${(replay.server.address() as AddressInfo).port}/v1/chat/completions`;
  const post = (message: unknown) => fetch(url, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ messages: [user, message, result] }) });

  const response = await post(assistant);
  const body = await response.text();
  assert.equal(response.status, 200, body);
  assert.match(response.headers.get('content-type') ?? '', /text\/event-stream/);
  assert.match(body, /continuation hit/);
  assert.match(body, /data: \[DONE\]/);
  assert.deepEqual(failures, []);
  assert.equal(replay.report().hits, 1);
  assert.equal(replay.report().consumed, 1);
  assert.equal(replay.report().unconsumed, 0);
  assert.equal(replay.report().misses, 0);
  assert.deepEqual(replyParts(assistant), [{ type: 'toolCall', id: 'fixture-call', name: 'nle_action', arguments: { action: 1 } }]);

  // Accepting null must not bypass content, role, or tool-call field checks.
  const invalid = [
    { ...assistant, role: 42 },
    { ...assistant, role: 'user' },
    { role: 'assistant', content: null },
    { ...assistant, tool_calls: [] },
    ...[42, {}, [null], [{ type: 'text', text: 42 }]].map(content => ({ ...assistant, content })),
    ...[null, {}, [null], [{ id: 42 }], [{ function: null }], [{ function: { name: 42 } }]].map(tool_calls => ({ ...assistant, tool_calls })),
    { ...assistant, tool_call_id: 42 },
  ];
  for (const message of invalid) {
    const rejected = await post(message);
    assert.equal(rejected.status, 400, JSON.stringify(message));
    assert.equal((await rejected.json()).error.message, 'messages must be an array of messages');
  }
  assert.equal(failures.length, invalid.length);
  assert.equal(replay.report().total_requests, 1, 'invalid messages must not reach key lookup');
  assert.equal(replay.report().exhausted, 0);
});

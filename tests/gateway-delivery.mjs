import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createServer, connect } from 'node:net';
import { once } from 'node:events';
import { AgentAccessCore, messages, writeMessage } from '../../pi/packages/pi-integration/src/core.ts';

test('delivery receipts deduplicate a retry but allow a new intentional execution', { timeout: 5000 }, async () => {
  const server = createServer();
  server.listen(0, '127.0.0.1');
  await once(server, 'listening');
  const accepted = once(server, 'connection');
  const client = connect(server.address().port, '127.0.0.1');
  await once(client, 'connect');
  const [peer] = await accepted;
  let executions = 0;
  const adapter = { id: 'fake', capabilities: {}, async handle() { executions++; } };
  const running = new AgentAccessCore().run({ socketPath: '', socket: client, adapter });
  const incoming = messages(peer)[Symbol.asyncIterator]();
  const command = { command: 'pause', branch_id: 'branch' };
  async function receipt(id) {
    await writeMessage(peer, { kind: 'delivery', payload: { id, command } });
    for (;;) {
      const next = await incoming.next();
      assert.equal(next.done, false);
      if (next.value.kind === 'received') {
        assert.equal(next.value.payload.id, id);
        return;
      }
    }
  }
  try {
    await receipt(1);
    await receipt(1);
    assert.equal(executions, 1);
    await receipt(2);
    await receipt(2);
    assert.equal(executions, 2);
    peer.end();
    await running;
  } finally {
    client.destroy();
    peer.destroy();
    server.close();
  }
});

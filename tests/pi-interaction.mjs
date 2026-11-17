import { test } from 'node:test';
import assert from 'node:assert/strict';
import { Interaction } from '../agent-access/pi/interaction.ts';

test('answers and explicit cancellation settle text prompts; silence stays pending', async () => {
    const events = [];
    const interaction = new Interaction(async event => { events.push(event); });
    const ui = interaction.context('authority');
    for (const [kind, args, answer] of [
        ['confirm', ['Approve', 'Run?'], true],
        ['select', ['Pick', ['a', 'b']], 'b'],
        ['input', ['Name'], 'hello'],
        ['editor', ['Edit', 'before'], 'one\ntwo'],
    ]) {
        let settled = false;
        const pending = ui[kind](...args).then(value => { settled = true; return value; });
        await Promise.resolve();
        assert.equal(settled, false);
        interaction.answer('wrong', events.at(-1).prompt_id, answer, false);
        await Promise.resolve();
        assert.equal(settled, false);
        interaction.answer('authority', events.at(-1).prompt_id, answer, false);
        await Promise.resolve();
        assert.equal(settled, true, `${kind} answer must settle`);
        assert.equal(await pending, answer);
    }
    let settled = false;
    const pending = ui.input('Cancel me').then(value => { settled = true; return value; });
    interaction.cancel('authority');
    await Promise.resolve();
    assert.equal(settled, true, 'branch cancellation must settle');
    assert.equal(await pending, undefined);
    interaction.resume('authority');
    const cancelled = ui.confirm('Cancel', '?');
    interaction.answer('authority', events.at(-1).prompt_id, null, true);
    assert.equal(await cancelled, false);
    const controller = new AbortController();
    const aborted = ui.select('Abort', ['a'], { signal: controller.signal });
    controller.abort();
    assert.equal(await aborted, undefined);
    assert.equal(events.at(-1).message, 'ui_prompt_cancelled');
});

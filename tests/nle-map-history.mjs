import test from 'node:test'
import assert from 'node:assert/strict'
import { eventHistory } from '../webui/src/nleMapHistory.ts'

const observation = (x) => ({ chars: ['.@.'], blstats: [x, 0], message: `x=${x}` })
const action = (sequence, task_id, branch_id, action_id, x) => ({ sequence, timestamp_ms: sequence, task_id, event: 'nle_map_action', branch_id, action_id, action: x, before: observation(x - 1), after: observation(x) })

test('authority window joins executed actions across promoted branches', () => {
  const result = eventHistory([
    action(10, 'task-1', 'A', 'a1', 1),
    { sequence: 11, timestamp_ms: 11, task_id: 'task-1', event: 'authority_changed', branch_id: 'B', previous_authority: 'A' },
    action(12, 'task-1', 'B', 'b1', 2),
    { sequence: 13, timestamp_ms: 13, task_id: 'task-1', event: 'authority_changed', branch_id: 'C', previous_authority: 'B' },
    action(14, 'task-1', 'C', 'c1', 3),
    { sequence: 15, timestamp_ms: 15, task_id: 'task-1', event: 'branch_created', branch_id: 'candidate', parent_branch_id: 'C', role: 'candidate', chance_id: null, speculation_kind: 'tool_execution', speculation_started_at_ms: 15 },
    action(16, 'task-2', 'other', 'other-1', 99),
  ])
  assert.deepEqual(result.history.map((item) => item.actionNumber), [0, 1, 2, 3])
  assert.deepEqual(result.branchSources, ['A', 'B', 'C'])
  assert.equal(result.quality, 'complete')
  assert.equal(result.history.at(-1)?.player.x, 3)
})

test('duplicate executed events are ignored without resetting the cursor history', () => {
  const result = eventHistory([action(1, 'task-1', 'A', 'a1', 1), action(2, 'task-1', 'B', 'b1', 1), action(3, 'task-1', 'B', 'b1', 2)])
  assert.deepEqual(result.history.map((item) => item.actionNumber), [0, 1, 2])
  assert.equal(result.quality, 'partial')
})

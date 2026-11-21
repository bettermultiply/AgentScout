import test from 'node:test'
import assert from 'node:assert/strict'
import { createServer } from '../webui/node_modules/vite/dist/node/index.js'

const server = await createServer({ root: new URL('../webui', import.meta.url).pathname, server: { middlewareMode: true, hmr: false }, appType: 'custom' })
const { foldEvents } = await server.ssrLoadModule('/src/fold.ts')
const { layoutTree, DEFAULT_TUNING } = await server.ssrLoadModule('/src/tree/index.ts')
const { createTreeLayoutCache } = await server.ssrLoadModule('/src/tree/geometry.ts')
const { buildTree } = await server.ssrLoadModule('/src/components/treeGraph.ts')
const { branchTone, callDetailLines } = await server.ssrLoadModule('/src/tree/index.ts')
test.after(() => server.close())

function fixture() {
  const events = []
  const add = (branch_id, event, fields = {}) => events.push({ task_id: 'cache', branch_id, event, action_id: null, sequence: events.length + 1, timestamp_ms: 0, ...fields })
  add('root', 'branch_created', { role: 'authority', parent_branch_id: null })
  add('root', 'model_usage', { model: 'fixture', total_tokens: 1, cost: null, started_at_ms: 0, finished_at_ms: 200, duration_ms: 200 })
  add('root', 'model_usage', { model: 'fixture', total_tokens: 1, cost: null, started_at_ms: 10000, finished_at_ms: 10200, duration_ms: 200 })
  for (const id of ['a', 'b']) {
    add('root', 'chance_created', { parent_branch_id: 'root', chance_id: `chance-${id}`, operation_id: id, tool_name: 'test', timestamp_ms: 3000, speculation_started_at_ms: 3000 })
    add(id, 'branch_created', { parent_branch_id: 'root', chance_id: `chance-${id}`, role: 'candidate' })
    add(id, 'model_usage', { model: 'fixture', total_tokens: 1, cost: null, started_at_ms: 4000, finished_at_ms: 4200, duration_ms: 200 })
    add(id, 'model_usage', { model: 'fixture', total_tokens: 1, cost: null, started_at_ms: 8000, finished_at_ms: 8200, duration_ms: 200 })
  }
  return foldEvents(events).tasks.cache
}

test('source admission and accepted Proposal validation determine geometry', () => {
  const events = [
    { event: 'branch_created', branch_id: 'root', role: 'authority', parent_branch_id: null },
    ...[0, 10000, 20000].map(time => ({ event: 'model_usage', branch_id: 'root', model: 'fixture', started_at_ms: time, finished_at_ms: time + 200, duration_ms: 200 })),
    { event: 'tool_finished', branch_id: 'root', action_id: 'later', tool_name: 'test', status: 'succeeded', started_at_ms: 18000, finished_at_ms: 18500, duration_ms: 500 },
    { event: 'chance_created', chance_id: 'c', parent_branch_id: 'root', parent_chance_id: null, operation_id: 'op', tool_name: 'test', stage: 'pending', authority_state: 'not_requested', speculation_started_at_ms: 5000, timestamp_ms: 19000 },
    { event: 'proposal_added', chance_id: 'c', proposal_id: 'p', payload: null },
    { event: 'branch_created', branch_id: 'child', parent_branch_id: 'root', role: 'candidate', chance_id: 'c' },
    { event: 'proposal_validated', chance_id: 'c', proposal_id: 'p', validation_id: 9, validated_at_ms: 15000 },
    { event: 'proposal_validated', chance_id: 'c', proposal_id: 'other', validation_id: 10, validated_at_ms: 19000 },
    { event: 'chance_stage_changed', chance_id: 'c', stage: { accepted: 'p' }, validation_id: 9 },
  ].map((event, i) => ({ task_id: 'timing', sequence: i + 1, timestamp_ms: 30000, ...event }))
  const task = foldEvents(events).tasks.timing
  const cache = createTreeLayoutCache()
  const geometry = cache.layout(task).get('root')
  const segment = DEFAULT_TUNING.segmentK * Math.log1p(10000 / DEFAULT_TUNING.segmentTauMs)
  const secondSegment = DEFAULT_TUNING.segmentK * Math.log1p(8000 / DEFAULT_TUNING.segmentTauMs)
  assert.ok(Math.abs(geometry.chancePositions.get('c') - segment / 2) < 1e-8)
  assert.ok(Math.abs(geometry.pathDistances.at(-1) - (segment + secondSegment * 5 / 8)) < 1e-8)
  assert.equal(geometry.calls.length, 2)
  assert.equal(task.branches.root.modelUsages.length, 3)
  assert.equal(task.branches.root.tools.length, 1)
  assert.ok(geometry.validationEnd)
  assert.deepEqual(cache.layout(task), layoutTree(task))
  const corrected = structuredClone(task)
  corrected.chances.c.validations[9].validatedAtMs = 12000
  corrected.chances.c.speculationStartedAtMs = 3000
  const updated = cache.layout(corrected)
  assert.deepEqual(updated, layoutTree(corrected))
  assert.ok(Math.abs(updated.get('root').chancePositions.get('c') - segment * .3) < 1e-8)
  assert.ok(Math.abs(updated.get('root').pathDistances.at(-1) - (segment + secondSegment * .25)) < 1e-8)
  const unrelated = structuredClone(corrected)
  unrelated.branches.root.status = 'cleaned'
  assert.strictEqual(cache.layout(unrelated).get('root').trunk, updated.get('root').trunk)
  const missing = structuredClone(corrected)
  missing.chances.c.validations[9].validatedAtMs = null
  assert.throws(() => cache.layout(missing), /缺少已接受 Proposal 的验证时间/)
  assert.equal(cache.stats.entries, 0)
  const wrongProposal = structuredClone(corrected)
  wrongProposal.chances.c.validations[9].proposalId = 'other'
  assert.throws(() => cache.layout(wrongProposal), /缺少已接受 Proposal 的验证时间/)
  const noCandidate = foldEvents(events.filter(e => !['branch_created', 'proposal_added', 'proposal_validated', 'chance_stage_changed'].includes(e.event) || e.branch_id === 'root')).tasks.timing
  assert.ok(Math.abs(layoutTree(noCandidate).get('root').chancePositions.get('c') - segment / 2) < 1e-8)
})

test('unchanged geometry survives unrelated/display changes and only changed branch is computed', () => {
  const cache = createTreeLayoutCache(), task = fixture()
  const first = cache.layout(task, DEFAULT_TUNING)
  assert.deepEqual(first, layoutTree(task, DEFAULT_TUNING))
  assert.equal(cache.stats.computedBranches, 3)
  assert.equal(cache.stats.computedWalks, 3)
  const display = structuredClone(task)
  display.branches.a.status = 'discarded'
  display.branches.a.events.find(e => e.event === 'model_usage').model = 'fresh-model'
  const fresh = cache.layout(display, DEFAULT_TUNING)
  assert.deepEqual(fresh, layoutTree(display, DEFAULT_TUNING))
  assert.strictEqual(fresh.get('a').trunk, first.get('a').trunk)
  assert.strictEqual(fresh.get('a').pathPoints, first.get('a').pathPoints)
  assert.equal(fresh.get('a').calls[0].events[0].model, 'fresh-model')
  const graph = buildTree(display, DEFAULT_TUNING, (t, p) => cache.layout(t, p))
  const node = graph.nodes.find(n => n.id === 'branch:a')
  assert.equal(branchTone(node.data.branch, node.data.isAuthorityPath, node.data.isCurrentAuthority), 'aborted')
  assert.ok(callDetailLines(node.data.geometry.calls[0]).some(line => line.text.includes('fresh-model')))
  assert.equal(cache.stats.computedBranches, 3)
  const changed = structuredClone(display)
  changed.branches.a.events.find(e => e.event === 'model_usage').duration_ms = 900
  assert.deepEqual(cache.layout(changed, DEFAULT_TUNING), layoutTree(changed, DEFAULT_TUNING))
  assert.equal(cache.stats.computedBranches, 4)
  assert.equal(cache.stats.computedWalks, 3, 'duration changes reuse all centre-line samples')
  const afterDuration = structuredClone(changed)
  afterDuration.branches.a.events.find(e => e.event === 'model_usage').duration_ms = 1200
  assert.deepEqual(cache.layout(afterDuration, DEFAULT_TUNING), layoutTree(afterDuration, DEFAULT_TUNING))
  assert.equal(cache.stats.computedBranches, 5)
  assert.equal(cache.stats.computedWalks, 3)
  console.log(`branch reuse: initial=3, display=3, duration updates=5; sampled walks=${cache.stats.computedWalks}`)
  assert.equal(cache.stats.entries, 3)
})

test('width, parent boundary, child root width, chance time and tuning invalidate safely', () => {
  const cache = createTreeLayoutCache()
  let task = fixture()
  for (const mutate of [
    () => {},
    t => { t.branches.a.events.push({ ...t.branches.a.events.at(-1), sequence: 100, started_at_ms: 14000 }) },
    t => { t.branches.root.events.find(e => e.event === 'model_usage').started_at_ms = -500 },
    t => { t.chances['chance-a'].speculationStartedAtMs = 7000 },
    t => { t.branches.b.chanceId = 'chance-a' },
    t => { t.branches.b.events[1].started_at_ms = 9000 },
    t => { t.branches.a.parentBranchId = 'b' },
  ]) {
    task = structuredClone(task)
    mutate(task)
    assert.deepEqual(cache.layout(task, DEFAULT_TUNING), layoutTree(task, DEFAULT_TUNING))
  }
  for (const key of Object.keys(DEFAULT_TUNING)) {
    const tuning = { ...DEFAULT_TUNING, [key]: DEFAULT_TUNING[key] + 0.1 }
    assert.deepEqual(cache.layout(task, tuning), layoutTree(task, tuning), key)
  }
})

test('errors replace success and removed tasks/branches release cached entries', () => {
  const cache = createTreeLayoutCache(), task = fixture()
  cache.layout(task, DEFAULT_TUNING)
  const broken = structuredClone(task)
  broken.branches.a.events.find(e => e.event === 'model_usage').started_at_ms = null
  assert.throws(() => cache.layout(broken, DEFAULT_TUNING), /缺少开始时间/)
  assert.equal(cache.stats.entries, 0)
  cache.layout(task, DEFAULT_TUNING)
  const missingChance = structuredClone(task)
  missingChance.chances['chance-a'].speculationStartedAtMs = null
  assert.throws(() => cache.layout(missingChance, DEFAULT_TUNING), /缺少投机起点时间/)
  assert.equal(cache.stats.entries, 0)
  cache.layout(task, DEFAULT_TUNING)
  const reduced = structuredClone(task)
  reduced.branchOrder = ['root']
  reduced.branches = { root: reduced.branches.root }
  assert.deepEqual(cache.layout(reduced, DEFAULT_TUNING), layoutTree(reduced, DEFAULT_TUNING))
  assert.equal(cache.stats.entries, 1)
  cache.clear()
  assert.equal(cache.stats.entries, 0)
  for (let i = 0; i < 100; i++) {
    const next = structuredClone(task)
    next.branches.a.events[1].duration_ms = i
    cache.layout(next, DEFAULT_TUNING)
    assert.equal(cache.stats.entries, 3)
  }
  cache.layout({ ...task, taskId: 'other' }, DEFAULT_TUNING)
  assert.equal(cache.stats.entries, 3)
  console.log(`100 revisions retain ${cache.stats.entries} entries (one per current branch)`)
})

test('latest Chance locates its drawn parent and yields to a newer Branch', () => {
  const task = fixture()
  task.chances['chance-a'].lastEventSequence = 100
  task.chances.orphan = { ...task.chances['chance-a'], chanceId: 'orphan', parentBranchId: 'missing', lastEventSequence: 102 }
  assert.equal(buildTree(task, DEFAULT_TUNING).latestNodeId, 'branch:root')
  task.branches.a.lastEventSequence = 101
  assert.equal(buildTree(task, DEFAULT_TUNING).latestNodeId, 'branch:a')
})

test('store updates while hidden/detail, returns latest without recomputation and releases on stop', async () => {
  globalThis.window = { localStorage: { getItem: () => null, setItem() {} }, matchMedia: () => ({ matches: false }) }
  try {
    const { useAppStore } = await server.ssrLoadModule('/src/store.ts')
    const { treeLayoutStats } = await server.ssrLoadModule('/src/tree/geometry.ts')
    const task = fixture()
    useAppStore.setState({ state: { sequence: 50, tasks: { cache: task }, taskOrder: ['cache'] }, selectedTaskId: 'cache' })
    useAppStore.getState().selectTask('cache')
    const before = treeLayoutStats()
    useAppStore.getState().openBranch('a')
    useAppStore.getState().ingestEvent({ event: 'model_usage', task_id: 'cache', branch_id: 'a', sequence: 51, timestamp_ms: 15000, model: 'fixture', total_tokens: 1, cost: null, started_at_ms: 15000, finished_at_ms: 15500, duration_ms: 500 })
    assert.ok(treeLayoutStats().computedBranches > before.computedBranches)
    useAppStore.getState().setPanelOpen(false)
    useAppStore.getState().ingestEvent({ event: 'model_usage', task_id: 'cache', branch_id: 'b', sequence: 52, timestamp_ms: 16000, model: 'fixture', total_tokens: 1, cost: null, started_at_ms: 16000, finished_at_ms: 16500, duration_ms: 500 })
    const hidden = treeLayoutStats()
    const graph = useAppStore.getState().treeGraph
    let sequence = 52
    for (const event of [
      { event: 'ui_notification', task_id: 'cache', message: 'notice' },
      { event: 'run_status', task_id: 'cache', connected: true },
      { event: 'branch_created', task_id: 'other', branch_id: 'a', parent_branch_id: null, role: 'authority' },
    ]) {
      useAppStore.getState().ingestEvent({ sequence: ++sequence, timestamp_ms: 17000, ...event })
      assert.strictEqual(useAppStore.getState().treeGraph, graph, event.event)
    }
    useAppStore.getState().setTreeTuning({ ...DEFAULT_TUNING })
    assert.strictEqual(useAppStore.getState().treeGraph, graph)
    assert.equal(graph.nodes.find(n => n.id === 'branch:b').data.geometry.sections.length, 3)
    useAppStore.getState().ingestEvent({ event: 'conversation_updated', task_id: 'cache', branch_id: 'b', sequence: ++sequence, timestamp_ms: 17000 })
    assert.equal(treeLayoutStats().computedBranches, hidden.computedBranches)
    assert.equal(treeLayoutStats().layouts, hidden.layouts)
    const updated = useAppStore.getState().treeGraph
    assert.equal(updated.latestNodeId, 'branch:b')
    assert.equal(updated.nodes[2].data.branch.conversationUpdatedSequence, sequence)
    assert.strictEqual(updated.nodes[2].data.geometry, graph.nodes[2].data.geometry)
    assert.strictEqual(updated.nodes[0], graph.nodes[0])
    assert.strictEqual(updated.nodes[1], graph.nodes[1])
    useAppStore.getState().setTreeTuning({ ...DEFAULT_TUNING, rootWidth: 12 })
    assert.deepEqual(useAppStore.getState().treeGraph, buildTree(useAppStore.getState().state.tasks.cache, { ...DEFAULT_TUNING, rootWidth: 12 }))
    const tuned = treeLayoutStats()
    useAppStore.getState().ingestEvent({ event: 'proposal_validated', task_id: 'cache', chance_id: 'chance-a', proposal_id: 'p', validation_id: 91, validated_at_ms: 5000, sequence: ++sequence, timestamp_ms: 18000 })
    const validated = useAppStore.getState().treeGraph
    assert.equal(validated.nodes[0].data.childForks.find(f => f.chance.chanceId === 'chance-a').chance.validations[91].proposalId, 'p')
    assert.equal(validated.latestNodeId, 'branch:root')
    useAppStore.getState().ingestEvent({ event: 'chance_stage_changed', task_id: 'cache', chance_id: 'chance-a', stage: { accepted: 'p' }, validation_id: 91, reason: null, sequence: ++sequence, timestamp_ms: 19000 })
    const acceptedGraph = useAppStore.getState().treeGraph
    assert.equal(acceptedGraph.latestNodeId, 'branch:root')
    assert.equal(acceptedGraph.nodes.find(n => n.id === 'branch:root').data.geometry.validationEnd.validatedAtMs, 5000)
    assert.deepEqual(acceptedGraph, buildTree(useAppStore.getState().state.tasks.cache, { ...DEFAULT_TUNING, rootWidth: 12 }))
    const returnCount = treeLayoutStats().computedBranches
    useAppStore.getState().setPanelOpen(true)
    useAppStore.getState().showTree()
    assert.equal(treeLayoutStats().computedBranches, returnCount)
    assert.ok(returnCount >= tuned.computedBranches)
    assert.equal(useAppStore.getState().treeGraph.nodes.find(n => n.id === 'branch:b').data.geometry.sections.length, 3)
    useAppStore.getState().selectTask('other')
    assert.equal(treeLayoutStats().entries, 1)
    assert.equal(useAppStore.getState().treeGraph.nodes[0].data.branch.role, 'authority')
    useAppStore.getState().selectTask('cache')
    assert.equal(treeLayoutStats().entries, 3)
    assert.deepEqual(useAppStore.getState().treeGraph, acceptedGraph)
    const beforeStop = treeLayoutStats().computedBranches
    useAppStore.getState().stop()
    assert.equal(treeLayoutStats().entries, 0)
    assert.equal(useAppStore.getState().treeGraph.nodes.length, 0)
    console.log(`layout counters: initial=${before.computedBranches}, hidden=${hidden.computedBranches}, return=${returnCount}; layouts=${hidden.layouts}`)
    assert.equal(treeLayoutStats().computedBranches, beforeStop)

    // Drive restoration through the normal state-event-set entry point.
    const { fetch, EventSource } = globalThis
    let events = [{ event: 'branch_created', task_id: 'cache', branch_id: 'a', parent_branch_id: null,
      role: 'authority', chance_id: null, sequence: 1, timestamp_ms: 100 }]
    try {
      globalThis.fetch = async () => Response.json({ events, sequence: events.at(-1)?.sequence ?? 0 })
      globalThis.EventSource = class { addEventListener() {} close() {} }
      useAppStore.getState().setPanelOpen(false)
      await useAppStore.getState().resync()
      const restored = useAppStore.getState().treeGraph
      assert.equal(restored.nodes.length, 1)
      assert.equal(restored.nodes[0].data.branch.role, 'authority')
      assert.equal(restored.nodes[0].data.tuning.rootWidth, 12)
      events = []
      await useAppStore.getState().resync()
      assert.equal(useAppStore.getState().selectedTaskId, null)
      assert.equal(useAppStore.getState().treeGraph.nodes.length, 0)
      assert.equal(treeLayoutStats().entries, 0)
    } finally {
      useAppStore.getState().stop()
      globalThis.fetch = fetch
      if (EventSource === undefined) delete globalThis.EventSource
      else globalThis.EventSource = EventSource
    }
  } finally { delete globalThis.window }
})

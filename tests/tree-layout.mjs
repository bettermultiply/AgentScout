import test from 'node:test'
import assert from 'node:assert/strict'
import { createElement } from '../webui/node_modules/react/index.js'
import { renderToStaticMarkup } from '../webui/node_modules/react-dom/server.node.js'
import { createServer } from '../webui/node_modules/vite/dist/node/index.js'

// Use the existing Vite TypeScript loader and Node test runner, without a new framework.
const server = await createServer({ root: new URL('../webui', import.meta.url).pathname, server: { middlewareMode: true }, appType: 'custom' })
const { foldEvents } = await server.ssrLoadModule('/src/fold.ts')
const { layoutTree, DEFAULT_TUNING, normalizeTuning } = await server.ssrLoadModule('/src/tree/index.ts')
const { buildTree } = await server.ssrLoadModule('/src/components/treeGraph.ts')
const { startLead } = await server.ssrLoadModule('/src/tree/startLead.ts')
const { sampleGeometry } = await server.ssrLoadModule('/src/tree/geometry.ts')
const { placeStems } = await server.ssrLoadModule('/src/tree/stemPlacement.ts')
test.after(() => server.close())

const stem = { id: 's', distance: 20, range: 10, preferred: 1, base: 20, angles: [10, 20, 30] }
const place = (s = stem, n = []) => placeStems([s], n).get(s.id)
test('no neighbors retains preferred side and nearest base', () => assert.deepEqual(place(), { side: 1, tilt: 20 }))
test('same psi child switches side', () => assert.deepEqual(place(stem, [{distance:20,side:1,angleDeg:70}]), {side:-1,tilt:20}))
test('two sides choose greatest minimum clearance', () => assert.deepEqual(place(stem, [{distance:20,side:1,angleDeg:70},{distance:20,side:-1,angleDeg:50}]), {side:-1,tilt:10}))
test('fixed mode changes side but not angle', () => assert.deepEqual(place({...stem,angles:[20]},[{distance:20,side:1,angleDeg:70}]),{side:-1,tilt:20}))
test('neighbors beyond S are ignored', () => assert.deepEqual(place(stem,[{distance:31,side:1,angleDeg:70}]),{side:1,tilt:20}))
test('placement is deterministic and sorts by distance', () => {
 const a={...stem,id:'a',distance:0}, b={...stem,id:'b',distance:5}
 assert.deepEqual(placeStems([b,a],[]),placeStems([a,b],[]))
 assert.deepEqual(placeStems([b,a],[]),placeStems([b,a],[]))
})

test('startLead uses initialAngle for a zero first segment without mutation', () => {
  const points = [{ x: 2, y: 3 }, { x: 2, y: 3 }], widths = [10, 4]
  const beforePoints = structuredClone(points), beforeWidths = [...widths]
  for (const initialAngle of [0, Math.PI / 3, -Math.PI / 4]) {
    const ratio = 0.5
    let drawn
    assert.doesNotThrow(() => { drawn = startLead(points, widths, initialAngle, ratio) })
    assert.deepEqual(drawn.points, [{
      x: points[0].x - Math.sin(initialAngle) * ratio * widths[0],
      y: points[0].y - Math.cos(initialAngle) * ratio * widths[0],
    }, ...points])
    assert.deepEqual(drawn.widths, [widths[0], ...widths])
    assert.deepEqual(points, beforePoints)
    assert.deepEqual(widths, beforeWidths)
  }
})

// Same 195 model / 194 tool call count as the reported session. Explicit times
// make the large sampled outline deterministic without private session data.
function taskWithCalls() {
  const base = { task_id: 'layout-regression', branch_id: 'authority' }
  const events = [{ ...base, event: 'branch_created', role: 'authority', parent_branch_id: null, sequence: 1, timestamp_ms: 0 }]
  for (let i = 0; i < 389; i++) {
    events.push({ ...base, event: i % 2 ? 'tool_finished' : 'model_usage', sequence: i + 2,
      timestamp_ms: i * 15000, started_at_ms: i * 15000, duration_ms: 1000,
      finished_at_ms: i * 15000 + 1000, status: 'succeeded' })
  }
  return foldEvents(events).tasks[base.task_id]
}

for (const [name, tuning] of [
  ['default', DEFAULT_TUNING],
  ['large legal sampled outline', normalizeTuning({ segmentK: 300, segmentTauMs: 500, sampleGap: 4 })],
]) {
  test(`layout preserves every call and sample: ${name}`, () => {
    const task = taskWithCalls()
    const geometry = layoutTree(task, tuning).get('authority')
    const expectedSamples = 1 + 388 * Math.max(1, Math.ceil(tuning.segmentK * Math.log1p(15000 / tuning.segmentTauMs) / tuning.sampleGap))
    assert.equal(geometry.pathPoints.length, expectedSamples)
    const drawn = expectedSamples + (tuning.startLeadRatio > 0 ? 1 : 0)
    assert.equal(geometry.trunk.points.length, 6 * drawn - 4)
    const lead0 = layoutTree(task, { ...tuning, startLeadRatio: 0 }).get('authority')
    assert.equal(lead0.trunk.points.length, 6 * expectedSamples - 4)
    assert.equal(geometry.calls.length, 389)
    assert.equal(geometry.sections.length, 195)
    assert.equal(geometry.sections.reduce((n, section) => n + section.leaves.length, 0), 194)
    assert.ok(Number.isFinite(geometry.width) && Number.isFinite(geometry.height))
    // Every outline control point is retained inside the computed node bounds.
    for (const point of geometry.trunk.points) {
      assert.ok(point.x + geometry.offsetX >= -1e-8 && point.x + geometry.offsetX <= geometry.width + 1e-8)
      assert.ok(point.y + geometry.offsetY >= -1e-8 && point.y + geometry.offsetY <= geometry.height + 1e-8)
    }
    const graph = buildTree(task, tuning)
    assert.equal(graph.error, null)
    assert.equal(graph.nodes.length, task.branchOrder.length)
  })
}

test('missing start timestamps remain visible errors, without dropping calls', () => {
  const task = taskWithCalls()
  task.branches.authority.events[1].started_at_ms = null
  const graph = buildTree(task, DEFAULT_TUNING)
  assert.match(graph.error, /缺少开始时间（事件 #2）/)
  assert.equal(graph.nodes.length, 0)
})

for (const starts of [[0], [0, 0, 0, 1000, 2000], [0, 1000, 2000, 3000, 4000]]) {
  test(`stem anchors use completion-time centre line: ${starts}`, () => {
    const events = [{ event: 'branch_created', task_id: 'anchors', branch_id: 'root', role: 'authority', parent_branch_id: null, sequence: 1, timestamp_ms: 0 },
      ...starts.map((time, i) => ({ event: 'model_usage', task_id: 'anchors', branch_id: 'root', sequence: i + 2,
        timestamp_ms: time + 500, started_at_ms: time, finished_at_ms: time + 500, duration_ms: 500 }))]
    const g = layoutTree(foldEvents(events).tasks.anchors, { ...DEFAULT_TUNING, rootWidth: 120, turnStepDeg: 40 }).get('root')
    for (const section of g.sections) {
      const anchor = section.shape.anchor
      const lengths = starts.map((time, i) => i ? DEFAULT_TUNING.segmentK * Math.log1p((time - starts[i - 1]) / DEFAULT_TUNING.segmentTauMs) : 0)
      const finish = section.twig.events[0].finished_at_ms
      let distance = 0
      for (let i = 1; i < starts.length; i++) {
        if (finish >= starts[i]) distance += lengths[i]
        else { distance += lengths[i] * Math.max(0, (finish - starts[i - 1]) / (starts[i] - starts[i - 1])); break }
      }
      const expected = sampleGeometry(g, distance)
      assert.ok(Math.abs(section.y - distance) < 1e-9)
      assert.ok(Math.hypot(expected.x - anchor.x, expected.y - anchor.y) < 1e-8)
    }
    assert.equal(g.sections.length, starts.length)
    for (let i = 1; i < g.pathDistances.length; i++) assert.ok(g.pathDistances[i] > g.pathDistances[i - 1])
    assert.ok(g.pathPoints.length > 1 || g.trunk.stroke > 0, 'zero-length branch must render its cross-section')
  })
}

test('tree failure heading does not misdiagnose an unpaired tool as missing timestamps', async () => {
  const task = taskWithCalls()
  task.branches.authority.events[1].event = 'tool_finished'
  globalThis.window = { location: { search: '' }, localStorage: { getItem: () => null }, matchMedia: () => ({ matches: false }) }
  try {
    const { SpeculationTree } = await server.ssrLoadModule('/src/components/SpeculationTree.tsx')
    const html = renderToStaticMarkup(createElement(SpeculationTree, { task }))
    assert.match(html, /投机树绘制失败/)
    assert.match(html, /之前没有模型调用/)
    assert.doesNotMatch(html, /观察数据缺少时间戳/)
  } finally {
    delete globalThis.window
  }
})

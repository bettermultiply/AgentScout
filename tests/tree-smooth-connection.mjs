import test from 'node:test'
import assert from 'node:assert/strict'
import { createServer } from '../webui/node_modules/vite/dist/node/index.js'

const server = await createServer({ root: new URL('../webui', import.meta.url).pathname, server: { middlewareMode: true, hmr: false }, appType: 'custom' })
test.after(() => server.close())
const { firstIntersection } = await server.ssrLoadModule('/src/geometry/curves.ts')
const { smoothConnection, roundedBase } = await server.ssrLoadModule('/src/geometry/smoothConnection.ts')

const vector = (a, b) => ({ x: b.x - a.x, y: b.y - a.y })
const distance = (a, b) => Math.hypot(b.x - a.x, b.y - a.y)
const along = (p, d, t) => ({ x: p.x + d.x * t, y: p.y + d.y * t })
const line = (start, end) => ({ start, c1: along(start, vector(start, end), 1 / 3), c2: along(start, vector(start, end), 2 / 3), end })
const closeTo = (a, b, tolerance, label) => assert.ok(Math.abs(a - b) < tolerance, `${label}: ${a} vs ${b}`)
const pointCloseTo = (a, b, tolerance, label) => assert.ok(distance(a, b) < tolerance, `${label}: ${JSON.stringify(a)} vs ${JSON.stringify(b)}`)
const sameDirection = (a, b) => {
  const scale = Math.hypot(a.x, a.y) * Math.hypot(b.x, b.y)
  assert.ok(scale > 1e-12, 'nonzero tangents')
  assert.ok(Math.abs(a.x * b.y - a.y * b.x) < scale * 1e-6, 'parallel tangents')
  assert.ok(a.x * b.x + a.y * b.y > 0, 'same direction, not a cusp')
}
const cubicPoint = (c, t) => {
  const u = 1 - t
  return {
    x: u ** 3 * c.start.x + 3 * u * u * t * c.c1.x + 3 * u * t * t * c.c2.x + t ** 3 * c.end.x,
    y: u ** 3 * c.start.y + 3 * u * u * t * c.c1.y + 3 * u * t * t * c.c2.y + t ** 3 * c.end.y,
  }
}
const pathPoints = path => {
  const points = [path.start]
  let start = path.start
  for (const segment of path.segments) {
    if (segment.kind === 'line') points.push(segment.end)
    else for (let i = 1; i <= 64; i++) points.push(cubicPoint({ start, ...segment }, i / 64))
    start = segment.end
  }
  return points
}
const cross = (a, b, c) => (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)
const onSegment = (p, a, b) => Math.abs(cross(a, b, p)) < 1e-10
  && p.x >= Math.min(a.x, b.x) - 1e-10 && p.x <= Math.max(a.x, b.x) + 1e-10
  && p.y >= Math.min(a.y, b.y) - 1e-10 && p.y <= Math.max(a.y, b.y) + 1e-10
const intersects = (a, b, c, d) => (cross(a, b, c) * cross(a, b, d) < 0 && cross(c, d, a) * cross(c, d, b) < 0)
  || onSegment(c, a, b) || onSegment(d, a, b) || onSegment(a, c, d) || onSegment(b, c, d)
function assertSimpleClosed(path) {
  pointCloseTo(path.segments.at(-1).end, path.start, 1e-9, 'explicit closure')
  const points = pathPoints(path), count = points.length - 1
  for (const p of points) assert.ok(Number.isFinite(p.x) && Number.isFinite(p.y), 'finite coordinates')
  for (let i = 0; i < count; i++) {
    assert.ok(distance(points[i], points[i + 1]) > 1e-10, `nonzero sampled segment ${i}`)
    for (let j = i + 2; j < count; j++) {
      if (i === 0 && j === count - 1) continue // Adjacent at the closing vertex.
      assert.ok(!intersects(points[i], points[i + 1], points[j], points[j + 1]), `nonadjacent sampled segments ${i} and ${j} intersect`)
    }
  }
}

// Straight cubics give an exact closed rectangle, without corner interpolation.
const corners = [{ x: -10, y: -100 }, { x: 10, y: -100 }, { x: 10, y: 100 }, { x: -10, y: 100 }]
const parent = corners.map((p, i) => line(p, corners[(i + 1) % corners.length]))
const perimeter = 440, parentDistance = 3, childDistance = 2
function childSides(direction, length = 60, radius = 2, root = { x: 0, y: 0 }) {
  const normal = { x: -direction.y, y: direction.x }
  return [1, -1].map(sign => {
    const start = along(root, normal, sign * radius)
    return [line(start, along(start, direction, length))]
  })
}

for (const degrees of [90, 30, 60]) for (const side of [-1, 1]) {
  test(`${degrees}-degree ${side < 0 ? 'left' : 'right'} branch: analytic exit, distances, tangents and simple closure`, () => {
    // Angle is measured from the parent's upward longitudinal direction.
    const radians = degrees * Math.PI / 180
    const direction = { x: side * Math.sin(radians), y: Math.cos(radians) }
    const sides = childSides(direction)
    const expected = sides.map(([c]) => {
      const arc = (side * 10 - c.start.x) / direction.x
      const point = along(c.start, direction, arc)
      return { arc, point, parentArc: side > 0 ? 120 + point.y : 340 - point.y }
    })
    const connection = smoothConnection({ parent, childSides: sides, parentDistance, childDistance })
    assert.ok(connection, 'both child sides leave the rectangle')
    const delta = ((expected[1].parentArc - expected[0].parentArc + perimeter * 1.5) % perimeter) - perimeter / 2
    const signs = delta > 0 ? [-1, 1] : [1, -1]
    for (let i = 0; i < 2; i++) {
      const hit = firstIntersection(sides[i], parent), analytic = expected[i], join = connection.joins[i]
      assert.ok(hit)
      pointCloseTo(hit.point, analytic.point, 1e-6, 'analytic first intersection')
      closeTo(hit.arcA, analytic.arc, 1e-6, 'analytic child exit arc')
      closeTo(hit.arcB, analytic.parentArc, 1e-6, 'analytic parent exit arc')
      pointCloseTo(join.exit, analytic.point, 1e-6, 'reported exit')
      closeTo(join.exitArc, analytic.arc, 1e-6, 'reported child exit arc')
      closeTo(join.parentArc, analytic.parentArc, 1e-6, 'reported parent exit arc')
      closeTo(join.parentDistance, parentDistance, 1e-3, 'reported parent distance')
      closeTo(join.childDistance, childDistance, 1e-3, 'reported child distance')
      // Cuts stay on straight edges, so Euclidean and arc distances agree.
      pointCloseTo(join.parentPoint, along(analytic.point, { x: 0, y: side * signs[i] }, parentDistance), 1e-3, 'parent cut position')
      pointCloseTo(join.childPoint, along(analytic.point, direction, childDistance), 1e-3, 'child cut position')
      closeTo(distance(join.exit, join.parentPoint), parentDistance, 1e-3, 'actual parent distance')
      closeTo(distance(join.exit, join.childPoint), childDistance, 1e-3, 'actual child distance')
    }
    const segments = connection.outline.segments
    const first = segments[0], last = segments.at(-3), beforeLast = segments.at(-4)
    assert.equal(first.kind, 'cubic')
    assert.equal(last.kind, 'cubic')
    pointCloseTo(connection.outline.start, connection.joins[0].parentPoint, 1e-9, 'first join start')
    pointCloseTo(first.end, connection.joins[0].childPoint, 1e-9, 'first join end')
    pointCloseTo(beforeLast.end, connection.joins[1].childPoint, 1e-9, 'last join start')
    pointCloseTo(last.end, connection.joins[1].parentPoint, 1e-9, 'last join end')
    sameDirection(vector(connection.outline.start, first.c1), { x: 0, y: -side * signs[0] })
    sameDirection(vector(first.c2, first.end), direction)
    sameDirection(vector(first.c2, first.end), vector(first.end, segments[1].c1))
    sameDirection(vector(beforeLast.c2, beforeLast.end), vector(beforeLast.end, last.c1))
    sameDirection(vector(beforeLast.end, last.c1), { x: -direction.x, y: -direction.y })
    sameDirection(vector(last.c2, last.end), { x: 0, y: side * signs[1] })
    assertSimpleClosed(connection.outline)
  })
}

test('firstIntersection selects the earliest crossing before later reentries', () => {
  const points = [{ x: 0, y: 7 }, { x: 30, y: 7 }, { x: -30, y: 7 }, { x: 30, y: 7 }]
  const child = points.slice(1).map((p, i) => line(points[i], p))
  const hit = firstIntersection(child, parent)
  assert.ok(hit)
  pointCloseTo(hit.point, { x: 10, y: 7 }, 1e-6, 'earliest crossing point')
  closeTo(hit.arcA, 10, 1e-6, 'earliest child arc')
  closeTo(hit.arcB, 127, 1e-6, 'earliest parent arc')
})

test('child sides that never leave the parent return null', () => {
  const sides = childSides({ x: 1, y: 0 }, 5)
  for (const child of sides) assert.equal(firstIntersection(child, parent), null)
  assert.equal(smoothConnection({ parent, childSides: sides, parentDistance, childDistance }), null)
})

test('roundedBase is a closed simple outline with a radius-one semicircle', () => {
  const direction = { x: 0.6, y: 0.8 }, root = { x: 4, y: -3 }, radius = 1
  const outline = roundedBase(childSides(direction, 12, radius, root))
  assertSimpleClosed(outline)
  const segments = outline.segments, cap = segments.slice(-2)
  let start = segments.at(-3).end
  for (const segment of cap) {
    assert.equal(segment.kind, 'cubic')
    for (let i = 0; i <= 256; i++) {
      const p = cubicPoint({ start, ...segment }, i / 256), offset = vector(root, p)
      closeTo(distance(root, p), radius, 1e-3, 'semicircle radial error')
      assert.ok(offset.x * direction.x + offset.y * direction.y <= 1e-9, 'cap stays behind root')
    }
    start = segment.end
  }
  pointCloseTo(cap[0].end, along(root, direction, -radius), 1e-9, 'semicircle midpoint')
  sameDirection(vector(segments.at(-3).c2, segments.at(-3).end), vector(segments.at(-3).end, cap[0].c1))
  sameDirection(vector(cap[0].c2, cap[0].end), vector(cap[0].end, cap[1].c1))
  sameDirection(vector(cap[1].c2, cap[1].end), vector(outline.start, segments[0].c1))
})

import { splitCubic, type Cubic, type ClosedPath, type Point, type PathSegment } from './curves'

// Coordinates follow the source centre line, not integrated boundary arc length.
// A replaced root interval maps to its new cubic; hidden ordinary edges are absent.
export interface ConnectionSide { curves: Cubic[]; positions: number[] }
export type ConnectionSides = [ConnectionSide, ConnectionSide]
const unit = (p: Point): Point => { const n = Math.hypot(p.x, p.y) || 1; return { x: p.x / n, y: p.y / n } }
const sub = (a: Point, b: Point): Point => ({ x: a.x - b.x, y: a.y - b.y })
const dot = (a: Point, b: Point) => a.x * b.x + a.y * b.y
const along = (p: Point, d: Point, n: number): Point => ({ x: p.x + d.x * n, y: p.y + d.y * n })
const distance = (a: Point, b: Point) => Math.hypot(a.x - b.x, a.y - b.y)
const reverse = (c: Cubic): Cubic => ({ start: c.end, c1: c.c2, c2: c.c1, end: c.start })
const segment = (c: Cubic): PathSegment => ({ kind: 'cubic', c1: c.c1, c2: c.c2, end: c.end })

export function connectionSide(curves: Cubic[], positions?: number[]): ConnectionSide {
  if (positions) return { curves, positions }
  const coords = [0]
  for (const c of curves) coords.push(coords.at(-1)! + distance(c.start, c.end))
  return { curves, positions: coords }
}

function at(side: ConnectionSide, position: number) {
  const { curves, positions } = side
  const value = Math.max(positions[0], Math.min(positions.at(-1)!, position))
  let lo = 0, hi = curves.length - 1
  while (lo < hi) { const mid = (lo + hi) >>> 1; if (positions[mid + 1] < value) lo = mid + 1; else hi = mid }
  const span = positions[lo + 1] - positions[lo]
  const t = span > 0 ? Math.max(0, Math.min(1, (value - positions[lo]) / span)) : 0
  const [head, tail] = splitCubic(curves[lo], t)
  let tangent = sub(tail.c1, tail.start)
  if (Math.hypot(tangent.x, tangent.y) < 1e-12) tangent = sub(head.end, head.c2)
  if (Math.hypot(tangent.x, tangent.y) < 1e-12) tangent = sub(curves[lo].end, curves[lo].start)
  return { point: head.end, tangent: unit(tangent), tail, index: lo, value }
}

export function connectSides(input: {
  parent: ConnectionSides; position: number; anchor: Point; child: ConnectionSides;
  parentDistance: number; childDistance: number; convert?: (p: Point) => Point;
}): { outline: ClosedPath; sides: ConnectionSides } | null {
  const { parent, child, position, anchor, parentDistance, childDistance } = input
  if (child.some(s => !s.curves.length) || parent.some(s => !s.curves.length)) return null
  const root = { x: (child[0].curves[0].start.x + child[1].curves[0].start.x) / 2,
    y: (child[0].curves[0].start.y + child[1].curves[0].start.y) / 2 }
  const convert = input.convert ?? ((p: Point) => p)
  const forward = unit(sub(child[0].curves[0].c1, child[0].curves[0].start))
  const centers = parent.map(s => at(s, position))
  const chosen = dot(sub(convert(centers[0].point), root), forward) >= dot(sub(convert(centers[1].point), root), forward) ? 0 : 1
  const boundary = parent[chosen], center = centers[chosen]
  const normal = unit(sub(center.point, anchor))
  const outward = Math.max(.25, Math.abs(dot(normal, forward)))
  const width = distance(child[0].curves[0].start, child[1].curves[0].start)
  if (width < 1e-9 || child.some(s => s.positions.at(-1)! - s.positions[0] < 1e-9)) return null
  const spread = Math.min(width * 2, width / (2 * outward) + parentDistance)
  const separation = sub(child[0].curves[0].start, child[1].curves[0].start)
  const sign = dot(separation, center.tangent) >= 0 ? 1 : -1
  const ends = child.map((side, i) => {
    const direction = i === 0 ? sign : -sign
    const p = at(boundary, position + direction * spread)
    const Q = convert(p.point)
    const requested = distance(center.point, anchor) / outward + childDistance
    const endPosition = side.positions[0] + Math.min(requested, (side.positions.at(-1)! - side.positions[0]) * .5)
    const end = at(side, endPosition), C = end.point
    const chord = distance(Q, C)
    const pHandle = Math.min(chord / 3, Math.max(width / 6, parentDistance * 2 / 3))
    const cHandle = Math.min(chord / 3, Math.max(width / 6, childDistance * 2 / 3), distance(root, C) / 2)
    const fillet: Cubic = { start: Q, c1: along(Q, p.tangent, -direction * pHandle),
      c2: along(C, end.tangent, -cHandle), end: C }
    const curves = [fillet, end.tail, ...side.curves.slice(end.index + 1)]
    const positions = [side.positions[0], end.value, ...side.positions.slice(end.index + 1)]
    return { curves, positions }
  })
  const [a, b] = ends
  return { sides: [a, b], outline: {
    start: a.curves[0].start,
    segments: [...a.curves.map(segment), { kind: 'line', end: b.curves.at(-1)!.end },
      ...[...b.curves].reverse().map(c => segment(reverse(c))),
      { kind: 'line', end: root }, { kind: 'line', end: a.curves[0].start }],
  } }
}

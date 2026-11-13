import { chainLength, cutAtArc, firstIntersection, pointAtArc } from './curves'
import type { ClosedPath, Cubic, PathSegment, Point } from './curves'

export interface Join {
  exit: Point
  exitArc: number
  parentArc: number
  parentPoint: Point
  childPoint: Point
  parentDistance: number
  childDistance: number
}

const nonnegativeMod = (value: number, length: number) => ((value % length) + length) % length
const unit = (v: Point): Point => { const length = Math.hypot(v.x, v.y) || 1; return { x: v.x / length, y: v.y / length } }
const along = (p: Point, d: Point, length: number): Point => ({ x: p.x + d.x * length, y: p.y + d.y * length })
const midpoint = (a: Point, b: Point): Point => ({ x: (a.x + b.x) / 2, y: (a.y + b.y) / 2 })
const segment = (c: Cubic): PathSegment => ({ kind: 'cubic', c1: c.c1, c2: c.c2, end: c.end })
const reversed = (c: Cubic): Cubic => ({ start: c.end, c1: c.c2, c2: c.c1, end: c.start })

export function smoothConnection(input: {
  parent: Cubic[]
  childSides: [Cubic[], Cubic[]]
  parentDistance: number
  childDistance: number
}): { outline: ClosedPath; joins: [Join, Join] } | null {
  const { parent, childSides, parentDistance, childDistance } = input
  const hit0 = firstIntersection(childSides[0], parent), hit1 = firstIntersection(childSides[1], parent)
  if (!hit0 || !hit1) return null
  const L = chainLength(parent), delta = nonnegativeMod(hit1.arcB - hit0.arcB + L / 2, L) - L / 2
  if (Math.abs(delta) < 1e-9) return null
  const s0 = delta > 0 ? -1 : 1, signs = [s0, -s0]
  const ends = [hit0, hit1].map((hit, index) => {
    const side = childSides[index], dC = Math.min(childDistance, chainLength(side) - hit.arcA), dP = Math.min(parentDistance, L / 4)
    if (dC < 1e-9 || dP < 1e-9) return null
    const parentArc = nonnegativeMod(hit.arcB + signs[index] * dP, L)
    const { point: Q, tangent: T } = pointAtArc(parent, parentArc)
    const { point: C, tangent } = pointAtArc(side, hit.arcA + dC)
    const u = along({ x: 0, y: 0 }, unit(T), -signs[index]), v = unit(tangent)
    const fillet: Cubic = { start: Q, c1: along(Q, u, (2 / 3) * dP), c2: along(C, v, -(2 / 3) * dC), end: C }
    const join: Join = { exit: hit.point, exitArc: hit.arcA, parentArc: hit.arcB, parentPoint: Q, childPoint: C, parentDistance: dP, childDistance: dC }
    return { fillet, rest: cutAtArc(side, hit.arcA + dC).rest, join }
  })
  const [a, b] = ends
  if (!a || !b) return null
  const root = midpoint(childSides[0][0].start, childSides[1][0].start)
  return {
    outline: {
      start: a.fillet.start,
      segments: [
        segment(a.fillet), ...a.rest.map(segment),
        { kind: 'line', end: childSides[1].at(-1)!.end },
        ...[...b.rest].reverse().map(c => segment(reversed(c))), segment(reversed(b.fillet)),
        { kind: 'line', end: root }, { kind: 'line', end: a.fillet.start },
      ],
    },
    joins: [a.join, b.join],
  }
}

export function roundedBase(childSides: [Cubic[], Cubic[]]): ClosedPath {
  const [side0, side1] = childSides, S1 = side0[0].start, S2 = side1[0].start
  const root = midpoint(S1, S2), r = Math.hypot(S1.x - S2.x, S1.y - S2.y) / 2
  const d = unit(pointAtArc(side0, 0).tangent), n = unit({ x: S1.x - S2.x, y: S1.y - S2.y })
  const k = .5522847498, B = along(root, d, -r)
  return {
    start: S1,
    segments: [
      ...side0.map(segment), { kind: 'line', end: side1.at(-1)!.end },
      ...[...side1].reverse().map(c => segment(reversed(c))),
      { kind: 'cubic', c1: along(S2, d, -k * r), c2: along(B, n, -k * r), end: B },
      { kind: 'cubic', c1: along(B, n, k * r), c2: along(S1, d, -k * r), end: S1 },
    ],
  }
}

export interface Point { x: number; y: number }
export interface Cubic { start: Point; c1: Point; c2: Point; end: Point }
export type PathSegment = { kind: 'line'; end: Point } | ({ kind: 'cubic' } & Omit<Cubic, 'start'>)
export interface ClosedPath { start: Point; segments: PathSegment[] }
const mix = (a: Point, b: Point, t: number): Point => ({ x: a.x + (b.x - a.x) * t, y: a.y + (b.y - a.y) * t })
const distance = (a: Point, b: Point) => Math.hypot(a.x - b.x, a.y - b.y)
export function splitCubic(c: Cubic, t: number): [Cubic, Cubic] {
  const a = mix(c.start, c.c1, t), b = mix(c.c1, c.c2, t), d = mix(c.c2, c.end, t)
  const e = mix(a, b, t), f = mix(b, d, t), p = mix(e, f, t)
  return [{ start: c.start, c1: a, c2: e, end: p }, { start: p, c1: f, c2: d, end: c.end }]
}
export function outlineCurves(points: Point[]): Cubic[] {
  return points.slice(1).map((end, i) => {
    const start = points[i], before = points[Math.max(0, i - 1)], after = points[Math.min(points.length - 1, i + 2)]
    return { start, c1: { x: start.x + (end.x - before.x) / 6, y: start.y + (end.y - before.y) / 6 },
      c2: { x: end.x - (after.x - start.x) / 6, y: end.y - (after.y - start.y) / 6 }, end }
  })
}
const unit = (a: Point, b: Point): Point => { const n = distance(a, b) || 1; return { x: (b.x - a.x) / n, y: (b.y - a.y) / n } }
const along = (p: Point, d: Point, n: number): Point => ({ x: p.x + d.x * n, y: p.y + d.y * n })
/** Exact endpoints/tangents of an ellipse's two longitudinal halves. Cubic
 * quarter-arcs use the standard kappa approximation, also used for drawing. */
export function ellipseSides(base: Point, direction: Point, length: number, width: number): [Cubic[], Cubic[]] {
  const k = 4 * (Math.sqrt(2) - 1) / 3
  const d = unit({ x: 0, y: 0 }, direction), n = { x: -d.y, y: d.x }
  const p = (x: number, y: number) => along(along(base, d, x), n, y)
  return [1, -1].map(s => [
    { start: p(0, 0), c1: p(0, s * width * k / 2), c2: p(length * (1 - k) / 2, s * width / 2), end: p(length / 2, s * width / 2) },
    { start: p(length / 2, s * width / 2), c1: p(length * (1 + k) / 2, s * width / 2), c2: p(length, s * width * k / 2), end: p(length, 0) },
  ]) as [Cubic[], Cubic[]]
}
export function sectionSides(root: Point, direction: Point, width: number, stroke: number): [Cubic[], Cubic[]] {
  const d = unit({ x: 0, y: 0 }, direction), n = { x: -d.y, y: d.x }, r = stroke / 2
  const p = (x: number, y: number) => along(along(root, d, x), n, y)
  const line = (start: Point, end: Point): Cubic => ({ start, c1: mix(start, end, 1 / 3), c2: mix(start, end, 2 / 3), end })
  return [1, -1].map(s => {
    const cap = ellipseSides(p(-r, s * width / 2), d, stroke, stroke)[s === 1 ? 0 : 1]
    return [line(p(-r, 0), cap[0].start), ...cap, line(cap.at(-1)!.end, p(r, 0))]
  }) as [Cubic[], Cubic[]]
}

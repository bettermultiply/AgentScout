/** Pure geometry: no renderer, application state or tuning dependencies. */
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
// Geometry callers construct new curves/points rather than editing published ones.
// Keep tolerance in the key: callers may request different integration accuracy.
const lengthCache = new WeakMap<Cubic, Map<number, number>>()
export function cubicLength(c: Cubic, tolerance = 1e-6): number {
  let values = lengthCache.get(c)
  const cached = values?.get(tolerance)
  if (cached !== undefined) return cached
  const length = measureCubic(c, tolerance)
  if (!values) { values = new Map(); lengthCache.set(c, values) }
  values.set(tolerance, length)
  return length
}
function measureCubic(c: Cubic, tolerance: number): number {
  const chord = distance(c.start, c.end), polygon = distance(c.start, c.c1) + distance(c.c1, c.c2) + distance(c.c2, c.end)
  if (polygon - chord <= tolerance) return (polygon + chord) / 2
  const [a, b] = splitCubic(c, .5)
  return measureCubic(a, tolerance / 2) + measureCubic(b, tolerance / 2)
}
export function outlineCurves(points: Point[]): Cubic[] {
  return points.slice(1).map((end, i) => {
    const start = points[i], before = points[Math.max(0, i - 1)], after = points[Math.min(points.length - 1, i + 2)]
    return { start, c1: { x: start.x + (end.x - before.x) / 6, y: start.y + (end.y - before.y) / 6 },
      c2: { x: end.x - (after.x - start.x) / 6, y: end.y - (after.y - start.y) / 6 }, end }
  })
}
/** Arc coordinate of a fixed curve parameter; never searches for a nearby edge. */
export function arcCoordinate(curves: Cubic[], index: number, t: number): number {
  const prefixes = chainData(curves).prefixes
  const prefix = Number.isInteger(index) && index >= 0 && index <= curves.length ? prefixes[index] : curves.slice(0, index).reduce((n, c) => n + cubicLength(c), 0)
  return prefix + (curves[index] ? cubicLength(splitCubic(curves[index], t)[0]) : 0)
}
function parameterAtLength(c: Cubic, length: number): number {
  const total = cubicLength(c)
  if (length <= 0) return 0
  if (length >= total) return 1
  let lo = 0, hi = 1
  for (let i = 0; i < 32; i++) {
    const mid = (lo + hi) / 2
    if (cubicLength(splitCubic(c, mid)[0]) < length) lo = mid
    else hi = mid
  }
  return (lo + hi) / 2
}
function cutAtLength(c: Cubic, length: number): [Cubic, Cubic] {
  return splitCubic(c, parameterAtLength(c, length))
}
export function curveInterval(curves: Cubic[], from: number, to: number): Cubic[] {
  const out: Cubic[] = []; let offset = 0
  for (const original of curves) {
    const length = cubicLength(original), a = Math.max(0, from - offset), b = Math.min(length, to - offset)
    if (b > a) {
      const prefix = b < length ? cutAtLength(original, b)[0] : original
      out.push(a > 0 ? cutAtLength(prefix, a)[1] : prefix)
    }
    offset += length
    if (offset >= to) break
  }
  return out
}
const unit = (a: Point, b: Point): Point => { const n = distance(a, b) || 1; return { x: (b.x - a.x) / n, y: (b.y - a.y) / n } }
const along = (p: Point, d: Point, n: number): Point => ({ x: p.x + d.x * n, y: p.y + d.y * n })
export const chainLength = (curves: Cubic[]): number => chainData(curves).prefixes[curves.length]
const bezierPoint = (c: Cubic, t: number): Point => {
  const u = 1 - t
  return {
    x: u * u * u * c.start.x + 3 * u * u * t * c.c1.x + 3 * u * t * t * c.c2.x + t * t * t * c.end.x,
    y: u * u * u * c.start.y + 3 * u * u * t * c.c1.y + 3 * u * t * t * c.c2.y + t * t * t * c.end.y,
  }
}
const bezierTangent = (c: Cubic, t: number): Point => {
  const u = 1 - t
  return {
    x: 3 * u * u * (c.c1.x - c.start.x) + 6 * u * t * (c.c2.x - c.c1.x) + 3 * t * t * (c.end.x - c.c2.x),
    y: 3 * u * u * (c.c1.y - c.start.y) + 6 * u * t * (c.c2.y - c.c1.y) + 3 * t * t * (c.end.y - c.c2.y),
  }
}
/** Point and forward tangent at an arc length along a chained sequence. */
export function pointAtArc(curves: Cubic[], arc: number): { point: Point; tangent: Point } {
  if (!curves.length) throw new Error('空曲线序列没有位置')
  const total = chainLength(curves)
  const lengths = chainData(curves).lengths
  let rest = Math.max(0, Math.min(total, arc))
  for (let index = 0; index < curves.length; index++) {
    const c = curves[index], length = lengths[index]
    if (rest <= length || index === curves.length - 1) {
      const t = parameterAtLength(c, Math.min(rest, length))
      const tangent = bezierTangent(c, t)
      return { point: bezierPoint(c, t), tangent: tangent.x || tangent.y ? tangent : unit(c.start, c.end) }
    }
    rest -= length
  }
  const last = curves[curves.length - 1]
  return { point: last.end, tangent: unit(last.c2, last.end) }
}
/** Cut a chain at an arc length. The tangent points forward, into `rest`. */
export function cutAtArc(curves: Cubic[], arc: number): { point: Point; tangent: Point; rest: Cubic[] } {
  const total = chainLength(curves)
  const lengths = chainData(curves).lengths
  let rest = Math.max(0, Math.min(total, arc))
  for (let index = 0; index < curves.length; index++) {
    const c = curves[index], length = lengths[index]
    if (rest < length || index === curves.length - 1) {
      const t = parameterAtLength(c, Math.min(rest, length))
      const [head, tail] = splitCubic(c, t)
      const tangent = bezierTangent(c, t)
      return {
        point: head.end,
        tangent: tangent.x || tangent.y ? tangent : unit(c.start, c.end),
        rest: [tail, ...curves.slice(index + 1)],
      }
    }
    rest -= length
  }
  const last = curves[curves.length - 1]
  return { point: last.end, tangent: unit(last.c2, last.end), rest: [] }
}
export function sampleCurves(curves: Cubic[], perCurve = 8): Point[] {
  const out: Point[] = []
  curves.forEach((c, index) => {
    for (let i = index === 0 ? 0 : 1; i <= perCurve; i++) out.push(bezierPoint(c, i / perCurve))
  })
  return out
}
export function pointInPolygon(point: Point, polygon: Point[]): boolean {
  let inside = false
  for (let i = 0, j = polygon.length - 1; i < polygon.length; j = i++) {
    const a = polygon[i], b = polygon[j]
    if ((a.y > point.y) !== (b.y > point.y) && point.x < (b.x - a.x) * (point.y - a.y) / (b.y - a.y) + a.x) inside = !inside
  }
  return inside
}
export function cubicCross(a: Cubic, b: Cubic, steps = 24): boolean {
  const A = sampleCurves([a], steps), B = sampleCurves([b], steps)
  const side = (p: Point, q: Point, r: Point) => (q.x - p.x) * (r.y - p.y) - (q.y - p.y) * (r.x - p.x)
  for (let i = 1; i < A.length; i++) for (let j = 1; j < B.length; j++) {
    if (side(A[i - 1], A[i], B[j - 1]) * side(A[i - 1], A[i], B[j]) < 0
      && side(B[j - 1], B[j], A[i - 1]) * side(B[j - 1], B[j], A[i]) < 0) return true
  }
  return false
}
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
/** The visible capsule of a round-stroked zero-longitudinal-length section. */
export function sectionSides(root: Point, direction: Point, width: number, stroke: number): [Cubic[], Cubic[]] {
  const d = unit({ x: 0, y: 0 }, direction), n = { x: -d.y, y: d.x }, r = stroke / 2
  const p = (x: number, y: number) => along(along(root, d, x), n, y)
  const line = (start: Point, end: Point): Cubic => ({ start, c1: mix(start, end, 1 / 3), c2: mix(start, end, 2 / 3), end })
  return [1, -1].map(s => {
    const cap = ellipseSides(p(-r, s * width / 2), d, stroke, stroke)[s === 1 ? 0 : 1]
    return [line(p(-r, 0), cap[0].start), ...cap, line(cap.at(-1)!.end, p(r, 0))]
  }) as [Cubic[], Cubic[]]
}
type IntersectionSample = { point: Point; t: number; arc: number }
const cross = (a: Point, b: Point) => a.x * b.y - a.y * b.x
const subtract = (a: Point, b: Point): Point => ({ x: a.x - b.x, y: a.y - b.y })
type Bounds = { minX: number; maxX: number; minY: number; maxY: number }
type ChainData = { prefixes: number[]; lengths: number[]; bounds: Bounds[]; samples: IntersectionSample[][] }
const chains = new WeakMap<Cubic[], ChainData>()
function chainData(curves: Cubic[]): ChainData {
  const cached = chains.get(curves)
  if (cached) return cached
  const prefixes = [0], lengths: number[] = [], bounds: Bounds[] = []
  for (const c of curves) {
    const length = cubicLength(c)
    lengths.push(length)
    prefixes.push(prefixes[prefixes.length - 1] + length)
    const points = [c.start, c.c1, c.c2, c.end]
    // Include floating evaluation error and segmentIntersection's endpoint slack.
    const pad = Math.max(1, ...points.flatMap(p => [Math.abs(p.x), Math.abs(p.y)])) * 1e-10
    bounds.push({ minX: Math.min(...points.map(p => p.x)) - pad, maxX: Math.max(...points.map(p => p.x)) + pad,
      minY: Math.min(...points.map(p => p.y)) - pad, maxY: Math.max(...points.map(p => p.y)) + pad })
  }
  const data: ChainData = { prefixes, lengths, bounds, samples: [] }
  chains.set(curves, data)
  return data
}
function intersectionSamples(curves: Cubic[], data: ChainData, index: number): IntersectionSample[] {
  if (data.samples[index]) return data.samples[index]
  const c = curves[index], offset = data.prefixes[index], samples: IntersectionSample[] = []
  for (let i = 0; i <= 48; i++) {
    const t = i / 48
    samples.push({ point: bezierPoint(c, t), t, arc: offset + cubicLength(splitCubic(c, t)[0]) })
  }
  data.samples[index] = samples
  return samples
}
function segmentIntersection(a: Point, b: Point, c: Point, d: Point): { u: number; v: number } | null {
  const r = subtract(b, a), s = subtract(d, c), q = subtract(c, a), determinant = cross(r, s)
  if (Math.abs(determinant) < 1e-12) return null
  const u = cross(q, s) / determinant, v = cross(q, r) / determinant
  return u >= -1e-12 && u <= 1 + 1e-12 && v >= -1e-12 && v <= 1 + 1e-12
    ? { u: Math.max(0, Math.min(1, u)), v: Math.max(0, Math.min(1, v)) } : null
}
/** Earliest ordinary crossing on a, seeded by 48 line segments per cubic. */
export function firstIntersection(a: Cubic[], b: Cubic[]): { point: Point; arcA: number; arcB: number } | null {
  const A = chainData(a), B = chainData(b)
  const whole = A.bounds.reduce((box, other) => ({ minX: Math.min(box.minX, other.minX), maxX: Math.max(box.maxX, other.maxX), minY: Math.min(box.minY, other.minY), maxY: Math.max(box.maxY, other.maxY) }), { minX: Infinity, maxX: -Infinity, minY: Infinity, maxY: -Infinity })
  const possible: number[] = []
  for (let ib = 0; ib < b.length; ib++) {
    const other = B.bounds[ib]
    if (whole.maxX < other.minX || other.maxX < whole.minX || whole.maxY < other.minY || other.maxY < whole.minY) continue
    possible.push(ib)
  }
  let seed: { ia: number; ib: number; a0: IntersectionSample; a1: IntersectionSample; b0: IntersectionSample; b1: IntersectionSample; u: number; v: number; arc: number } | null = null
  for (let ia = 0; ia < a.length; ia++) {
    const box = A.bounds[ia], candidates: number[] = []
    for (const ib of possible) {
      const other = B.bounds[ib]
      if (box.maxX < other.minX || other.maxX < box.minX || box.maxY < other.minY || other.maxY < box.minY) continue
      candidates.push(ib)
    }
    if (!candidates.length) continue
    const samplesA = intersectionSamples(a, A, ia)
    for (let i = 1; i <= 48; i++) {
      const a0 = samplesA[i - 1], a1 = samplesA[i]
      for (const ib of candidates) {
        const samplesB = intersectionSamples(b, B, ib)
        for (let j = 1; j <= 48; j++) {
          const b0 = samplesB[j - 1], b1 = samplesB[j], hit = segmentIntersection(a0.point, a1.point, b0.point, b1.point)
          if (!hit) continue
          const arc = a0.arc + (a1.arc - a0.arc) * hit.u
          if (!seed || arc < seed.arc) seed = { ia, ib, a0, a1, b0, b1, ...hit, arc }
        }
      }
    }
  }
  if (!seed) return null
  const { ia, ib, a0, a1, b0, b1, u, v } = seed
  const ta = a0.t + (a1.t - a0.t) * u, tb = b0.t + (b1.t - b0.t) * v
  let x = ta, y = tb
  for (let k = 0; k < 20; k++) {
    const pa = bezierPoint(a[ia], x), pb = bezierPoint(b[ib], y), f = subtract(pa, pb)
    if (distance(pa, pb) < 1e-9) return { point: mix(pa, pb, .5), arcA: arcCoordinate(a, ia, x), arcB: arcCoordinate(b, ib, y) }
    const da = bezierTangent(a[ia], x), db = bezierTangent(b[ib], y), determinant = cross(da, db)
    if (Math.abs(determinant) < 1e-12) break
    const nextX = x - cross(f, db) / determinant, nextY = y - cross(f, da) / determinant
    if (nextX < 0 || nextX > 1 || nextY < 0 || nextY > 1) break
    x = nextX; y = nextY
  }
  // Fall back to the original crossing brackets, bisecting both curve segments.
  // If subdivision loses the crossing, the polyline seed is not a real crossing.
  let loA = a0.t, hiA = a1.t, loB = b0.t, hiB = b1.t
  for (let k = 0; k < 64; k++) {
    const p0 = bezierPoint(a[ia], loA), p1 = bezierPoint(a[ia], hiA)
    const q0 = bezierPoint(b[ib], loB), q1 = bezierPoint(b[ib], hiB)
    const hit = segmentIntersection(p0, p1, q0, q1)
    if (!hit) return null
    x = loA + (hiA - loA) * hit.u; y = loB + (hiB - loB) * hit.v
    const pa = bezierPoint(a[ia], x), pb = bezierPoint(b[ib], y)
    if (distance(pa, pb) < 1e-9) return { point: mix(pa, pb, .5), arcA: arcCoordinate(a, ia, x), arcB: arcCoordinate(b, ib, y) }
    const midA = (loA + hiA) / 2, midB = (loB + hiB) / 2
    const rangesA = [[loA, midA], [midA, hiA]], rangesB = [[loB, midB], [midB, hiB]]
    let next: [number, number, number, number] | null = null
    for (const [fromA, toA] of rangesA) for (const [fromB, toB] of rangesB) {
      if (!next && segmentIntersection(bezierPoint(a[ia], fromA), bezierPoint(a[ia], toA), bezierPoint(b[ib], fromB), bezierPoint(b[ib], toB))) next = [fromA, toA, fromB, toB]
    }
    if (!next) return null
    ;[loA, hiA, loB, hiB] = next
  }
  return null
}

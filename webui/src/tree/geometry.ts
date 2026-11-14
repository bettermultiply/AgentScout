import type { BranchState, TaskState } from '../fold'
import { groupBranchCalls, pairCalls, requiredStart, type CallGroup, type CallUnit } from './calls'
import { DEFAULT_TUNING, FIXED_GEOMETRY, childAngleOf, ranged, type TreeTuning } from './tuning'
import { chooseForkSide } from './forkSide'
import { makeWidthAt } from './widthProfile'
import { startLead } from './startLead'
import { placeStems } from './stemPlacement'
import { ordinaryOutline } from './ordinaryOutline'
import { ellipseSides, sectionSides, outlineCurves, type ClosedPath, type Cubic } from '../geometry/curves'
import { connectionSide, connectSides, type ConnectionSides } from '../geometry/connection'

/**
 * Static, observation-time geometry. A Branch owns its path; there is no subtree
 * packing.
 *
 * Length is summed in segments: one segment per pair of consecutive call starts,
 * `K * ln(1 + interval / tau)`. The first call sits at distance 0 and a Branch
 * never grows a tail beyond its last call.
 *
 * Direction is a stepwise walk relative to the previous segment. At the middle
 * of a Branch the two sides are equally likely; the further the next segment
 * would land to one side, the more likely the walk turns back. A Branch may lean
 * but never grows sideways or upward.
 *
 * Width is a function of the calls that still happen at or below a point
 * (including descendant Branches), scaled by the configured root width.
 */
export const NODE_HALF_WIDTH = FIXED_GEOMETRY.nodeOrigin
export const NODE_WIDTH = NODE_HALF_WIDTH * 2
export const HEAD_CENTER_Y = 0

export interface BranchSection {
  id: string
  twig: CallGroup
  leaves: CallGroup[]
  side: -1 | 1
  /** Distance along the Branch where the stem leaves the trunk. */
  y: number
  x: number
  tx: number
  ty: number
  nx: number
  ny: number
  width: number
  shape?: StemGeometry
}

interface PathGeometry { d: string; points: Point[]; stroke: number }
interface EllipseGeometry { cx: number; cy: number; rx: number; ry: number }
interface LeafGeometry {
  tool: CallGroup
  at: Point
  angle: number
  transform: string
  size: number
  width: number
  ellipse: EllipseGeometry
  stroke: number
  petiole: PathGeometry
  vein: PathGeometry
}
interface StemGeometry {
  width: number
  outline?: PathGeometry
  cap?: { cx: number; cy: number; r: number }
  anchor: Point
  angle: number
  transform: string
  length: number
  path: PathGeometry
  tip: Point
  tipRadius: number
  leaves: LeafGeometry[]
}

export interface BranchGeometry {
  branchId: string
  calls: CallGroup[]
  sections: BranchSection[]
  x: number
  y: number
  width: number
  offsetX: number
  offsetY: number
  initialAngle: number
  height: number
  /** Absolute parent-branch Chance positions, used only for authority styling. */
  forkPoints: Map<string, { x: number; y: number }>
  chancePositions: Map<string, number>
  pathPoints: Array<{ x: number; y: number }>
  /** Arc length (summed log intervals) at each path point. */
  pathDistances: number[]
  trunkWidths: number[]
  trunk: PathGeometry
  connection?: PathGeometry & { rootWidth: number }
  bud: { transform: string; angle: number; base: Point; tip: Point; ellipse: EllipseGeometry }
  chanceCircles: Array<{ chanceId: string; branchId: string; x: number; y: number; radius: number }>
  validationEnd: { x1: number; y1: number; x2: number; y2: number; validatedAtMs: number } | null
}

/** Stem length in tree units: one model call, no pixel compensation. */
export function stemLength(durationMs: number | null, tuning: TreeTuning = DEFAULT_TUNING): number {
  return tuning.stemK * Math.log1p((durationMs ?? 0) / tuning.stemTauMs)
}

/** Leaf length in tree units; tool duration only changes the size. */
export function leafLength(durationMs: number | null, tuning: TreeTuning = DEFAULT_TUNING): number {
  const share = Math.min(1, Math.log1p((durationMs ?? 0) / tuning.leafTauMs) / Math.log1p(tuning.leafReferenceMs / tuning.leafTauMs))
  return tuning.leafMin + (tuning.leafMax - tuning.leafMin) * share
}

type Point = { x: number; y: number }

function hash(value: string): number {
  let h = 2166136261
  for (let i = 0; i < value.length; i++) {
    h ^= value.charCodeAt(i)
    h = Math.imul(h, 16777619)
  }
  return (h >>> 0) / 4294967296
}

function segmentLength(dt: number, tuning: TreeTuning): number {
  return tuning.segmentK * Math.log1p(Math.max(0, dt) / tuning.segmentTauMs)
}

export function sampleGeometry(geometry: BranchGeometry, distance: number): { x: number; y: number; width: number } {
  const points = geometry.pathPoints
  if (points.length === 1) return { ...points[0], width: geometry.trunkWidths[0] }
  const distances = geometry.pathDistances
  let index = 1
  while (index < points.length - 1 && distances[index] < distance) index++
  const span = Math.max(1e-6, distances[index] - distances[index - 1])
  const t = Math.max(0, Math.min(1, (distance - distances[index - 1]) / span))
  return {
    x: points[index - 1].x + (points[index].x - points[index - 1].x) * t,
    y: points[index - 1].y + (points[index].y - points[index - 1].y) * t,
    width: geometry.trunkWidths[index - 1] + (geometry.trunkWidths[index] - geometry.trunkWidths[index - 1]) * t,
  }
}

function normalAt(points: Point[], index: number, initialAngle: number): Point {
  const before = points[Math.max(0, index - 1)]
  const after = points[Math.min(points.length - 1, index + 1)]
  const length = Math.hypot(after.x - before.x, after.y - before.y)
  return length > 0
    ? { x: -(after.y - before.y) / length, y: (after.x - before.x) / length }
    : { x: -Math.cos(initialAngle), y: Math.sin(initialAngle) }
}

/** The same outer-edge interpolation as trunkOutline; no modified trunk. */
function trunkSide(g: BranchGeometry, side: number): Cubic[] {
  const direction = { x: Math.sin(g.initialAngle), y: Math.cos(g.initialAngle) }
  if (!g.calls.length) return ellipseSides(g.bud.base, direction, g.bud.ellipse.rx * 2, g.bud.ellipse.ry * 2)[side === 1 ? 0 : 1]
  if (g.pathPoints.length === 1) return sectionSides(g.pathPoints[0], direction, g.trunkWidths[0], g.trunk.stroke)[side === 1 ? 0 : 1]
  return outlineCurves(g.pathPoints.map((p, i) => {
    const n = normalAt(g.pathPoints, i, g.initialAngle)
    return { x: p.x + side * n.x * g.trunkWidths[i] / 2, y: p.y + side * n.y * g.trunkWidths[i] / 2 }
  }))
}

function connectionPath(path: ClosedPath): PathGeometry {
  const points = [path.start]
  const parts = [`M ${path.start.x} ${path.start.y}`]
  for (const segment of path.segments) {
    if (segment.kind === 'cubic') {
      points.push(segment.c1, segment.c2)
      parts.push(`C ${segment.c1.x} ${segment.c1.y} ${segment.c2.x} ${segment.c2.y} ${segment.end.x} ${segment.end.y}`)
    } else parts.push(`L ${segment.end.x} ${segment.end.y}`)
    points.push(segment.end)
  }
  return { d: `${parts.join(' ')} Z`, points, stroke: 0 }
}

/** One closed outline with a full-width flat end. */
export function trunkOutline(points: Point[], widths: number[], initialAngle = 0, crossSectionStroke = 1): PathGeometry {
  const left: Point[] = []
  const right: Point[] = []
  points.forEach((point, index) => {
    const { x: nx, y: ny } = normalAt(points, index, initialAngle)
    left.push({ x: point.x + nx * widths[index] / 2, y: point.y + ny * widths[index] / 2 })
    right.push({ x: point.x - nx * widths[index] / 2, y: point.y - ny * widths[index] / 2 })
  })
  // A single call (or several calls at one time) has no longitudinal length,
  // but still owns a width. Draw that cross-section, without inventing a tail.
  if (points.length === 1) return {
    d: `M ${left[0].x} ${left[0].y} L ${right[0].x} ${right[0].y}`,
    points: [left[0], right[0]], stroke: crossSectionStroke,
  }
  const controls: Point[] = [...left, ...right]
  const curve = (side: Point[]) => side.slice(1).map((point, index) => {
    const start = side[index]
    const before = side[Math.max(0, index - 1)]
    const after = side[Math.min(side.length - 1, index + 2)]
    const c1 = { x: start.x + (point.x - before.x) / 6, y: start.y + (point.y - before.y) / 6 }
    const c2 = { x: point.x - (after.x - start.x) / 6, y: point.y - (after.y - start.y) / 6 }
    controls.push(c1, c2)
    return `C ${c1.x} ${c1.y} ${c2.x} ${c2.y} ${point.x} ${point.y}`
  }).join(' ')
  const d = `M ${left[0].x} ${left[0].y} ${curve(left)} L ${right.at(-1)!.x} ${right.at(-1)!.y} ${curve([...right].reverse())} Z`
  return { d, points: controls, stroke: 0 }
}

const rotate = (p: Point, angle: number, origin: Point): Point => ({
  x: origin.x + p.x * Math.cos(angle) - p.y * Math.sin(angle),
  y: origin.y + p.x * Math.sin(angle) + p.y * Math.cos(angle),
})
const transform = (p: Point, angle: number) => `translate(${p.x} ${p.y}) rotate(${angle * 180 / Math.PI})`

function stemGeometry(p: BranchSection, t: TreeTuning, tiltDeg: number, anchor: Point): StemGeometry {
  const tilt = tiltDeg * Math.PI / 180
  const bend = ranged(t, 'stemBend', p.id)
  const angle = Math.atan2(p.side * p.ny * Math.cos(tilt) + p.ty * Math.sin(tilt), p.side * p.nx * Math.cos(tilt) + p.tx * Math.sin(tilt))
  const length = stemLength(p.twig.durationMs, t)
  const control = { x: length * t.stemControlX, y: -length * t.stemControlY * bend }
  const tip = { x: length, y: -length * t.stemTipY * bend }
  const leaves = p.leaves.map((tool, index): LeafGeometry => {
    const u = p.leaves.length === 1 ? 1 : t.leafStart + (1 - t.leafStart) * index / (p.leaves.length - 1)
    const at = { x: 2 * (1 - u) * u * control.x + u * u * tip.x, y: 2 * (1 - u) * u * control.y + u * u * tip.y }
    const tangent = { x: 2 * (1 - u) * control.x + 2 * u * (tip.x - control.x), y: 2 * (1 - u) * control.y + 2 * u * (tip.y - control.y) }
    const random = (Math.imul(tool.firstSequence, 2654435761) >>> 0) / 4294967296
    const leafAngle = Math.atan2(tangent.y, tangent.x) + (random < t.leafSideProbability ? -p.side : p.side) * t.leafSpread * Math.PI / 180
    const size = leafLength(tool.durationMs, t)
    const width = size * ranged(t, 'leafWidth', tool.id)
    const line = (a: number, b: number, stroke: number): PathGeometry => ({ d: `M ${a} 0 L ${b} 0`, points: [{ x: a, y: 0 }, { x: b, y: 0 }], stroke })
    return { tool, at, angle: leafAngle, transform: transform(at, leafAngle), size, width, stroke: FIXED_GEOMETRY.leafStrokeWidth,
      ellipse: { cx: t.petioleLength + size / 2, cy: 0, rx: size / 2, ry: width / 2 },
      petiole: line(0, t.petioleLength, FIXED_GEOMETRY.petioleWidth),
      vein: line(t.petioleLength + size * FIXED_GEOMETRY.veinStart, t.petioleLength + size * FIXED_GEOMETRY.veinEnd, FIXED_GEOMETRY.veinWidth) }
  })
  return { anchor, angle, transform: transform(anchor, angle), length, tip, tipRadius: t.modelTipRadius, leaves,
    width: Math.min(t.stemWidth, t.stemWidthRatio * p.width),
    path: { d: `M 0 0 Q ${control.x} ${control.y} ${tip.x} ${tip.y}`, points: [{ x: 0, y: 0 }, control, tip], stroke: t.stemWidth } }
}

function contentOf(branch: BranchState, units: CallUnit[], distanceOfCall: Map<string, number>): BranchSection[] {
  const baseSide: -1 | 1 = hash(`${branch.branchId}/side`) < .5 ? -1 : 1
  return units.map((unit, index) => {
    const side: -1 | 1 = index % 2 === 0 ? baseSide : baseSide === 1 ? -1 : 1
    return {
      id: unit.model.id,
      twig: unit.model,
      leaves: unit.tools,
      side,
      y: distanceOfCall.get(unit.model.id) ?? 0,
      x: 0, tx: 0, ty: 1, nx: 1, ny: 0, width: 0,
    }
  })
}

interface LayoutEntry {
  key: string
  globalKey: string
  raw: BranchGeometry
  final: BranchGeometry
  boundary: ConnectionSides
  input: { root: Point; initialAngle: number; startWidth: number; attach?: { center: Point; distance: number; width: number; side: -1 | 1; heading: number } }
  revision: number
  walkKey: string
  walk: { points: Point[]; distances: number[]; callDistances: number[] }
}

// JSON normally aliases NaN/Infinity to null, which is unsafe for invalidation.
const inputKey = (value: unknown) => JSON.stringify(value, (_key, item: unknown) =>
  typeof item === 'number' && !Number.isFinite(item) ? String(item) : item)

/** One latest entry per current Branch, never an archive of input revisions. */
export function createTreeLayoutCache() {
  let entries = new Map<string, LayoutEntry>()
  let taskId: string | null = null
  const stats = { computedBranches: 0, computedWalks: 0, layouts: 0, entries: 0 }
  return {
    stats,
    clear() { entries.clear(); taskId = null; stats.entries = 0 },
    layout(task: TaskState, tuning: TreeTuning = DEFAULT_TUNING) {
      if (taskId !== task.taskId) entries.clear()
      taskId = task.taskId
      const next = new Map<string, LayoutEntry>()
      try {
        const result = computeLayout(task, tuning, entries, next, stats)
        entries = next
        stats.entries = entries.size
        return result
      } catch (error) {
        entries.clear()
        stats.entries = 0
        throw error
      }
    },
  }
}

const activeTreeLayouts = createTreeLayoutCache()
export function cachedLayoutTree(task: TaskState, tuning: TreeTuning = DEFAULT_TUNING) {
  return activeTreeLayouts.layout(task, tuning)
}
export function clearCachedTreeLayout() { activeTreeLayouts.clear() }
export function treeLayoutStats() { return { ...activeTreeLayouts.stats } }

/** Refresh call facts without copying sampled paths or retaining old event payloads. */
function withCalls(g: BranchGeometry, calls: CallGroup[]): BranchGeometry {
  const byId = new Map(calls.map(call => [call.id, call]))
  return { ...g, calls, sections: g.sections.map(section => ({ ...section,
    twig: byId.get(section.twig.id)!, leaves: section.leaves.map(call => byId.get(call.id)!),
    shape: section.shape ? { ...section.shape, leaves: section.shape.leaves.map(leaf => ({ ...leaf, tool: byId.get(leaf.tool.id)! })) } : undefined,
  })) }
}

export function layoutTree(task: TaskState, tuning: TreeTuning = DEFAULT_TUNING): Map<string, BranchGeometry> {
  return computeLayout(task, tuning)
}

function computeLayout(task: TaskState, tuning: TreeTuning,
  previous = new Map<string, LayoutEntry>(), next = new Map<string, LayoutEntry>(),
  stats?: { computedBranches: number; computedWalks: number; layouts: number }): Map<string, BranchGeometry> {
  const result = new Map<string, BranchGeometry>()
  // Final visible sides and their source positions, scoped to this layout.
  const boundaries = new WeakMap<BranchGeometry, ConnectionSides>()
  const branches = task.branchOrder.map(id => task.branches[id]).filter(Boolean)
  const children = (id: string) => branches.filter(b => b.parentBranchId === id)
  const cutoff = (branch: BranchState): number | null => {
    const accepted = Object.values(task.chances).filter(c => c.parentBranchId === branch.branchId && typeof c.stage === 'object' && 'accepted' in c.stage)
    if (accepted.length > 1) throw new Error(`Branch ${branch.branchId} 有多个已接受的 Chance，无法确定截止点`)
    if (!accepted.length) return null
    const chance = accepted[0]
    const proposal = (chance.stage as { accepted: string }).accepted
    const validation = chance.validationId === null ? undefined : chance.validations[chance.validationId]
    if (!validation || validation.proposalId !== proposal || validation.validatedAtMs === null || !Number.isFinite(validation.validatedAtMs)) {
      throw new Error(`Chance ${chance.chanceId} 缺少已接受 Proposal 的验证时间，无法确定父枝截止点`)
    }
    return validation.validatedAtMs
  }
  const cutoffs = new Map(branches.map(branch => [branch.branchId, cutoff(branch)]))
  const visibleCalls = (branch: BranchState): CallGroup[] => {
    const end = cutoffs.get(branch.branchId) ?? null
    return groupBranchCalls(branch).filter(call => end === null || requiredStart(call, `Branch ${branch.branchId} 的调用`) <= end)
  }
  const counts = new Map<string, number>()
  const count = (b: BranchState): number => {
    if (!counts.has(b.branchId)) counts.set(b.branchId, visibleCalls(b).length + children(b.branchId).reduce((sum, child) => sum + count(child), 0))
    return counts.get(b.branchId)!
  }
  const hits = new Map<string, LayoutEntry>()
  let computed = false
  const tuningKey = inputKey(tuning)
  // Attachment direction, root widths and Chance lookup can change even on
  // another branch. Conservatively invalidate the tree's structural changes.
  const structureKey = JSON.stringify([branches.map(b => [b.branchId, b.parentBranchId, b.chanceId]),
    Object.values(task.chances).map(c => [c.chanceId, c.parentBranchId])])
  const workloadKey = JSON.stringify(branches.map(b => [b.branchId, count(b)]))
  const globalKey = JSON.stringify([structureKey, workloadKey])
  let revision = 0
  for (const entry of previous.values()) revision = Math.max(revision, entry.revision)

  const place = (branch: BranchState, root: Point, initialAngle: number, startWidth: number,
    attach?: { parent: BranchGeometry; center: Point; distance: number; width: number; side: -1 | 1; heading: number },
    ): void => {
    const allCalls = [...groupBranchCalls(branch)].sort(
      (a, b) => (a.startMs ?? 0) - (b.startMs ?? 0) || a.firstSequence - b.firstSequence,
    )
    const starts = allCalls.map(call => requiredStart(call, `Branch ${branch.branchId} 的调用`))
    const end = cutoffs.get(branch.branchId) ?? null
    const calls = end === null ? allCalls : allCalls.filter(call => call.startMs! <= end)
    const units = pairCalls(calls)
    const kids = children(branch.branchId)
    const chances = Object.values(task.chances).filter(chance => chance.parentBranchId === branch.branchId)
    // Every value read by geometry, including full descendant workload and the
    // parent's final visible boundary. No assumption that growth affects a tail.
    const key = inputKey([tuningKey, root, initialAngle, startWidth,
      branch.parentBranchId, branch.chanceId, end,
      allCalls.map(c => [c.id, c.kind, c.firstSequence, c.startMs, c.durationMs, c.events[0]?.finished_at_ms]),
      kids.map(child => [child.branchId, child.chanceId, count(child),
        groupBranchCalls(child).map(c => [c.firstSequence, c.startMs]),
        cutoffs.get(child.branchId),
        Object.values(task.chances).filter(c => c.parentBranchId === child.branchId).map(c => [c.chanceId, c.speculationStartedAtMs])]),
      chances.map(c => [c.chanceId, c.speculationStartedAtMs]),
      attach ? [next.get(attach.parent.branchId)?.revision, attach.center, attach.distance, attach.width, attach.side, attach.heading] : null])
    const cached = previous.get(branch.branchId)
    if (cached?.key === key && cached.globalKey === globalKey) {
      const geometry = withCalls(cached.raw, calls)
      geometry.forkPoints = new Map(cached.raw.forkPoints)
      result.set(branch.branchId, geometry)
      boundaries.set(geometry, cached.boundary)
      hits.set(branch.branchId, cached)
      next.set(branch.branchId, { ...cached, raw: geometry })
      // Descendants still inspect their own inputs; an unchanged parent does
      // not imply unchanged model durations or payloads on a child.
      for (const child of kids) {
        const old = previous.get(child.branchId)
        if (!old) throw new Error('缺少子枝布局缓存')
        const input = old.input
        place(child, input.root, input.initialAngle, input.startWidth,
          input.attach ? { ...input.attach, parent: geometry } : undefined)
      }
      return
    }
    if (stats) stats.computedBranches++
    computed = true

    // One step per call interval. Each step has its own log length and turns
    // relative to the previous one; nothing outside the calls splits a step.
    const walkKey = inputKey([initialAngle, tuning.segmentK, tuning.segmentTauMs,
      tuning.recenterRange, tuning.turnStepDeg, tuning.maxLeanDeg, tuning.sampleGap,
      allCalls.map(c => [c.firstSequence, c.startMs])])
    const walk = cached?.walkKey === walkKey ? cached.walk : null
    const walkPoints: Point[] = walk?.points ?? [{ x: NODE_HALF_WIDTH, y: 0 }]
    const walkDistances: number[] = walk?.distances ?? [0]
    const callDistances: number[] = walk?.callDistances ?? [0]
    if (!walk && stats) stats.computedWalks++
    let heading = initialAngle
    let x = NODE_HALF_WIDTH
    let y = 0
    for (let i = 1; !walk && i < allCalls.length; i++) {
      const length = segmentLength(starts[i] - starts[i - 1], tuning)
      if (length === 0) {
        callDistances.push(callDistances[i - 1])
        continue
      }
      const aim = (x - NODE_HALF_WIDTH) + tuning.recenterRange * Math.sin(heading)
      const pRight = Math.max(0, Math.min(1, .5 - aim / (2 * tuning.recenterRange)))
      const turn = tuning.turnStepDeg * Math.PI / 180
      const lean = tuning.maxLeanDeg * Math.PI / 180
      const next = Math.max(-lean, Math.min(lean, heading + (hash(`${branch.branchId}/turn/${allCalls[i].firstSequence}`) < pRight ? turn : -turn)))
      const steps = Math.max(1, Math.ceil(length / tuning.sampleGap))
      for (let step = 1; step <= steps; step++) {
        const t = step / steps
        const eased = t * t * (3 - 2 * t)
        const h = heading + (next - heading) * eased
        x += Math.sin(h) * length / steps
        y += Math.cos(h) * length / steps
        walkPoints.push({ x, y })
        walkDistances.push(callDistances[i - 1] + length * t)
      }
      heading = next
      callDistances.push(callDistances[i - 1] + length)
    }
    const fullLength = callDistances[callDistances.length - 1]
    const fullDistanceAtTime = (time: number): number => {
      if (!starts.length || time <= starts[0]) return 0
      if (time >= starts[starts.length - 1]) return fullLength
      let index = 1
      while (index < starts.length && starts[index] < time) index++
      const span = starts[index] - starts[index - 1]
      return callDistances[index - 1] + (callDistances[index] - callDistances[index - 1]) * (span > 0 ? (time - starts[index - 1]) / span : 0)
    }
    const totalLength = end === null ? fullLength : fullDistanceAtTime(end)
    let points = walkPoints, pointDistances = walkDistances
    if (end !== null && totalLength < fullLength) {
      let index = 1
      while (index < walkDistances.length && walkDistances[index] < totalLength) index++
      points = walkPoints.slice(0, index)
      pointDistances = walkDistances.slice(0, index)
      if (totalLength > 0 && index < walkDistances.length) {
        const t = (totalLength - walkDistances[index - 1]) / (walkDistances[index] - walkDistances[index - 1])
        points.push({ x: walkPoints[index - 1].x + (walkPoints[index].x - walkPoints[index - 1].x) * t,
          y: walkPoints[index - 1].y + (walkPoints[index].y - walkPoints[index - 1].y) * t })
        pointDistances.push(totalLength)
      }
    }
    // Build the local growth path before choosing its parent-side anchor.
    // Placement must use the direction actually drawn, not the requested angle.
    if (attach) {
      const direction = points.length > 1
        ? { x: points[1].x - points[0].x, y: points[1].y - points[0].y }
        : { x: Math.sin(initialAngle), y: Math.cos(initialAngle) }
      const normal = { x: Math.cos(attach.heading), y: -Math.sin(attach.heading) }
      const projection = direction.x * normal.x + direction.y * normal.y
      const side: -1 | 1 = projection === 0 ? attach.side : projection > 0 ? 1 : -1
      const fork = {
        x: attach.center.x + side * normal.x * attach.width / 2,
        y: attach.center.y + side * normal.y * attach.width / 2,
      }
      attach.parent.forkPoints.set(branch.branchId, fork)
      root = { x: attach.parent.x + attach.center.x, y: attach.parent.y + attach.center.y }
    }
    const distanceOfCall = new Map(allCalls.map((call, index) => [call.id, callDistances[index]]))

    // Chances and completion points share the original time mapping, capped by
    // the accepted Proposal's validation point when there is one.
    const chancePositions = new Map<string, number>()
    const distanceAtTime = (time: number): number => Math.min(totalLength, fullDistanceAtTime(time))
    for (const chance of chances) {
      if (chance.speculationStartedAtMs === null || !Number.isFinite(chance.speculationStartedAtMs)) {
        throw new Error(`Chance ${chance.chanceId} 缺少投机起点时间，无法确定它在枝上的位置`)
      }
      chancePositions.set(chance.chanceId, distanceAtTime(chance.speculationStartedAtMs))
    }

    const at = (distance: number): Point => {
      if (points.length === 1) return points[0]
      let index = 1
      while (index < points.length - 1 && pointDistances[index] < distance) index++
      const span = Math.max(1e-6, pointDistances[index] - pointDistances[index - 1])
      const t = Math.max(0, Math.min(1, (distance - pointDistances[index - 1]) / span))
      return {
        x: points[index - 1].x + (points[index].x - points[index - 1].x) * t,
        y: points[index - 1].y + (points[index].y - points[index - 1].y) * t,
      }
    }
    const headingAt = (distance: number): number => {
      if (points.length === 1) return initialAngle
      let index = 1
      while (index < pointDistances.length - 1 && pointDistances[index] < distance) index++
      const before = points[index - 1]
      const after = points[index]
      return Math.atan2(after.x - before.x, after.y - before.y)
    }

    // Width follows the work left at or below a point.
    const work = [
      ...calls.map(call => ({ d: distanceOfCall.get(call.id) ?? 0, n: 1 })),
      ...kids.map(child => ({ d: chancePositions.get(child.chanceId ?? '') ?? 0, n: count(child) })),
    ].sort((a, b) => a.d - b.d)
    const total = count(branch)
    const remainingAt = (distance: number): number => {
      let remaining = total
      for (const item of work) if (item.d <= distance) remaining -= item.n
      return Math.max(0, remaining)
    }
    const widthAt = makeWidthAt(work, total, startWidth, tuning)

    const childPlacements = kids.map(child => {
      const distance = chancePositions.get(child.chanceId ?? '') ?? 0
      const point = at(distance), localHeading = headingAt(distance)
      const pRight = Math.max(0, Math.min(1, .5 - (point.x - NODE_HALF_WIDTH) / (2 * tuning.recenterRange)))
      const preferred = hash(child.branchId + '/side') < pRight ? 1 : -1
      const angleDeg = childAngleOf(tuning, child.branchId)
      return { child, distance, angleDeg, ...chooseForkSide({ heading: localHeading, angleDeg, preferred }) }
    })
    const sections = contentOf(branch, units, distanceOfCall).map(section => {
      const finished = section.twig.events[0]?.finished_at_ms
      const finish = typeof finished === 'number' && Number.isFinite(finished) ? finished : section.twig.startMs! + (section.twig.durationMs ?? 0)
      const stemDistance = distanceAtTime(finish)
      const point = at(stemDistance)
      const localHeading = headingAt(stemDistance)
      return {
        ...section,
        side: (hash(`${section.id}/side`) < .5 - tuning.stemOppositeBias * Math.sin(localHeading) ? 1 : -1) as -1 | 1,
        ...point,
        y: stemDistance,
        tx: Math.sin(localHeading),
        ty: Math.cos(localHeading),
        nx: Math.cos(localHeading),
        ny: -Math.sin(localHeading),
        width: widthAt(stemDistance),
      }
    })
    const placements = placeStems(sections.map(s => {
      const base = ranged(tuning, 'stemAngle', s.id)
      const min = Math.min(tuning.stemAngleMin, tuning.stemAngleMax), max = Math.max(tuning.stemAngleMin, tuning.stemAngleMax)
      return { id: s.id, distance: s.y, range: stemLength(s.twig.durationMs, tuning), preferred: s.side, base,
        angles: tuning.stemAngleRandom ? Array.from({ length: 8 }, (_, i) => min + (max - min) * i / 7) : [base] }
    }), childPlacements)

    const forkPoints = new Map<string, Point>()
    const widths = pointDistances.map(widthAt)
    const outline = attach ? { points, widths } : startLead(points, widths, initialAngle, tuning.startLeadRatio)
    const maxStem = calls.reduce((max, call) => call.kind === 'model' ? Math.max(max, stemLength(call.durationMs, tuning)) : max, 0)
    const geometry: BranchGeometry = {
      branchId: branch.branchId,
      calls,
      sections,
      x: root.x - NODE_HALF_WIDTH,
      y: root.y,
      width: NODE_WIDTH,
      offsetX: 0,
      offsetY: 0,
      initialAngle,
      height: Math.max(0, points[points.length - 1].y) + maxStem + tuning.leafMax,
      forkPoints,
      chancePositions,
      pathPoints: points,
      pathDistances: pointDistances,
      trunkWidths: widths,
      trunk: trunkOutline(outline.points, outline.widths, initialAngle, tuning.minTrunkWidth),
      bud: {
        base: points[0], tip: rotate({ x: tuning.budLength, y: 0 }, Math.PI / 2 - initialAngle, points[0]),
        angle: Math.PI / 2 - initialAngle, transform: transform(points[0], Math.PI / 2 - initialAngle),
        ellipse: { cx: tuning.budLength / 2, cy: 0, rx: tuning.budLength / 2, ry: tuning.budWidth / 2 },
      },
      chanceCircles: [],
      validationEnd: end === null ? null : (() => {
        const point = at(totalLength), h = headingAt(totalLength), half = widthAt(totalLength) / 2
        return { x1: point.x - Math.cos(h) * half, y1: point.y + Math.sin(h) * half,
          x2: point.x + Math.cos(h) * half, y2: point.y - Math.sin(h) * half, validatedAtMs: end }
      })(),
    }
    sections.forEach(section => {
      const placement = placements.get(section.id)!
      section.side = placement.side
      section.shape = stemGeometry(section, tuning, placement.tilt, at(section.y))
    })
    let boundary: ConnectionSides
    if (points.length > 1) {
      const positions = outline.points.length > points.length
        ? [-tuning.startLeadRatio * widths[0], ...pointDistances] : pointDistances
      boundary = [1, -1].map(side => connectionSide(outlineCurves(outline.points.map((p, i) => {
        const n = normalAt(outline.points, i, initialAngle)
        return { x: p.x + side * n.x * outline.widths[i] / 2, y: p.y + side * n.y * outline.widths[i] / 2 }
      })), positions)) as ConnectionSides
    } else {
      boundary = [connectionSide(trunkSide(geometry, 1)), connectionSide(trunkSide(geometry, -1))]
      // Centre the zero-length section's coordinate on its visible side.
      for (const side of boundary) { const half = side.positions.at(-1)! / 2; side.positions = side.positions.map(p => p - half) }
    }
    result.set(branch.branchId, geometry)

    // Empty buds keep their ordinary outline at the centre-line origin.
    if (attach && geometry.calls.length) {
      const { parent, center } = attach
      const convert = (p: Point): Point => ({ x: p.x - center.x + NODE_HALF_WIDTH, y: p.y - center.y })
      const W = geometry.trunkWidths[0]
      const connected = connectSides({ parent: boundaries.get(parent)!, child: boundary,
        position: attach.distance, anchor: center, convert,
        parentDistance: tuning.connectionParentRatio * W, childDistance: tuning.connectionRootRatio * W })
      if (connected) {
        geometry.connection = { ...connectionPath(connected.outline), rootWidth: W }
        boundary = connected.sides
      }
    }

    // The branch outline is final here; hidden ordinary root sides are gone.
    boundaries.set(geometry, boundary)
    for (const section of sections) {
      const shape = section.shape!, [origin, control, tip] = shape.path.points
      const edges: [Point[], Point[]] = [[], []]
      for (let i = 0; i <= 16; i++) {
        const u = i / 16, v = 1 - u
        const center = rotate({ x: v * v * origin.x + 2 * v * u * control.x + u * u * tip.x, y: v * v * origin.y + 2 * v * u * control.y + u * u * tip.y }, shape.angle, shape.anchor)
        const dx = 2 * v * (control.x - origin.x) + 2 * u * (tip.x - control.x), dy = 2 * v * (control.y - origin.y) + 2 * u * (tip.y - control.y)
        const derivative = rotate({ x: dx, y: dy }, shape.angle, { x: 0, y: 0 }), length = Math.hypot(dx, dy) || 1
        for (const side of [0, 1]) { const sign = side === 0 ? 1 : -1; edges[side].push({ x: center.x - sign * derivative.y / length * shape.width / 2, y: center.y + sign * derivative.x / length * shape.width / 2 }) }
      }
      const sides: [Cubic[], Cubic[]] = [outlineCurves(edges[0]), outlineCurves(edges[1])]
      const connected = connectSides({ parent: boundary, position: section.y, anchor: shape.anchor,
        child: [connectionSide(sides[0]), connectionSide(sides[1])],
        parentDistance: tuning.connectionParentRatio * shape.width, childDistance: tuning.connectionRootRatio * shape.width })
      shape.outline = connectionPath(connected?.outline ?? ordinaryOutline(sides))
      const cap = rotate(tip, shape.angle, shape.anchor)
      shape.cap = { cx: cap.x, cy: cap.y, r: shape.width / 2 }
    }
    next.set(branch.branchId, { key, globalKey, raw: geometry, final: geometry, boundary,
      revision: ++revision,
      walkKey, walk: { points: walkPoints, distances: walkDistances, callDistances },
      input: { root, initialAngle, startWidth, attach: attach ? { center: attach.center, distance: attach.distance, width: attach.width, side: attach.side, heading: attach.heading } : undefined } })

    for (const { child, distance, side, angle } of childPlacements) {
      const point = at(distance)
      const width = widthAt(distance)
      const localHeading = headingAt(distance)
      const childWidth = width * Math.pow(
        count(child) / Math.max(1, remainingAt(distance) + count(child)),
        tuning.widthExponent,
      )
      place(child, { x: geometry.x + point.x, y: geometry.y + point.y }, angle, childWidth,
        { parent: geometry, center: point, distance, width, side, heading: localHeading })
    }
  }

  for (const branch of branches.filter(b => !b.parentBranchId)) {
    place(branch, { x: 0, y: 0 }, 0, tuning.rootWidth)
  }

  // Conservative bounds in both axes. Quadratic curves stay in their control
  // hull; rotations cannot exceed the control hull's largest radius. Include
  // wide leaves, petioles, all strokes, buds and Chance circles, not just tips.
  for (const geometry of result.values()) {
    const hit = hits.get(geometry.branchId)
    if (hit) {
      const final = withCalls(hit.final, geometry.calls)
      result.set(geometry.branchId, final)
      next.get(geometry.branchId)!.final = final
      continue
    }
    // Keep the unshifted coordinates for child attachment on the next update.
    next.get(geometry.branchId)!.raw = { ...geometry, chanceCircles: [] }
    // Sample counts can exceed the engine's function-argument limit even for
    // legal tuning values. Accumulate bounds without spreading point arrays.
    let minX = Infinity, maxX = -Infinity, minY = Infinity, maxY = -Infinity
    const add = (point: Point, radius: number) => {
      minX = Math.min(minX, point.x - radius)
      maxX = Math.max(maxX, point.x + radius)
      minY = Math.min(minY, point.y - radius)
      maxY = Math.max(maxY, point.y + radius)
    }
    const ellipsePoints = (e: EllipseGeometry): Point[] => [-1, 1].flatMap(x => [-1, 1].map(y => ({ x: e.cx + x * e.rx, y: e.cy + y * e.ry })))
    // Transform local bounding rectangles as SVG/DOM does, retaining the same
    // control points instead of independently reconstructing curve formulas.
    if (geometry.connection) geometry.connection.points.forEach(point => add(point, 0))
    else if (geometry.calls.length) geometry.trunk.points.forEach(point => add(point, geometry.trunk.stroke / 2))
    else ellipsePoints(geometry.bud.ellipse).forEach(point => add(rotate(point, geometry.bud.angle, geometry.bud.base), 0))
    for (const section of geometry.sections) {
      const shape = section.shape!
      const stemPoint = (point: Point) => rotate(point, shape.angle, shape.anchor)
      shape.outline!.points.forEach(point => add(point, 0))
      add({ x: shape.cap!.cx, y: shape.cap!.cy }, shape.cap!.r)
      if (!shape.leaves.length) ellipsePoints({ cx: shape.tip.x, cy: shape.tip.y, rx: shape.tipRadius, ry: shape.tipRadius }).forEach(point => add(stemPoint(point), 0))
      for (const leaf of shape.leaves) {
        const leafPoint = (point: Point) => stemPoint(rotate(point, leaf.angle, leaf.at))
        ellipsePoints(leaf.ellipse).forEach(point => add(leafPoint(point), leaf.stroke / 2))
        for (const path of [leaf.petiole, leaf.vein]) path.points.forEach(point => add(leafPoint(point), path.stroke / 2))
      }
    }
    if (geometry.validationEnd) {
      add({ x: geometry.validationEnd.x1, y: geometry.validationEnd.y1 }, 1)
      add({ x: geometry.validationEnd.x2, y: geometry.validationEnd.y2 }, 1)
    }
    for (const [chanceId, distance] of geometry.chancePositions) {
      const child = branches.find(b => b.chanceId === chanceId)
      const sample = sampleGeometry(geometry, distance)
      const point = geometry.forkPoints.get(child?.branchId ?? '') ?? sample
      const width = result.get(child?.branchId ?? '')?.trunkWidths[0] ?? sample.width
      const circle = { chanceId, branchId: child?.branchId ?? geometry.branchId, ...point, radius: width * tuning.chanceRadiusRatio }
      geometry.chanceCircles.push(circle)
      add(circle, circle.radius)
    }
    geometry.offsetX = -minX
    geometry.offsetY = -minY
    geometry.x += minX
    geometry.y += minY
    geometry.width = Math.max(1, maxX - minX)
    geometry.height = Math.max(1, maxY - minY)
  }
  if (computed && stats) stats.layouts++
  return result
}

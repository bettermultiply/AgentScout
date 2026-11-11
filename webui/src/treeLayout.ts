import {
  actionStatusLabel,
  type BranchState,
  type TaskState,
} from './fold'
import {
  formatCost,
  formatDuration,
  formatSignedDuration,
  formatSignedMoney,
  formatTokens,
  shortId,
} from './format'
import type {
  ActionStatus,
  ModelUsageEvent,
  ObservationAmount,
  ToolFinishedEvent,
} from './types'

/**
 * Geometry for the branch tree.
 *
 * A Branch is one thick branch (a trunk). Each model call is a small twig
 * growing out of the trunk; the tool calls after that model call sit as leaves
 * along the same twig. Tool calls before the first model call are leaves on the
 * trunk itself.
 *
 * The geometry is derived from the Branch tree alone: a branch's position
 * depends only on `parentBranchId` and `createdAtSequence`. The current
 * authority path is used for styling by the renderer and never enters these
 * calculations, so promoting a branch cannot move a fork or reshape the tree.
 * A branch forks from its parent at the trunk slot of the parent call that was
 * current when the child was created, not at a shared row below the parent.
 */

/** Rendered width of one Branch node. Twigs grow down inside this box. */
export const NODE_HALF_WIDTH = 64
export const NODE_WIDTH = NODE_HALF_WIDTH * 2
/**
 * Horizontal room actually used by a Branch drawing when sibling lanes are
 * packed. It is smaller than the node box because the box also carries count
 * badges and the click area, which may safely overlap a neighbour's box as
 * long as the drawn trunk, twigs and leaves stay apart.
 */
export const LANE_HALF_WIDTH = 58
/** Compact Branch head marker. */
export const HEAD_SIZE = 12
export const HEAD_CENTER_Y = HEAD_SIZE / 2
/** Vertical gap between a fork point and the child head. */
export const FORK_DROP = 40
/** Gap between a branch lane and its neighbouring sibling lanes. */
export const CHILD_GAP = 18
/** Gap between separate root branches. */
export const ROOT_GAP = 40
/** Extra trunk below the last call. */
export const TRUNK_TAIL = 12
/** Vertical room for a tool leaf that grows directly from the trunk. */
export const LEAD_ROW = 18
/** Distance from the trunk to a leading tool leaf. */
export const LEAD_REACH = 25
const TWIG_TOP_GAP = 6
const TWIG_TAIL = 8
/** Horizontal run of a model twig before it bends downward. */
export const TWIG_OUT = 24
export const TWIG_RADIUS = 9
/** Distance between two leaves along one model twig. */
export const LEAF_GAP = 13
/** Perpendicular offset of a leaf from its twig. */
export const LEAF_OFFSET = 4
/** Fork below the head when the parent has no calls yet. */
export const STUB_FORK = 16
export const TRUNK_WIDTH = 9

export type CallKind = 'tool' | 'model'

export type LeafTone =
  | 'neutral'
  | 'accent'
  | 'success'
  | 'danger'
  | 'warning'
  | 'info'
  | 'muted'

export type CallEvent = ToolFinishedEvent | ModelUsageEvent

export interface CallGroup {
  id: string
  kind: CallKind
  tone: LeafTone
  signature: string
  events: CallEvent[]
  firstSequence: number
  lastSequence: number
}

export interface TwigLeafPoint {
  x: number
  y: number
  /** Rotation of the leaf shape in degrees, already mirrored for the side. */
  angle: number
}

export interface TwigShape {
  d: string
  leaves: TwigLeafPoint[]
  tip: { x: number; y: number }
  /** Vertical distance from the twig root to its tip. */
  depth: number
}

export interface BranchSection {
  id: string
  /** Model call that starts this twig; null for a tool leaf on the trunk. */
  twig: CallGroup | null
  /** Tool calls on this twig, or the single tool leaf when `twig` is null. */
  leaves: CallGroup[]
  side: -1 | 1
  /** Local y of the twig root, or of the leading tool leaf, from the node top. */
  y: number
  /** Vertical room consumed by this section. */
  height: number
  /** Twig path and leaf anchors; null for a leading tool leaf. */
  shape: TwigShape | null
}

export interface BranchGeometry {
  branchId: string
  calls: CallGroup[]
  sections: BranchSection[]
  /** Absolute flow x of the branch axis. */
  x: number
  /** Absolute flow y of the node top (the head's top edge). */
  y: number
  width: number
  height: number
  /** Child branch id -> absolute flow y of the fork point on this trunk. */
  forks: Map<string, number>
}

function toolTone(status: ActionStatus): LeafTone {
  switch (status) {
    case 'succeeded':
      return 'success'
    case 'failed':
      return 'danger'
    case 'cancelled':
      return 'muted'
    case 'timed_out':
      return 'warning'
  }
}

export function toolName(event: ToolFinishedEvent): string {
  return event.tool_name ?? '未上报'
}

function callSpec(event: CallEvent): {
  kind: CallKind
  tone: LeafTone
  signature: string
} {
  if (event.event === 'tool_finished') {
    return {
      kind: 'tool',
      tone: toolTone(event.status),
      signature: `tool:${toolName(event)}:${event.status}`,
    }
  }
  return {
    kind: 'model',
    tone: 'accent',
    signature: `model:${event.model}`,
  }
}

// Branch objects are immutable: a Branch gets a new object only when one of its
// own events arrives, so grouping can be reused until that Branch changes.
const groupCache = new WeakMap<BranchState, CallGroup[]>()

/**
 * Tool calls become leaves, model calls become twigs. Other events are carried
 * by the branch head (lifecycle icon, settlement marker) and are not drawn as
 * calls. Consecutive calls with the same rendering signature merge into one
 * node with a count. Because a model call has a different signature from a tool
 * call, a merged group never crosses a model-call boundary.
 */
export function groupBranchCalls(branch: BranchState): CallGroup[] {
  const cached = groupCache.get(branch)
  if (cached) return cached

  const groups: CallGroup[] = []
  for (const event of branch.events) {
    if (event.event !== 'tool_finished' && event.event !== 'model_usage') continue
    const spec = callSpec(event)
    const previous = groups.at(-1)
    if (previous && previous.signature === spec.signature) {
      groups[groups.length - 1] = {
        ...previous,
        events: [...previous.events, event],
        lastSequence: event.sequence,
      }
    } else {
      groups.push({
        id: `${branch.branchId}::call::${event.sequence}`,
        ...spec,
        events: [event],
        firstSequence: event.sequence,
        lastSequence: event.sequence,
      })
    }
  }

  groupCache.set(branch, groups)
  return groups
}

function twigDistance(index: number): number {
  return TWIG_OUT * 0.72 + index * LEAF_GAP
}

function twigLeafPoint(
  axis: number,
  y: number,
  side: -1 | 1,
  distance: number,
): TwigLeafPoint {
  const arc = (Math.PI * TWIG_RADIUS) / 2
  let localX = 0
  let localY = 0
  let angle = 0
  if (distance <= TWIG_OUT) {
    localX = distance
  } else if (distance <= TWIG_OUT + arc) {
    const radians = -Math.PI / 2 + (distance - TWIG_OUT) / TWIG_RADIUS
    localX = TWIG_OUT + TWIG_RADIUS * Math.cos(radians)
    localY = TWIG_RADIUS + TWIG_RADIUS * Math.sin(radians)
    angle = radians + Math.PI / 2
  } else {
    localX = TWIG_OUT + TWIG_RADIUS
    localY = TWIG_RADIUS + (distance - TWIG_OUT - arc)
    angle = Math.PI / 2
  }
  // The leaf hangs on the upper/outer side of the twig.
  const normalX = Math.sin(angle)
  const normalY = -Math.cos(angle)
  const mirrored = side > 0 ? angle : Math.PI - angle
  return {
    x: axis + side * (localX + normalX * LEAF_OFFSET),
    y: y + localY + normalY * LEAF_OFFSET,
    angle: (mirrored * 180) / Math.PI,
  }
}

/**
 * One model twig: a short horizontal run out of the trunk, a rounded bend and a
 * downward stem. Extra leaves lengthen the downward stem, never the horizontal
 * reach, so a growing twig stays inside its fixed Branch lane.
 */
export function twigShape(
  axis: number,
  y: number,
  side: -1 | 1,
  leafCount: number,
): TwigShape {
  const arc = (Math.PI * TWIG_RADIUS) / 2
  const needed =
    leafCount > 0
      ? twigDistance(leafCount - 1) + 8
      : TWIG_OUT + arc + 10
  const vertical = Math.max(10, needed - TWIG_OUT - arc)
  const depth = TWIG_RADIUS + vertical
  const bendX = axis + side * (TWIG_OUT + TWIG_RADIUS)
  const bendY = y + TWIG_RADIUS
  const d = [
    `M ${axis} ${y}`,
    `L ${axis + side * TWIG_OUT} ${y}`,
    `A ${TWIG_RADIUS} ${TWIG_RADIUS} 0 0 ${side > 0 ? 1 : 0} ${bendX} ${bendY}`,
    `L ${bendX} ${y + depth}`,
  ].join(' ')
  const leaves = Array.from({ length: leafCount }, (_, index) =>
    twigLeafPoint(axis, y, side, twigDistance(index)),
  )
  return {
    d,
    leaves,
    tip: { x: bendX, y: y + depth },
    depth,
  }
}

interface BranchContent {
  calls: CallGroup[]
  sections: BranchSection[]
  height: number
  /** Call group id -> local y of its trunk slot. */
  slotY: Map<string, number>
}

function buildContent(branch: BranchState): BranchContent {
  const calls = groupBranchCalls(branch)
  const sections: BranchSection[] = []
  const slotY = new Map<string, number>()

  let current: BranchSection | null = null
  let twigIndex = 0
  let leadIndex = 0
  for (const group of calls) {
    if (group.kind === 'model') {
      const side: -1 | 1 = twigIndex % 2 === 0 ? 1 : -1
      twigIndex += 1
      current = {
        id: group.id,
        twig: group,
        leaves: [],
        side,
        y: 0,
        height: 0,
        shape: null,
      }
      sections.push(current)
    } else if (current) {
      current.leaves.push(group)
    } else {
      const side: -1 | 1 = leadIndex % 2 === 0 ? 1 : -1
      leadIndex += 1
      sections.push({
        id: group.id,
        twig: null,
        leaves: [group],
        side,
        y: 0,
        height: 0,
        shape: null,
      })
    }
  }

  let cursor = HEAD_CENTER_Y
  for (const section of sections) {
    if (section.twig) {
      section.y = cursor + TWIG_TOP_GAP
      section.shape = twigShape(
        NODE_HALF_WIDTH,
        section.y,
        section.side,
        section.leaves.length,
      )
      section.height = TWIG_TOP_GAP + section.shape.depth + TWIG_TAIL
      slotY.set(section.twig.id, section.y)
      section.leaves.forEach((leaf, index) => {
        const point = section.shape?.leaves[index]
        if (point) slotY.set(leaf.id, point.y)
      })
    } else {
      section.y = cursor + LEAD_ROW / 2
      section.height = LEAD_ROW
      for (const leaf of section.leaves) slotY.set(leaf.id, section.y)
    }
    cursor += section.height
  }

  return {
    calls,
    sections,
    height: Math.max(cursor + TRUNK_TAIL, HEAD_CENTER_Y + 28),
    slotY,
  }
}

function childrenOf(task: TaskState): Map<string, BranchState[]> {
  const children = new Map<string, BranchState[]>()
  for (const branchId of task.branchOrder) {
    const branch = task.branches[branchId]
    if (!branch?.parentBranchId) continue
    const siblings = children.get(branch.parentBranchId) ?? []
    siblings.push(branch)
    children.set(branch.parentBranchId, siblings)
  }
  return children
}

interface Span {
  left: number
  right: number
}

interface ChildPlacement {
  childId: string
  /** Horizontal offset of the child axis from the parent axis. */
  offset: number
}

export function layoutTree(task: TaskState): Map<string, BranchGeometry> {
  const geometry = new Map<string, BranchGeometry>()
  const children = childrenOf(task)
  const content = new Map<string, BranchContent>()
  const spans = new Map<string, Span>()
  const plans = new Map<string, ChildPlacement[]>()
  const measuring = new Set<string>()

  function contentOf(branch: BranchState): BranchContent {
    const cached = content.get(branch.branchId)
    if (cached) return cached
    const built = buildContent(branch)
    content.set(branch.branchId, built)
    return built
  }

  function spanOf(branch: BranchState): Span {
    const cached = spans.get(branch.branchId)
    if (cached) return cached
    if (measuring.has(branch.branchId)) {
      return { left: LANE_HALF_WIDTH, right: LANE_HALF_WIDTH }
    }
    measuring.add(branch.branchId)

    const kids = children.get(branch.branchId) ?? []
    const left: Array<{ branch: BranchState; span: Span }> = []
    const right: Array<{ branch: BranchState; span: Span }> = []
    let leftWeight = 0
    let rightWeight = 0
    for (const child of kids) {
      const childSpan = spanOf(child)
      const weight = childSpan.left + childSpan.right + CHILD_GAP
      if (leftWeight <= rightWeight) {
        left.push({ branch: child, span: childSpan })
        leftWeight += weight
      } else {
        right.push({ branch: child, span: childSpan })
        rightWeight += weight
      }
    }

    const placements: ChildPlacement[] = []
    let spanLeft = LANE_HALF_WIDTH
    let cursor = LANE_HALF_WIDTH + CHILD_GAP
    for (const child of left) {
      const offset = -(cursor + child.span.right)
      placements.push({ childId: child.branch.branchId, offset })
      spanLeft = Math.max(spanLeft, -offset + child.span.left)
      cursor = -offset + child.span.left + CHILD_GAP
    }
    let spanRight = LANE_HALF_WIDTH
    cursor = LANE_HALF_WIDTH + CHILD_GAP
    for (const child of right) {
      const offset = cursor + child.span.left
      placements.push({ childId: child.branch.branchId, offset })
      spanRight = Math.max(spanRight, offset + child.span.right)
      cursor = offset + child.span.right + CHILD_GAP
    }

    plans.set(branch.branchId, placements)
    const span = { left: spanLeft, right: spanRight }
    spans.set(branch.branchId, span)
    measuring.delete(branch.branchId)
    return span
  }

  function place(branch: BranchState, axisX: number, topY: number) {
    const branchContent = contentOf(branch)
    const forks = new Map<string, number>()
    const kids = children.get(branch.branchId) ?? []
    for (const child of kids) {
      let forkY = HEAD_CENTER_Y + STUB_FORK
      for (const group of branchContent.calls) {
        if (group.firstSequence <= child.createdAtSequence) {
          forkY = branchContent.slotY.get(group.id) ?? forkY
        } else {
          break
        }
      }
      forks.set(child.branchId, topY + forkY)
    }

    geometry.set(branch.branchId, {
      branchId: branch.branchId,
      calls: branchContent.calls,
      sections: branchContent.sections,
      x: axisX,
      y: topY,
      width: NODE_WIDTH,
      height: branchContent.height,
      forks,
    })

    for (const placement of plans.get(branch.branchId) ?? []) {
      const child = task.branches[placement.childId]
      if (!child) continue
      const forkY = forks.get(child.branchId) ?? topY + HEAD_CENTER_Y + STUB_FORK
      place(child, axisX + placement.offset, forkY + FORK_DROP)
    }
  }

  const roots = task.branchOrder
    .map((branchId) => task.branches[branchId])
    .filter((branch): branch is BranchState => Boolean(branch && !branch.parentBranchId))
  const main = roots[0]
  if (main) {
    const mainSpan = spanOf(main)
    place(main, 0, 0)
    let cursor = mainSpan.right + ROOT_GAP
    for (const root of roots.slice(1)) {
      const span = spanOf(root)
      const axisX = cursor + span.left
      place(root, axisX, 0)
      cursor = axisX + span.right + ROOT_GAP
    }
  }

  return geometry
}

export function callKindLabel(kind: CallKind): string {
  return kind === 'tool' ? '工具调用' : '模型调用'
}

export interface CallDetail {
  sequence: number
  actionId: string | null
  text: string
}

function tokenText(usage: ModelUsageEvent): string {
  if (usage.total_tokens !== null) return `${formatTokens(usage.total_tokens)} token`
  if (usage.input_tokens !== null || usage.output_tokens !== null) {
    return `${formatTokens(usage.input_tokens ?? 0)} 入 / ${formatTokens(
      usage.output_tokens ?? 0,
    )} 出`
  }
  return 'token 未上报'
}

function costText(usage: ModelUsageEvent): string {
  return usage.cost === null ? '费用未上报' : formatCost(usage.cost)
}

function actionText(actionId: string | null): string {
  return actionId === null ? 'action 未配对' : `action ${shortId(actionId)}`
}

/** One line per call in a group; merged groups list every call. */
export function callDetailLines(group: CallGroup): CallDetail[] {
  return group.events.map((event) => {
    if (event.event === 'tool_finished') {
      const duration =
        event.duration_ms === null ? '耗时未上报' : formatDuration(event.duration_ms)
      return {
        sequence: event.sequence,
        actionId: event.action_id,
        text: `${toolName(event)} · ${actionStatusLabel(event.status)} · ${duration} · ${actionText(
          event.action_id,
        )}`,
      }
    }
    const duration =
      event.duration_ms === null ? '耗时未上报' : formatDuration(event.duration_ms)
    return {
      sequence: event.sequence,
      actionId: event.action_id,
      text: `${event.model} · ${tokenText(event)} · ${duration} · ${costText(
        event,
      )} · ${actionText(event.action_id)}`,
    }
  })
}

/** Short text for the settlement marker beside a branch head. */
export function settlementSummary(branch: BranchState): string | null {
  const settlement = branch.settlements.at(-1)
  if (!settlement) return null
  return settlement.hit ? '命中' : '未命中'
}

function amountSummary(
  amount: ObservationAmount,
  format: (value: number) => string,
): string {
  const parts: string[] = []
  if (amount.measured !== null) parts.push(`实测 ${format(amount.measured)}`)
  if (amount.estimated !== null) parts.push(`估计 ${format(amount.estimated)}`)
  return parts.length > 0 ? parts.join(' · ') : '暂无数据'
}

/** Detail lines for the settlement marker carried by a branch head. */
export function settlementLines(branch: BranchState): string[] {
  const settlement = branch.settlements.at(-1)
  if (!settlement) return []
  return [
    `结算：${settlement.hit ? '命中' : '未命中'}`,
    `时间：${amountSummary(settlement.time_ms, formatSignedDuration)}`,
    `金额：${amountSummary(settlement.money, (value) =>
      formatSignedMoney(value, settlement.currency ?? '未上报'),
    )}`,
  ]
}

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
 * Geometry for the branch-and-leaf tree.
 *
 * A Branch is one thick branch (a trunk). Each of its tool calls is a leaf on
 * the trunk; each model call is a small twig growing out of the trunk. There is
 * no time axis: the vertical distance is only a readable event order.
 *
 * The current authority path is the central trunk (x = 0). Speculative
 * candidates fork from the trunk or from a parent candidate and grow
 * diagonally to the left or right. Sibling subtrees get disjoint horizontal
 * bands, and all children of one parent share one header row. With straight
 * fork edges this keeps every connector above the header row, so a connector
 * never crosses another Branch head.
 */
export const HEADER_SIZE = 44
export const TRUNK_WIDTH = 14
export const TRUNK_TOP_GAP = 0
export const SLOT_TOP = 26
export const SLOT_SPACING = 34
export const TRUNK_TAIL = 20
export const MIN_TRUNK_HEIGHT = 64
export const FORK_GAP = 20
export const STUB_FORK = 10

/** Horizontal room reserved around a branch axis, on the trunk side. */
const INNER_HALF = 38
/** Horizontal room reserved around a branch axis, on the outward side. */
const OUTER_HALF = 68
const CHILD_GAP = 22
const ROOT_GAP = 48
const TILT_MAX = 26
const TILT_RATIO = 0.12

/** Click target around a tool leaf; the visible square is smaller. */
export const TOOL_LEAF_SIZE = 18
export const TOOL_LEAF_OFFSET = 8
export const TWIG_LENGTH = 26
export const TWIG_LEAF_SIZE = 8
export const TWIG_WIDTH = TWIG_LENGTH + TWIG_LEAF_SIZE
/** Click target around a model twig; the visible line is thinner. */
export const TWIG_HEIGHT = 20

export type CallKind = 'tool' | 'model'

export type CallSide = -1 | 0 | 1

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

function callSpec(event: CallEvent): { kind: CallKind; tone: LeafTone; signature: string } {
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
 * node with a count; the hover card still lists every call in the group.
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

export interface BranchGeometry {
  branchId: string
  calls: CallGroup[]
  headerX: number
  headerY: number
  trunkTop: number
  trunkHeight: number
  /** Horizontal drift of the trunk bottom relative to its top. */
  trunkDx: number
  /** Absolute centre y of each call slot, in event order. */
  slotCenters: number[]
  /** Child branch id -> source offset from the trunk top. */
  forks: Map<string, number>
  side: CallSide
  /** Outward direction of each call slot: model twigs use it, leaves lean to it. */
  callSides: Array<-1 | 1>
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

export function layoutTree(task: TaskState): Map<string, BranchGeometry> {
  const geometry = new Map<string, BranchGeometry>()
  const children = childrenOf(task)
  const spine = new Set<string>()
  let cursor = task.authority
  while (cursor && task.branches[cursor] && !spine.has(cursor)) {
    spine.add(cursor)
    const branch = task.branches[cursor]
    cursor = branch.parentBranchId
  }

  let minX = Number.POSITIVE_INFINITY
  let maxX = Number.NEGATIVE_INFINITY

  const widths = new Map<string, number>()
  let leftWeight = 0
  let rightWeight = 0
  function widthOf(branch: BranchState): number {
    const cached = widths.get(branch.branchId)
    if (cached !== undefined) return cached
    const kids = children.get(branch.branchId) ?? []
    let width = INNER_HALF + OUTER_HALF
    if (kids.length > 0) {
      width += CHILD_GAP * (kids.length + 1)
      for (const kid of kids) width += widthOf(kid)
    }
    widths.set(branch.branchId, width)
    return width
  }

  function splitSides(candidates: BranchState[]): {
    left: BranchState[]
    right: BranchState[]
  } {
    const left: BranchState[] = []
    const right: BranchState[] = []
    // Balance across the whole spine, not only within one fork, so broad
    // subtrees do not all pile up on the same side of the central trunk.
    for (const candidate of candidates) {
      const width = widthOf(candidate)
      if (leftWeight <= rightWeight) {
        left.push(candidate)
        leftWeight += width + CHILD_GAP
      } else {
        right.push(candidate)
        rightWeight += width + CHILD_GAP
      }
    }
    return { left, right }
  }

  function track(axisX: number, side: CallSide) {
    const leftHalf = side > 0 ? INNER_HALF : OUTER_HALF
    const rightHalf = side < 0 ? INNER_HALF : OUTER_HALF
    minX = Math.min(minX, axisX - leftHalf)
    maxX = Math.max(maxX, axisX + rightHalf)
  }

  function place(
    branch: BranchState,
    headerX: number,
    headerY: number,
    side: CallSide,
    onSpine: boolean,
  ) {
    const calls = groupBranchCalls(branch)
    const trunkTop = headerY + HEADER_SIZE + TRUNK_TOP_GAP
    const slotCenters = calls.map(
      (_, index) => trunkTop + SLOT_TOP + index * SLOT_SPACING,
    )
    const trunkHeight = Math.max(
      MIN_TRUNK_HEIGHT,
      SLOT_TOP + calls.length * SLOT_SPACING + TRUNK_TAIL,
    )
    const trunkDx =
      side === 0 ? 0 : side * Math.min(TILT_MAX, trunkHeight * TILT_RATIO)

    const forks = new Map<string, number>()
    for (const child of children.get(branch.branchId) ?? []) {
      let index = -1
      for (let i = 0; i < calls.length; i += 1) {
        if (calls[i].firstSequence <= child.createdAtSequence) index = i
        else break
      }
      const forkY =
        index >= 0
          ? slotCenters[index]
          : trunkTop + STUB_FORK
      forks.set(child.branchId, forkY - trunkTop)
    }

    let toolIndex = 0
    let modelIndex = 0
    const callSides = calls.map((call): -1 | 1 => {
      if (side !== 0) return side
      if (call.kind === 'tool') {
        const value = toolIndex % 2 === 0 ? 1 : -1
        toolIndex += 1
        return value
      }
      const value = modelIndex % 2 === 0 ? 1 : -1
      modelIndex += 1
      return value
    })

    geometry.set(branch.branchId, {
      branchId: branch.branchId,
      calls,
      headerX,
      headerY,
      trunkTop,
      trunkHeight,
      trunkDx,
      slotCenters,
      forks,
      side,
      callSides,
    })
    track(headerX, side)

    const childHeaderY = trunkTop + trunkHeight + FORK_GAP
    const kids = children.get(branch.branchId) ?? []
    if (onSpine) {
      const spineKids = kids.filter((kid) => spine.has(kid.branchId))
      const candidates = kids.filter((kid) => !spine.has(kid.branchId))
      const { left, right } = splitSides(candidates)

      let leftCursor = 0
      for (const child of left) {
        const edge = headerX - OUTER_HALF - CHILD_GAP - leftCursor
        const childAxis = edge - INNER_HALF
        place(child, childAxis, childHeaderY, -1, false)
        leftCursor += widthOf(child) + CHILD_GAP
      }

      let rightCursor = 0
      for (const child of right) {
        const edge = headerX + OUTER_HALF + CHILD_GAP + rightCursor
        const childAxis = edge + INNER_HALF
        place(child, childAxis, childHeaderY, 1, false)
        rightCursor += widthOf(child) + CHILD_GAP
      }

      const next = spineKids[0]
      if (next) place(next, headerX, childHeaderY, 0, true)
    } else {
      const direction = side === 0 ? 1 : side
      let outward = 0
      for (const child of kids) {
        const childAxis =
          direction > 0
            ? headerX + OUTER_HALF + CHILD_GAP + outward + INNER_HALF
            : headerX - OUTER_HALF - CHILD_GAP - outward - INNER_HALF
        place(child, childAxis, childHeaderY, direction, false)
        outward += widthOf(child) + CHILD_GAP
      }
    }
  }

  const roots = task.branchOrder
    .map((branchId) => task.branches[branchId])
    .filter((branch): branch is BranchState => Boolean(branch && !branch.parentBranchId))
  const main = roots.find((root) => spine.has(root.branchId)) ?? roots[0]
  if (main) {
    const onSpine = spine.has(main.branchId)
    place(main, 0, 0, onSpine ? 0 : 1, onSpine)
    for (const root of roots) {
      if (root.branchId === main.branchId) continue
      const axis = maxX + ROOT_GAP + INNER_HALF
      place(root, axis, 0, 1, false)
    }
  }

  if (Number.isFinite(minX) && Number.isFinite(maxX)) {
    const shift = -(minX + maxX) / 2
    if (shift !== 0) {
      for (const entry of geometry.values()) {
        entry.headerX += shift
      }
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

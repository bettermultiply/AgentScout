import {
  actionStatusLabel,
  runtimeStateLabel,
  statusLabel,
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
  BranchObservationEvent,
  BranchStatus,
  ObservationAmount,
} from './types'

/**
 * Geometry for the branch-and-leaf tree. A Branch is one vertical trunk; each
 * event (or run of identical events) is a small leaf on that trunk. There is no
 * time axis: the vertical distance is only a readable ordering of events.
 */
export const LANE_WIDTH = 112
export const HEADER_SIZE = 44
export const TRUNK_TOP_OFFSET = 36
export const TRUNK_STUB = 22
export const TRUNK_WIDTH = 16
export const FIRST_LEAF_CENTER = 64
export const LEAF_SPACING = 20
export const LEAF_SIZE = 14
export const FORK_GAP = 20

export type LeafKind =
  | 'conversation'
  | 'model'
  | 'tool'
  | 'status'
  | 'environment'
  | 'settlement'
  | 'authority'

export type LeafTone =
  | 'neutral'
  | 'accent'
  | 'success'
  | 'danger'
  | 'warning'
  | 'info'
  | 'muted'

export interface LeafGroup {
  id: string
  kind: LeafKind
  tone: LeafTone
  signature: string
  events: BranchObservationEvent[]
  firstSequence: number
  lastSequence: number
}

interface LeafSpec {
  kind: LeafKind
  tone: LeafTone
  signature: string
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

function statusTone(status: BranchStatus): LeafTone {
  if (typeof status === 'object') return 'warning'
  switch (status) {
    case 'created':
      return 'neutral'
    case 'running':
    case 'promoting':
      return 'accent'
    case 'finished':
    case 'discarding':
    case 'cleaning':
      return 'warning'
    case 'promoted':
      return 'success'
    case 'discarded':
    case 'cleaned':
      return 'muted'
  }
}

function leafSpec(event: BranchObservationEvent): LeafSpec | null {
  switch (event.event) {
    case 'branch_created':
      // The branch head already carries the creation reason and lifecycle.
      return null
    case 'conversation_updated':
      return { kind: 'conversation', tone: 'neutral', signature: 'conversation' }
    case 'model_usage':
      return {
        kind: 'model',
        tone: 'accent',
        signature: `model:${event.model}`,
      }
    case 'tool_finished':
      return {
        kind: 'tool',
        tone: toolTone(event.status),
        signature: `tool:${event.status}`,
      }
    case 'status_changed':
      return {
        kind: 'status',
        tone: statusTone(event.status),
        signature: `status:${JSON.stringify(event.status)}`,
      }
    case 'authority_changed':
      return { kind: 'authority', tone: 'accent', signature: 'authority' }
    case 'environment_changed':
      return {
        kind: 'environment',
        tone:
          event.state === 'failed'
            ? 'danger'
            : event.state === 'terminated' || event.state === 'cleaned'
              ? 'muted'
              : 'info',
        signature: `environment:${event.state}`,
      }
    case 'environment_cleaned':
      return {
        kind: 'environment',
        tone: event.succeeded ? 'success' : 'danger',
        signature: `environment-cleaned:${event.succeeded}`,
      }
    case 'settlement':
      return {
        kind: 'settlement',
        tone: event.hit ? 'success' : 'danger',
        signature: `settlement:${event.hit}`,
      }
  }
}

// Branch objects are immutable: a Branch gets a new object only when one of its
// own events arrives, so grouping can be reused until that Branch changes.
const groupCache = new WeakMap<BranchState, LeafGroup[]>()

/**
 * One leaf per event, except that consecutive events with the same rendering
 * signature collapse into one leaf with a count (for example a streamed reply,
 * which emits one conversation_updated per chunk).
 */
export function groupBranchEvents(branch: BranchState): LeafGroup[] {
  const cached = groupCache.get(branch)
  if (cached) return cached

  const groups: LeafGroup[] = []
  for (const event of branch.events) {
    const spec = leafSpec(event)
    if (!spec) continue
    const previous = groups.at(-1)
    if (previous && previous.signature === spec.signature) {
      groups[groups.length - 1] = {
        ...previous,
        events: [...previous.events, event],
        lastSequence: event.sequence,
      }
    } else {
      groups.push({
        id: `${branch.branchId}::leaf::${event.sequence}`,
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
  groups: LeafGroup[]
  headerY: number
  trunkTop: number
  trunkHeight: number
  /** Absolute centre y of each leaf, in event order. */
  leafCenters: number[]
  /** Child branch id -> source offset from the trunk top. */
  forks: Map<string, number>
}

/**
 * Lay out every Branch under the parent's fork point. Each Branch belongs to one
 * lane; its header sits just below the parent leaf where the child was created,
 * so only the relative fork relationship is preserved, never elapsed time.
 */
export function layoutTree(task: TaskState): Map<string, BranchGeometry> {
  const geometry = new Map<string, BranchGeometry>()
  const children = new Map<string, BranchState[]>()

  for (const branchId of task.branchOrder) {
    const branch = task.branches[branchId]
    if (!branch) continue
    const parentId = branch.parentBranchId
    if (!parentId) continue
    const siblings = children.get(parentId) ?? []
    siblings.push(branch)
    children.set(parentId, siblings)
  }

  for (const branchId of task.branchOrder) {
    const branch = task.branches[branchId]
    if (!branch) continue

    const parent = branch.parentBranchId
      ? geometry.get(branch.parentBranchId)
      : undefined
    const parentFork =
      parent && branch.parentBranchId
        ? parent.forks.get(branchId)
        : undefined
    const headerY =
      parent && parentFork !== undefined
        ? parent.trunkTop + parentFork + FORK_GAP
        : 0

    const groups = groupBranchEvents(branch)
    const trunkTop = headerY + TRUNK_TOP_OFFSET
    const stubEnd = trunkTop + TRUNK_STUB
    const leafCenters = groups.map(
      (_, index) => headerY + FIRST_LEAF_CENTER + index * LEAF_SPACING,
    )

    const forks = new Map<string, number>()
    for (const child of children.get(branchId) ?? []) {
      let forkY = stubEnd
      if (groups.length > 0) {
        let index = -1
        for (let i = 0; i < groups.length; i += 1) {
          if (groups[i].firstSequence <= child.createdAtSequence) index = i
          else break
        }
        if (index >= 0) forkY = leafCenters[index]
      }
      forks.set(child.branchId, forkY - trunkTop)
    }

    let bottom = stubEnd
    const lastLeaf = leafCenters.at(-1)
    if (lastLeaf !== undefined) {
      bottom = Math.max(bottom, lastLeaf + LEAF_SIZE / 2 + 6)
    }
    for (const offset of forks.values()) {
      bottom = Math.max(bottom, trunkTop + offset + 10)
    }

    geometry.set(branchId, {
      branchId,
      groups,
      headerY,
      trunkTop,
      trunkHeight: bottom - trunkTop,
      leafCenters,
      forks,
    })
  }

  return geometry
}

export function leafKindLabel(kind: LeafKind): string {
  switch (kind) {
    case 'conversation':
      return '对话更新'
    case 'model':
      return '模型用量'
    case 'tool':
      return '工具完成'
    case 'status':
      return '状态变化'
    case 'environment':
      return '环境'
    case 'settlement':
      return '结算'
    case 'authority':
      return '权威切换'
  }
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

function durationText(value: number | null): string {
  return value === null ? '未上报' : formatDuration(value)
}

function tokenText(
  usage: Extract<BranchObservationEvent, { event: 'model_usage' }>,
): string {
  if (usage.total_tokens !== null) return formatTokens(usage.total_tokens)
  if (usage.input_tokens !== null || usage.output_tokens !== null) {
    return `${formatTokens(usage.input_tokens ?? 0)} 入 / ${formatTokens(usage.output_tokens ?? 0)} 出`
  }
  return '未上报'
}

/** Human-readable lines for a leaf tooltip; the latest event supplies details. */
export function leafDetailLines(group: LeafGroup): string[] {
  const event = group.events.at(-1)
  if (!event) return []
  switch (event.event) {
    case 'conversation_updated':
      return ['流式对话更新']
    case 'model_usage':
      return [
        `模型：${event.model}`,
        `token：${tokenText(event)}`,
        `费用：${event.cost === null ? '未上报' : formatCost(event.cost)}`,
        `耗时：${durationText(event.duration_ms)}`,
      ]
    case 'tool_finished':
      return [
        `结果：${actionStatusLabel(event.status)}`,
        `耗时：${durationText(event.duration_ms)}`,
        `action：${shortId(event.action_id)}`,
      ]
    case 'status_changed':
      return [
        `状态：${statusLabel(event.status)}`,
        event.reason ? `原因：${event.reason}` : '原因：未上报',
      ]
    case 'environment_changed':
      return [
        `环境：${runtimeStateLabel(event.state)}`,
        `实例：${
          event.independent_instance === null
            ? '未上报'
            : event.independent_instance
              ? '独立'
              : '共享'
        }`,
        `父环境：${event.parent_environment ?? '未上报'}`,
      ]
    case 'environment_cleaned':
      return [
        `清理：${event.succeeded ? '成功' : '失败'}`,
        event.reason ? `原因：${event.reason}` : '原因：未上报',
      ]
    case 'settlement':
      return [
        `结果：${event.hit ? '命中' : '未命中'}`,
        `时间：${amountSummary(event.time_ms, formatSignedDuration)}`,
        `金额：${amountSummary(event.money, (value) =>
          formatSignedMoney(value, event.currency ?? '未上报'),
        )}`,
      ]
    case 'authority_changed':
      return [
        event.previous_authority
          ? `上一权威：${shortId(event.previous_authority)}`
          : '上一权威：无',
      ]
    case 'branch_created':
      return []
  }
}

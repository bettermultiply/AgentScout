import type { BranchState } from '../fold'
import type { ActionStatus, ModelUsageEvent, ToolFinishedEvent } from '../types'

export type CallKind = 'tool' | 'model'
export type LeafTone = 'success' | 'danger' | 'cancelled' | 'timeout'
export type BranchTone = 'authority' | 'authority-finished' | 'candidate' | 'aborted'
export type CallEvent = ToolFinishedEvent | ModelUsageEvent

export interface CallGroup {
  id: string
  kind: CallKind
  tone: LeafTone
  events: CallEvent[]
  firstSequence: number
  startMs: number | null
  durationMs: number | null
}

export interface CallUnit {
  model: CallGroup
  tools: CallGroup[]
}

function toolTone(status: ActionStatus): LeafTone {
  return status === 'succeeded' ? 'success' : status === 'failed' ? 'danger' : status === 'cancelled' ? 'cancelled' : 'timeout'
}

export function toolName(event: ToolFinishedEvent): string {
  return event.tool_name ?? '未上报'
}

function callSpec(event: CallEvent): { kind: CallKind; tone: LeafTone } {
  return event.event === 'tool_finished'
    ? { kind: 'tool', tone: toolTone(event.status) }
    : { kind: 'model', tone: 'success' }
}

const groupCache = new WeakMap<BranchState, CallGroup[]>()

export function groupBranchCalls(branch: BranchState): CallGroup[] {
  const cached = groupCache.get(branch)
  if (cached) return cached
  const groups: CallGroup[] = []
  for (const event of branch.events) {
    if (event.event !== 'tool_finished' && event.event !== 'model_usage') continue
    const spec = callSpec(event)
    groups.push({
      id: `${branch.branchId}::call::${event.sequence}`,
      ...spec,
      events: [event],
      firstSequence: event.sequence,
      startMs: event.started_at_ms,
      durationMs: event.duration_ms,
    })
  }
  groupCache.set(branch, groups)
  return groups
}

export function requiredStart(group: CallGroup, label: string): number {
  if (group.startMs === null || !Number.isFinite(group.startMs)) {
    throw new Error(`${label}缺少开始时间（事件 #${group.firstSequence}），无法确定树的纵向位置`)
  }
  return group.startMs
}

export function pairCalls(calls: CallGroup[]): CallUnit[] {
  const units: CallUnit[] = []
  let current: CallUnit | null = null
  for (const call of calls) {
    if (call.kind === 'model') {
      current = { model: call, tools: [] }
      units.push(current)
    } else if (current) {
      current.tools.push(call)
    } else {
      throw new Error(`工具调用 #${call.firstSequence} 之前没有模型调用，无法配对`)
    }
  }
  return units
}

export function branchTone(branch: BranchState, isAuthorityPath: boolean, isCurrentAuthority: boolean): BranchTone {
  if (isCurrentAuthority) return 'authority'
  if (isAuthorityPath || branch.role === 'authority' || branch.status === 'promoted' || branch.status === 'promoting') {
    return 'authority-finished'
  }
  if (branch.status === 'discarded' || branch.status === 'cleaned' || branch.status === 'discarding' || branch.status === 'cleaning') {
    return 'aborted'
  }
  return 'candidate'
}

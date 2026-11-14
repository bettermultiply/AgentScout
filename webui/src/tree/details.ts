import { actionStatusLabel } from '../fold'
import { formatCost, formatDuration, formatTokens, shortId } from '../format'
import type { ModelUsageEvent } from '../types'
import { toolName, type CallGroup } from './calls'

export interface CallDetail {
  sequence: number
  actionId: string | null
  text: string
}

function tokenText(usage: ModelUsageEvent): string {
  if (usage.total_tokens !== null) return `${formatTokens(usage.total_tokens)} token`
  if (usage.input_tokens !== null || usage.output_tokens !== null) {
    return `${usage.input_tokens === null ? '未上报' : formatTokens(usage.input_tokens)} 入 / `
      + `${usage.output_tokens === null ? '未上报' : formatTokens(usage.output_tokens)} 出`
  }
  return 'token 未上报'
}

function actionText(id: string | null): string {
  return id === null ? 'action 未配对' : `action ${shortId(id)}`
}

export function callDetailLines(group: CallGroup): CallDetail[] {
  return group.events.map(event => event.event === 'tool_finished'
    ? {
        sequence: event.sequence,
        actionId: event.action_id,
        text: `${toolName(event)} · ${actionStatusLabel(event.status)} · `
          + `${event.duration_ms === null ? '耗时未上报' : formatDuration(event.duration_ms)} · `
          + actionText(event.action_id),
      }
    : {
        sequence: event.sequence,
        actionId: event.action_id,
        text: `${event.model} · ${tokenText(event)} · `
          + `${event.duration_ms === null ? '耗时未上报' : formatDuration(event.duration_ms)} · `
          + `${event.cost === null ? '费用未上报' : formatCost(event.cost)} · `
          + actionText(event.action_id),
      })
}

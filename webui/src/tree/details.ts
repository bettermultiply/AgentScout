import { actionStatusLabel, type BranchState } from '../fold'
import { formatCost, formatDuration, formatSignedDuration, formatSignedMoney, formatTokens, shortId } from '../format'
import type { ModelUsageEvent, ObservationAmount } from '../types'
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

function amountSummary(amount: ObservationAmount, format: (value: number) => string): string {
  const parts: string[] = []
  if (amount.measured !== null) parts.push(`实测 ${format(amount.measured)}`)
  if (amount.estimated !== null) parts.push(`估计 ${format(amount.estimated)}`)
  return parts.length ? parts.join(' · ') : '暂无数据'
}

export function settlementLines(branch: BranchState): string[] {
  const settlement = branch.settlements.at(-1)
  if (!settlement) return []
  return [
    `结算：${settlement.hit ? '命中' : '未命中'}`,
    `时间：${amountSummary(settlement.time_ms, formatSignedDuration)}`,
    `金额：${amountSummary(settlement.money, value => formatSignedMoney(value, settlement.currency ?? '未上报'))}`,
  ]
}

import type { NleMapActionEvent, NlePublicObservation, ObservationEvent } from './types'

export type MapCell = { x: number; y: number; glyph: string }
export type MapSnapshot = {
  action: string
  actionNumber: number
  player: { x: number; y: number }
  visible: MapCell[]
  message: string
}

export type MapHistoryResult = {
  history: MapSnapshot[]
  branchId: string | null
  branchSources: string[]
  quality: 'complete' | 'partial' | 'invalid'
  qualityMessage: string | null
}

function publicObservation(value: unknown): NlePublicObservation | null {
  if (!value || typeof value !== 'object') return null
  const observation = value as Partial<NlePublicObservation>
  if (!Array.isArray(observation.chars) || !Array.isArray(observation.blstats)) return null
  if (!observation.chars.every((line) => typeof line === 'string')) return null
  if (!observation.blstats.every((item) => typeof item === 'number')) return null
  return observation as NlePublicObservation
}

function toCells(observation: NlePublicObservation): { player: { x: number; y: number }; cells: MapCell[] } | null {
  const x = observation.blstats[0]
  const y = observation.blstats[1]
  if (!Number.isInteger(x) || !Number.isInteger(y)) return null
  const playerRow = observation.chars.findIndex((line) => line.includes('@'))
  if (playerRow < 0) return null
  const playerColumn = observation.chars[playerRow].indexOf('@')
  const cells: MapCell[] = []
  observation.chars.forEach((line, rowIndex) => [...line].forEach((glyph, columnIndex) => {
    if (glyph !== ' ') cells.push({ x: x + columnIndex - playerColumn, y: y + rowIndex - playerRow, glyph })
  }))
  return { player: { x, y }, cells }
}

export function eventHistory(events: ObservationEvent[]): MapHistoryResult {
  const actions = events.filter((event): event is NleMapActionEvent => event.event === 'nle_map_action')
  if (!actions.length) return { history: [], branchId: null, branchSources: [], quality: 'invalid', qualityMessage: null }

  // A map is one real world's executed history. The first action's task is the
  // map scope; candidate branches with no invoke event never enter this list.
  const taskId = [...actions].sort((a, b) => a.sequence - b.sequence)[0].task_id
  const scopedActions = actions.filter((event) => event.task_id === taskId)
  const branchSources = [...new Set(scopedActions.map((event) => event.branch_id))]
  const branchActions = [...scopedActions].sort((a, b) => a.sequence - b.sequence)
  let incomplete = false
  const firstIndex = branchActions.findIndex((event) => {
    const observation = publicObservation(event.before)
    return observation !== null && toCells(observation) !== null
  })
  if (firstIndex > 0) incomplete = true
  const first = publicObservation(branchActions[firstIndex < 0 ? 0 : firstIndex]?.before)
  const initial = first ? toCells(first) : null
  if (!initial) return { history: [], branchId: null, branchSources, quality: 'invalid', qualityMessage: '没有可解析的起点观测。' }

  const history: MapSnapshot[] = [{ action: '起点观测', actionNumber: 0, player: initial.player, visible: initial.cells, message: '真实 Runtime 公开观测 · 起点。' }]
  let previousSequence = -1
  const seen = new Set<string>()
  branchActions.slice(firstIndex < 0 ? 0 : firstIndex).forEach((event) => {
    const identity = `${event.branch_id}:${event.action_id}`
    if (event.sequence <= previousSequence || seen.has(identity)) { incomplete = true; return }
    previousSequence = event.sequence
    seen.add(identity)
    const observation = publicObservation(event.after)
    const parsed = observation ? toCells(observation) : null
    if (!parsed || !observation) { incomplete = true; return }
    history.push({ action: `动作 ${event.action}`, actionNumber: history.length, player: parsed.player, visible: parsed.cells, message: observation.message || `真实 Runtime 公开观测 · action ${event.action}。` })
  })
  const sourceText = branchSources.length > 1 ? `权威窗口，动作来源：${branchSources.join('、')}` : `权威窗口，动作来源：${branchSources[0]}`
  return { history, branchId: null, branchSources, quality: incomplete ? 'partial' : 'complete', qualityMessage: incomplete ? `${sourceText}；存在乱序、重复或无效事件，仅展示可解析历史。` : `${sourceText}，按 Ledger sequence 排序。` }
}

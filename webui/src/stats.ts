import { isActiveCandidate } from './fold'
import type { BranchState, TaskState } from './fold'
import type { ObservationAmount } from './types'

export type Completeness = 'complete' | 'partial' | 'none'

export interface SavingsValue {
  measured: number | null
  estimated: number | null
}

export interface MoneySavings extends SavingsValue {
  currency: string
}

export interface RunStats {
  activeCandidates: number
  tokens: { value: number | null; completeness: Completeness }
  cost: { value: number | null; completeness: Completeness }
  time: SavingsValue
  money: MoneySavings[]
}

function branches(task: TaskState | null): BranchState[] {
  if (!task) return []
  return task.branchOrder
    .map((id) => task.branches[id])
    .filter((branch): branch is BranchState => Boolean(branch))
}

function addAmount(target: SavingsValue, amount: ObservationAmount): void {
  if (amount.measured !== null) {
    target.measured = (target.measured ?? 0) + amount.measured
  }
  if (amount.estimated !== null) {
    target.estimated = (target.estimated ?? 0) + amount.estimated
  }
}

export function computeRunStats(task: TaskState | null): RunStats {
  const all = branches(task)
  const activeCandidates = all.filter(isActiveCandidate).length

  let tokenValue = 0
  let tokenKnown = false
  let tokenComplete = true
  let costValue = 0
  let costKnown = false
  let costComplete = true

  for (const branch of all) {
    for (const usage of branch.modelUsages) {
      if (usage.total_tokens !== null) {
        tokenValue += usage.total_tokens
        tokenKnown = true
      } else if (usage.input_tokens !== null || usage.output_tokens !== null) {
        tokenValue += (usage.input_tokens ?? 0) + (usage.output_tokens ?? 0)
        tokenKnown = true
        tokenComplete = false
      } else {
        tokenComplete = false
      }

      if (usage.cost !== null) {
        costValue += usage.cost
        costKnown = true
      } else {
        costComplete = false
      }
    }
  }

  const time: SavingsValue = { measured: null, estimated: null }
  const money = new Map<string, SavingsValue>()

  for (const branch of all) {
    for (const settlement of branch.settlements) {
      addAmount(time, settlement.time_ms)

      const hasMoney =
        settlement.money.measured !== null || settlement.money.estimated !== null
      if (!hasMoney) continue
      const currency = settlement.currency ?? '未上报'
      let entry = money.get(currency)
      if (!entry) {
        entry = { measured: null, estimated: null }
        money.set(currency, entry)
      }
      addAmount(entry, settlement.money)
    }
  }

  return {
    activeCandidates,
    tokens: {
      value: tokenKnown ? tokenValue : null,
      completeness: !tokenKnown ? 'none' : tokenComplete ? 'complete' : 'partial',
    },
    cost: {
      value: costKnown ? costValue : null,
      completeness: !costKnown ? 'none' : costComplete ? 'complete' : 'partial',
    },
    time,
    money: [...money.entries()].map(([currency, value]) => ({ currency, ...value })),
  }
}

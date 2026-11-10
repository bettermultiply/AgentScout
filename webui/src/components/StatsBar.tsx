import { useMemo } from 'react'
import {
  formatCost,
  formatCount,
  formatSignedDuration,
  formatSignedMoney,
  formatTokens,
} from '../format'
import { computeRunStats, type SavingsValue } from '../stats'
import type { TaskState } from '../fold'

function Badge({ children }: { children: string }) {
  return <span className="stat-badge">{children}</span>
}

function amountClass(value: number | null): string {
  if (value === null || value === 0) return 'stat-amount'
  return value > 0 ? 'stat-amount stat-positive' : 'stat-amount stat-negative'
}

function TimeSavings({ value }: { value: SavingsValue }) {
  if (value.measured === null && value.estimated === null) {
    return <span className="stat-empty">暂无数据</span>
  }
  return (
    <span className="stat-lines">
      <span className="stat-line">
        <span className="stat-kind">实测</span>
        <span className={amountClass(value.measured)}>
          {value.measured === null ? '暂无' : formatSignedDuration(value.measured)}
        </span>
      </span>
      <span className="stat-line">
        <span className="stat-kind">估计</span>
        <span className={amountClass(value.estimated)}>
          {value.estimated === null ? '暂无' : formatSignedDuration(value.estimated)}
        </span>
      </span>
    </span>
  )
}

export function StatsBar({ task }: { task: TaskState | null }) {
  const stats = useMemo(() => computeRunStats(task), [task])

  const tokenNote =
    stats.tokens.completeness === 'none'
      ? null
      : stats.tokens.completeness === 'complete'
        ? '已上报'
        : '部分上报'
  const costNote =
    stats.cost.completeness === 'none'
      ? null
      : stats.cost.completeness === 'complete'
        ? '已上报'
        : '部分上报'

  return (
    <div className="stats" aria-label="投机统计">
      <div className="stat" title="当前仍处于候选角色的 Branch 数量">
        <span className="stat-label">当前投机</span>
        <span className="stat-value">{formatCount(stats.activeCandidates)}</span>
      </div>
      <div className="stat" title="所有 model_usage 事件中已上报的 token 之和；缺失字段不补零">
        <span className="stat-label">累计 token</span>
        <span className="stat-value">
          {stats.tokens.value === null ? '暂无数据' : formatTokens(stats.tokens.value)}
        </span>
        {tokenNote ? <Badge>{tokenNote}</Badge> : null}
      </div>
      <div className="stat" title="所有 model_usage 事件中已上报的 cost.total 之和（Pi SDK 以 USD 计）；缺失字段不补零">
        <span className="stat-label">累计费用</span>
        <span className="stat-value">
          {stats.cost.value === null ? '暂无数据' : formatCost(stats.cost.value)}
        </span>
        {costNote ? <Badge>{costNote}</Badge> : null}
      </div>
      <div
        className="stat stat-savings"
        title="settlement.time_ms：实测值与估计值分开累加，不合并为一个数字"
      >
        <span className="stat-label">节约时间</span>
        <TimeSavings value={stats.time} />
      </div>
      <div
        className="stat stat-savings stat-money"
        title="settlement.money：按币种分别累加实测值与估计值，不合并为一个数字"
      >
        <span className="stat-label">节约金钱</span>
        {stats.money.length === 0 ? (
          <span className="stat-empty">暂无数据</span>
        ) : (
          stats.money.map((entry) => (
            <span className="stat-money-currency" key={entry.currency}>
              <span className="stat-currency">{entry.currency}</span>
              <span className="stat-lines">
                <span className="stat-line">
                  <span className="stat-kind">实测</span>
                  <span className={amountClass(entry.measured)}>
                    {entry.measured === null
                      ? '暂无'
                      : formatSignedMoney(entry.measured, entry.currency)}
                  </span>
                </span>
                <span className="stat-line">
                  <span className="stat-kind">估计</span>
                  <span className={amountClass(entry.estimated)}>
                    {entry.estimated === null
                      ? '暂无'
                      : formatSignedMoney(entry.estimated, entry.currency)}
                  </span>
                </span>
              </span>
            </span>
          ))
        )}
      </div>
    </div>
  )
}

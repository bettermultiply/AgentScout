import { useEffect, useRef } from 'react'
import {
  actionStatusLabel,
  authorityStateLabel,
  branchTokenTotal,
  chanceStageLabel,
  isTerminalStatus,
  latestModel,
  roleLabel,
  runtimeStateLabel,
  speculationKindLabel,
  statusLabel,
  type BranchState,
  type ChanceState,
} from '../fold'
import {
  formatCost,
  formatDuration,
  formatJson,
  formatTimestamp,
  formatTokens,
} from '../format'
import { toolName } from '../tree'
import { useConversation, useNow } from '../useConversation'
import type { DetailAnchor } from '../store'
import { ConversationView } from './ConversationView'

function Field({
  label,
  children,
  title,
}: {
  label: string
  children: React.ReactNode
  title?: string
}) {
  return (
    <div className="detail-field" title={title}>
      <dt>{label}</dt>
      <dd>{children}</dd>
    </div>
  )
}

function Section({
  title,
  aside,
  children,
}: {
  title: string
  aside?: React.ReactNode
  children: React.ReactNode
}) {
  return (
    <section className="detail-block">
      <header className="detail-block-head">
        <h3>{title}</h3>
        {aside}
      </header>
      {children}
    </section>
  )
}

function modelTokenText(usage: BranchState['modelUsages'][number]): string {
  if (usage.total_tokens !== null) return formatTokens(usage.total_tokens)
  if (usage.input_tokens !== null || usage.output_tokens !== null) {
    return formatTokens((usage.input_tokens ?? 0) + (usage.output_tokens ?? 0))
  }
  return '未上报'
}

export function BranchDetails({
  branch,
  chance,
  anchor,
}: {
  branch: BranchState
  chance: ChanceState | null
  anchor: DetailAnchor | null
}) {
  const active = !isTerminalStatus(branch.status)
  const now = useNow(active && branch.speculationStartedAtMs !== null)
  const conversation = useConversation(branch.branchId, true)
  const tokens = branchTokenTotal(branch)
  const model = latestModel(branch)
  const rootRef = useRef<HTMLDivElement>(null)
  const highlighted = new Set(anchor?.sequences ?? [])

  useEffect(() => {
    const root = rootRef.current
    if (!root) return
    if (!anchor) {
      const scroller = root.closest('.panel-content')
      if (scroller instanceof HTMLElement) {
        scroller.scrollTo({ top: 0, behavior: 'smooth' })
      }
      return
    }
    const rows = Array.from(
      root.querySelectorAll<HTMLElement>(
        `[data-call-kind="${anchor.kind}"]`,
      ),
    )
    const target =
      (anchor.actionId
        ? rows.find((row) => row.dataset.callAction === anchor.actionId)
        : undefined) ??
      rows.find((row) =>
        anchor.sequences.includes(Number(row.dataset.callSequence)),
      )
    if (!target) return
    const frame = window.requestAnimationFrame(() => {
      target.scrollIntoView({ block: 'center', behavior: 'smooth' })
    })
    return () => window.cancelAnimationFrame(frame)
  }, [branch.branchId, anchor])

  const survival =
    branch.speculationStartedAtMs === null
      ? null
      : Math.max(
          0,
          (active
            ? now
            : branch.terminalAtMs ?? branch.lastTimestampMs) -
            branch.speculationStartedAtMs,
        )

  return (
    <div className="branch-details" ref={rootRef}>
      <div className={`branch-banner banner-${branch.role}`}>
        <span className={`badge badge-role-${branch.role}`}>{roleLabel(branch.role)}</span>
        <span
          className={`badge badge-status-${
            typeof branch.status === 'string' ? branch.status : 'blocked'
          }`}
        >
          {statusLabel(branch.status)}
        </span>
        <code title={branch.branchId}>{branch.branchId}</code>
      </div>

      <Section title="基本信息">
        <dl className="detail-grid">
          <Field label="角色">{roleLabel(branch.role)}</Field>
          <Field label="状态">
            {statusLabel(branch.status)}
            {branch.statusReason ? (
              <span className="field-note"> · {branch.statusReason}</span>
            ) : null}
          </Field>
          <Field label="模型">{model ?? <span className="muted">未上报</span>}</Field>
          <Field label="投机来源">{speculationKindLabel(branch.speculationKind)}</Field>
          <Field label="Chance">
            {chance ? <code>{chance.chanceId}</code> : <span className="muted">未上报</span>}
          </Field>
          <Field label="Chance 阶段">
            {chance ? chanceStageLabel(chance.stage) : <span className="muted">未上报</span>}
          </Field>
          <Field label="权威状态">
            {chance ? (
              authorityStateLabel(chance.authorityState)
            ) : (
              <span className="muted">未上报</span>
            )}
          </Field>
          <Field label="投机起点" title="branch_created.speculation_started_at_ms">
            {formatTimestamp(branch.speculationStartedAtMs)}
          </Field>
          <Field
            label="存活时长"
            title="按 Instrumentation 投机起点与观察事件时刻计算，不表示精确执行耗时"
          >
            {survival === null ? (
              <span className="muted">未上报</span>
            ) : (
              formatDuration(survival)
            )}
          </Field>
          <Field label="父 Branch">
            {branch.parentBranchId ? (
              <code>{branch.parentBranchId}</code>
            ) : (
              <span className="muted">无（根分支）</span>
            )}
          </Field>
          <Field label="所属 Chance">
            {branch.chanceId ?? <span className="muted">无（初始权威）</span>}
          </Field>
        </dl>
      </Section>

      <Section title="隔离环境">
        {branch.environment ? (
          <dl className="detail-grid">
            <Field label="Runtime 状态">
              {runtimeStateLabel(branch.environment.state)}
            </Field>
            <Field label="独立 Instance">
              {branch.environment.independentInstance === null ? (
                <span className="muted">未上报</span>
              ) : branch.environment.independentInstance ? (
                '是，独立 Instance'
              ) : (
                '否，共享环境'
              )}
            </Field>
            <Field label="嵌套关系">
              {branch.environment.parentEnvironment ?? (
                <span className="muted">未上报</span>
              )}
            </Field>
            <Field label="配置摘要">
              {Object.keys(branch.environment.configuration).length === 0 ? (
                <span className="muted">未上报</span>
              ) : (
                <pre className="config-summary">
                  {formatJson(branch.environment.configuration)}
                </pre>
              )}
            </Field>
          </dl>
        ) : (
          <p className="muted">未上报环境事实。</p>
        )}
        {branch.environmentCleaned ? (
          <p className={branch.environmentCleaned.succeeded ? 'success-note' : 'error-note'}>
            环境清理{branch.environmentCleaned.succeeded ? '成功' : '失败'}
            {branch.environmentCleaned.reason
              ? `：${branch.environmentCleaned.reason}`
              : ''}
          </p>
        ) : null}
      </Section>

      <Section
        title="用量"
        aside={
          tokens.known ? (
            <span className="section-note">
              已知 token {formatTokens(tokens.value)}
              {tokens.complete ? '' : '（部分）'}
            </span>
          ) : null
        }
      >
        <h4 className="detail-subhead">模型调用</h4>
        {branch.modelUsages.length === 0 ? (
          <p className="muted">暂无模型调用记录。</p>
        ) : (
          <div className="table-scroll">
            <table className="usage-table">
              <thead>
                <tr>
                  <th>动作</th>
                  <th>模型</th>
                  <th>Token</th>
                  <th>费用</th>
                  <th>耗时</th>
                </tr>
              </thead>
              <tbody>
                {branch.modelUsages.map((usage) => (
                  <tr
                    key={usage.sequence}
                    data-call-sequence={usage.sequence}
                    data-call-kind="model"
                    data-call-action={usage.action_id ?? ''}
                    className={
                      highlighted.has(usage.sequence)
                        ? 'usage-row-anchor'
                        : undefined
                    }
                  >
                    <td>
                      {usage.action_id === null ? (
                        <span className="muted">未配对</span>
                      ) : (
                        <code>{usage.action_id}</code>
                      )}
                    </td>
                    <td>{usage.model}</td>
                    <td>
                      <span className="token-main">{modelTokenText(usage)}</span>
                      <span className="token-detail">
                        入 {usage.input_tokens ?? '—'} · 出 {usage.output_tokens ?? '—'}
                        {usage.cache_read_tokens !== null ||
                        usage.cache_write_tokens !== null
                          ? ` · 缓存 ${usage.cache_read_tokens ?? '—'}/${usage.cache_write_tokens ?? '—'}`
                          : ''}
                      </span>
                    </td>
                    <td>{usage.cost === null ? <span className="muted">未上报</span> : formatCost(usage.cost)}</td>
                    <td>
                      {usage.duration_ms === null ? (
                        <span className="muted">未上报</span>
                      ) : (
                        <span title={`${formatTimestamp(usage.started_at_ms)} → ${formatTimestamp(usage.finished_at_ms)}`}>
                          {formatDuration(usage.duration_ms)}
                        </span>
                      )}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}

        <h4 className="detail-subhead">工具调用</h4>
        {branch.tools.length === 0 ? (
          <p className="muted">暂无工具调用记录。</p>
        ) : (
          <div className="table-scroll">
            <table className="usage-table">
              <thead>
                <tr>
                  <th>动作</th>
                  <th>工具</th>
                  <th>状态</th>
                  <th>耗时</th>
                </tr>
              </thead>
              <tbody>
                {branch.tools.map((tool) => (
                  <tr
                    key={tool.sequence}
                    data-call-sequence={tool.sequence}
                    data-call-kind="tool"
                    data-call-action={tool.action_id ?? ''}
                    className={
                      highlighted.has(tool.sequence)
                        ? 'usage-row-anchor'
                        : undefined
                    }
                  >
                    <td>
                      {tool.action_id === null ? (
                        <span className="muted">未配对</span>
                      ) : (
                        <code>{tool.action_id}</code>
                      )}
                    </td>
                    <td>{toolName(tool)}</td>
                    <td>{actionStatusLabel(tool.status)}</td>
                    <td>
                      {tool.duration_ms === null ? (
                        <span className="muted">未上报</span>
                      ) : (
                        <span title={`${formatTimestamp(tool.started_at_ms)} → ${formatTimestamp(tool.finished_at_ms)}`}>
                          {formatDuration(tool.duration_ms)}
                        </span>
                      )}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </Section>

      <Section
        title="对话"
        aside={
          <span className="section-note">
            {roleLabel(branch.role)} · {statusLabel(branch.status)} · 只读
          </span>
        }
      >
        <p className="conversation-note">
          {branch.role === 'candidate'
            ? '候选对话与主对话分开；查看不代表接受，也不会注入其他 Branch 的上下文。'
            : '这是权威窗口的对话；右侧观察不会改变输入路由或接受条件。'}
        </p>
        <ConversationView
          entry={conversation}
          variant="detail"
          emptyText="这个 Branch 尚无消息。"
        />
      </Section>
    </div>
  )
}

import { useEffect, useMemo, useRef, useState } from 'react'
import { sendRequest } from '../api'
import {
  authorityWindows,
  currentAuthority,
  roleLabel,
  statusLabel,
} from '../fold'
import { shortId } from '../format'
import { PanelIcon, SendIcon } from '../icons'
import { useAppStore } from '../store'
import type { BranchRole, BranchStatus } from '../types'
import { useConversation } from '../useConversation'
import { ConversationView } from './ConversationView'

function connectionLabel(connection: string): string {
  switch (connection) {
    case 'idle':
      return '未连接'
    case 'loading':
      return '读取状态…'
    case 'connecting':
      return '正在连接…'
    case 'open':
      return '已连接'
    case 'reconnecting':
      return '正在重连…'
    case 'closed':
      return '控制面已断开'
    case 'error':
      return '连接失败'
    default:
      return connection
  }
}

function BranchBadges({
  role,
  status,
  isCurrent,
}: {
  role: BranchRole
  status: BranchStatus
  isCurrent: boolean
}) {
  if (!isCurrent) {
    return (
      <span className="badges">
        <span className="badge badge-history">历史权威</span>
      </span>
    )
  }
  return (
    <span className="badges">
      <span className={`badge badge-role-${role}`}>{roleLabel(role)}</span>
      <span className={`badge badge-status-${status}`}>{statusLabel(status)}</span>
    </span>
  )
}

export function ChatPane() {
  const state = useAppStore((store) => store.state)
  const selectedTaskId = useAppStore((store) => store.selectedTaskId)
  const explicitAuthority = useAppStore((store) => store.selectedAuthorityBranchId)
  const connection = useAppStore((store) => store.connection)
  const panelOpen = useAppStore((store) => store.panelOpen)
  const setPanelOpen = useAppStore((store) => store.setPanelOpen)
  const selectTask = useAppStore((store) => store.selectTask)
  const selectAuthorityWindow = useAppStore((store) => store.selectAuthorityWindow)
  const error = useAppStore((store) => store.error)
  const dismissError = useAppStore((store) => store.dismissError)
  const resync = useAppStore((store) => store.resync)

  const task = selectedTaskId ? state.tasks[selectedTaskId] ?? null : null
  const current = currentAuthority(task)
  const selected = explicitAuthority
    ? task?.branches[explicitAuthority] ?? current
    : current
  const windows = useMemo(() => authorityWindows(task), [task])
  const conversation = useConversation(selected?.branchId ?? null, Boolean(selected))

  const [prompt, setPrompt] = useState('')
  const [busy, setBusy] = useState(false)
  const [requestError, setRequestError] = useState<string | null>(null)
  const textarea = useRef<HTMLTextAreaElement>(null)

  const connected = connection === 'open'
  const ready = state.runStatus?.ready ?? false
  const isCurrent = Boolean(
    selected && current && selected.branchId === current.branchId,
  )
  const canSend = Boolean(selected && isCurrent && connected && ready && !busy)
  const notice = state.runStatus?.notice ?? ''

  useEffect(() => {
    if (!requestError) return
    const timer = window.setTimeout(() => setRequestError(null), 6000)
    return () => window.clearTimeout(timer)
  }, [requestError])

  async function submit() {
    const text = prompt.trim()
    if (!text || !canSend) return
    setBusy(true)
    setRequestError(null)
    try {
      await sendRequest({ action: 'prompt', prompt: text })
      setPrompt('')
      textarea.current?.focus()
    } catch (failure) {
      setRequestError(failure instanceof Error ? failure.message : String(failure))
    } finally {
      setBusy(false)
    }
  }

  async function interrupt() {
    setRequestError(null)
    try {
      await sendRequest({ action: 'interrupt' })
    } catch (failure) {
      setRequestError(failure instanceof Error ? failure.message : String(failure))
    }
  }

  async function shutdown() {
    if (!window.confirm('结束这次运行？')) return
    setRequestError(null)
    try {
      await sendRequest({ action: 'shutdown' })
    } catch (failure) {
      setRequestError(failure instanceof Error ? failure.message : String(failure))
    }
  }

  return (
    <section className="chat-pane" aria-label="权威对话">
      <header className="chat-header">
        <div className="brand">
          <span className="brand-mark" aria-hidden="true">
            A
          </span>
          <span>
            <strong>AScout</strong>
            <small>执行观察</small>
          </span>
        </div>
        <span className={`connection connection-${connection}`} role="status">
          <span className="connection-dot" aria-hidden="true" />
          {connectionLabel(connection)}
        </span>
        <div className="header-actions">
          <button
            type="button"
            onClick={interrupt}
            disabled={!connected}
            title="请求中断当前执行"
          >
            中断
          </button>
          <button
            type="button"
            onClick={shutdown}
            disabled={!connected}
            title="结束这次运行"
          >
            结束运行
          </button>
          <button
            type="button"
            className={panelOpen ? 'active' : ''}
            aria-expanded={panelOpen}
            aria-controls="speculation-panel"
            title={panelOpen ? '隐藏投机面板' : '显示投机面板'}
            onClick={() => setPanelOpen(!panelOpen)}
          >
            <PanelIcon />
            投机面板
          </button>
        </div>
      </header>

      <div className="window-bar">
        <label className="field">
          <span>任务</span>
          {state.taskOrder.length > 1 ? (
            <select
              value={selectedTaskId ?? ''}
              onChange={(event) => selectTask(event.target.value)}
            >
              {state.taskOrder.map((taskId) => (
                <option key={taskId} value={taskId}>
                  {shortId(taskId)}
                </option>
              ))}
            </select>
          ) : (
            <span className="field-value">{selectedTaskId ? shortId(selectedTaskId) : '等待任务'}</span>
          )}
        </label>
        <label className="field">
          <span>对话窗口</span>
          <select
            value={explicitAuthority ?? ''}
            disabled={windows.length === 0}
            onChange={(event) =>
              selectAuthorityWindow(event.target.value || null)
            }
          >
            <option value="">
              当前权威窗口{current ? ` · ${shortId(current.branchId)}` : ''}
            </option>
            {windows.map((window) => (
              <option key={window.branchId} value={window.branchId}>
                {shortId(window.branchId)} ·{' '}
                {window.branchId === current?.branchId ? '当前权威' : '历史权威'}
              </option>
            ))}
          </select>
        </label>
        {selected ? (
          <div className="window-identity">
            <code title={selected.branchId}>{shortId(selected.branchId)}</code>
            <BranchBadges
              role={selected.role}
              status={selected.status}
              isCurrent={isCurrent}
            />
            {!isCurrent ? <span className="readonly-note">只读</span> : null}
          </div>
        ) : null}
      </div>

      {notice ? (
        <div className="notice" role="status">
          {notice}
        </div>
      ) : null}
      {error ? (
        <div className="notice notice-error" role="alert">
          <span>{error}</span>
          <button
            type="button"
            className="notice-dismiss"
            onClick={() => {
              dismissError()
              void resync()
            }}
          >
            重试
          </button>
          <button type="button" className="notice-dismiss" onClick={dismissError}>
            关闭
          </button>
        </div>
      ) : null}

      {selected ? (
        <ConversationView
          entry={conversation}
          variant="main"
          emptyText={
            isCurrent
              ? '当前权威窗口尚无消息。'
              : '这个历史权威窗口没有可展示的消息。'
          }
        />
      ) : (
        <div className="empty-chat">
          <div className="empty-chat-mark" aria-hidden="true">
            A
          </div>
          <h2>等待权威窗口</h2>
          <p>控制面建立任务后，这里显示当前权威路径的对话。</p>
          <p className="muted">候选内容只出现在右侧投机面板，且始终保持只读。</p>
        </div>
      )}

      <footer className="composer">
        {requestError ? <div className="composer-error">{requestError}</div> : null}
        {!isCurrent && selected ? (
          <div className="composer-hint">历史权威窗口只读；切回当前权威窗口后可继续输入。</div>
        ) : !connected ? (
          <div className="composer-hint">控制面未连接，输入暂不可用。</div>
        ) : !ready ? (
          <div className="composer-hint">等待控制面就绪。</div>
        ) : null}
        <div className="composer-box">
          <textarea
            ref={textarea}
            rows={3}
            value={prompt}
            disabled={!canSend}
            placeholder={
              isCurrent ? '向当前权威路径发送消息' : '历史权威窗口只读'
            }
            aria-label="消息"
            onChange={(event) => setPrompt(event.target.value)}
            onKeyDown={(event) => {
              if (
                event.key === 'Enter' &&
                !event.shiftKey &&
                !event.nativeEvent.isComposing
              ) {
                event.preventDefault()
                void submit()
              }
            }}
          />
          <button
            type="button"
            className="send-button"
            disabled={!canSend || !prompt.trim()}
            onClick={() => void submit()}
            aria-label="发送消息"
          >
            <SendIcon />
            {busy ? '发送中…' : '发送'}
          </button>
        </div>
        <div className="composer-foot">
          <span>Enter 发送 · Shift+Enter 换行</span>
          <span>只向当前权威路径发送，不会注入候选窗口。</span>
        </div>
      </footer>
    </section>
  )
}


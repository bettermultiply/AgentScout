import { useEffect, useRef } from 'react'
import { formatJson } from '../format'
import type { ConversationMessage } from '../types'

function roleLabel(role: string): string {
  switch (role) {
    case 'user':
      return '你'
    case 'assistant':
      return 'Agent'
    case 'toolResult':
      return '工具结果'
    case 'system':
      return '系统'
    default:
      return role || '消息'
  }
}

function partText(part: unknown): string {
  if (typeof part === 'string') return part
  if (part && typeof part === 'object' && 'text' in part) {
    const text = (part as { text?: unknown }).text
    return typeof text === 'string' ? text : ''
  }
  return ''
}

function MessageParts({ content }: { content: unknown }) {
  if (typeof content === 'string') {
    return <div className="message-text">{content}</div>
  }
  if (!Array.isArray(content)) {
    return (
      <details className="message-detail">
        <summary>未展示的内容</summary>
        <pre>{formatJson(content)}</pre>
      </details>
    )
  }
  if (content.length === 0) {
    return <div className="muted">（空内容）</div>
  }
  return (
    <>
      {content.map((part, index) => {
        const type =
          part && typeof part === 'object' && 'type' in part
            ? String((part as { type?: unknown }).type)
            : ''
        if (type === 'text') {
          const text = partText(part)
          return (
            <div className="message-text" key={index}>
              {text || <span className="muted">（空文本）</span>}
            </div>
          )
        }
        if (type === 'thinking') {
          const thinking =
            part && typeof part === 'object' && 'thinking' in part
              ? String((part as { thinking?: unknown }).thinking ?? '')
              : ''
          return (
            <details className="message-detail" key={index}>
              <summary>思考内容</summary>
              <pre>{thinking}</pre>
            </details>
          )
        }
        if (type === 'toolCall') {
          const call = part as {
            name?: unknown
            arguments?: unknown
          }
          return (
            <details className="message-detail" key={index}>
              <summary>
                工具调用
                {typeof call.name === 'string' ? ` · ${call.name}` : ''}
              </summary>
              <pre>{formatJson(call.arguments)}</pre>
            </details>
          )
        }
        return (
          <details className="message-detail" key={index}>
            <summary>未展示的内容{type ? ` · ${type}` : ''}</summary>
            <pre>{formatJson(part)}</pre>
          </details>
        )
      })}
    </>
  )
}

export function MessageView({ message }: { message: ConversationMessage }) {
  const role = typeof message.role === 'string' ? message.role : ''
  const toolName =
    typeof message.toolName === 'string' ? message.toolName : null
  const errorMessage =
    typeof message.errorMessage === 'string' ? message.errorMessage : null
  return (
    <article className={`message message-${role || 'unknown'}`}>
      <header className="message-head">
        <strong>{roleLabel(role)}</strong>
        {toolName ? <span className="message-tool">{toolName}</span> : null}
      </header>
      <MessageParts content={message.content} />
      {errorMessage ? (
        <div className="message-error">{errorMessage}</div>
      ) : null}
    </article>
  )
}

export function useStickyScroll(
  dependency: unknown,
  enabled: boolean,
): React.RefObject<HTMLDivElement | null> {
  const ref = useRef<HTMLDivElement>(null)
  const stick = useRef(true)

  useEffect(() => {
    const element = ref.current
    if (!element) return
    const onScroll = () => {
      stick.current =
        element.scrollHeight - element.scrollTop - element.clientHeight < 90
    }
    element.addEventListener('scroll', onScroll, { passive: true })
    return () => element.removeEventListener('scroll', onScroll)
  }, [])

  useEffect(() => {
    const element = ref.current
    if (!element || !enabled || !stick.current) return
    element.scrollTop = element.scrollHeight
  }, [dependency, enabled])

  return ref
}

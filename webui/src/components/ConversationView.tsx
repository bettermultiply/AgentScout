import type { ConversationMessage } from '../types'
import type { ConversationEntry } from '../useConversation'
import { MessageView, useStickyScroll } from './MessageView'

function stableMessageKey(message: ConversationMessage, index: number): string {
  const value = message.id ?? message.timestamp ?? message.ts ?? message.message_id
  return typeof value === 'string' || typeof value === 'number'
    ? String(value)
    : `${index}:${JSON.stringify(message)}`
}

interface Props {
  entry: ConversationEntry
  variant: 'main' | 'detail'
  emptyText: string
  pendingMessages?: ConversationMessage[]
}

export function ConversationView({
  entry,
  variant,
  emptyText,
  pendingMessages = [],
}: Props) {
  const data = entry.data
  const scrollRef = useStickyScroll(
    `${data?.inherited.length ?? 0}:${data?.messages.length ?? 0}:${data?.streaming.length ?? 0}:${entry.status}`,
    variant === 'main',
  )

  if (entry.status === 'loading' && !data) {
    return (
      <div className={variant === 'main' ? 'conversation-scroll' : ''}>
        <div className="skeleton-stack" aria-label="正在读取对话">
          <span />
          <span />
          <span />
        </div>
        {pendingMessages.map((message, index) => (
          <MessageView key={`pending-loading-${index}`} message={message} />
        ))}
      </div>
    )
  }

  if (entry.status === 'error' && !data) {
    return (
      <div className={variant === 'main' ? 'conversation-scroll' : ''}>
        <div className="inline-error">{entry.error ?? '读取对话失败'}</div>
      </div>
    )
  }

  if (!data) {
    return (
      <div className={variant === 'main' ? 'conversation-scroll' : ''}>
        <div className="muted conversation-empty">{emptyText}</div>
        {pendingMessages.map((message, index) => (
          <MessageView key={`pending-empty-${index}`} message={message} />
        ))}
      </div>
    )
  }

  const hasContent =
    data.inherited.length > 0 || data.messages.length > 0 || Boolean(data.streaming) || pendingMessages.length > 0

  return (
    <div
      ref={scrollRef}
      className={variant === 'main' ? 'conversation-scroll' : 'conversation-inline'}
    >
      {entry.status === 'refreshing' ? (
        <div className="refreshing" role="status">
          更新中…
        </div>
      ) : null}
      {entry.status === 'error' ? (
        <div className="inline-error">{entry.error ?? '读取对话失败'}</div>
      ) : null}

      {data.inherited.map((message, index) => (
        <MessageView key={`inherited-${stableMessageKey(message, index)}`} message={message} />
      ))}

      {data.messages.map((message, index) => (
        <MessageView key={`message-${stableMessageKey(message, index)}`} message={message} />
      ))}

      {pendingMessages.map((message, index) => (
        <MessageView key={`pending-${index}`} message={message} />
      ))}

      {data.streaming ? (
        <div className="streaming">
          <pre>{data.streaming}</pre>
          <span className="streaming-caret" aria-hidden="true" />
        </div>
      ) : null}

      {!hasContent ? <div className="muted conversation-empty">{emptyText}</div> : null}
    </div>
  )
}

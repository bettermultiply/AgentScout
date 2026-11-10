import type { ConversationEntry } from '../useConversation'
import { MessageView, useStickyScroll } from './MessageView'

interface Props {
  entry: ConversationEntry
  variant: 'main' | 'detail'
  emptyText: string
}

export function ConversationView({ entry, variant, emptyText }: Props) {
  const data = entry.data
  const scrollRef = useStickyScroll(
    `${data?.messages.length ?? 0}:${data?.streaming.length ?? 0}:${entry.status}`,
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
      </div>
    )
  }

  const hasContent =
    data.inherited.length > 0 || data.messages.length > 0 || Boolean(data.streaming)

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

      {data.inherited.length > 0 ? (
        <details className="inherited">
          <summary>继承上下文 · {data.inherited.length} 条消息</summary>
          <div className="inherited-body">
            {data.inherited.map((message, index) => (
              <MessageView key={index} message={message} />
            ))}
          </div>
        </details>
      ) : null}

      {data.inherited.length > 0 ? (
        <div className="conversation-divider">
          <span>本 Branch 独有内容</span>
        </div>
      ) : null}

      {data.messages.map((message, index) => (
        <MessageView key={index} message={message} />
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

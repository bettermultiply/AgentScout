import { useEffect, useRef, useState } from 'react'
import { fetchConversation } from './api'
import { authorityWindows, currentAuthority, type TaskState } from './fold'
import type { ConversationMessage } from './types'
import type { ConversationEntry } from './useConversation'
import { useAppStore } from './store'

function messageIdentity(message: ConversationMessage, index: number): string {
  const value = message.id ?? message.timestamp ?? message.ts ?? message.message_id
  return typeof value === 'string' || typeof value === 'number'
    ? String(value)
    : `${index}:${JSON.stringify(message)}`
}

function appendStable(
  previous: ConversationMessage[],
  next: ConversationMessage[],
): ConversationMessage[] {
  const previousById = new Map(previous.map((message, index) => [messageIdentity(message, index), message]))
  return next.map((message, index) => previousById.get(messageIdentity(message, index)) ?? message)
}

export function useAuthorityConversation(task: TaskState | null): ConversationEntry {
  const versions = useAppStore((state) => state.conversationVersions)
  const ids = authorityWindows(task).map((branch) => branch.branchId)
  const current = currentAuthority(task)
  const [data, setData] = useState<ConversationEntry['data']>(null)
  const [status, setStatus] = useState<ConversationEntry['status']>('idle')
  const [error, setError] = useState<string | null>(null)
  const previous = useRef<ConversationEntry['data']>(null)
  useEffect(() => {
    if (!current || ids.length === 0) { setData(null); setStatus('idle'); return }
    let cancelled = false
    setStatus(data ? 'refreshing' : 'loading')
    Promise.all(ids.map((id) => fetchConversation(id))).then((conversations) => {
      if (cancelled) return
      const inherited: ConversationMessage[] = []
      const messages: ConversationMessage[] = []
      let streaming = ''
      conversations.forEach((conversation, branchIndex) => {
        if (branchIndex === 0) inherited.push(...conversation.inherited)
        messages.push(...conversation.messages)
        if (branchIndex === conversations.length - 1) streaming = conversation.streaming
      })
      const stablePrevious = previous.current
      const next = {
        inherited: appendStable(stablePrevious?.inherited ?? [], inherited),
        messages: appendStable(stablePrevious?.messages ?? [], messages),
        streaming,
        role: 'authority' as const,
        status: current.status,
      }
      previous.current = next
      setData(next)
      setStatus('ready'); setError(null)
    }).catch((failure: unknown) => {
      if (!cancelled) { setStatus('error'); setError(failure instanceof Error ? failure.message : String(failure)) }
    })
    return () => { cancelled = true }
    // Only observation versions invalidate the transcript.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [ids.join('|'), current?.branchId, ids.map((id) => versions[id] ?? 0).join('|')])
  return { branchId: current?.branchId ?? null, status, data, error }
}

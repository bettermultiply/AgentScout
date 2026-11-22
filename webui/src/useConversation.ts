import { useEffect, useState } from 'react'
import { fetchConversation } from './api'
import { useAppStore } from './store'
import type { Conversation } from './types'

export type ConversationStatus = 'idle' | 'loading' | 'refreshing' | 'ready' | 'error'

export interface ConversationEntry {
  branchId: string | null
  status: ConversationStatus
  data: Conversation | null
  error: string | null
}

const idle: ConversationEntry = {
  branchId: null,
  status: 'idle',
  data: null,
  error: null,
}

/** Debounce updates: a streamed chunk can trigger its own conversation_updated event. */
export function useConversation(
  branchId: string | null,
  enabled: boolean,
): ConversationEntry {
  const version = useAppStore((state) =>
    branchId ? state.conversationVersions[branchId] ?? 0 : 0,
  )
  const [entry, setEntry] = useState<ConversationEntry>(idle)

  useEffect(() => {
    if (!branchId || !enabled) {
      setEntry(idle)
      return
    }
    let cancelled = false
    setEntry((previous) =>
      previous.branchId === branchId && previous.data
        ? { ...previous, status: 'refreshing', error: null }
        : { branchId, status: 'loading', data: null, error: null },
    )
    const timer = window.setTimeout(() => {
      fetchConversation(branchId)
        .then((data) => {
          if (cancelled) return
          setEntry({ branchId, status: 'ready', data, error: null })
        })
        .catch((error: unknown) => {
          if (cancelled) return
          setEntry((previous) => ({
            branchId,
            status: 'error',
            data: previous.branchId === branchId ? previous.data : null,
            error: error instanceof Error ? error.message : String(error),
          }))
        })
    }, 140)
    return () => {
      cancelled = true
      window.clearTimeout(timer)
    }
  }, [branchId, enabled, version])

  return entry
}

export function useNow(active: boolean): number {
  const [now, setNow] = useState(() => Date.now())
  useEffect(() => {
    if (!active) return
    setNow(Date.now())
    const timer = window.setInterval(() => setNow(Date.now()), 1000)
    return () => window.clearInterval(timer)
  }, [active])
  return now
}

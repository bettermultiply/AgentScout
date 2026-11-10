import type { Conversation, Request, StateEventSet } from './types'

async function readError(response: Response): Promise<string> {
  const text = (await response.text()).trim()
  return text || `HTTP ${response.status}`
}

export async function fetchStateEventSet(): Promise<StateEventSet> {
  const response = await fetch('/api/observation/state-event-set', {
    headers: { Accept: 'application/json' },
  })
  if (!response.ok) throw new Error(`读取状态事件集失败：${await readError(response)}`)
  return (await response.json()) as StateEventSet
}

export async function fetchConversation(branchId: string): Promise<Conversation> {
  const response = await fetch(
    `/api/branches/${encodeURIComponent(branchId)}/conversation`,
    { headers: { Accept: 'application/json' } },
  )
  if (!response.ok) throw new Error(`读取对话失败：${await readError(response)}`)
  return (await response.json()) as Conversation
}

export async function sendRequest(request: Request): Promise<void> {
  const response = await fetch('/api/request', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(request),
  })
  if (!response.ok) throw new Error(await readError(response))
}

export function eventStreamUrl(after: number): string {
  return `/api/observation/events?after=${encodeURIComponent(String(after))}`
}

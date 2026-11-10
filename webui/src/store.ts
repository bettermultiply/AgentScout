import { create } from 'zustand'
import { eventStreamUrl, fetchStateEventSet } from './api'
import { applyEvent, emptyRunState, foldEvents } from './fold'
import type { RunState } from './fold'
import type { ObservationEvent } from './types'

export type ConnectionState =
  | 'idle'
  | 'loading'
  | 'connecting'
  | 'open'
  | 'reconnecting'
  | 'closed'
  | 'error'

interface AppStore {
  state: RunState
  eventCount: number
  connection: ConnectionState
  error: string | null
  selectedTaskId: string | null
  /** null means "follow the current authority window". */
  selectedAuthorityBranchId: string | null
  panelOpen: boolean
  panelView: 'tree' | 'detail'
  selectedBranchId: string | null
  conversationVersions: Record<string, number>
  ingestEvent: (event: ObservationEvent) => void
  start: () => Promise<void>
  stop: () => void
  resync: () => Promise<void>
  selectTask: (taskId: string) => void
  selectAuthorityWindow: (branchId: string | null) => void
  setPanelOpen: (open: boolean) => void
  openBranch: (branchId: string) => void
  showTree: () => void
  dismissError: () => void
}

// Kept outside the store so long event histories do not require a new array on
// every append. `replayState` is the pure entry point for a future replay UI.
let eventLog: ObservationEvent[] = []
let source: EventSource | null = null
let generation = 0
let started = false

export function getEventLog(): readonly ObservationEvent[] {
  return eventLog
}

/** Replay helper: the state after events 1..sequence. */
export function replayState(sequence: number): RunState {
  return foldEvents(eventLog, sequence)
}

function readPanelOpen(): boolean {
  try {
    const stored = window.localStorage.getItem('ascout.panel')
    if (stored !== null) return stored !== 'closed'
  } catch {
    // Storage can be unavailable; fall through to the viewport default.
  }
  return !window.matchMedia('(max-width: 1020px)').matches
}

function closeSource(): void {
  source?.close()
  source = null
}

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}

function pickTask(state: RunState, current: string | null): string | null {
  if (current && state.tasks[current]) return current
  return state.taskOrder.at(-1) ?? null
}

export const useAppStore = create<AppStore>((set, get) => {
  function openSource(after: number, currentGeneration: number): void {
    closeSource()
    const stream = new EventSource(eventStreamUrl(after))
    source = stream
    let errors = 0

    stream.onopen = () => {
      if (currentGeneration !== generation) return
      errors = 0
      set({ connection: 'open', error: null })
    }

    stream.onmessage = (message) => {
      if (currentGeneration !== generation) return
      let event: ObservationEvent
      try {
        event = JSON.parse(message.data) as ObservationEvent
      } catch {
        set({ error: '收到无法解析的观察事件' })
        return
      }
      const last = get().state.sequence
      if (event.sequence <= last) return
      if (event.sequence !== last + 1) {
        void get().resync()
        return
      }
      get().ingestEvent(event)
      if (event.event === 'run_status' && !event.connected) {
        closeSource()
        set({ connection: 'closed' })
      }
    }

    stream.onerror = () => {
      if (currentGeneration !== generation) return
      errors += 1
      if (errors > 3) {
        void get().resync()
        return
      }
      set({ connection: 'reconnecting' })
    }
  }

  function loadStateEventSet(events: ObservationEvent[]): void {
    eventLog = events.slice()
    const state = foldEvents(eventLog)
    const selectedTaskId = pickTask(state, get().selectedTaskId)
    const task = selectedTaskId ? state.tasks[selectedTaskId] : null
    const explicit = get().selectedAuthorityBranchId
    const selectedAuthorityBranchId =
      explicit && task?.branches[explicit] ? explicit : null
    set({
      state,
      eventCount: eventLog.length,
      selectedTaskId,
      selectedAuthorityBranchId,
    })
  }

  return {
    state: emptyRunState(),
    eventCount: 0,
    connection: 'idle',
    error: null,
    selectedTaskId: null,
    selectedAuthorityBranchId: null,
    panelOpen: readPanelOpen(),
    panelView: 'tree',
    selectedBranchId: null,
    conversationVersions: {},

    async start() {
      if (started) return
      started = true
      await get().resync()
    },

    stop() {
      started = false
      generation += 1
      closeSource()
      set({ connection: 'idle' })
    },

    async resync() {
      const currentGeneration = ++generation
      closeSource()
      set({ connection: 'loading', error: null })
      try {
        const stateEventSet = await fetchStateEventSet()
        if (currentGeneration !== generation) return
        loadStateEventSet(stateEventSet.events)
        set({ connection: 'connecting' })
        openSource(stateEventSet.sequence, currentGeneration)
      } catch (error) {
        if (currentGeneration !== generation) return
        set({ connection: 'error', error: errorText(error) })
      }
    },

    ingestEvent(event) {
      eventLog.push(event)
      const state = applyEvent(get().state, event)
      const selectedTaskId = pickTask(state, get().selectedTaskId)
      const task = selectedTaskId ? state.tasks[selectedTaskId] : null
      const explicit = get().selectedAuthorityBranchId
      const selectedAuthorityBranchId =
        explicit && task?.branches[explicit] ? explicit : null

      const conversationVersions = { ...get().conversationVersions }
      if (event.event === 'conversation_updated') {
        conversationVersions[event.branch_id] =
          (conversationVersions[event.branch_id] ?? 0) + 1
      }
      if (event.event === 'authority_changed') {
        conversationVersions[event.branch_id] =
          (conversationVersions[event.branch_id] ?? 0) + 1
        if (event.previous_authority) {
          conversationVersions[event.previous_authority] =
            (conversationVersions[event.previous_authority] ?? 0) + 1
        }
      }

      set({
        state,
        eventCount: eventLog.length,
        selectedTaskId,
        selectedAuthorityBranchId,
        conversationVersions,
      })
    },

    selectTask(taskId) {
      set({ selectedTaskId: taskId, selectedAuthorityBranchId: null })
    },

    selectAuthorityWindow(branchId) {
      set({ selectedAuthorityBranchId: branchId })
    },

    setPanelOpen(open) {
      try {
        window.localStorage.setItem('ascout.panel', open ? 'open' : 'closed')
      } catch {
        // Storage can be unavailable in private browsing; the panel still works.
      }
      set(
        open
          ? { panelOpen: true }
          : { panelOpen: false, panelView: 'tree', selectedBranchId: null },
      )
    },

    openBranch(branchId) {
      try {
        window.localStorage.setItem('ascout.panel', 'open')
      } catch {
        // Ignore unavailable storage.
      }
      set({
        selectedBranchId: branchId,
        panelView: 'detail',
        panelOpen: true,
      })
    },

    showTree() {
      set({ panelView: 'tree', selectedBranchId: null })
    },

    dismissError() {
      set({ error: null })
    },
  }
})

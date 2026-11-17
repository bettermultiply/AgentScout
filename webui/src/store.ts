import { create } from 'zustand'
import { eventStreamUrl, fetchStateEventSet } from './api'
import { applyEvent, emptyRunState, foldEvents } from './fold'
import type { RunState } from './fold'
import type { ObservationEvent, UiPromptEvent, CommandsListedEvent, UiNotificationEvent } from './types'
import { buildTree, type TreeGraph } from './components/treeGraph'
import { cachedLayoutTree, clearCachedTreeLayout } from './tree/geometry'
import { DEFAULT_TUNING, normalizeTuning, type TreeTuning } from './tree/tuning'

export type ConnectionState =
  | 'idle'
  | 'loading'
  | 'connecting'
  | 'open'
  | 'reconnecting'
  | 'closed'
  | 'error'

/**
 * Where to land inside the Branch detail after clicking a call node.
 * `sequences` are the event sequences of the clicked group; the detail uses
 * them to scroll and highlight, and `actionId` is the anchor when present.
 */
export interface DetailAnchor {
  kind: 'tool' | 'model'
  actionId: string | null
  sequences: number[]
}

interface AppStore {
  uiPrompts: Record<string, UiPromptEvent>
  agentCommands: CommandsListedEvent | null
  notification: UiNotificationEvent | null
  treeGraph: TreeGraph
  treeTuning: TreeTuning
  setTreeTuning: (tuning: TreeTuning) => void
  state: RunState
  connection: ConnectionState
  error: string | null
  selectedTaskId: string | null
  /** null means "follow the current authority window". */
  selectedAuthorityBranchId: string | null
  panelOpen: boolean
  panelView: 'tree' | 'detail'
  selectedBranchId: string | null
  detailAnchor: DetailAnchor | null
  conversationVersions: Record<string, number>
  ingestEvent: (event: ObservationEvent) => void
  start: () => Promise<void>
  stop: () => void
  resync: () => Promise<void>
  selectTask: (taskId: string) => void
  selectAuthorityWindow: (branchId: string | null) => void
  setPanelOpen: (open: boolean) => void
  openBranch: (branchId: string, anchor?: DetailAnchor | null) => void
  showTree: () => void
  dismissError: () => void
}

let source: EventSource | null = null
let generation = 0
let started = false

function foldInteraction(state: Pick<AppStore, 'uiPrompts' | 'agentCommands' | 'notification'>, event: ObservationEvent) {
  let { uiPrompts, agentCommands, notification } = state
  if (event.event === 'ui_prompt_requested') uiPrompts = { ...uiPrompts, [event.prompt_id]: event }
  if (event.event === 'ui_prompt_resolved') {
    uiPrompts = { ...uiPrompts }
    delete uiPrompts[event.prompt_id]
  }
  if (event.event === 'commands_listed') agentCommands = event
  if (event.event === 'ui_notification') notification = event
  if (event.event === 'authority_changed') uiPrompts = Object.fromEntries(Object.entries(uiPrompts).filter(([, prompt]) => prompt.branch_id === event.branch_id))
  if (event.event === 'run_status' && !event.connected) uiPrompts = {}
  return { uiPrompts, agentCommands, notification }
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
  function graphFor(state: RunState, taskId: string | null, tuning = get().treeTuning) {
    const task = taskId ? state.tasks[taskId] ?? null : null
    if (!task) clearCachedTreeLayout()
    return buildTree(task, tuning, cachedLayoutTree)
  }
  function openSource(after: number, currentGeneration: number): void {
    closeSource()
    const stream = new EventSource(eventStreamUrl(after))
    source = stream
    let errors = 0

    stream.onopen = () => {
      if (currentGeneration !== generation) return
      if (errors > 0) { void get().resync(); return }
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
      set({ connection: 'reconnecting', uiPrompts: {} })
    }
  }

  function loadStateEventSet(events: ObservationEvent[]): void {
    let interaction = { uiPrompts: {} as Record<string, UiPromptEvent>, agentCommands: null as CommandsListedEvent | null, notification: null as UiNotificationEvent | null }
    for (const event of events) interaction = foldInteraction(interaction, event)
    const state = foldEvents(events)
    const selectedTaskId = pickTask(state, get().selectedTaskId)
    const task = selectedTaskId ? state.tasks[selectedTaskId] : null
    const explicit = get().selectedAuthorityBranchId
    const selectedAuthorityBranchId =
      explicit && task?.branches[explicit] ? explicit : null
    set({
      ...interaction,
      state,
      selectedTaskId,
      selectedAuthorityBranchId,
      treeGraph: graphFor(state, selectedTaskId),
    })
  }

  return {
    uiPrompts: {}, agentCommands: null, notification: null,
    treeGraph: { nodes: [], latestNodeId: null, error: null },
    treeTuning: (() => {
      try {
        const stored = window.localStorage.getItem('ascout-tree-tuning')
        return stored ? normalizeTuning(JSON.parse(stored)) : DEFAULT_TUNING
      } catch { return DEFAULT_TUNING }
    })(),
    setTreeTuning(tuning) {
      set({ treeTuning: tuning, treeGraph: graphFor(get().state, get().selectedTaskId, tuning) })
    },
    state: emptyRunState(),
    connection: 'idle',
    error: null,
    selectedTaskId: null,
    selectedAuthorityBranchId: null,
    panelOpen: readPanelOpen(),
    panelView: 'tree',
    selectedBranchId: null,
    detailAnchor: null,
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
      clearCachedTreeLayout()
      set({ treeGraph: { nodes: [], latestNodeId: null, error: null } })
      set({ connection: 'idle', uiPrompts: {} })
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
        ...foldInteraction(get(), event),
        state,
        selectedTaskId,
        selectedAuthorityBranchId,
        conversationVersions,
        treeGraph: graphFor(state, selectedTaskId),
      })
    },

    selectTask(taskId) {
      set({ selectedTaskId: taskId, selectedAuthorityBranchId: null, treeGraph: graphFor(get().state, taskId) })
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
          : {
              panelOpen: false,
              panelView: 'tree',
              selectedBranchId: null,
              detailAnchor: null,
            },
      )
    },

    openBranch(branchId, anchor = null) {
      try {
        window.localStorage.setItem('ascout.panel', 'open')
      } catch {
        // Ignore unavailable storage.
      }
      set({
        selectedBranchId: branchId,
        panelView: 'detail',
        panelOpen: true,
        detailAnchor: anchor,
      })
    },

    showTree() {
      set({ panelView: 'tree', selectedBranchId: null, detailAnchor: null })
    },

    dismissError() {
      set({ error: null })
    },
  }
})

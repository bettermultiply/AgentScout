/** Wire types for the control plane's read-only observation API. */

export type BranchRole = 'authority' | 'candidate'

export type BranchStatus =
  | 'created'
  | 'running'
  | 'awaiting_validation'
  | 'promoting'
  | 'promoted'
  | 'discarding'
  | 'discarded'
  | 'cleaning'
  | 'cleaned'

export type ActionKind =
  | 'model_inference'
  | 'tool_execution'
  | 'start_subagent'
  | 'send_message'
  | 'wait'
  | 'finish'
  | 'other'

export type ActionStatus = 'succeeded' | 'failed' | 'cancelled' | 'timed_out'

export type RuntimeExecutionState =
  | 'unknown'
  | 'environment_requested'
  | 'ready'
  | 'promotion_requested'
  | 'promoted'
  | 'termination_requested'
  | 'terminated'
  | 'cleaned'
  | 'failed'

/**
 * One settlement dimension. Measured and estimated values are separate
 * accumulators; the UI never merges them into one number.
 */
export interface ObservationAmount {
  measured: number | null
  estimated: number | null
}

interface EventBase {
  sequence: number
  timestamp_ms: number
  task_id: string
}

export interface BranchCreatedEvent extends EventBase {
  event: 'branch_created'
  branch_id: string
  parent_branch_id: string | null
  role: BranchRole
  edge_id: string | null
  speculation_kind: ActionKind | null
  speculation_started_at_ms: number | null
}

export interface AuthorityChangedEvent extends EventBase {
  event: 'authority_changed'
  branch_id: string
  previous_authority: string | null
}

export interface StatusChangedEvent extends EventBase {
  event: 'status_changed'
  branch_id: string
  status: BranchStatus
  reason: string | null
}

export interface EnvironmentChangedEvent extends EventBase {
  event: 'environment_changed'
  branch_id: string
  state: RuntimeExecutionState
  independent_instance: boolean | null
  parent_environment: string | null
  configuration: Record<string, unknown>
}

export interface EnvironmentCleanedEvent extends EventBase {
  event: 'environment_cleaned'
  branch_id: string
  succeeded: boolean
  reason: string | null
}

export interface ConversationUpdatedEvent extends EventBase {
  event: 'conversation_updated'
  branch_id: string
}

export interface ModelUsageEvent extends EventBase {
  event: 'model_usage'
  branch_id: string
  action_id: string
  model: string
  input_tokens: number | null
  output_tokens: number | null
  cache_read_tokens: number | null
  cache_write_tokens: number | null
  total_tokens: number | null
  cost: number | null
  started_at_ms: number | null
  finished_at_ms: number | null
  duration_ms: number | null
}

export interface ToolFinishedEvent extends EventBase {
  event: 'tool_finished'
  branch_id: string
  action_id: string
  status: ActionStatus
  started_at_ms: number | null
  finished_at_ms: number | null
  duration_ms: number | null
}

export interface SettlementEvent extends EventBase {
  event: 'settlement'
  branch_id: string
  hit: boolean
  time_ms: ObservationAmount
  money: ObservationAmount
  currency: string | null
}

export interface RunStatusEvent extends EventBase {
  event: 'run_status'
  ready: boolean
  running: boolean
  notice: string
  connected: boolean
}

export type BranchObservationEvent =
  | BranchCreatedEvent
  | AuthorityChangedEvent
  | StatusChangedEvent
  | EnvironmentChangedEvent
  | EnvironmentCleanedEvent
  | ConversationUpdatedEvent
  | ModelUsageEvent
  | ToolFinishedEvent
  | SettlementEvent

export type ObservationEvent = BranchObservationEvent | RunStatusEvent

export interface StateEventSet {
  sequence: number
  events: ObservationEvent[]
}

export interface RunStatus {
  ready: boolean
  running: boolean
  notice: string
  connected: boolean
}

/** Pi transcript entries are product-shaped JSON; the UI renders known parts. */
export type ConversationMessage = Record<string, unknown>

export interface Conversation {
  inherited: ConversationMessage[]
  messages: ConversationMessage[]
  streaming: string
  role: BranchRole
  status: BranchStatus
}

export type Request =
  | { action: 'prompt'; prompt: string }
  | { action: 'interrupt' }
  | { action: 'shutdown' }

export function hasBranchId(event: ObservationEvent): event is BranchObservationEvent {
  return event.event !== 'run_status'
}

/** Wire types for the control plane's read-only observation API. */

export type BranchRole = 'authority' | 'candidate'

export type BlockedReason = 'extent_reached' | 'rebalanced' | 'effect_gate'

export type BranchStatus =
  | 'created'
  | 'running'
  | { blocked: BlockedReason }
  | 'finished'
  | 'promoting'
  | 'promoted'
  | 'discarding'
  | 'discarded'
  | 'cleaning'
  | 'cleaned'

export type DiscardedReason =
  | 'match_rejected'
  | 'not_selected'
  | 'ancestor_invalidated'
  | 'budget_exhausted'
  | 'failed'

export type ChanceStage =
  | 'pending'
  | 'running'
  | 'resolving'
  | { accepted: string }
  | { discarded: DiscardedReason }

export type AuthorityState =
  | 'not_requested'
  | 'fetching'
  | 'arrived'
  | 'unavailable'

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

export interface UiPromptEvent extends EventBase {
  event: 'ui_prompt_requested'
  branch_id: string
  prompt_id: string
  kind: 'confirm' | 'select' | 'input' | 'editor'
  title: string
  message_text: string | null
  options: string[]
  placeholder: string | null
  prefill: string | null
}
export interface UiPromptResolvedEvent extends EventBase {
  event: 'ui_prompt_resolved'
  branch_id: string
  prompt_id: string
  cancelled: boolean
  reason: string | null
}
export interface AgentSlashCommand {
  name: string
  description?: string
  source: string
  sourceInfo: unknown
}
export interface CommandsListedEvent extends EventBase {
  event: 'commands_listed'
  branch_id: string
  request_id: string
  commands: AgentSlashCommand[]
}
export interface UiNotificationEvent extends EventBase {
  event: 'ui_notification'
  branch_id: string
  text: string
  level: string
}

export interface BranchCreatedEvent extends EventBase {
  event: 'branch_created'
  branch_id: string
  parent_branch_id: string | null
  role: BranchRole
  chance_id: string | null
  speculation_kind: ActionKind | null
  speculation_started_at_ms: number | null
}

export interface ChanceCreatedEvent extends EventBase {
  event: 'chance_created'
  chance_id: string
  parent_branch_id: string
  parent_chance_id: string | null
  operation_id: string
  tool_name: string
  stage: ChanceStage
  authority_state: AuthorityState
  speculation_started_at_ms: number | null
}

export interface ChanceStageChangedEvent extends EventBase {
  event: 'chance_stage_changed'
  chance_id: string
  stage: ChanceStage
  reason: string | null
  validation_id: number | null
}

export interface ProposalValidatedEvent extends EventBase {
  event: 'proposal_validated'
  chance_id: string
  proposal_id: string
  validation_id: number
  validated_at_ms: number | null
}

export interface AuthorityStateChangedEvent extends EventBase {
  event: 'authority_state_changed'
  chance_id: string
  state: AuthorityState
}

export interface ProposalAddedEvent extends EventBase {
  event: 'proposal_added'
  chance_id: string
  proposal_id: string
  payload: unknown
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
  action_id: string | null
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
  /** 接入侧无法配对时仍上报结果，此时为 null，界面不伪造 id。 */
  action_id: string | null
  /** 来自同一 action 的 ActionStarted.name；没有 ActionStarted 时为 null。 */
  tool_name: string | null
  status: ActionStatus
  started_at_ms: number | null
  finished_at_ms: number | null
  duration_ms: number | null
}

export interface NleMapActionEvent extends EventBase {
  event: 'nle_map_action'
  branch_id: string
  action_id: string
  action: number
  before: NlePublicObservation
  after: NlePublicObservation
}

export interface NlePublicObservation {
  chars: string[]
  blstats: number[]
  message?: string
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
  | NleMapActionEvent
  | SettlementEvent

export type ChanceObservationEvent =
  | ChanceCreatedEvent
  | ChanceStageChangedEvent
  | AuthorityStateChangedEvent
  | ProposalAddedEvent
  | ProposalValidatedEvent

export type ObservationEvent =
  | UiPromptEvent
  | UiPromptResolvedEvent
  | CommandsListedEvent
  | UiNotificationEvent
  | BranchObservationEvent
  | ChanceObservationEvent
  | RunStatusEvent

export interface ObservedChance {
  chance_id: string
  parent_branch_id: string
  parent_chance_id: string | null
  operation_id: string
  tool_name: string
  stage: ChanceStage
  authority_state: AuthorityState
  speculation_started_at_ms: number | null
  validation_id: number | null
  proposal_ids: string[]
  branch_ids: string[]
}

export interface StateEventSet {
  sequence: number
  chances: ObservedChance[]
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
  | { action: 'ui_prompt_answer'; prompt_id: string; answer: unknown; cancelled?: boolean }
  | { action: 'get_commands'; request_id: string }
  | { action: 'model'; model: string }
  | { action: 'prompt'; prompt: string }
  | { action: 'interrupt' }
  | { action: 'pause' }
  | { action: 'continue' }
  | { action: 'shutdown' }

export function isChanceEvent(event: ObservationEvent): event is ChanceObservationEvent {
  return (
    event.event === 'chance_created' ||
    event.event === 'chance_stage_changed' ||
    event.event === 'authority_state_changed' ||
    event.event === 'proposal_added' ||
    event.event === 'proposal_validated'
  )
}

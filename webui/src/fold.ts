import type {
  ActionKind,
  ActionStatus,
  BranchObservationEvent,
  BranchRole,
  BranchStatus,
  EnvironmentChangedEvent,
  ObservationEvent,
  RunStatus,
  RuntimeExecutionState,
  SettlementEvent,
} from './types'

export type SpeculationReason = 'tool' | 'model' | 'other'

export interface EnvironmentState {
  state: RuntimeExecutionState
  independentInstance: boolean | null
  parentEnvironment: string | null
  configuration: Record<string, unknown>
  sequence: number
  timestampMs: number
}

export interface EnvironmentCleanState {
  succeeded: boolean
  reason: string | null
  sequence: number
}

export interface BranchState {
  branchId: string
  parentBranchId: string | null
  role: BranchRole
  status: BranchStatus
  statusReason: string | null
  edgeId: string | null
  speculationKind: ActionKind | null
  speculationStartedAtMs: number | null
  createdAtSequence: number
  createdTimestampMs: number
  environment: EnvironmentState | null
  environmentCleaned: EnvironmentCleanState | null
  modelUsages: Array<Extract<BranchObservationEvent, { event: 'model_usage' }>>
  tools: Array<Extract<BranchObservationEvent, { event: 'tool_finished' }>>
  settlements: SettlementEvent[]
  events: BranchObservationEvent[]
  lastEventSequence: number
  lastTimestampMs: number
  conversationUpdatedSequence: number | null
  authorityChangedAtMs: number | null
  /** First timestamp at which the Branch reached a terminal lifecycle status. */
  terminalAtMs: number | null
}

export interface TaskState {
  taskId: string
  branches: Record<string, BranchState>
  branchOrder: string[]
  authority: string | null
  authorityHistory: string[]
  lastSequence: number
  lastTimestampMs: number
}

export interface RunState {
  sequence: number
  tasks: Record<string, TaskState>
  taskOrder: string[]
  runStatus: RunStatus | null
  runStatusSequence: number
}

export function emptyRunState(): RunState {
  return {
    sequence: 0,
    tasks: {},
    taskOrder: [],
    runStatus: null,
    runStatusSequence: 0,
  }
}

function newTask(taskId: string, sequence: number, timestampMs: number): TaskState {
  return {
    taskId,
    branches: {},
    branchOrder: [],
    authority: null,
    authorityHistory: [],
    lastSequence: sequence,
    lastTimestampMs: timestampMs,
  }
}

function newBranch(
  branchId: string,
  sequence: number,
  timestampMs: number,
): BranchState {
  return {
    branchId,
    parentBranchId: null,
    role: 'candidate',
    status: 'created',
    statusReason: null,
    edgeId: null,
    speculationKind: null,
    speculationStartedAtMs: null,
    createdAtSequence: sequence,
    createdTimestampMs: timestampMs,
    environment: null,
    environmentCleaned: null,
    modelUsages: [],
    tools: [],
    settlements: [],
    events: [],
    lastEventSequence: sequence,
    lastTimestampMs: timestampMs,
    conversationUpdatedSequence: null,
    authorityChangedAtMs: null,
    terminalAtMs: null,
  }
}

function environmentFromEvent(event: EnvironmentChangedEvent): EnvironmentState {
  return {
    state: event.state,
    independentInstance: event.independent_instance,
    parentEnvironment: event.parent_environment,
    configuration: event.configuration,
    sequence: event.sequence,
    timestampMs: event.timestamp_ms,
  }
}

function appendUnique(values: string[], value: string): string[] {
  return values.includes(value) ? values : [...values, value]
}

/**
 * Pure fold over observation events. `upToSequence` makes the same function
 * usable for replay: every state is exactly the result of events 1..N.
 */
export function foldEvents(
  events: readonly ObservationEvent[],
  upToSequence = Number.POSITIVE_INFINITY,
): RunState {
  let state = emptyRunState()
  for (const event of events) {
    if (event.sequence > upToSequence) break
    state = applyEvent(state, event)
  }
  return state
}

/** Incremental, immutable application of one already-ordered event. */
export function applyEvent(state: RunState, event: ObservationEvent): RunState {
  const taskExists = state.tasks[event.task_id] !== undefined
  const task: TaskState = taskExists
    ? { ...state.tasks[event.task_id] }
    : newTask(event.task_id, event.sequence, event.timestamp_ms)
  task.lastSequence = Math.max(task.lastSequence, event.sequence)
  task.lastTimestampMs = Math.max(task.lastTimestampMs, event.timestamp_ms)

  let runStatus = state.runStatus
  let runStatusSequence = state.runStatusSequence

  if (event.event === 'run_status') {
    runStatus = {
      ready: event.ready,
      running: event.running,
      notice: event.notice,
      connected: event.connected,
    }
    runStatusSequence = event.sequence
  } else {
    const branches = { ...task.branches }
    const branch: BranchState =
      branches[event.branch_id] !== undefined
        ? { ...branches[event.branch_id] }
        : newBranch(event.branch_id, event.sequence, event.timestamp_ms)
    if (branches[event.branch_id] === undefined) {
      task.branchOrder = [...task.branchOrder, event.branch_id]
    }

    branch.events = [...branch.events, event]
    branch.lastEventSequence = event.sequence
    branch.lastTimestampMs = event.timestamp_ms

    switch (event.event) {
      case 'branch_created':
        branch.parentBranchId = event.parent_branch_id
        branch.role = event.role
        branch.edgeId = event.edge_id
        branch.speculationKind = event.speculation_kind
        branch.speculationStartedAtMs = event.speculation_started_at_ms
        branch.createdAtSequence = event.sequence
        branch.createdTimestampMs = event.timestamp_ms
        if (event.role === 'authority') {
          if (task.authority === null) task.authority = event.branch_id
          task.authorityHistory = appendUnique(task.authorityHistory, event.branch_id)
        }
        break
      case 'authority_changed':
        if (event.previous_authority && branches[event.previous_authority]) {
          const previous = { ...branches[event.previous_authority] }
          previous.role = 'candidate'
          branches[event.previous_authority] = previous
        }
        branch.role = 'authority'
        branch.authorityChangedAtMs = event.timestamp_ms
        task.authority = event.branch_id
        task.authorityHistory = appendUnique(task.authorityHistory, event.branch_id)
        break
      case 'status_changed':
        branch.status = event.status
        branch.statusReason = event.reason
        if (
          branch.terminalAtMs === null &&
          (event.status === 'promoted' ||
            event.status === 'discarded' ||
            event.status === 'cleaned')
        ) {
          branch.terminalAtMs = event.timestamp_ms
        }
        break
      case 'environment_changed':
        branch.environment = environmentFromEvent(event)
        break
      case 'environment_cleaned':
        branch.environmentCleaned = {
          succeeded: event.succeeded,
          reason: event.reason,
          sequence: event.sequence,
        }
        break
      case 'conversation_updated':
        branch.conversationUpdatedSequence = event.sequence
        break
      case 'model_usage':
        branch.modelUsages = [...branch.modelUsages, event]
        break
      case 'tool_finished':
        branch.tools = [...branch.tools, event]
        break
      case 'settlement':
        branch.settlements = [...branch.settlements, event]
        break
    }

    branches[event.branch_id] = branch
    task.branches = branches
  }

  const tasks = { ...state.tasks }
  tasks[event.task_id] = task
  const taskOrder = taskExists ? state.taskOrder : [...state.taskOrder, event.task_id]

  return {
    sequence: Math.max(state.sequence, event.sequence),
    tasks,
    taskOrder,
    runStatus,
    runStatusSequence,
  }
}

export function currentAuthority(task: TaskState | null): BranchState | null {
  if (!task || !task.authority) return null
  return task.branches[task.authority] ?? null
}

export function authorityPath(task: TaskState | null): Set<string> {
  const path = new Set<string>()
  if (!task) return path
  let branch = currentAuthority(task)
  while (branch) {
    if (path.has(branch.branchId)) break
    path.add(branch.branchId)
    branch = branch.parentBranchId ? task.branches[branch.parentBranchId] ?? null : null
  }
  return path
}

export function authorityWindows(task: TaskState | null): BranchState[] {
  if (!task) return []
  const ids = task.authorityHistory.length
    ? task.authorityHistory
    : task.branchOrder.filter((id) => task.branches[id]?.role === 'authority')
  return ids
    .map((id) => task.branches[id])
    .filter((branch): branch is BranchState => Boolean(branch))
}

export function isTerminalStatus(status: BranchStatus): boolean {
  return status === 'promoted' || status === 'discarded' || status === 'cleaned'
}

export function isActiveCandidate(branch: BranchState): boolean {
  return (
    branch.role === 'candidate' &&
    branch.status !== 'promoted' &&
    branch.status !== 'discarded' &&
    branch.status !== 'cleaned'
  )
}

export function speculationReason(kind: ActionKind | null): SpeculationReason {
  if (kind === 'tool_execution') return 'tool'
  if (kind === 'model_inference') return 'model'
  return 'other'
}

export function latestModel(branch: BranchState): string | null {
  return branch.modelUsages.at(-1)?.model ?? null
}

export function branchTokenTotal(branch: BranchState): {
  value: number
  complete: boolean
  known: boolean
} {
  let value = 0
  let complete = true
  let known = false
  for (const usage of branch.modelUsages) {
    if (usage.total_tokens !== null) {
      value += usage.total_tokens
      known = true
    } else if (usage.input_tokens !== null || usage.output_tokens !== null) {
      value += (usage.input_tokens ?? 0) + (usage.output_tokens ?? 0)
      known = true
      complete = false
    } else {
      complete = false
    }
  }
  return { value, complete: complete && known, known }
}

export function statusLabel(status: BranchStatus): string {
  switch (status) {
    case 'created':
      return '已创建'
    case 'running':
      return '运行中'
    case 'awaiting_validation':
      return '等待验证'
    case 'promoting':
      return '正在晋级'
    case 'promoted':
      return '已晋级'
    case 'discarding':
      return '正在丢弃'
    case 'discarded':
      return '已丢弃'
    case 'cleaning':
      return '正在清理'
    case 'cleaned':
      return '已清理'
  }
}

export function roleLabel(role: BranchRole): string {
  return role === 'authority' ? '权威' : '候选'
}

export function actionStatusLabel(status: ActionStatus): string {
  switch (status) {
    case 'succeeded':
      return '成功'
    case 'failed':
      return '失败'
    case 'cancelled':
      return '取消'
    case 'timed_out':
      return '超时'
  }
}

export function runtimeStateLabel(state: RuntimeExecutionState): string {
  switch (state) {
    case 'unknown':
      return '未知'
    case 'environment_requested':
      return '已请求环境'
    case 'ready':
      return '环境就绪'
    case 'promotion_requested':
      return '已请求晋级'
    case 'promoted':
      return '环境已晋级'
    case 'termination_requested':
      return '已请求终止'
    case 'terminated':
      return '已终止'
    case 'cleaned':
      return '已清理'
    case 'failed':
      return '失败'
  }
}

export function speculationKindLabel(kind: ActionKind | null): string {
  switch (kind) {
    case 'tool_execution':
      return '工具执行'
    case 'model_inference':
      return '模型推理'
    case 'start_subagent':
      return '启动 sub-agent'
    case 'send_message':
      return '发送消息'
    case 'wait':
      return '等待'
    case 'finish':
      return '结束'
    case 'other':
      return '其他'
    default:
      return '未上报'
  }
}

import { evaluatePureDecision, type PureDecision } from './pure-policy.ts';

export type PolicyArtifact = {
    readonly digest: string;
    readonly has_predictor?: boolean;
    readonly decisions: Readonly<Record<string, string>>;
    readonly pure?: Readonly<Record<string, PureDecision>>;
    readonly decision_inputs?: Readonly<Record<string, readonly string[]>>;
    readonly policy_model_grant?: {
        readonly decisions: readonly string[];
        readonly models: readonly string[];
        readonly max_requests: number;
        readonly timeout_ms: number;
    };
};

export type EvaluatePolicy = (
    artifact: PolicyArtifact,
    decision: string,
    data: unknown,
    signal?: AbortSignal,
) => Promise<unknown>;

export type ExecutionPoint = {
    branch: string;
    /** Product action identity, used to pair repeated hooks of one action. */
    actionId: string;
    actionKind: string;
    name: string;
    arguments?: unknown;
    op: Record<string, unknown>;
    contractContext?: unknown;
    admit: boolean | (() => boolean);
    admissionFacts?: Record<string, unknown> | (() => Record<string, unknown>);
    fastStart?: { chanceId: string; start: (prediction: unknown) => void };
    signal?: AbortSignal;
    /** Reconnect an already admitted hook; do not count or admit it twice. */
    admitted?: boolean;
};

export type Disposition =
    | { kind: 'allow' }
    | { kind: 'admit'; policyDigest: string };

export type ControlCommand = {
    command: string;
    branch_id: string;
    [field: string]: unknown;
};

type ActionCompletion = {
    branch: string;
    actionId: string;
    actionKind: string;
    name: string;
    status: string;
    isError: boolean;
    arguments?: unknown;
};

type ActionSummary = { kind: string; name: string; status: string; is_error: boolean };
type CompletedTool = ActionSummary & { arguments?: unknown };
type BranchState = {
    is_candidate: boolean;
    assumption_depth: number;
    steps_completed: number;
    tool_counts: Record<string, { succeeded: number; failed: number }>;
    recent_actions: ActionSummary[];
};

type BranchProgress = {
    state: BranchState;
    startedAt: number;
    hookCount: number;
    modelInferences: number;
    toolExecutions: number;
    failedTools?: number;
    speculationKind?: 'model_inference' | 'tool_execution';
    predictionKind?: 'predicted_tool_calls' | 'branch_model' | 'tool_result';
    lastAction?: ActionSummary;
    parentStepsCompleted?: number;
    completedTools?: CompletedTool[];
};

export type Interposer = {
    setPolicy(branch: string, artifact: PolicyArtifact): void;
    control(command: ControlCommand): Promise<void>;
    beforeAction(point: ExecutionPoint): Promise<Disposition>;
    afterAction(action: ActionCompletion): void;
    releaseAll(): void;
    snapshot(): unknown;
    restore(record: any): void;
    hasPredictor(branch: string): Promise<boolean>;
};

type Hold = { promise: Promise<void>; release: () => void };

type Options = {
    taskId: string;
    report: (message: Record<string, unknown>) => Promise<void>;
    evaluate: EvaluatePolicy;
    now?: () => number;
    traceEvaluation?: <T>(decision: string, path: string, run: () => T) => T;
};

function makeHold(): Hold {
    let release!: () => void;
    const promise = new Promise<void>(resolve => (release = resolve));
    return { promise, release };
}

async function waitFor(hold: Hold, signal: AbortSignal | undefined, message: string) {
    await new Promise<void>((resolve, reject) => {
        const abort = () => reject(Error(message));
        hold.promise.then(resolve, reject).finally(() => signal?.removeEventListener('abort', abort));
        signal?.addEventListener('abort', abort, { once: true });
        if (signal?.aborted) abort();
    });
}

export function createInterposer({ taskId, report, evaluate: sandbox, now = Date.now, traceEvaluation = (_decision, _path, run) => run() }: Options): Interposer {
    const inMemory: EvaluatePolicy = async (artifact, decision, data, signal) => pure(artifact, decision, data, signal)?.result;
    function pure(artifact: PolicyArtifact, decision: string, data: unknown, signal?: AbortSignal) {
        const binding = artifact.pure?.[decision];
        if (binding && !artifact.policy_model_grant?.decisions.includes(decision)) {
            return traceEvaluation(decision, 'pure', () => evaluatePureDecision(binding, decision, data, artifact.decision_inputs?.[decision] ?? [], signal));
        }
    }
    const evaluate: EvaluatePolicy = async (artifact, decision, data, signal) => {
        const evaluated = pure(artifact, decision, data, signal);
        if (evaluated) return evaluated.result;
        if (decision === 'admit.opportunity_pass' && artifact.pure !== undefined) {
            throw Error('admit.opportunity_pass could not be evaluated in memory');
        }
        return sandbox(artifact, decision, data, signal);
    };
    const policies = new Map<string, PolicyArtifact>();
    const effectHolds = new Map<string, Hold>();
    const pauseHolds = new Map<string, Hold>();
    const extentHolds = new Map<string, Hold>();
    const progress = new Map<string, BranchProgress>();
    const countedHooks = new Set<string>();

    function localInputs(binding: PureDecision, roots: string[]) {
        const argument = binding.program.argument;
        function allowed(node: unknown): boolean {
            if (!Array.isArray(node)) return true;
            if (node[0] === 'var' && node[1] === argument) return false;
            if (node[0] === 'index' && Array.isArray(node[1]) && node[1][0] === 'var' && node[1][1] === argument) {
                const key = node[2];
                return Array.isArray(key) && key[0] === 'const' && roots.includes(key[1]);
            }
            return node.every(allowed);
        }
        return allowed(binding.program.body);
    }

    function fastPrediction(artifact: PolicyArtifact, point: ExecutionPoint) {
        const request = point.fastStart;
        if (!request || point.op.authority !== true || [...progress.values()].some(value => value.state.is_candidate)) return;
        const run = (decision: string, data: unknown, roots: string[]) => {
            const binding = artifact.pure?.[decision];
            if (!binding || artifact.policy_model_grant?.decisions.includes(decision) || !localInputs(binding, roots)) return;
            const inputs = artifact.decision_inputs?.[decision] ?? [];
            if (inputs.some(path => !roots.includes(path.split('.')[0]))) return;
            return traceEvaluation(decision, 'local_fast', () => evaluatePureDecision(binding, decision, data, inputs, point.signal))?.result;
        };
        const chance = {chance_id: request.chanceId, tool_name: point.name, arguments: point.arguments ?? {}};
        try {
            const selection = run('select', {frontier_view: [chance]}, ['frontier_view']);
            if (!Array.isArray(selection) || selection.length !== 1 || selection[0] !== request.chanceId) return;
            if (run('launch', {chance_context: {chance_id: chance.chance_id, tool_name: chance.tool_name}}, ['chance_context']) !== true) return;
            const result = run('predict', {chance_context: {...chance, kind: 'toolexecution'}, process_context: {}}, ['chance_context']);
            const proposals = (result as {proposals?: {payload: unknown; provenance?: unknown}[]; abstain?: boolean} | undefined);
            if (proposals?.abstain === true) return {abstain: true} as const;
            if (proposals?.proposals?.length === 1) return proposals.proposals[0];
        } catch {
        }
    }

    function newProgress(parent?: BranchState, candidate = false): BranchProgress {
        return {
            state: {
                is_candidate: candidate,
                assumption_depth: candidate ? (parent?.assumption_depth ?? 0) + 1 : 0,
                steps_completed: 0,
                tool_counts: Object.assign(Object.create(null), structuredClone(parent?.tool_counts ?? {})),
                recent_actions: structuredClone(parent?.recent_actions ?? []),
            },
            startedAt: now(), hookCount: 0, modelInferences: 0, toolExecutions: 0,
            parentStepsCompleted: parent?.steps_completed ?? 0,
            completedTools: [],
        };
    }

    function branchProgress(branch: string) {
        let value = progress.get(branch);
        if (!value) progress.set(branch, value = newProgress());
        return value;
    }

    async function extentGate(point: ExecutionPoint, branch: BranchProgress) {
        const artifact = policies.get(point.branch);
        if (!artifact || !branch.state.is_candidate) return;
        const eventKey = `${point.branch}\u0000${point.actionId}`;
        if (!countedHooks.has(eventKey)) {
            countedHooks.add(eventKey);
            branch.hookCount++;
        }
        const process_state: Record<string, unknown> = {
            steps_completed: branch.state.steps_completed,
            phase: point.actionKind,
        };
        for (const input of artifact.decision_inputs?.extent ?? []) {
            switch (input) {
                case 'process_state.hook_count': process_state.hook_count = branch.hookCount; break;
                case 'process_state.active_time': process_state.active_time = Math.max(0, now() - branch.startedAt) / 1000; break;
                case 'process_state.resource_counters':
                    process_state.resource_counters = {model_inferences: branch.modelInferences,
                        tool_executions: branch.toolExecutions, failed_tools: branch.failedTools ?? 0};
                    break;
                case 'process_state.result_facts':
                    process_state.result_facts = {
                        last_action: branch.lastAction ?? {}, completed_tools: branch.completedTools ?? [],
                    };
                    break;
                case 'process_state.semantic_facts':
                    process_state.semantic_facts = {
                        speculation_kind: branch.speculationKind,
                        prediction_kind: branch.predictionKind,
                        parent_steps_completed: branch.parentStepsCompleted,
                        next_action: {kind: point.actionKind, name: point.name, arguments: point.arguments ?? {}},
                        next_action_kind: point.actionKind, next_action_name: point.name, arguments: point.arguments ?? {},
                    };
                    break;
            }
        }
        if (await evaluate(artifact, 'extent', {process_state}, point.signal) !== 'stop'
            || !branch.state.is_candidate || point.signal?.aborted || progress.get(point.branch) !== branch) return;
        const existing = extentHolds.get(point.branch);
        const hold = existing ?? makeHold();
        extentHolds.set(point.branch, hold);
        if (!existing) await report({ message: 'branch_suspended', branch_id: point.branch, reason: 'extent_reached' });
        await waitFor(hold, point.signal, 'Extent stop cancelled');
    }

    async function effectGate(point: ExecutionPoint) {
        const hold = effectHolds.get(point.branch);
        if (!hold) return;
        const artifact = policies.get(point.branch);
        let detail: string | undefined;
        if (artifact) {
            const plan = await evaluate(
                artifact,
                'effect',
                {
                    effect_descriptor: {
                        kind: point.actionKind,
                        tool: point.name,
                        arguments: point.arguments,
                    },
                    context: { branch_id: point.branch, capabilities:point.contractContext },
                },
                point.signal,
            );
            const effect = plan as {mode?: string; reason?: string} | null;
            if (effect?.mode === 'allow' && (point.contractContext as any)?.isolated === true) return;
            if (effect?.mode === 'stage' || effect?.mode === 'simulate') {
                detail = `${effect.mode}: external effect coordinator unavailable`;
                if (effect.reason) detail += ` (${effect.reason})`;
            } else {
                detail = effect?.reason;
            }
            if (effect?.mode === 'deny') {
                throw Error('Effect denied by Policy');
            }
        }
        await report({ message: 'branch_suspended', branch_id: point.branch, reason: 'effect_gate', detail });
        await waitFor(hold, point.signal, 'Candidate cancelled');
        await report({ message: 'branch_resumed', branch_id: point.branch });
    }

    async function pauseGate(branch: string, signal: AbortSignal | undefined) {
        const hold = pauseHolds.get(branch);
        if (!hold) return;
        await report({ message: 'branch_suspended', branch_id: branch, reason: 'rebalanced' });
        await waitFor(hold, signal, 'Branch cancelled while paused');
        await report({ message: 'branch_resumed', branch_id: branch });
    }

    async function localAdmission(point: ExecutionPoint) {
        const artifact = policies.get(point.branch);
        if (!artifact) return {admitted:false, pure:false};
        let entirelyLocal = true;
        const admit = async (decision: string, data: unknown) => {
            const result = await inMemory(artifact, decision, data, point.signal);
            if (result !== undefined) return result;
            entirelyLocal = false;
            if (decision === 'admit.opportunity_pass' && artifact.pure !== undefined) {
                throw Error('admit.opportunity_pass could not be evaluated in memory');
            }
            return sandbox(artifact, decision, data, point.signal);
        };
        if (await admit('admit.opportunity_pass', {op:point.op}) !== true) return {admitted:false, pure:entirelyLocal};
        const feasibility = await admit('admit.feasibility_pass', {
            op:point.op, state:branchProgress(point.branch).state, contract_context:point.contractContext ?? {},
        });
        return {admitted:feasibility === true, pure:entirelyLocal};
    }

    function setPolicy(branch: string, artifact: PolicyArtifact) {
        if (typeof artifact.digest !== 'string' || !artifact.decisions || typeof artifact.decisions !== 'object') {
            throw Error('Invalid local Policy artifact');
        }
        policies.set(branch, { ...artifact, has_predictor: artifact.has_predictor === true });
    }

    return {
        setPolicy,
        async hasPredictor(branch) {
            return policies.get(branch)?.has_predictor === true;
        },

        snapshot() {
            return {policies:[...policies],effectHolds:[...effectHolds.keys()],pauseHolds:[...pauseHolds.keys()],
                extentHolds:[...extentHolds.keys()],progress:structuredClone([...progress]),countedHooks:[...countedHooks]};
        },

        restore(record) {
            for(const [branch,artifact] of record.policies)setPolicy(branch,artifact);
            for(const branch of record.effectHolds)effectHolds.set(branch,makeHold());
            for(const branch of record.pauseHolds)pauseHolds.set(branch,makeHold());
            // Older snapshots identify held candidates without recording their hook identities.
            for (const branch of record.effectHolds) progress.set(branch, newProgress(undefined, true));
            for (const [branch, count] of record.hookCounts ?? []) branchProgress(branch).hookCount = count;
            for (const [branch, value] of record.progress ?? []) {
                value.state.tool_counts = Object.assign(Object.create(null), value.state.tool_counts);
                progress.set(branch, value);
            }
            for (const branch of record.extentHolds) {
                if (progress.get(branch)?.state.is_candidate) extentHolds.set(branch, makeHold());
            }
            for(const key of record.countedHooks)countedHooks.add(key);
        },

        async control(command) {
            const branch = command.branch_id;
            switch (command.command) {
                case 'local_policy': {
                    setPolicy(branch, {
                        digest: command.policy_digest as string,
                        has_predictor: command.has_predictor === true,
                        decisions: command.decisions as Record<string, string>,
                        pure: command.pure as PolicyArtifact['pure'],
                        decision_inputs: command.decision_inputs as PolicyArtifact['decision_inputs'],
                        policy_model_grant: command.policy_model_grant as PolicyArtifact['policy_model_grant'],
                    });
                    return;
                }
                case 'start_candidate': {
                    const parent = command.parent_branch_id as string;
                    const parentPolicy = policies.get(parent);
                    if (parentPolicy) policies.set(branch, parentPolicy);
                    if (!progress.has(branch)) {
                        const candidate = newProgress(progress.get(parent)?.state, true);
                        const prediction = command.prediction as {predict?: string; kind?: string} | undefined;
                        candidate.predictionKind = prediction?.predict === 'branch_model' ? 'branch_model'
                            : prediction?.kind === 'predicted_tool_calls' ? 'predicted_tool_calls' : 'tool_result';
                        candidate.speculationKind = candidate.predictionKind === 'tool_result' ? 'tool_execution' : 'model_inference';
                        progress.set(branch, candidate);
                    }
                    if (!effectHolds.has(branch)) effectHolds.set(branch, makeHold());
                    return;
                }
                case 'pause':
                    if (!pauseHolds.has(branch)) pauseHolds.set(branch, makeHold());
                    return;
                case 'resume':
                    pauseHolds.get(branch)?.release();
                    pauseHolds.delete(branch);
                    return;
                case 'resolve_pending': {
                    const hold = extentHolds.get(branch);
                    if (!hold) return;
                    extentHolds.delete(branch);
                    hold.release();
                    if (command.outcome === undefined) {
                        await report({ message: 'branch_resumed', branch_id: branch });
                    }
                    return;
                }
                case 'promote':
                    branchProgress(branch).state.is_candidate = false;
                    branchProgress(branch).state.assumption_depth = 0;
                    effectHolds.get(branch)?.release();
                    effectHolds.delete(branch);
                    extentHolds.get(branch)?.release();
                    extentHolds.delete(branch);
                    return;
                case 'discard':
                    if (progress.has(branch)) progress.get(branch)!.state.is_candidate = false;
                    effectHolds.get(branch)?.release();
                    effectHolds.delete(branch);
                    pauseHolds.get(branch)?.release();
                    pauseHolds.delete(branch);
                    extentHolds.get(branch)?.release();
                    extentHolds.delete(branch);
                    return;
                case 'cleanup':
                    effectHolds.get(branch)?.release();
                    effectHolds.delete(branch);
                    pauseHolds.get(branch)?.release();
                    pauseHolds.delete(branch);
                    extentHolds.get(branch)?.release();
                    extentHolds.delete(branch);
                    policies.delete(branch);
                    progress.delete(branch);
                    for (const key of countedHooks) {
                        if (key.startsWith(`${branch}\u0000`)) countedHooks.delete(key);
                    }
                    return;
                default:
                    return;
            }
        },

        afterAction({branch, actionId, actionKind, name, status, isError, arguments: args}) {
            countedHooks.delete(`${branch}\u0000${actionId}`);
            const value = progress.get(branch);
            if (!value) return;
            const {state} = value;
            const summary = {kind: actionKind, name, status, is_error: isError};
            value.lastAction = summary;
            state.recent_actions.push(summary);
            if (state.recent_actions.length > 16) state.recent_actions.shift();
            if (actionKind === 'model_inference') value.modelInferences++;
            if (actionKind === 'tool_execution') {
                value.toolExecutions++;
                if (state.is_candidate && value.speculationKind === 'model_inference') {
                    (value.completedTools ??= []).push({...summary, arguments: args});
                }
                const counts = state.tool_counts[name] ??= {succeeded: 0, failed: 0};
                if (status === 'succeeded' && !isError) counts.succeeded++;
                else if (status === 'failed' || isError) {
                    counts.failed++;
                    value.failedTools = (value.failedTools ?? 0) + 1;
                }
            }
            if (actionKind === 'model_inference' || actionKind === 'tool_execution') state.steps_completed++;
        },

        async beforeAction(point) {
            const effectful = point.actionKind === 'tool_execution';
            const managed = effectful || point.actionKind === 'model_inference';
            const branch = branchProgress(point.branch);
            await pauseGate(point.branch, point.signal);
            if (point.admitted) {
                const hold = extentHolds.get(point.branch);
                if (hold && branch.state.is_candidate) await waitFor(hold, point.signal, 'Extent stop cancelled');
            } else if (managed) {
                await extentGate(point, branch);
            }
            if (effectful) await effectGate(point);
            if (point.admitted || !managed) return { kind: 'allow' };

            const admit = typeof point.admit === 'function' ? point.admit() : point.admit;
            const artifact = policies.get(point.branch);
            if (!admit || !artifact || point.op.has_predictor === false) return { kind: 'allow' };
            const admission = await localAdmission(point);
            if (!admission.admitted) return { kind: 'allow' };
            const facts = typeof point.admissionFacts === 'function'
                ? point.admissionFacts()
                : point.admissionFacts;
            const prediction = admission.pure ? fastPrediction(artifact, point) : undefined;
            if (prediction && 'abstain' in prediction) return {kind: 'allow'};
            const sent = report({
                message: 'speculative_admission_request',
                task_id: taskId,
                policy_digest: artifact.digest,
                branch_id: point.branch,
                action_id: point.actionId,
                action_kind: point.actionKind,
                name: point.name,
                arguments: point.arguments,
                ...facts,
                ...(prediction && 'payload' in prediction && {fast_start:{chance_id:point.fastStart!.chanceId, prediction:prediction.payload, provenance:prediction.provenance}}),
                timestamp_ms: now(),
            });
            if (prediction && 'payload' in prediction) {
                void sent;
                point.fastStart!.start(prediction.payload);
                return { kind: 'allow' };
            }
            await sent;
            return { kind: 'admit', policyDigest: artifact.digest };
        },

        releaseAll() {
            for (const hold of effectHolds.values()) hold.release();
            for (const hold of pauseHolds.values()) hold.release();
            for (const hold of extentHolds.values()) hold.release();
        },
    };
}

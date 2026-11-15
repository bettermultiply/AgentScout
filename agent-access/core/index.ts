/**
 * Product-independent Agent access.
 *
 * This module owns the parts of the Agent access side that do not depend on a
 * concrete Agent product: local Policy evaluation, per-Branch hook counting and
 * `extent`, the `effect` gate, control-plane reports, and waiting for the
 * control plane's disposition. A Product Integration only turns its own
 * execution points into `ExecutionPoint` values and executes the disposition;
 * it does not evaluate `extent` or `effect` itself.
 */
export type PolicyArtifact = {
    readonly digest: string;
    readonly decisions: Readonly<Record<string, string>>;
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
    /** Generic action kind such as `tool_execution` or `model_inference`. */
    actionKind: string;
    name: string;
    arguments?: unknown;
    /** Policy `op` description presented to `admit`. */
    op: Record<string, unknown>;
    /** Product-supplied `contract_context` for `admit.feasibility_pass`. */
    contractContext?: unknown;
    /** Whether this point may be admitted speculatively. */
    admit: boolean | (() => boolean);
    /** Extra product facts copied into `speculative_admission_request`. */
    admissionFacts?: Record<string, unknown> | (() => Record<string, unknown>);
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

export type AgentAccess = {
    setPolicy(branch: string, artifact: PolicyArtifact): void;
    /** Apply the hold bookkeeping for a control-plane command. */
    control(command: ControlCommand): Promise<void>;
    beforeAction(point: ExecutionPoint): Promise<Disposition>;
    releaseAll(): void;
    snapshot(): unknown;
    restore(record: any): void;
    predict(branch: string, data: unknown, signal?: AbortSignal): Promise<any>;
    hasPredictor(branch: string, op: unknown, signal?: AbortSignal): Promise<boolean>;
};

type Hold = { promise: Promise<void>; release: () => void };

type Options = {
    taskId: string;
    report: (message: Record<string, unknown>) => Promise<void>;
    evaluate: EvaluatePolicy;
    now?: () => number;
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

export function createAgentAccess({ taskId, report, evaluate, now = Date.now }: Options): AgentAccess {
    const policies = new Map<string, PolicyArtifact>();
    const effectHolds = new Map<string, Hold>();
    const pauseHolds = new Map<string, Hold>();
    const extentHolds = new Map<string, Hold>();
    const hookCounts = new Map<string, number>();
    const countedHooks = new Set<string>();

    // A Branch that has not been accepted yet may not produce effects. The
    // `effect` Decision is evaluated at the boundary; while the gate holds, the
    // Branch is reported as blocked by the effect gate.
    async function effectGate(point: ExecutionPoint) {
        const hold = effectHolds.get(point.branch);
        if (!hold) return;
        const artifact = policies.get(point.branch);
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
            if ((plan as { mode?: string } | null)?.mode === 'allow' && (point.contractContext as any)?.isolated === true) return;
            if ((plan as { mode?: string } | null)?.mode === 'deny') {
                throw Error('Effect denied by Policy');
            }
        }
        await report({ message: 'branch_suspended', branch_id: point.branch, reason: 'effect_gate' });
        await waitFor(hold, point.signal, 'Candidate cancelled');
        await report({ message: 'branch_resumed', branch_id: point.branch });
    }

    // `rebalance` pause: hold the Branch at its next boundary until resumed.
    async function pauseGate(branch: string, signal: AbortSignal | undefined) {
        const hold = pauseHolds.get(branch);
        if (!hold) return;
        await report({ message: 'branch_suspended', branch_id: branch, reason: 'rebalanced' });
        await waitFor(hold, signal, 'Branch cancelled while paused');
        await report({ message: 'branch_resumed', branch_id: branch });
    }

    async function localAdmission(point: ExecutionPoint) {
        const artifact = policies.get(point.branch);
        if (!artifact) return false;
        const opportunity = await evaluate(
            artifact,
            'admit.opportunity_pass',
            { op: point.op },
            point.signal,
        );
        if (opportunity !== true) return false;
        const feasibility = await evaluate(
            artifact,
            'admit.feasibility_pass',
            { op: point.op, state: {}, contract_context: point.contractContext },
            point.signal,
        );
        return feasibility === true;
    }

    function setPolicy(branch: string, artifact: PolicyArtifact) {
        if (typeof artifact.digest !== 'string' || !artifact.decisions || typeof artifact.decisions !== 'object') {
            throw Error('Invalid local Policy artifact');
        }
        policies.set(branch, artifact);
    }

    return {
        setPolicy,
        async hasPredictor(branch, op, signal) {
            const artifact=policies.get(branch);
            return !!artifact && await evaluate(artifact,'admit.opportunity_pass',{op},signal) === true;
        },
        async predict(branch, data, signal) {
            const artifact=policies.get(branch);
            return artifact?.decisions.predict ? evaluate(artifact,'predict',data,signal) : {proposals:[],generation:'batch'};
        },

        snapshot() {
            return {policies:[...policies],effectHolds:[...effectHolds.keys()],pauseHolds:[...pauseHolds.keys()],
                extentHolds:[...extentHolds.keys()],hookCounts:[...hookCounts],countedHooks:[...countedHooks]};
        },

        restore(record) {
            for(const [branch,artifact] of record.policies)setPolicy(branch,artifact);
            for(const branch of record.effectHolds)effectHolds.set(branch,makeHold());
            for(const branch of record.pauseHolds)pauseHolds.set(branch,makeHold());
            for(const branch of record.extentHolds)extentHolds.set(branch,makeHold());
            for(const [branch,count] of record.hookCounts)hookCounts.set(branch,count);
            for(const key of record.countedHooks)countedHooks.add(key);
        },

        async control(command) {
            const branch = command.branch_id;
            switch (command.command) {
                case 'local_policy': {
                    setPolicy(branch, {
                        digest: command.policy_digest as string,
                        decisions: command.decisions as Record<string, string>,
                    });
                    return;
                }
                case 'start_candidate': {
                    const parentPolicy = policies.get(command.parent_branch_id as string);
                    if (parentPolicy) policies.set(branch, parentPolicy);
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
                    // A disposal without an outcome only releases the extent
                    // hold; the Branch becomes runnable again.
                    if (command.outcome === undefined) {
                        await report({ message: 'branch_resumed', branch_id: branch });
                    }
                    return;
                }
                case 'promote':
                    effectHolds.get(branch)?.release();
                    effectHolds.delete(branch);
                    extentHolds.get(branch)?.release();
                    extentHolds.delete(branch);
                    return;
                case 'discard':
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
                    hookCounts.delete(branch);
                    for (const key of countedHooks) {
                        if (key.startsWith(`${branch}\u0000`)) countedHooks.delete(key);
                    }
                    return;
                default:
                    return;
            }
        },

        async beforeAction(point) {
            const effectful = point.actionKind === 'tool_execution';
            if (effectful) await effectGate(point);
            await pauseGate(point.branch, point.signal);
            if (point.admitted) {
                const hold = extentHolds.get(point.branch);
                if (hold) await waitFor(hold, point.signal, 'Extent stop cancelled');
                return {kind:'allow'};
            }

            const admit = typeof point.admit === 'function' ? point.admit() : point.admit;
            if (!admit || !policies.has(point.branch) || point.op.has_predictor === false) return { kind: 'allow' };
            if (!(await localAdmission(point))) return { kind: 'allow' };
            if (!effectful && point.actionKind !== 'model_inference') return { kind: 'allow' };

            const artifact = policies.get(point.branch)!;
            const facts = typeof point.admissionFacts === 'function'
                ? point.admissionFacts()
                : point.admissionFacts;
            await report({
                message: 'speculative_admission_request',
                task_id: taskId,
                policy_digest: artifact.digest,
                branch_id: point.branch,
                action_id: point.actionId,
                action_kind: point.actionKind,
                name: point.name,
                arguments: point.arguments,
                ...facts,
                timestamp_ms: now(),
            });

            const eventKey = `${point.branch}\u0000${point.actionId}`;
            if (!countedHooks.has(eventKey)) {
                countedHooks.add(eventKey);
                hookCounts.set(point.branch, (hookCounts.get(point.branch) ?? 0) + 1);
            }
            const steps = hookCounts.get(point.branch) ?? 0;
            const extent = await evaluate(
                artifact,
                'extent',
                {
                    process_state: {
                        steps_completed: steps,
                        hook_count: steps,
                        phase: point.actionKind,
                    },
                },
                point.signal,
            );
            if (extent === 'stop') {
                const hold = makeHold();
                extentHolds.set(point.branch, hold);
                await report({ message: 'branch_suspended', branch_id: point.branch, reason: 'extent_reached' });
                await waitFor(hold, point.signal, 'Extent stop cancelled');
            }
            return { kind: 'admit', policyDigest: artifact.digest };
        },

        releaseAll() {
            for (const hold of effectHolds.values()) hold.release();
            for (const hold of pauseHolds.values()) hold.release();
            for (const hold of extentHolds.values()) hold.release();
        },
    };
}

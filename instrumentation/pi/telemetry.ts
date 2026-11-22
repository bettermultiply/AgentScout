import { context, ROOT_CONTEXT, trace, SpanStatusCode, type Context, type Span } from '@opentelemetry/api';
import type { W3CTraceContextPropagator } from '@opentelemetry/core';
import type { NodeTracerProvider, ReadableSpan } from '@opentelemetry/sdk-trace-node';

let propagator: W3CTraceContextPropagator | undefined;
let provider: NodeTracerProvider | undefined;
let ended = 0, exported = 0, failed = 0;
if (process.env.OTEL_SDK_DISABLED?.toLowerCase() !== 'true'
    && (process.env.OTEL_EXPORTER_OTLP_TRACES_ENDPOINT?.trim() || process.env.OTEL_EXPORTER_OTLP_ENDPOINT?.trim())) {
    try {
        const [core, contextHooks, sdk, otlp, resources] = await Promise.all([
            import('@opentelemetry/core'),
            import('@opentelemetry/context-async-hooks'),
            import('@opentelemetry/sdk-trace-node'),
            import('@opentelemetry/exporter-trace-otlp-proto'),
            import('@opentelemetry/resources'),
        ]);
        propagator = new core.W3CTraceContextPropagator();
        const exporter = new otlp.OTLPTraceExporter({ timeoutMillis: 750, concurrencyLimit: 1 });
        const batch = new sdk.BatchSpanProcessor({
            export(spans, done) {
                exporter.export(spans, result => {
                    if (result.code === 0) exported += spans.length;
                    else failed += spans.length;
                    done(result);
                });
            },
            shutdown: () => exporter.shutdown(),
        }, { maxQueueSize: 1024, maxExportBatchSize: 128, scheduledDelayMillis: 300, exportTimeoutMillis: 800 });
        provider = new sdk.NodeTracerProvider({
            resource: resources.resourceFromAttributes({ 'service.name': 'ascout-pi' }),
            spanLimits: { attributeCountLimit: 32, eventCountLimit: 16, attributeValueLengthLimit: 128 },
            spanProcessors: [{
                onStart: (span, parent) => batch.onStart(span, parent),
                onEnd(span: ReadableSpan) {
                    if (span.spanContext().traceFlags & 1) ended++;
                    batch.onEnd(span);
                },
                forceFlush: () => batch.forceFlush(),
                shutdown: () => batch.shutdown(),
            }],
        });
        provider.register({ contextManager: new contextHooks.AsyncLocalStorageContextManager(), propagator });
    } catch {
        provider = undefined;
        process.stderr.write('ascout-pi: OTLP exporter configuration failed\n');
    }
}
const tracer = provider?.getTracer('ascout');

export function carrier(cx?: Context) {
    if (!tracer || !propagator) return undefined;
    const headers: Record<string, string> = {};
    propagator.inject(cx ?? context.active(), headers, { set: (target, name, value) => { target[name] = value; } });
    return headers.traceparent ? headers : undefined;
}

export function fromCarrier(value: unknown) {
    if (!tracer || !propagator || !value || typeof value !== 'object') return ROOT_CONTEXT;
    const headers = value as Record<string, string>;
    if (typeof headers.traceparent !== 'string' || headers.traceparent.length !== 55) return ROOT_CONTEXT;
    return propagator.extract(ROOT_CONTEXT, headers, {
        keys: () => ['traceparent', 'tracestate'],
        get: (source, key) => typeof source[key] === 'string' && source[key].length <= 512 ? source[key] : undefined,
    });
}

const branches = new Map<string, { span: Span, cx: Context, role: 'authority' | 'candidate' }>();
const actions = new Map<string, { span: Span, model: boolean, finished?: number }>();
let task = ROOT_CONTEXT;
const active = () => trace.getSpanContext(context.active()) ? context.active() : task;
const branchContext = (branch: string) => branches.get(branch)?.cx ?? active();
const actionKey = (branch: string, id: string) => JSON.stringify([branch, id]);

export function startBranch(branch: string, role: 'authority' | 'candidate', parent?: string, local = false, recovered = false) {
    if (!tracer || branches.has(branch)) return;
    if (role === 'authority' && !recovered && !trace.getSpanContext(task)) task = context.active();
    const cx = parent ? branchContext(parent) : active();
    if (!trace.getSpanContext(cx)) return;
    const span = tracer.startSpan(`ascout.branch.${role}`, { attributes: { local, recovered } }, cx);
    branches.set(branch, { span, cx: trace.setSpan(cx, span), role });
}

function endBranch(branch: string, result: string) {
    const interval = branches.get(branch);
    if (!interval) return;
    interval.span.setAttribute('result', result);
    interval.span.end();
    branches.delete(branch);
}

export function prompt(branch: string) {
    if (!tracer) return;
    closeIntervals('stopped');
    task = context.active();
    startBranch(branch, 'authority');
}

export function recover(branch: string, authority: string) {
    if (!tracer) return;
    task = context.active();
    startBranch(branch, branch === authority ? 'authority' : 'candidate', undefined, false, true);
}

export function promote(branch: string, previous: string, authority: boolean) {
    if (!tracer) return;
    endBranch(branch, 'promoted');
    if (authority) {
        endBranch(previous, 'replaced');
        startBranch(branch, 'authority');
    }
}

export function toolContext<T>(branch: string, operation: string, run: () => T): T {
    if (!tracer) return run();
    const cx = branchContext(branch);
    const action = actions.get(actionKey(branch, operation));
    return context.with(action ? trace.setSpan(cx, action.span) : cx, run);
}

const statuses = new Set(['succeeded', 'failed', 'cancelled', 'applied', 'accepted', 'started', 'rejected']);
export function observe(payload: any) {
    if (!tracer) return undefined;
    const branch = payload.branch_id;
    const id = actionKey(branch, payload.action_id);
    const cx = branchContext(branch);
    switch (payload.message) {
        case 'action_started': {
            if (!trace.getSpanContext(cx) || actions.has(id)) break;
            const model = payload.action_kind === 'model_inference';
            const attributes: Record<string, string> = {};
            if (model && typeof payload.arguments?.model === 'string') attributes['gen_ai.request.model'] = payload.arguments.model.slice(0, 128);
            if (!model && typeof payload.name === 'string') attributes['gen_ai.tool.name'] = payload.name.slice(0, 128);
            const span = tracer.startSpan(model ? 'ascout.model' : 'ascout.tool', { startTime: payload.timestamp_ms, attributes }, cx);
            actions.set(id, { span, model });
            return carrier(trace.setSpan(cx, span));
        }
        case 'action_finished': {
            const action = actions.get(id);
            if (!action) break;
            action.span.setAttribute('result', statuses.has(payload.status) ? payload.status : 'unknown');
            if (payload.status === 'failed') action.span.setStatus({ code: SpanStatusCode.ERROR });
            if (action.model) action.finished = payload.timestamp_ms;
            else {
                action.span.end(payload.timestamp_ms);
                actions.delete(id);
            }
            return carrier(trace.setSpan(cx, action.span));
        }
        case 'usage': {
            const action = actions.get(id);
            if (!action) break;
            usageAttributes(action.span, payload.usage);
            action.span.end(payload.finished_at_ms ?? action.finished);
            actions.delete(id);
            break;
        }
        case 'branch_released': endBranch(branch, 'released'); break;
        case 'branch_failed': endBranch(branch, 'failed'); break;
        case 'turn_completed':
            if (branches.get(branch)?.role === 'authority') endBranch(branch, 'turn_finished');
            break;
    }
    if (['command_result', 'environment_requested', 'speculation_chance', 'recovery_loaded', 'recovery_saved'].includes(payload.message)) return carrier(cx);
    return undefined;
}

function usageAttributes(span: Span | undefined, usage: any) {
    for (const [attribute, value] of Object.entries({
        'gen_ai.usage.input_tokens': usage?.input,
        'gen_ai.usage.output_tokens': usage?.output,
        'ascout.usage.cache_read_tokens': usage?.cacheRead,
        'ascout.usage.cache_write_tokens': usage?.cacheWrite,
        'ascout.usage.total_tokens': usage?.totalTokens,
        'ascout.usage.cost': usage?.cost?.total,
    })) if (typeof value === 'number' && Number.isFinite(value)) span?.setAttribute(attribute, value);
}

function finishPolicy(span: Span, value: any) {
    if (typeof value === 'boolean') span.setAttribute('allowed', value);
    if (Array.isArray(value)) span.setAttribute('result_count', value.length);
    const allowed = ['Promote', 'Discard', 'Retain', 'Accept', 'Drop', 'compatible', 'incompatible', 'indeterminate', 'allow', 'block', 'continue', 'stop'];
    const result = value?.actions?.[0]?.intent ?? value?.verdict ?? value?.mode ?? value;
    if (typeof result === 'string' && allowed.includes(result)) span.setAttribute('result', result);
}

export function evaluate<T>(decision: string, path: string, run: () => T): T {
    if (!tracer) return run();
    const cx = active();
    if (!trace.getSpanContext(cx)) return run();
    const span = tracer.startSpan('ascout.policy.evaluate', { attributes: { decision, path } }, cx);
    const complete = (value: any) => { finishPolicy(span, value?.result ?? value); span.end(); return value; };
    const fail = (error: unknown): never => { span.setStatus({ code: SpanStatusCode.ERROR }); span.end(); throw error; };
    try {
        const value = context.with(trace.setSpan(cx, span), run);
        if (value instanceof Promise) return value.then(complete, fail) as T;
        return complete(value);
    } catch (error) { return fail(error); }
}

export function recordPolicyModelUsage(usage: unknown) {
    if (tracer) usageAttributes(trace.getActiveSpan(), usage);
}

export async function policyModel<T>(run: () => Promise<T>): Promise<T> {
    if (!tracer) return run();
    const cx = active();
    if (!trace.getSpanContext(cx)) return run();
    const span = tracer.startSpan('ascout.policy.provider', {}, cx);
    try {
        return await context.with(trace.setSpan(cx, span), run);
    } catch (error) {
        span.setStatus({ code: SpanStatusCode.ERROR });
        throw error;
    } finally { span.end(); }
}

export function command<T>(name: string, incoming: unknown, run: () => Promise<T>): Promise<T> {
    if (!tracer) return run();
    const parent = incoming ? fromCarrier(incoming) : active();
    if (!trace.getSpanContext(parent)) return run();
    return tracer.startActiveSpan('ascout.agent.command', { attributes: { command: name.slice(0, 64) } }, parent, async span => {
        try {
            const value = await run();
            const result = (value as any)?.status;
            if (statuses.has(result)) span.setAttribute('result', result);
            if (result === 'failed' || (value as any)?.message === 'branch_failed') span.setStatus({ code: SpanStatusCode.ERROR });
            return value;
        } finally { span.end(); }
    });
}

function closeIntervals(result: string) {
    for (const action of actions.values()) {
        if (action.finished === undefined) action.span.setAttribute('result', result);
        action.span.end(action.finished);
    }
    actions.clear();
    for (const branch of branches.keys()) endBranch(branch, result);
}

export async function shutdown(paused: boolean) {
    if (!provider) return;
    closeIntervals(paused ? 'paused' : 'stopped');
    let timer: ReturnType<typeof setTimeout> | undefined;
    let shutdownFailed = false;
    try {
        await Promise.race([
            provider.shutdown(),
            new Promise((_, reject) => { timer = setTimeout(reject, 1500); }),
        ]);
    } catch { shutdownFailed = true; }
    finally { clearTimeout(timer); }
    const pending = Math.max(0, ended - exported - failed);
    if (shutdownFailed || failed || pending) process.stderr.write(`ascout-pi: OTLP ended=${ended} exported=${exported} failed=${failed} dropped_or_pending=${pending} shutdown_failed=${shutdownFailed}\n`);
}

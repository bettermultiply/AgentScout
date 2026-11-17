import { randomUUID } from 'node:crypto';

type Report = (message: Record<string, unknown>) => Promise<void>;
type Kind = 'confirm' | 'select' | 'input' | 'editor';
type Pending = { branch: string; kind: Kind; resolve: (answer: any) => void };

/**
 * Explicit dismissal resolves instead of rejecting, matching Pi RPC's cancel
 * values: confirmation is `false`, selection/text is `undefined`. Silence is
 * not dismissal: an unanswered prompt remains pending without a timeout.
 */
function dismissed(kind: Kind): any {
    return kind === 'confirm' ? false : undefined;
}

/** Text-only extension UI; no Policy or product command semantics. */
export class Interaction {
    private pending = new Map<string, Pending>();
    private closed = new Set<string>();
    private shuttingDown = false;
    private report: Report;
    constructor(report: Report) { this.report = report; }
    /** End outstanding prompts after discard, interrupt, pause or shutdown. */
    cancel(branch?: string) {
        if (branch === undefined) this.shuttingDown = true;
        else this.closed.add(branch);
        for (const [id, pending] of [...this.pending]) {
            if (branch !== undefined && pending.branch !== branch) continue;
            pending.resolve(dismissed(pending.kind));
        }
    }
    resume(branch: string) { this.closed.delete(branch); this.shuttingDown = false; }
    /** `reason` only annotates control-plane cancellations in the protocol. */
    answer(branch: string, id: string, answer: unknown, cancelled: boolean, _reason?: string) {
        const pending = this.pending.get(id);
        if (!pending || pending.branch !== branch) return;
        pending.resolve(cancelled ? dismissed(pending.kind) : answer);
    }
    context(branch: string) {
        const request = (kind: Kind, title: string, fields: Record<string, unknown>, opts?: { signal?: AbortSignal }) => {
            if (this.shuttingDown || this.closed.has(branch) || opts?.signal?.aborted) {
                return Promise.resolve(dismissed(kind));
            }
            const prompt_id = randomUUID();
            return new Promise<any>(resolve => {
                const finish = (value: any) => {
                    if (!this.pending.delete(prompt_id)) return;
                    opts?.signal?.removeEventListener('abort', abort);
                    resolve(value);
                };
                const abort = () => {
                    finish(dismissed(kind));
                    void this.report({ message: 'ui_prompt_cancelled', branch_id: branch, prompt_id }).catch(() => {});
                };
                this.pending.set(prompt_id, { branch, kind, resolve: finish });
                opts?.signal?.addEventListener('abort', abort, { once: true });
                if (opts?.signal?.aborted) { abort(); return; }
                void this.report({ message: 'ui_prompt_requested', branch_id: branch, prompt_id, kind, title,
                    message_text: null, options: [], placeholder: null, prefill: null, ...fields })
                    // Delivery failure cancels the prompt; user silence does
                    // not run this path and never supplies a default answer.
                    .catch(() => finish(dismissed(kind)));
            });
        };
        const noop = () => {};
        return {
            confirm: (title: string, message: string, opts?: { signal?: AbortSignal }) => request('confirm', title, { message_text: message }, opts),
            select: (title: string, options: string[], opts?: { signal?: AbortSignal }) => request('select', title, { options }, opts),
            input: (title: string, placeholder?: string, opts?: { signal?: AbortSignal }) => request('input', title, { placeholder: placeholder ?? null }, opts),
            editor: (title: string, prefill?: string) => request('editor', title, { prefill: prefill ?? null }),
            custom: () => Promise.reject(Error('AScout does not support Pi custom UI components; use text prompts')),
            notify: (text: string, level = 'info') => { void this.report({ message: 'ui_notification', branch_id: branch, text, level }).catch(() => {}); },
            onTerminalInput: () => noop,
            setStatus: noop, setWorkingMessage: noop, setWorkingVisible: noop, setWorkingIndicator: noop,
            setHiddenThinkingLabel: noop, setWidget: noop, setFooter: noop, setHeader: noop, setTitle: noop,
            pasteToEditor: noop, setEditorText: noop, getEditorText: () => '', addAutocompleteProvider: noop,
            setEditorComponent: noop, getEditorComponent: () => undefined,
            getAllThemes: () => [], getTheme: () => undefined,
            setTheme: () => ({ success: false, error: 'AScout has no terminal theme' }),
            getToolsExpanded: () => false, setToolsExpanded: noop,
        };
    }
}

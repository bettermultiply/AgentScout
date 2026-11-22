export type PureDecision = {
    readonly program: { readonly argument: string; readonly body: readonly unknown[] };
    readonly policy_config: unknown;
};

const fallback = Symbol('evaluate in sandbox');
class Tuple {
    readonly values: Py[];
    constructor(values: Py[]) { this.values = values; }
}
class Float {
    readonly value: number;
    constructor(value: number) { this.value = value; }
}
type Dict = Map<string, Py>;
type Py = null | boolean | number | Float | string | Py[] | Tuple | Dict;
const isSequence = (value: Py): value is Py[] | Tuple => Array.isArray(value) || value instanceof Tuple;
const sequence = (value: Py[] | Tuple) => value instanceof Tuple ? value.values : value;
const pack = (like: Py[] | Tuple, values: Py[]) => like instanceof Tuple ? new Tuple(values) : values;
function fail(kind: string, message: string): never { throw Error(`${kind}: ${message}`); }
function str(value: Py): string {
    if (typeof value === 'string') return value;
    return fail('TypeError', 'expected a string');
}
function int(value: Py): number | undefined {
    if (typeof value === 'boolean') return Number(value);
    if (typeof value === 'number') return value;
}
function integer(value: Py): number { return int(value) ?? fail('TypeError', 'expected an integer'); }
function numeric(value: Py): number | undefined { return value instanceof Float ? value.value : int(value); }
function truth(value: Py): boolean {
    if (isSequence(value)) return sequence(value).length !== 0;
    if (value instanceof Map) return value.size !== 0;
    if (value instanceof Float) return value.value !== 0;
    return Boolean(value);
}
function items(value: Py): Py[] {
    if (isSequence(value)) return sequence(value);
    if (typeof value === 'string') return [...value];
    if (value instanceof Map) return [...value.keys()];
    return fail('TypeError', 'object is not iterable');
}
function lookup(value: Dict, key: Py): Py | undefined {
    if (typeof key === 'string') return value.get(key);
    if (Array.isArray(key) || key instanceof Map) fail('TypeError', 'unhashable key');
    if (key instanceof Tuple) throw fallback;
}
function equal(a: Py, b: Py): boolean {
    if (a === b) return true;
    const x = numeric(a), y = numeric(b);
    if (x !== undefined && y !== undefined) return x === y;
    if (isSequence(a) && isSequence(b) && (a instanceof Tuple) === (b instanceof Tuple)) {
        const left = sequence(a), right = sequence(b);
        return left.length === right.length && left.every((value, i) => equal(value, right[i]));
    }
    if (a instanceof Map && b instanceof Map) {
        return a.size === b.size && [...a].every(([key, value]) => b.has(key) && equal(value, b.get(key)!));
    }
    return false;
}
function order(a: Py, b: Py): number {
    const x = numeric(a), y = numeric(b);
    if (x !== undefined && y !== undefined) return Math.sign(x - y);
    if (typeof a === 'string' && typeof b === 'string') {
        if (a === b) return 0;
        const left = [...a], right = [...b];
        for (let i = 0; i < Math.min(left.length, right.length); i++) {
            if (left[i] !== right[i]) return Math.sign(left[i].codePointAt(0)! - right[i].codePointAt(0)!);
        }
        return Math.sign(left.length - right.length);
    }
    if (isSequence(a) && isSequence(b) && (a instanceof Tuple) === (b instanceof Tuple)) {
        const left = sequence(a), right = sequence(b);
        for (let i = 0; i < Math.min(left.length, right.length); i++) {
            if (!equal(left[i], right[i])) return order(left[i], right[i]);
        }
        return Math.sign(left.length - right.length);
    }
    return fail('TypeError', 'values cannot be ordered');
}
function compare(op: string, a: Py, b: Py): boolean {
    switch (op) {
        case 'Eq': return equal(a, b);
        case 'NotEq': return !equal(a, b);
        case 'Lt': return order(a, b) < 0;
        case 'LtE': return order(a, b) <= 0;
        case 'Gt': return order(a, b) > 0;
        case 'GtE': return order(a, b) >= 0;
        case 'Is': case 'IsNot':
            if (a === null || b === null || typeof a === 'boolean' || typeof b === 'boolean') return (a === b) === (op === 'Is');
            throw fallback;
        case 'In': case 'NotIn': {
            const contains = typeof b === 'string' ? b.includes(str(a)) : b instanceof Map
                ? lookup(b, a) !== undefined : items(b).some(value => equal(a, value));
            return contains === (op === 'In');
        }
        default: throw fallback;
    }
}

const space = (character: string) => /[\u0009-\u000D\u001C-\u0020\u0085\u00A0\u1680\u2000-\u200A\u2028\u2029\u202F\u205F\u3000]/u.test(character);
function method(name: string, obj: Py, args: Py[]): Py {
    if (obj instanceof Map) {
        if (name === 'get' && (args.length === 1 || args.length === 2)) {
            const value = lookup(obj, args[0]);
            return value === undefined ? args.length === 2 ? args[1] : null : value;
        }
        if (name === 'items' && !args.length) return [...obj].map(pair => new Tuple(pair));
        throw fallback;
    }
    if (typeof obj !== 'string') throw fallback;
    switch (name) {
        case 'lower':
            if (args.length || /[^\x00-\x7f]/.test(obj)) throw fallback;
            return obj.toLowerCase();
        case 'startswith': case 'endswith': {
            if (!args.length || args.length > 3) throw fallback;
            let window: string | undefined = obj;
            if (args.length > 1) {
                const chars = [...obj];
                const normalize = (index: number) => index < 0 ? Math.max(chars.length + index, 0) : index;
                const start = args[1] === null ? 0 : normalize(integer(args[1]));
                const end = args.length < 3 || args[2] === null ? chars.length : Math.min(chars.length, normalize(integer(args[2])));
                window = start <= end ? chars.slice(start, end).join('') : undefined;
            }
            const test = (value: Py) => {
                const prefix = str(value);
                return window !== undefined && (name === 'startswith' ? window.startsWith(prefix) : window.endsWith(prefix));
            };
            return args[0] instanceof Tuple ? args[0].values.some(test) : test(args[0]);
        }
        case 'strip': {
            if (args.length > 1) throw fallback;
            const characters = args.length && args[0] !== null ? new Set(str(args[0])) : null;
            const trim = characters ? (ch: string) => characters.has(ch) : space;
            const value = [...obj];
            let first = 0, last = value.length;
            while (first < last && trim(value[first])) first++;
            while (first < last && trim(value[last - 1])) last--;
            return value.slice(first, last).join('');
        }
        case 'split': {
            if (args.length > 2) throw fallback;
            const max = args.length === 2 ? integer(args[1]) : -1;
            const result: string[] = [];
            if (args.length && args[0] !== null) {
                const sep = str(args[0]);
                if (!sep) fail('ValueError', 'empty separator');
                let start = 0;
                while (max < 0 || result.length < max) {
                    const end = obj.indexOf(sep, start);
                    if (end < 0) break;
                    result.push(obj.slice(start, end));
                    start = end + sep.length;
                }
                result.push(obj.slice(start));
            } else {
                const value = [...obj];
                let start = 0;
                while (start < value.length) {
                    while (start < value.length && space(value[start])) start++;
                    if (start === value.length) break;
                    let end = start;
                    if (max >= 0 && result.length === max) end = value.length;
                    else while (end < value.length && !space(value[end])) end++;
                    result.push(value.slice(start, end).join(''));
                    start = end;
                }
            }
            return result;
        }
        default: throw fallback;
    }
}

const maxMessage = 1024 * 1024;
const isRecord = (value: unknown): value is Record<string, unknown> => !!value && typeof value === 'object' && !Array.isArray(value);
function validateData(decision: string, data: unknown, inputs: readonly string[]) {
    const required: Record<string, readonly string[]> = {
        'admit.opportunity_pass': ['op'],
        'admit.feasibility_pass': ['op', 'state', 'contract_context'],
        extent: ['process_state'], effect: ['effect_descriptor', 'context'],
        select: ['frontier_view'], launch: ['chance_context'], predict: ['chance_context','process_context'],
    };
    if (!Object.hasOwn(required, decision)) throw fallback;
    const fields = required[decision];
    if (!isRecord(data) || fields.some(key => key === 'frontier_view' ? !Array.isArray(data[key]) : !isRecord(data[key])) || Object.keys(data).some(key => !fields.includes(key) && !inputs.includes(key))) {
        throw Error(`Invalid ${decision} input`);
    }
    for (const path of inputs) {
        let value: unknown = data;
        for (const key of path.split('.')) {
            if (!isRecord(value) || !Object.hasOwn(value, key)) throw Error(`Missing Decision input ${path}`);
            value = value[key];
        }
        if (!path.includes('.') && !isRecord(value)) throw Error(`Invalid Decision input ${path}`);
    }
    if (decision === 'extent') {
        const state = data.process_state as Record<string, unknown>;
        const validMetric = (key: string) => ['steps_completed', 'phase'].includes(key) || inputs.includes(`process_state.${key}`);
        if (Object.keys(state).some(key => !validMetric(key)) || !Number.isInteger(state.steps_completed)
            || Number(state.steps_completed) < 0 || typeof state.phase !== 'string') throw Error('Invalid process progress');
        if (Object.hasOwn(state, 'active_time') && !(typeof state.active_time === 'number' && state.active_time >= 0)) throw Error('Invalid process active_time');
        for (const key of ['resource_counters', 'result_facts', 'semantic_facts']) {
            if (Object.hasOwn(state, key) && !isRecord(state[key])) throw Error(`Invalid process ${key}`);
        }
    }
    if (decision.startsWith('admit.')) {
        const op = data.op as Record<string, unknown>;
        if (typeof op.kind !== 'string' || typeof op.has_predictor !== 'boolean') throw Error('op must contain kind and the predictor fact');
    }
}
function validateResult(decision: string, result: unknown) {
    let valid = false;
    if (decision.startsWith('admit.')) valid = typeof result === 'boolean';
    if (decision === 'extent') valid = result === 'stop' || result === 'continue';
    if (decision === 'effect') valid = isRecord(result) && Object.keys(result).every(key => ['mode', 'reason'].includes(key))
        && (!Object.hasOwn(result, 'reason') || typeof result.reason === 'string')
        && ['allow', 'deny', 'block', 'default', 'stage', 'simulate'].includes(result.mode as string);
    if (decision === 'select') valid = Array.isArray(result) && result.every(id => typeof id === 'string');
    if (decision === 'launch') valid = typeof result === 'boolean';
    if (decision === 'predict') valid = isRecord(result) && (
        (result.abstain === true && Object.keys(result).every(key => ['abstain', 'reason'].includes(key))
            && (!Object.hasOwn(result, 'reason') || typeof result.reason === 'string'))
        || (Object.keys(result).length === 2 && result.generation === 'batch'
            && Array.isArray(result.proposals) && result.proposals.length > 0
            && result.proposals.every(item => isRecord(item) && Object.hasOwn(item, 'payload')
                && Object.keys(item).every(key => ['payload', 'provenance'].includes(key)))));
    if (!valid) throw Error(`Invalid ${decision} result`);
}

export function evaluatePureDecision(binding: PureDecision, decision: string, data: unknown, inputs: readonly string[], signal?: AbortSignal): { result: unknown } | undefined {
    if (signal?.aborted) throw Error('Policy evaluation cancelled');
    try {
        // The Python worker sees JSON after normal JS serialization, including omitted undefined fields.
        const encoded = JSON.stringify(data);
        if (encoded === undefined || Buffer.byteLength(encoded) > maxMessage - 128) throw fallback;
        const normalized: unknown = JSON.parse(encoded);
        validateData(decision, normalized, inputs);
        const result = new Interpreter().run(binding, normalized);
        validateResult(decision, result);
        return {result};
    } catch (error) {
        if (error === fallback) return undefined;
        throw error;
    }
}

class Interpreter {
    readonly scope = new Map<string, Py>();
    budget = 8192;

    spend(amount = 1) { if ((this.budget -= amount) < 0) throw fallback; }

    chargeValue(value: Py, depth = 0) {
        this.spend();
        if (depth > 32) throw fallback;
        if (typeof value === 'string') this.spend(Math.floor(value.length / 8));
        else if (isSequence(value)) {
            for (const item of sequence(value)) this.chargeValue(item, depth + 1);
        } else if (value instanceof Map) {
            for (const [key, item] of value) {
                this.spend(Math.floor(key.length / 8));
                this.chargeValue(item, depth + 1);
            }
        }
    }

    fromJson(value: unknown, depth = 0): Py {
        this.spend();
        if (depth > 32) fail('ValueError', 'input nesting exceeds 32');
        if (value === null || typeof value === 'boolean') return value;
        if (typeof value === 'string') {
            if (!value.isWellFormed()) throw fallback;
            this.spend(Math.floor(value.length / 8));
            return value;
        }
        if (typeof value === 'number') {
            if (!Number.isInteger(value)) return new Float(value);
            if (!Number.isSafeInteger(value)) throw fallback;
            return value;
        }
        if (Array.isArray(value)) return value.map(item => this.fromJson(item, depth + 1));
        if (isRecord(value)) return new Map(Object.entries(value).map(([key, item]) => {
            if (!key.isWellFormed()) throw fallback;
            this.spend(Math.floor(key.length / 8));
            return [key, this.fromJson(item, depth + 1)];
        }));
        throw fallback;
    }

    toJson(value: Py, depth = 0): unknown {
        this.spend();
        if (depth > 32) fail('ValueError', 'result nesting exceeds 32');
        if (value instanceof Tuple) fail('TypeError', 'result must contain only JSON values with bounded integers');
        if (value instanceof Map) return Object.fromEntries([...value].map(([key, item]) => [key, this.toJson(item, depth + 1)]));
        if (value instanceof Float) return value.value;
        if (Array.isArray(value)) return value.map(item => this.toJson(item, depth + 1));
        return value;
    }

    run(binding: PureDecision, data: unknown): unknown {
        this.scope.set(binding.program.argument, this.fromJson(data));
        this.scope.set('policy_config', this.eval(binding.policy_config));
        return this.toJson(this.block(binding.program.body)?.value ?? null);
    }

    block(body: readonly unknown[]): { value: Py } | undefined {
        for (const instruction of body) {
            this.spend();
            const [op, ...args] = instruction as unknown[];
            if (op === 'return') return {value: this.eval(args[0])};
            if (op === 'assign') this.scope.set(args[0] as string, this.eval(args[1]));
            else if (op === 'branch') {
                const result = this.block(args[truth(this.eval(args[0])) ? 1 : 2] as unknown[]);
                if (result) return result;
            } else throw fallback;
        }
    }

    arithmetic(op: string, a: Py, b: Py): Py {
        const x = numeric(a), y = numeric(b);
        if (x !== undefined && y !== undefined) {
            const result = op === 'Add' ? x + y : op === 'Sub' ? x - y : x * y;
            if (!Number.isFinite(result)) throw fallback;
            if (a instanceof Float || b instanceof Float) return new Float(result);
            if (!Number.isSafeInteger(result)) throw fallback;
            return result;
        }
        if (op === 'Add') {
            if (typeof a === 'string' && typeof b === 'string') {
                this.spend((a.length + b.length) / 8);
                return a + b;
            }
            if (isSequence(a) && isSequence(b) && (a instanceof Tuple) === (b instanceof Tuple)) {
                this.spend(sequence(a).length + sequence(b).length);
                return pack(a, [...sequence(a), ...sequence(b)]);
            }
        }
        if (op === 'Mult') {
            const count = int(b) ?? int(a);
            const value = int(b) !== undefined ? a : b;
            if (count !== undefined && (typeof value === 'string' || isSequence(value))) {
                const times = Math.max(count, 0);
                const length = typeof value === 'string' ? value.length : sequence(value).length;
                if (length === 0) return value;
                this.spend(times * (1 + length));
                if (typeof value === 'string') return value.repeat(times);
                const result: Py[] = [];
                for (let i = 0; i < times; i++) result.push(...sequence(value));
                return pack(value, result);
            }
        }
        return fail('TypeError', `unsupported operands for ${op}`);
    }

    comprehension(mode: string, target: string | string[], values: Py[], body: unknown, filters: unknown[]): Py {
        const names = typeof target === 'string' ? [target] : target;
        const saved = names.map(name => this.scope.get(name));
        const results: Py[] = [];
        try {
            for (const item of values) {
                this.spend();
                const parts = typeof target === 'string' ? [item] : items(item);
                if (parts.length !== names.length) fail('ValueError', 'wrong number of values to unpack');
                names.forEach((name, i) => this.scope.set(name, parts[i]));
                if (!filters.every(test => truth(this.eval(test)))) continue;
                const value = this.eval(body);
                if (mode === 'any' && truth(value)) return true;
                if (mode === 'all' && !truth(value)) return false;
                if (mode === 'list') results.push(value);
            }
            return mode === 'list' ? results : mode === 'all';
        } finally {
            names.forEach((name, i) => {
                if (saved[i] === undefined) this.scope.delete(name);
                else this.scope.set(name, saved[i]!);
            });
        }
    }

    eval(instruction: unknown): Py {
        this.spend();
        const [op, ...args] = instruction as unknown[];
        const read = (i: number) => this.eval(args[i]);
        switch (op) {
            case 'const': return this.fromJson(args[0]);
            case 'float': return new Float(args[0] as number);
            case 'var': {
                const value = this.scope.get(args[0] as string);
                if (value === undefined) fail('UnboundLocalError', String(args[0]));
                this.chargeValue(value);
                return value;
            }
            case 'list': return args.map(arg => this.eval(arg));
            case 'tuple': return new Tuple(args.map(arg => this.eval(arg)));
            case 'dict': return new Map((args as [string, unknown][]).map(([key, value]) => [key, this.eval(value)]));
            case 'if': return read(truth(read(0)) ? 1 : 2);
            case 'and': case 'or': {
                let value = read(0);
                for (let i = 1; i < args.length && truth(value) !== (op === 'or'); i++) value = read(i);
                return value;
            }
            case 'compare': {
                let left = read(0);
                for (const [operator, operand] of args.slice(1) as [string, unknown][]) {
                    const right = this.eval(operand);
                    if (!compare(operator, left, right)) return false;
                    left = right;
                }
                return true;
            }
            case 'Not': return !truth(read(0));
            case 'bool': return truth(read(0));
            case 'len': {
                const value = read(0);
                return value instanceof Map ? value.size : items(value).length;
            }
            case 'any': return items(read(0)).some(truth);
            case 'all': return items(read(0)).every(truth);
            case 'UAdd': case 'USub': {
                const value = read(0), n = numeric(value);
                if (n === undefined) fail('TypeError', 'expected a number');
                const result = op === 'USub' ? -n : n;
                return value instanceof Float ? new Float(result) : result;
            }
            case 'Add': case 'Sub': case 'Mult': return this.arithmetic(op, read(0), read(1));
            case 'method': {
                const name = args[0] as string, value = read(1);
                if (!(value instanceof Map ? ['get', 'items'].includes(name) : typeof value === 'string' && !['get', 'items'].includes(name))) {
                    fail('AttributeError', `object has no ${name} method`);
                }
                return method(name, value, args.slice(2).map(arg => this.eval(arg)));
            }
            case 'index': {
                const value = read(0), key = read(1);
                if (value instanceof Map) {
                    const result = lookup(value, key);
                    return result === undefined ? fail('KeyError', 'missing dictionary key') : result;
                }
                const index = integer(key), array = items(value);
                const position = index < 0 ? array.length + index : index;
                if (position < 0 || position >= array.length) fail('IndexError', 'index out of range');
                return array[position];
            }
            case 'slice': {
                const value = read(0), start = read(1), end = read(2);
                if (!(typeof value === 'string' || isSequence(value))) fail('TypeError', 'object cannot be sliced');
                const sliced = items(value).slice(start === null ? 0 : integer(start), end === null ? undefined : integer(end));
                return typeof value === 'string' ? sliced.join('') : pack(value, sliced);
            }
            case 'comprehension': return this.comprehension(args[0] as string, args[1] as string | string[], items(read(2)), args[3], args.slice(4));
            default: throw fallback;
        }
    }
}

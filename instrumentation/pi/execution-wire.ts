import type { Socket } from 'node:net';

const HEADER_LIMIT = 1024 * 1024;
const FRAME_SIZE = 64 * 1024;

export function operationBody(operation: string, args: Record<string, unknown>) {
    const metadata = { ...args };
    let parts: Uint8Array[] | undefined;
    if (operation === 'write_file' || (operation === 'exec' && args.input !== undefined)) {
        const key = operation === 'write_file' ? 'bytes' : 'input';
        const bytes = args[key];
        if (!(bytes instanceof Uint8Array)) throw Error(`${key} must be bytes`);
        parts = [bytes];
        delete metadata[key];
    } else if (operation === 'replace_text') {
        if (!Array.isArray(args.replacements)) throw Error('Replacements must be an array');
        const texts: Uint8Array[] = [];
        parts = texts;
        metadata.replacements = args.replacements.map((edit: Record<string, unknown>) => {
            if (typeof edit.old_text !== 'string' || typeof edit.new_text !== 'string') {
                throw Error('Replacement texts must be strings');
            }
            const oldText = Buffer.from(edit.old_text);
            const newText = Buffer.from(edit.new_text);
            texts.push(oldText, newText);
            return { old_length: oldText.length, new_length: newText.length, match: edit.match };
        });
    }
    return { metadata, parts };
}

export async function writeExecution(socket: Socket, metadata: object, parts?: Uint8Array[]) {
    const write = (bytes: Uint8Array) => new Promise<void>((resolve, reject) => {
        socket.write(bytes, error => error ? reject(error) : resolve());
    });
    const header = Buffer.from(JSON.stringify(parts ? {
        ...metadata, body_length: parts.reduce((length, part) => length + part.byteLength, 0),
    } : metadata) + '\n');
    if (header.length > HEADER_LIMIT) throw Error('Oversized execution header');
    await write(header);
    if (!parts) return;
    const frame = Buffer.alloc(4);
    for (const part of parts) {
        for (let offset = 0; offset < part.byteLength; offset += FRAME_SIZE) {
            const chunk = part.subarray(offset, offset + FRAME_SIZE);
            frame.writeUInt32BE(chunk.byteLength);
            await write(frame);
            await write(chunk);
        }
    }
    frame.writeUInt32BE(0);
    await write(frame);
}

export async function* executionMessages(socket: Socket) {
    const stream = socket.iterator({ destroyOnReturn: false });
    let buffer = Buffer.alloc(0);
    async function fill() {
        if (buffer.length) return true;
        const next = await stream.next();
        if (next.done) return false;
        buffer = next.value;
        return true;
    }
    async function exact(length: number) {
        const parts: Buffer[] = [];
        let remaining = length;
        while (remaining) {
            if (!await fill()) throw Error('Truncated execution body');
            const size = Math.min(remaining, buffer.length);
            parts.push(buffer.subarray(0, size));
            buffer = buffer.subarray(size);
            remaining -= size;
        }
        return parts.length === 1 ? parts[0] : Buffer.concat(parts, length);
    }
    try {
        while (await fill()) {
            const parts: Buffer[] = [];
            let size = 0;
            for (;;) {
                const end = buffer.indexOf(10);
                const length = end < 0 ? buffer.length : end + 1;
                size += length;
                if (size > HEADER_LIMIT) throw Error('Oversized execution header');
                parts.push(buffer.subarray(0, length));
                buffer = buffer.subarray(length);
                if (end >= 0) break;
                if (!await fill()) throw Error('Truncated execution header');
            }
            const metadata = JSON.parse(Buffer.concat(parts, size).toString('utf8'));
            const declared = metadata.body_length;
            if (declared === undefined) {
                yield { metadata, body: undefined };
                continue;
            }
            if (!Number.isSafeInteger(declared) || declared < 0) throw Error('Invalid execution body length');
            delete metadata.body_length;
            let remaining = declared;
            const chunks: Buffer[] = [];
            for (;;) {
                const length = (await exact(4)).readUInt32BE();
                if (!length) break;
                if (length > FRAME_SIZE || length > remaining) throw Error('Invalid execution frame length');
                chunks.push(await exact(length));
                remaining -= length;
            }
            if (remaining) throw Error('Incomplete declared execution body');
            yield { metadata, body: chunks.length === 1 ? chunks[0] : Buffer.concat(chunks, declared) };
        }
    } finally {
        await stream.return?.();
    }
}

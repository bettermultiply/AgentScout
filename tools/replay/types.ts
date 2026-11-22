export interface ContentPart {
  type?: string;
  text?: string;
  id?: string;
  toolCallId?: string;
  name?: string;
  arguments?: unknown;
  function?: { name?: string; arguments?: unknown };
}
export interface Message {
  role?: string;
  content?: string | ContentPart[] | null;
  tool_calls?: ContentPart[];
  toolCallId?: string;
  tool_call_id?: string;
  toolName?: string;
  stopReason?: string;
  errorMessage?: string;
}
export interface Entry {
  type: string;
  id: string;
  parentId?: string | null;
  message?: Message;
  customType?: string;
  content?: string | ContentPart[];
  display?: boolean;
  details?: { command_id?: string };
  timestamp?: string;
  firstKeptEntryId?: string;
  targetId?: string;
  replacement?: { content: string | ContentPart[] } | null;
  summary?: string;
  systemMessage?: Message;
}
export type ReplyPart = { type: 'text'; text: string } | { type: 'toolCall'; id?: string; name?: string; arguments: unknown };
export interface RecordingItem {
  key: string;
  reply?: ReplyPart[];
  stopReason?: 'aborted';
  branch_id?: string;
  session_entry_id?: string;
}

export function isObject(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === 'object' && !Array.isArray(value);
}

function isPart(value: unknown): value is ContentPart {
  if (!isObject(value)) return false;
  if (value.type !== undefined && typeof value.type !== 'string') return false;
  for (const key of ['text', 'id', 'toolCallId', 'name']) {
    if (value[key] !== undefined && typeof value[key] !== 'string') return false;
  }
  return value.function === undefined || (isObject(value.function) &&
    (value.function.name === undefined || typeof value.function.name === 'string'));
}

export function isMessage(value: unknown): value is Message {
  if (!isObject(value) || typeof value.role !== 'string') return false;
  // Pi's OpenAI provider uses null for assistant messages containing only tool calls.
  const toolOnly = value.role === 'assistant' && Array.isArray(value.tool_calls) && value.tool_calls.length > 0;
  if (value.content === null && !toolOnly) return false;
  if (value.content !== undefined && value.content !== null && typeof value.content !== 'string' &&
      !(Array.isArray(value.content) && value.content.every(isPart))) return false;
  if (value.tool_calls !== undefined && !(Array.isArray(value.tool_calls) && value.tool_calls.every(isPart))) return false;
  return ['toolCallId', 'tool_call_id', 'toolName', 'stopReason', 'errorMessage'].every(
    key => value[key] === undefined || typeof value[key] === 'string',
  );
}

export function parseRecording(value: unknown): RecordingItem[] {
  if (!Array.isArray(value)) throw Error('Recording must be an array');
  for (const item of value) {
    if (!isObject(item) || typeof item.key !== 'string') throw Error('Recording item requires a string key');
    if (item.stopReason === 'aborted') continue;
    if (item.stopReason !== undefined || !Array.isArray(item.reply) || !item.reply.every(part =>
      isPart(part) && (part.type === 'text' ? typeof part.text === 'string' :
        part.type === 'toolCall' && typeof part.id === 'string' && typeof part.name === 'string'),
    )) throw Error(`Invalid reply for recording key ${item.key}`);
  }
  return value as RecordingItem[];
}

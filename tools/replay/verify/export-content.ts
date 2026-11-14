// Different keys are independent queues; retain every item and same-key order.
import type { RecordingItem } from '../types.ts';
export function exportContent(records: RecordingItem[]) {
  const queues = new Map<string, Omit<RecordingItem, 'key' | 'session_entry_id'>[]>();
  for (const {key, branch_id, reply, stopReason} of records) {
    const queue = queues.get(key) ?? [];
    queue.push({branch_id, reply, ...(stopReason ? {stopReason} : {})});
    queues.set(key, queue);
  }
  return [...queues].sort(([a], [b]) => a.localeCompare(b));
}

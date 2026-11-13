// Different keys are independent queues; retain every item and same-key order.
export function exportContent(records) {
  const queues = new Map();
  for (const {key, branch_id, reply} of records) {
    const queue = queues.get(key) ?? [];
    queue.push({branch_id, reply});
    queues.set(key, queue);
  }
  return [...queues].sort(([a], [b]) => a.localeCompare(b));
}

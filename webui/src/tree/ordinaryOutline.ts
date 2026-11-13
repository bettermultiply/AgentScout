import type { ClosedPath, Cubic } from '../geometry/curves'

/** Flat root and tip closure, without a connection fillet. */
export function ordinaryOutline([a, b]: [Cubic[], Cubic[]]): ClosedPath {
  return { start: a[0].start, segments: [
    ...a.map(c => ({ kind: 'cubic' as const, c1: c.c1, c2: c.c2, end: c.end })),
    { kind: 'line', end: b[b.length - 1].end },
    ...[...b].reverse().map(c => ({ kind: 'cubic' as const, c1: c.c2, c2: c.c1, end: c.start })),
    { kind: 'line', end: a[0].start },
  ] }
}

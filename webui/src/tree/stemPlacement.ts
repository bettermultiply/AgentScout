export interface StemNeighbor { distance: number; side: -1 | 1; angleDeg: number }
export interface StemCandidate {
  id: string
  distance: number
  range: number
  preferred: -1 | 1
  base: number
  angles: number[]
}

export function placeStems(stems: StemCandidate[], children: StemNeighbor[]): Map<string, { side: -1 | 1; tilt: number }> {
  const placed: StemNeighbor[] = []
  const result = new Map<string, { side: -1 | 1; tilt: number }>()
  for (const stem of [...stems].sort((a, b) => a.distance - b.distance)) {
    const choices = ([-1, 1] as const).map(side => {
      const nearby = [...children, ...placed].filter(n => n.side === side && Math.abs(n.distance - stem.distance) <= stem.range)
      const choices = stem.angles.map(tilt => ({ tilt, clearance: nearby.reduce((gap, n) => Math.min(gap, Math.abs(90 - tilt - n.angleDeg)), Infinity) }))
      choices.sort((a, b) => (a.clearance === b.clearance ? 0 : a.clearance > b.clearance ? -1 : 1) || Math.abs(a.tilt - stem.base) - Math.abs(b.tilt - stem.base) || a.tilt - b.tilt)
      return { side, ...choices[0] }
    })
    choices.sort((a, b) => (a.clearance === b.clearance ? 0 : a.clearance > b.clearance ? -1 : 1) || Number(b.side === stem.preferred) - Number(a.side === stem.preferred))
    const best = choices[0]
    result.set(stem.id, { side: best.side, tilt: best.tilt })
    placed.push({ distance: stem.distance, side: best.side, angleDeg: 90 - best.tilt })
  }
  return result
}

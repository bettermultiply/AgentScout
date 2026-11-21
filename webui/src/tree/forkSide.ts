type Side = -1 | 1
interface Fork { distance: number; heading: number }

/** Place forks in parent-distance order, preserving observation order at ties. */
export function placeForks<T extends Fork>(forks: T[], angleMin: number, angleMax: number) {
  const min = Math.min(angleMin, angleMax)
  const max = Math.max(angleMin, angleMax)
  const ordered = [...forks].sort((a, b) => a.distance - b.distance)
  const last = { [-1]: -Infinity, [1]: -Infinity }
  const placements: Array<T & { side: Side; angleDeg: number; angle: number }> = []
  for (let start = 0; start < ordered.length;) {
    const { distance, heading } = ordered[start]
    let end = start + 1
    while (end < ordered.length && ordered[end].distance === distance) end++
    const headingDeg = heading * 180 / Math.PI
    const limit = { [-1]: Math.min(max, 90 + headingDeg), [1]: Math.min(max, 90 - headingDeg) }
    const count = { [-1]: 0, [1]: 0 }
    const sides: Side[] = []
    for (let i = start; i < end; i++) {
      // Prefer the side with more room since its last fork. With equal spacing,
      // prefer the side with more room before reaching a horizontal heading.
      const preferred: Side = last[-1] < last[1] || (last[-1] === last[1] && headingDeg > 0) ? -1 : 1
      const side = limit[preferred] + 1e-9 >= min ? preferred : preferred === 1 ? -1 : 1
      sides.push(side)
      count[side]++
      last[side] = distance
    }
    const rank = { [-1]: 0, [1]: 0 }
    for (let i = start; i < end; i++) {
      const side = sides[i - start]
      const share = (++rank[side] - .5) / count[side]
      const angleDeg = min + share * Math.max(0, limit[side] - min)
      placements.push({ ...ordered[i], side, angleDeg, angle: heading + side * angleDeg * Math.PI / 180 })
    }
    start = end
  }
  return placements
}

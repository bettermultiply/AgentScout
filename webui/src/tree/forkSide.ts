export function chooseForkSide(input: {
  heading: number
  angleDeg: number
  preferred: -1 | 1
}): { side: -1 | 1; angle: number } {
  const { heading, preferred } = input
  const a = input.angleDeg * Math.PI / 180
  const angle = heading + preferred * a
  if (Math.abs(angle) <= Math.PI / 2 + 1e-9) return { side: preferred, angle }
  const other: -1 | 1 = preferred === 1 ? -1 : 1
  const opposite = heading + other * a
  if (Math.abs(opposite) <= Math.PI / 2 + 1e-9) return { side: other, angle: opposite }
  console.warn('Neither fork side has a downward initial direction', input)
  return { side: preferred, angle: Math.max(-Math.PI / 2, Math.min(Math.PI / 2, angle)) }
}

export function makeWidthAt(
  items: { d: number; n: number }[],
  total: number,
  startWidth: number,
  tuning: { widthExponent: number; minTrunkWidth: number },
): (distance: number) => number {
  const width = (remaining: number) => Math.max(tuning.minTrunkWidth,
    startWidth * Math.pow(total ? remaining / total : 1, tuning.widthExponent))
  const anchors = [{ d: 0, w: width(total) }]
  let remaining = total
  for (const item of items) {
    remaining -= item.n
    const anchor = { d: item.d, w: width(Math.max(0, remaining)) }
    if (Math.abs(anchor.d - anchors[anchors.length - 1].d) < 1e-9) anchors[anchors.length - 1] = anchor
    else anchors.push(anchor)
  }
  return (distance: number): number => {
    if (distance <= anchors[0].d) return anchors[0].w
    if (distance >= anchors[anchors.length - 1].d) return anchors[anchors.length - 1].w
    let index = 1
    while (anchors[index].d < distance) index++
    const before = anchors[index - 1], after = anchors[index]
    const t = (distance - before.d) / (after.d - before.d)
    return before.w + (after.w - before.w) * (t * t * (3 - 2 * t))
  }
}

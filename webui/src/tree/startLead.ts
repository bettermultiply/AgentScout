type Point = { x: number; y: number }

export function startLead(points: Point[], widths: number[], initialAngle: number, ratio: number): { points: Point[]; widths: number[] } {
  if (points.length < 2 || ratio <= 0) return { points, widths }
  const dx = points[1].x - points[0].x, dy = points[1].y - points[0].y
  const length = Math.hypot(dx, dy)
  const directionX = length > 0 ? dx / length : Math.sin(initialAngle)
  const directionY = length > 0 ? dy / length : Math.cos(initialAngle)
  const distance = ratio * widths[0]
  return {
    points: [{ x: points[0].x - directionX * distance, y: points[0].y - directionY * distance }, ...points],
    widths: [widths[0], ...widths],
  }
}

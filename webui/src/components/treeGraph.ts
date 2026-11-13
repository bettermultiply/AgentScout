import { authorityPath, type TaskState } from '../fold'
import type { BranchFlowNode } from '../components/BranchNode'
import { layoutTree, type TreeTuning } from '../tree'

export interface TreeGraph {
  nodes: BranchFlowNode[]
  latestNodeId: string | null
  error: string | null
}

/** Turn the folded task state into React Flow nodes. */
export function buildTree(task: TaskState | null, tuning: TreeTuning): TreeGraph {
  if (!task) return { nodes: [], latestNodeId: null, error: null }
  const path = authorityPath(task)
  let geometry: ReturnType<typeof layoutTree>
  try {
    geometry = layoutTree(task, tuning)
  } catch (error) {
    return {
      nodes: [],
      latestNodeId: null,
      error: error instanceof Error ? error.message : String(error),
    }
  }

  const nodes: BranchFlowNode[] = []
  const depth = new Map<string, number>()
  const branchDepth = (branchId: string): number => {
    const cached = depth.get(branchId)
    if (cached !== undefined) return cached
    const parentId = task.branches[branchId]?.parentBranchId
    const value = parentId ? branchDepth(parentId) + 1 : 0
    depth.set(branchId, value)
    return value
  }
  let latestNodeId: string | null = null
  let latestSequence = -1

  for (const branchId of task.branchOrder) {
    const branch = task.branches[branchId]
    const layout = geometry.get(branchId)
    if (!branch || !layout) continue

    const isAuthorityPath = path.has(branchId)
    const isCurrentAuthority = task.authority === branchId

    nodes.push({
      id: `branch:${branchId}`,
      type: 'branch',
      position: { x: layout.x, y: layout.y },
      width: layout.width,
      height: layout.height,
      data: {
        branch,
        geometry: layout,
        tuning,
        isAuthorityPath,
        isCurrentAuthority,
        childForks: layout.chanceCircles.map(circle => ({ ...circle, chance: task.chances[circle.chanceId] ?? null })),
      },
      draggable: false,
      selectable: false,
      focusable: false,
      // Parent outlines protect the uninterrupted trunk silhouette.
      zIndex: 100 - branchDepth(branchId),
    })

    if (branch.lastEventSequence > latestSequence) {
      latestSequence = branch.lastEventSequence
      latestNodeId = `branch:${branchId}`
    }
  }

  return { nodes, latestNodeId, error: null }
}

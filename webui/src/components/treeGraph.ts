import { authorityPath, type TaskState } from '../fold'
import type { BranchFlowNode } from '../components/BranchNode'
import { layoutTree, type TreeTuning } from '../tree'

export interface TreeGraph {
  nodes: BranchFlowNode[]
  latestNodeId: string | null
  error: string | null
}

export function buildTree(task: TaskState | null, tuning: TreeTuning, layout = layoutTree, previous?: TreeGraph): TreeGraph {
  if (!task) return { nodes: [], latestNodeId: null, error: null }
  const path = authorityPath(task)
  let geometry: ReturnType<typeof layoutTree>
  try {
    geometry = layout(task, tuning)
  } catch (error) {
    return {
      nodes: [],
      latestNodeId: null,
      error: error instanceof Error ? error.message : String(error),
    }
  }

  const nodes: BranchFlowNode[] = []
  const previousNodes = new Map(previous?.nodes.map(node => [node.id, node]))
  const drawn = new Set<string>()
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

    const id = `branch:${branchId}`
    const old = previousNodes.get(id)
    const zIndex = 100 - branchDepth(branchId)
    const oldData = old?.data
    const childForks = oldData?.geometry === layout && oldData.childForks.every((fork, i) =>
      fork.chance === (task.chances[layout.chanceCircles[i].chanceId] ?? null))
      ? oldData.childForks
      : layout.chanceCircles.map(circle => ({ ...circle, chance: task.chances[circle.chanceId] ?? null }))
    const data = oldData && oldData.branch === branch && oldData.geometry === layout && oldData.tuning === tuning &&
      oldData.isAuthorityPath === isAuthorityPath && oldData.isCurrentAuthority === isCurrentAuthority && oldData.childForks === childForks
      ? oldData : { branch, geometry: layout, tuning, isAuthorityPath, isCurrentAuthority, childForks }

    nodes.push(old && old.data === data && old.zIndex === zIndex ? old : {
      id,
      type: 'branch',
      position: old && old.position.x === layout.x && old.position.y === layout.y ? old.position : { x: layout.x, y: layout.y },
      width: layout.width,
      height: layout.height,
      data,
      draggable: false,
      selectable: false,
      focusable: false,
      zIndex,
    })
    drawn.add(branchId)

    if (branch.lastEventSequence > latestSequence) {
      latestSequence = branch.lastEventSequence
      latestNodeId = `branch:${branchId}`
    }
  }

  for (const chance of Object.values(task.chances)) {
    if (chance.lastEventSequence > latestSequence && drawn.has(chance.parentBranchId)) {
      latestSequence = chance.lastEventSequence
      latestNodeId = `branch:${chance.parentBranchId}`
    }
  }

  const sameNodes = previous && nodes.length === previous.nodes.length && nodes.every((node, i) => node === previous.nodes[i])
  if (sameNodes && previous.error === null && previous.latestNodeId === latestNodeId) return previous
  return { nodes: sameNodes ? previous.nodes : nodes, latestNodeId, error: null }
}

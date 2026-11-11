import { useMemo } from 'react'
import {
  Background,
  BackgroundVariant,
  BaseEdge,
  Controls,
  ReactFlow,
  ReactFlowProvider,
  useReactFlow,
  type Edge,
  type EdgeProps,
  type NodeTypes,
} from '@xyflow/react'
import '@xyflow/react/dist/style.css'
import {
  authorityPath,
  isActiveCandidate,
  type BranchState,
  type TaskState,
} from '../fold'
import { LatestIcon } from '../icons'
import {
  NODE_HALF_WIDTH,
  NODE_WIDTH,
  layoutTree,
} from '../treeLayout'
import { BranchNode, type BranchFlowNode } from './BranchNode'

const nodeTypes: NodeTypes = { branch: BranchNode }

/**
 * A fork leaves the parent trunk downward-outward, then turns vertical into
 * the child head. The default bezier leaves the trunk vertically first, which
 * makes a wide, wire-like S over the long horizontal run between lanes.
 */
function ForkEdge({
  sourceX,
  sourceY,
  targetX,
  targetY,
  style,
}: EdgeProps) {
  const dx = targetX - sourceX
  const sign = dx >= 0 ? 1 : -1
  // Drop a few pixels while still hidden by the parent trunk, then run just
  // below the fork slot until the edge is clear of the parent's own twigs and
  // leaves. Only then does it dive outward into the child head. Leaving at the
  // fork row would cut through calls the parent made after the fork.
  const startY = sourceY + 6
  const out = Math.min(Math.abs(dx) * 0.6, 52)
  const path = [
    `M ${sourceX} ${sourceY}`,
    `L ${sourceX} ${startY}`,
    `C ${sourceX + sign * out} ${startY}`,
    `${targetX - sign * 6} ${targetY - Math.max(14, (targetY - startY) * 0.35)}`,
    `${targetX} ${targetY}`,
  ].join(' ')
  return <BaseEdge path={path} style={style} />
}

const edgeTypes = { fork: ForkEdge }

interface TreeGraph {
  nodes: BranchFlowNode[]
  edges: Edge[]
  latestNodeId: string | null
}

function isAbandoned(status: BranchState['status']): boolean {
  return status === 'discarded' || status === 'cleaned'
}

function forkStyle(branch: BranchState, authority: boolean) {
  if (isAbandoned(branch.status)) {
    return {
      stroke: 'var(--muted)',
      strokeWidth: 3,
      strokeDasharray: '5 4',
      opacity: 0.3,
    }
  }
  if (authority) {
    return { stroke: 'var(--accent)', strokeWidth: 7, opacity: 0.9 }
  }
  return { stroke: 'var(--accent)', strokeWidth: 5.5, opacity: 0.45 }
}

function buildTree(task: TaskState | null): TreeGraph {
  if (!task) return { nodes: [], edges: [], latestNodeId: null }
  const path = authorityPath(task)
  const geometry = layoutTree(task)
  const nodes: BranchFlowNode[] = []
  const edges: Edge[] = []
  let latestNodeId: string | null = null
  let latestSequence = -1

  for (const branchId of task.branchOrder) {
    const branch = task.branches[branchId]
    const layout = geometry.get(branchId)
    if (!branch || !layout) continue

    const isAuthorityPath = path.has(branchId)
    const isCurrentAuthority = task.authority === branchId
    let authorityForkY: number | null = null
    if (isAuthorityPath && !isCurrentAuthority) {
      const authorityChild = task.branchOrder.find(
        (childId) =>
          path.has(childId) &&
          task.branches[childId]?.parentBranchId === branchId,
      )
      if (authorityChild) {
        authorityForkY = layout.forks.get(authorityChild) ?? null
      }
    }

    nodes.push({
      id: `branch:${branchId}`,
      type: 'branch',
      position: {
        x: layout.x - NODE_HALF_WIDTH,
        y: layout.y,
      },
      width: NODE_WIDTH,
      height: layout.height,
      data: {
        branch,
        chance: branch.chanceId
          ? task.chances[branch.chanceId] ?? null
          : null,
        geometry: layout,
        isAuthorityPath,
        isCurrentAuthority,
        isLive: isActiveCandidate(branch),
        authorityForkY,
      },
      draggable: false,
      selectable: false,
      focusable: false,
      zIndex: 10,
    })

    if (branch.lastEventSequence > latestSequence) {
      latestSequence = branch.lastEventSequence
      latestNodeId = `branch:${branchId}`
    }
  }

  for (const branchId of task.branchOrder) {
    const branch = task.branches[branchId]
    if (!branch?.parentBranchId) continue
    if (!task.branches[branch.parentBranchId]) continue
    const forkY = geometry.get(branch.parentBranchId)?.forks.get(branchId)
    if (forkY === undefined) continue
    edges.push({
      id: `fork:${branchId}`,
      source: `branch:${branch.parentBranchId}`,
      sourceHandle: `fork:${branchId}`,
      target: `branch:${branchId}`,
      targetHandle: 'in',
      // The child head sits FORK_DROP below the parent's fork slot. A smooth
      // curve from the slot to that head is the only line crossing lanes.
      type: 'fork',
      style: forkStyle(branch, path.has(branchId)),
      zIndex: 0,
    })
  }

  return { nodes, edges, latestNodeId }
}

function TreeCanvas({ graph }: { graph: TreeGraph }) {
  const { fitView } = useReactFlow()

  return (
    <div className="tree-canvas">
      <ReactFlow
        nodes={graph.nodes}
        edges={graph.edges}
        nodeTypes={nodeTypes}
        edgeTypes={edgeTypes}
        fitView
        fitViewOptions={{ padding: 0.3, maxZoom: 1 }}
        minZoom={0.08}
        maxZoom={1.8}
        nodesDraggable={false}
        nodesConnectable={false}
        nodesFocusable={false}
        elementsSelectable={false}
        onlyRenderVisibleElements
        panOnScroll
        zoomOnDoubleClick={false}
        proOptions={{ hideAttribution: true }}
        aria-label="投机树"
      >
        <Background
          variant={BackgroundVariant.Dots}
          gap={20}
          size={1}
          color="var(--tree-grid)"
        />
        <Controls showInteractive={false} />
      </ReactFlow>
      <button
        type="button"
        className="tree-latest-button"
        onClick={() => {
          if (!graph.latestNodeId) return
          void fitView({
            nodes: [{ id: graph.latestNodeId }],
            duration: 320,
            padding: 2.4,
            maxZoom: 1.1,
          })
        }}
        disabled={!graph.latestNodeId}
        title="定位到最新事件"
      >
        <LatestIcon />
        最新
      </button>
    </div>
  )
}

export function SpeculationTree({ task }: { task: TaskState | null }) {
  const graph = useMemo(() => buildTree(task), [task])

  if (graph.nodes.length === 0) {
    return (
      <div className="tree-empty">
        <h3>还没有 Branch 事件</h3>
        <p>控制面创建 Branch 后，投机树会在这里自上而下生长。</p>
        <p className="muted">
          树枝头部、模型小枝条和工具叶子可以点击进入该 Branch 的只读详情。
        </p>
      </div>
    )
  }

  return (
    <ReactFlowProvider>
      <TreeCanvas graph={graph} />
    </ReactFlowProvider>
  )
}

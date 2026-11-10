import { useMemo, useRef, useState } from 'react'
import {
  Background,
  BackgroundVariant,
  Controls,
  ReactFlow,
  ReactFlowProvider,
  useReactFlow,
  type Edge,
  type NodeTypes,
} from '@xyflow/react'
import '@xyflow/react/dist/style.css'
import {
  authorityPath,
  branchTokenTotal,
  isActiveCandidate,
  latestModel,
  roleLabel,
  speculationKindLabel,
  statusLabel,
  type BranchState,
  type TaskState,
} from '../fold'
import { formatTokens } from '../format'
import { LatestIcon } from '../icons'
import { useAppStore } from '../store'
import {
  HEADER_SIZE,
  LANE_WIDTH,
  LEAF_SIZE,
  TRUNK_WIDTH,
  groupBranchEvents,
  layoutTree,
  leafDetailLines,
  leafKindLabel,
  type BranchGeometry,
} from '../treeLayout'
import { BranchNode, type BranchHeaderFlowNode } from './BranchNode'
import { BranchTrunk, type BranchTrunkFlowNode } from './BranchTrunk'
import { LeafNode, type LeafFlowNode } from './LeafNode'

const nodeTypes: NodeTypes = {
  branchHeader: BranchNode,
  branchTrunk: BranchTrunk,
  leaf: LeafNode,
}

type TreeFlowNode = BranchHeaderFlowNode | BranchTrunkFlowNode | LeafFlowNode

interface TreeGraph {
  nodes: TreeFlowNode[]
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
      strokeWidth: 1.4,
      strokeDasharray: '4 4',
      opacity: 0.55,
    }
  }
  if (authority) {
    return { stroke: 'var(--accent)', strokeWidth: 3 }
  }
  return {
    stroke: 'var(--accent)',
    strokeWidth: 1.5,
    strokeDasharray: '5 4',
    opacity: 0.8,
  }
}

function buildTree(task: TaskState | null): TreeGraph {
  if (!task) return { nodes: [], edges: [], latestNodeId: null }
  const path = authorityPath(task)
  const geometry = layoutTree(task)
  const nodes: TreeFlowNode[] = []
  const edges: Edge[] = []
  let latestNodeId: string | null = null
  let latestSequence = -1

  task.branchOrder.forEach((branchId, lane) => {
    const branch = task.branches[branchId]
    const layout: BranchGeometry | undefined = geometry.get(branchId)
    if (!branch || !layout) return

    const authority = path.has(branchId)
    const live = isActiveCandidate(branch)
    const laneX = lane * LANE_WIDTH + LANE_WIDTH / 2

    nodes.push({
      id: `header:${branchId}`,
      type: 'branchHeader',
      position: { x: laneX - HEADER_SIZE / 2, y: layout.headerY },
      data: {
        branch,
        isAuthorityPath: authority,
        isLive: live,
        leafCount: layout.groups.length,
      },
      draggable: false,
      selectable: false,
      focusable: true,
      zIndex: 20,
    })

    nodes.push({
      id: `trunk:${branchId}`,
      type: 'branchTrunk',
      position: { x: laneX - TRUNK_WIDTH / 2, y: layout.trunkTop },
      data: {
        branch,
        isAuthorityPath: authority,
        isLive: live,
        height: layout.trunkHeight,
        forks: [...layout.forks].map(([childId, offset]) => ({
          childId,
          offset,
        })),
      },
      draggable: false,
      selectable: false,
      focusable: false,
      zIndex: 5,
    })

    layout.groups.forEach((group, index) => {
      nodes.push({
        id: group.id,
        type: 'leaf',
        position: {
          x: laneX - LEAF_SIZE / 2,
          y: layout.leafCenters[index] - LEAF_SIZE / 2,
        },
        data: {
          branch,
          group,
          isAuthorityPath: authority,
        },
        draggable: false,
        selectable: false,
        focusable: true,
        zIndex: 10,
      })
    })

    if (branch.lastEventSequence > latestSequence) {
      latestSequence = branch.lastEventSequence
      latestNodeId =
        layout.groups.length > 0
          ? layout.groups[layout.groups.length - 1].id
          : `header:${branchId}`
    }
  })

  for (const branchId of task.branchOrder) {
    const branch = task.branches[branchId]
    if (!branch?.parentBranchId) continue
    if (!task.branches[branch.parentBranchId]) continue
    edges.push({
      id: `fork:${branchId}`,
      source: `trunk:${branch.parentBranchId}`,
      sourceHandle: `fork:${branchId}`,
      target: `header:${branchId}`,
      targetHandle: 'header-in',
      type: 'default',
      style: forkStyle(branch, path.has(branchId)),
    })
  }

  return { nodes, edges, latestNodeId }
}

function NodeHoverCard({
  node,
  x,
  y,
  below,
}: {
  node: TreeFlowNode
  x: number
  y: number
  below: boolean
}) {
  const branch = node.data.branch
  const tokens = branchTokenTotal(branch)
  const model = latestModel(branch)

  return (
    <div
      className={`node-tooltip node-tooltip-float${below ? ' tooltip-below' : ''}`}
      style={{ left: x, top: y }}
      role="tooltip"
    >
      <span className="node-tooltip-id">{branch.branchId}</span>
      <span>
        {roleLabel(branch.role)} · {statusLabel(branch.status)}
      </span>
      <span>模型：{model ?? '未上报'}</span>
      <span>
        token：{tokens.known ? formatTokens(tokens.value) : '未上报'}
        {tokens.known && !tokens.complete ? '（部分）' : ''}
      </span>
      {node.type === 'leaf' ? (
        <>
          <span className="node-tooltip-section">
            {leafKindLabel(node.data.group.kind)}
            {node.data.group.events.length > 1
              ? ` · 连续 ${node.data.group.events.length} 条`
              : ''}
          </span>
          <span>
            事件 #{node.data.group.firstSequence}
            {node.data.group.lastSequence !== node.data.group.firstSequence
              ? `–#${node.data.group.lastSequence}`
              : ''}
          </span>
          {leafDetailLines(node.data.group).map((line) => (
            <span key={line}>{line}</span>
          ))}
        </>
      ) : (
        <span className="node-tooltip-section">
          {groupBranchEvents(branch).length} 个叶子 · {branch.events.length} 个事件
        </span>
      )}
      <span>投机：{speculationKindLabel(branch.speculationKind)}</span>
      <span className="node-tooltip-hint">点击查看分支详情</span>
    </div>
  )
}

function TreeCanvas({ graph }: { graph: TreeGraph }) {
  const openBranch = useAppStore((store) => store.openBranch)
  const { fitView } = useReactFlow<TreeFlowNode, Edge>()
  const canvas = useRef<HTMLDivElement>(null)
  const [hover, setHover] = useState<{
    node: TreeFlowNode
    x: number
    y: number
    below: boolean
  } | null>(null)

  return (
    <div className="tree-canvas" ref={canvas}>
      <ReactFlow<TreeFlowNode, Edge>
        nodes={graph.nodes}
        edges={graph.edges}
        nodeTypes={nodeTypes}
        onNodeClick={(_, node) => {
          setHover(null)
          openBranch(node.data.branch.branchId)
        }}
        onNodeMouseEnter={(event, node) => {
          const rect = canvas.current?.getBoundingClientRect()
          const left = rect?.left ?? 0
          const right = rect?.right ?? window.innerWidth
          const top = rect?.top ?? 0
          setHover({
            node,
            x: Math.min(Math.max(event.clientX, left + 150), right - 150),
            y: event.clientY,
            below: event.clientY - top < 130,
          })
        }}
        onNodeMouseLeave={() => setHover(null)}
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
      {hover ? (
        <NodeHoverCard
          node={hover.node}
          x={hover.x}
          y={hover.y}
          below={hover.below}
        />
      ) : null}
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
        <p className="muted">点击树枝头部或叶子可以查看该 Branch 的只读详情。</p>
      </div>
    )
  }

  return (
    <ReactFlowProvider>
      <TreeCanvas graph={graph} />
    </ReactFlowProvider>
  )
}

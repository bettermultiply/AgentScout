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
import { BranchNode, type BranchFlowNode } from './BranchNode'

const LANE_WIDTH = 104
const ROW_HEIGHT = 44

const nodeTypes: NodeTypes = { branchEvent: BranchNode }

interface LanePoint {
  id: string
  sequence: number
}

interface BranchChain {
  nodes: BranchFlowNode[]
  edges: Edge[]
  points: LanePoint[]
  first: LanePoint | null
  last: LanePoint | null
}

interface TreeGraph {
  nodes: BranchFlowNode[]
  edges: Edge[]
  latestNodeId: string | null
}

function edgeStyle(authority: boolean) {
  return {
    stroke: authority ? 'var(--accent)' : 'var(--tree-edge)',
    strokeWidth: authority ? 2.8 : 1.4,
    strokeDasharray: authority ? undefined : '4 4',
  }
}

// Branch objects are immutable: a Branch gets a new object only when one of its
// own events arrives. Reusing chains for untouched Branches keeps the React
// Flow update local when the stream grows.
const chainCache = new WeakMap<BranchState, Map<string, BranchChain>>()

function buildChain(
  branch: BranchState,
  lane: number,
  authority: boolean,
): BranchChain {
  const cacheKey = `${lane}:${authority ? 1 : 0}`
  let entries = chainCache.get(branch)
  if (!entries) {
    entries = new Map()
    chainCache.set(branch, entries)
  }
  const cached = entries.get(cacheKey)
  if (cached) return cached

  const nodes: BranchFlowNode[] = []
  const edges: Edge[] = []
  const points: LanePoint[] = []
  let previous: LanePoint | null = null
  // A streamed reply emits one conversation_updated per chunk. Chunks are one
  // continuous conversation step, not one tree node each: keep the existing
  // node and update it in place until another event type separates the run.
  let previousWasConversation = false

  for (const event of branch.events) {
    const conversation = event.event === 'conversation_updated'
    if (conversation && previousWasConversation && nodes.length > 0) {
      const latest = nodes[nodes.length - 1]
      nodes[nodes.length - 1] = {
        ...latest,
        data: { ...latest.data, event },
      }
      continue
    }

    const id = `${branch.branchId}::${event.sequence}`
    const point = { id, sequence: event.sequence }
    points.push(point)
    nodes.push({
      id,
      type: 'branchEvent',
      position: { x: lane * LANE_WIDTH, y: event.sequence * ROW_HEIGHT },
      data: {
        branch,
        event,
        isAuthorityPath: authority,
        isLatest: event.sequence === branch.lastEventSequence,
      },
      draggable: false,
      selectable: false,
      focusable: true,
    })
    if (previous) {
      edges.push({
        id: `branch:${branch.branchId}:${event.sequence}`,
        source: previous.id,
        target: id,
        type: 'smoothstep',
        style: edgeStyle(authority),
      })
    }
    previous = point
    previousWasConversation = conversation
  }

  if (nodes.length > 0) {
    const last = nodes[nodes.length - 1]
    nodes[nodes.length - 1] = {
      ...last,
      data: { ...last.data, isLatest: true },
    }
  }

  const chain: BranchChain = {
    nodes,
    edges,
    points,
    first: points[0] ?? null,
    last: points.at(-1) ?? null,
  }
  entries.set(cacheKey, chain)
  return chain
}

function buildTree(task: TaskState | null): TreeGraph {
  if (!task) return { nodes: [], edges: [], latestNodeId: null }
  const path = authorityPath(task)
  const chains = new Map<string, BranchChain>()
  const nodes: BranchFlowNode[] = []
  const edges: Edge[] = []
  let latestNodeId: string | null = null
  let latestSequence = -1

  task.branchOrder.forEach((branchId, lane) => {
    const branch = task.branches[branchId]
    if (!branch) return
    const inPath = path.has(branchId)
    const chain = buildChain(branch, lane, inPath)
    chains.set(branchId, chain)
    nodes.push(...chain.nodes)
    edges.push(...chain.edges)
    if (chain.last && branch.lastEventSequence > latestSequence) {
      latestSequence = branch.lastEventSequence
      latestNodeId = chain.last.id
    }
  })

  for (const branchId of task.branchOrder) {
    const branch = task.branches[branchId]
    if (!branch?.parentBranchId) continue
    const parent = chains.get(branch.parentBranchId)
    const target = chains.get(branchId)?.first
    if (!parent?.points.length || !target) continue
    const source =
      [...parent.points]
        .reverse()
        .find((point) => point.sequence <= branch.createdAtSequence) ??
      parent.points[0]
    edges.push({
      id: `fork:${branchId}`,
      source: source.id,
      target: target.id,
      type: 'smoothstep',
      style: edgeStyle(path.has(branch.parentBranchId) && path.has(branchId)),
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
  node: BranchFlowNode
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
      <span>
        事件 #{node.data.event.sequence} · {speculationKindLabel(branch.speculationKind)}
      </span>
      <span className="node-tooltip-hint">点击查看分支详情</span>
    </div>
  )
}

function TreeCanvas({ graph }: { graph: TreeGraph }) {
  const openBranch = useAppStore((store) => store.openBranch)
  const { fitView } = useReactFlow<BranchFlowNode, Edge>()
  const canvas = useRef<HTMLDivElement>(null)
  const [hover, setHover] = useState<{
    node: BranchFlowNode
    x: number
    y: number
    below: boolean
  } | null>(null)

  return (
    <div className="tree-canvas" ref={canvas}>
      <ReactFlow<BranchFlowNode, Edge>
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
        <p className="muted">点击节点可以查看该 Branch 的只读详情。</p>
      </div>
    )
  }

  return (
    <ReactFlowProvider>
      <TreeCanvas graph={graph} />
    </ReactFlowProvider>
  )
}

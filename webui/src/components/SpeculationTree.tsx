import { useEffect, useMemo, useRef, useState } from 'react'
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
  authorityStateLabel,
  branchTokenTotal,
  chanceStageLabel,
  isActiveCandidate,
  latestModel,
  roleLabel,
  speculationKindLabel,
  statusLabel,
  type BranchState,
  type ChanceState,
  type TaskState,
} from '../fold'
import { formatTokens } from '../format'
import { LatestIcon } from '../icons'
import { useAppStore } from '../store'
import {
  HEADER_SIZE,
  TOOL_LEAF_OFFSET,
  TOOL_LEAF_SIZE,
  TRUNK_WIDTH,
  TWIG_HEIGHT,
  TWIG_WIDTH,
  callDetailLines,
  callKindLabel,
  layoutTree,
  settlementLines,
  settlementSummary,
  type BranchGeometry,
} from '../treeLayout'
import { BranchNode, type BranchHeaderFlowNode } from './BranchNode'
import { BranchTrunk, type BranchTrunkFlowNode } from './BranchTrunk'
import { ModelTwigNode, type ModelTwigFlowNode } from './ModelTwigNode'
import { ToolLeafNode, type ToolLeafFlowNode } from './ToolLeafNode'

const nodeTypes: NodeTypes = {
  branchHeader: BranchNode,
  branchTrunk: BranchTrunk,
  toolLeaf: ToolLeafNode,
  modelTwig: ModelTwigNode,
}

type TreeFlowNode =
  | BranchHeaderFlowNode
  | BranchTrunkFlowNode
  | ToolLeafFlowNode
  | ModelTwigFlowNode

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
      strokeWidth: 1.2,
      strokeDasharray: '4 4',
      opacity: 0.5,
    }
  }
  if (authority) {
    return { stroke: 'var(--accent)', strokeWidth: 2, opacity: 0.8 }
  }
  return {
    stroke: 'var(--accent)',
    strokeWidth: 1.3,
    strokeDasharray: '5 4',
    opacity: 0.75,
  }
}

function axisX(layout: BranchGeometry, slotIndex: number): number {
  if (layout.trunkHeight === 0) return layout.headerX
  const offset = layout.slotCenters[slotIndex] - layout.trunkTop
  return layout.headerX + (offset / layout.trunkHeight) * layout.trunkDx
}

function buildTree(task: TaskState | null): TreeGraph {
  if (!task) return { nodes: [], edges: [], latestNodeId: null }
  const path = authorityPath(task)
  const geometry = layoutTree(task)
  const nodes: TreeFlowNode[] = []
  const edges: Edge[] = []
  let latestNodeId: string | null = null
  let latestSequence = -1

  for (const branchId of task.branchOrder) {
    const branch = task.branches[branchId]
    const layout = geometry.get(branchId)
    if (!branch || !layout) continue

    const authority = path.has(branchId)
    const live = isActiveCandidate(branch)
    const chance = branch.chanceId
      ? task.chances[branch.chanceId] ?? null
      : null

    nodes.push({
      id: `header:${branchId}`,
      type: 'branchHeader',
      position: {
        x: layout.headerX - HEADER_SIZE / 2,
        y: layout.headerY,
      },
      data: {
        branch,
        chance,
        isAuthorityPath: authority,
        isLive: live,
        callCount: layout.calls.reduce(
          (total, group) => total + group.events.length,
          0,
        ),
      },
      draggable: false,
      selectable: false,
      focusable: true,
      zIndex: 20,
    })

    nodes.push({
      id: `trunk:${branchId}`,
      type: 'branchTrunk',
      position: {
        x: layout.headerX - TRUNK_WIDTH / 2,
        y: layout.trunkTop,
      },
      data: {
        branch,
        isAuthorityPath: authority,
        isLive: live,
        height: layout.trunkHeight,
        dx: layout.trunkDx,
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

    layout.calls.forEach((group, index) => {
      const x = axisX(layout, index)
      const y = layout.slotCenters[index]
      const side = layout.callSides[index]
      if (group.kind === 'tool') {
        nodes.push({
          id: group.id,
          type: 'toolLeaf',
          position: {
            x: x + side * TOOL_LEAF_OFFSET - TOOL_LEAF_SIZE / 2,
            y: y - TOOL_LEAF_SIZE / 2,
          },
          data: { branch, group, isAuthorityPath: authority, side },
          draggable: false,
          selectable: false,
          focusable: true,
          zIndex: 10,
        })
      } else {
        nodes.push({
          id: group.id,
          type: 'modelTwig',
          position: {
            x: side > 0 ? x : x - TWIG_WIDTH,
            y: y - TWIG_HEIGHT / 2,
          },
          data: { branch, group, isAuthorityPath: authority, direction: side },
          draggable: false,
          selectable: false,
          focusable: true,
          zIndex: 10,
        })
      }
      if (group.lastSequence > latestSequence) {
        latestSequence = group.lastSequence
        latestNodeId = group.id
      }
    })

    if (branch.lastEventSequence > latestSequence) {
      latestSequence = branch.lastEventSequence
      latestNodeId = `header:${branchId}`
    }
  }

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
      // Straight edges plus one shared header row per sibling group guarantee
      // that an edge cannot pass through another Branch head.
      type: 'straight',
      style: forkStyle(branch, path.has(branchId)),
      zIndex: 0,
    })
  }

  return { nodes, edges, latestNodeId }
}

function chanceLines(chance: ChanceState | null): string[] {
  if (!chance) return []
  return [
    `Chance 工具：${chance.toolName || '未上报'}`,
    `Chance 阶段：${chanceStageLabel(chance.stage)}`,
    `权威状态：${authorityStateLabel(chance.authorityState)}`,
    `父 Chance：${chance.parentChanceId ?? '未上报'}`,
  ]
}

function NodeHoverCard({
  node,
  x,
  y,
  below,
  onEnter,
  onLeave,
}: {
  node: TreeFlowNode
  x: number
  y: number
  below: boolean
  onEnter: () => void
  onLeave: () => void
}) {
  const branch = node.data.branch
  const tokens = branchTokenTotal(branch)
  const model = latestModel(branch)
  const callNode =
    node.type === 'toolLeaf' || node.type === 'modelTwig' ? node : null
  const chance =
    node.type === 'branchHeader' ? node.data.chance : null
  const calls = callNode ? callDetailLines(callNode.data.group) : []
  const settlements = settlementLines(branch)

  return (
    <div
      className={`node-tooltip node-tooltip-float${below ? ' tooltip-below' : ''}`}
      style={{ left: x, top: y }}
      role="tooltip"
      onMouseEnter={onEnter}
      onMouseLeave={onLeave}
    >
      <span className="node-tooltip-id">{branch.branchId}</span>
      <span>
        {roleLabel(branch.role)} · {statusLabel(branch.status)}
      </span>
      {callNode ? (
        <>
          <span className="node-tooltip-section">
            {callKindLabel(callNode.data.group.kind)}
            {callNode.data.group.events.length > 1
              ? ` · 连续 ${callNode.data.group.events.length} 条`
              : ''}
          </span>
          <span className="node-tooltip-id">
            事件 #{callNode.data.group.firstSequence}
            {callNode.data.group.lastSequence !==
            callNode.data.group.firstSequence
              ? `–#${callNode.data.group.lastSequence}`
              : ''}
          </span>
          <div className="node-tooltip-calls">
            {calls.map((call) => (
              <span className="node-tooltip-call" key={call.sequence}>
                #{call.sequence} {call.text}
              </span>
            ))}
          </div>
          <span className="node-tooltip-hint">点击定位并高亮详情中的调用</span>
        </>
      ) : (
        <>
          <span>模型：{model ?? '未上报'}</span>
          <span>
            token：{tokens.known ? formatTokens(tokens.value) : '未上报'}
            {tokens.known && !tokens.complete ? '（部分）' : ''}
          </span>
          <span>投机原因：{speculationKindLabel(branch.speculationKind)}</span>
          {branch.chanceId ? (
            <>
              <span className="node-tooltip-id">Chance：{branch.chanceId}</span>
              {chance ? (
                chanceLines(chance).map((line) => (
                  <span key={line}>{line}</span>
                ))
              ) : (
                <span>Chance 详情：未上报</span>
              )}
            </>
          ) : (
            <span>Chance：无（初始权威）</span>
          )}
          {settlementSummary(branch) ? (
            <span className="node-tooltip-section">
              {settlements.map((line) => (
                <span className="node-tooltip-call" key={line}>
                  {line}
                </span>
              ))}
            </span>
          ) : null}
          <span className="node-tooltip-section">
            {branch.modelUsages.length + branch.tools.length} 次调用 ·{' '}
            {branch.events.length} 个事件
          </span>
          <span className="node-tooltip-hint">点击进入分支详情顶部</span>
        </>
      )}
    </div>
  )
}

function TreeCanvas({ graph }: { graph: TreeGraph }) {
  const openBranch = useAppStore((store) => store.openBranch)
  const { fitView } = useReactFlow<TreeFlowNode, Edge>()
  const canvas = useRef<HTMLDivElement>(null)
  const closeTimer = useRef<number | null>(null)
  const [hover, setHover] = useState<{
    node: TreeFlowNode
    x: number
    y: number
    below: boolean
  } | null>(null)

  const cancelClose = () => {
    if (closeTimer.current !== null) {
      window.clearTimeout(closeTimer.current)
      closeTimer.current = null
    }
  }

  const scheduleClose = () => {
    cancelClose()
    closeTimer.current = window.setTimeout(() => {
      closeTimer.current = null
      setHover(null)
    }, 180)
  }

  useEffect(() => cancelClose, [])

  return (
    <div className="tree-canvas" ref={canvas}>
      <ReactFlow<TreeFlowNode, Edge>
        nodes={graph.nodes}
        edges={graph.edges}
        nodeTypes={nodeTypes}
        onNodeClick={(_, node) => {
          cancelClose()
          setHover(null)
          if (node.type === 'branchHeader') {
            openBranch(node.data.branch.branchId)
          } else if (node.type === 'toolLeaf' || node.type === 'modelTwig') {
            openBranch(node.data.branch.branchId, {
              kind: node.data.group.kind,
              actionId: node.data.group.events[0]?.action_id ?? null,
              sequences: node.data.group.events.map((event) => event.sequence),
            })
          }
        }}
        onNodeMouseEnter={(event, node) => {
          cancelClose()
          const rect = canvas.current?.getBoundingClientRect()
          const left = rect?.left ?? 0
          const right = rect?.right ?? window.innerWidth
          const top = rect?.top ?? 0
          setHover({
            node,
            x: Math.min(Math.max(event.clientX, left + 170), right - 170),
            y: event.clientY,
            below: event.clientY - top < 150,
          })
        }}
        onNodeMouseLeave={scheduleClose}
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
          onEnter={cancelClose}
          onLeave={scheduleClose}
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
        <p className="muted">
          树枝头部和调用节点可以点击进入该 Branch 的只读详情。
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

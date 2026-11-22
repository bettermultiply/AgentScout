import { useEffect, useMemo, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import {
  Background,
  BackgroundVariant,
  Controls,
  ReactFlow,
  ReactFlowProvider,
  getNodesBounds,
  useReactFlow,
  useStoreApi,
  type NodeTypes,
} from '@xyflow/react'
import '@xyflow/react/dist/style.css'
import { statusLabel, type TaskState } from '../fold'
import { LatestIcon } from '../icons'
import { callDetailLines, type TreeTuning } from '../tree'
import { buildTree, type TreeGraph } from './treeGraph'
import { useTreeTuning } from '../useTreeTuning'
import { BranchNode } from './BranchNode'
import { TreeTuningPanel } from './TreeTuningPanel'
import { useAppStore } from '../store'

const nodeTypes: NodeTypes = { branch: BranchNode }
const minZoom = 0.008

type PointerPosition = { x: number; y: number }
type Hover = PointerPosition & { branchId: string; callId: string | null; chanceId: string | null }

function hitInk(target: EventTarget | null, point: PointerPosition): Hover | null {
  if (!(target instanceof SVGElement)) return null
  const branchId = target.closest('.tree-branch')?.getAttribute('data-branch-id')
  if (branchId === undefined || branchId === null) return null
  return { ...point, branchId,
    callId: target.closest('[data-tree-call]')?.getAttribute('data-tree-call') ?? null,
    chanceId: target.getAttribute('data-chance-id'),
  }
}

function sameInk(a: Hover | null, b: Hover | null) {
  return a?.branchId === b?.branchId && a?.callId === b?.callId && a?.chanceId === b?.chanceId
}

function TreeCanvas({ graph, tuning, onTuning }: { graph: TreeGraph; tuning: TreeTuning; onTuning: (next: TreeTuning) => void }) {
  const { fitView, setViewport, viewportInitialized } = useReactFlow()
  const flowStore = useStoreApi()
  const canvasRef = useRef<HTMLDivElement>(null)
  const initialFitHandled = useRef(false)
  const pointer = useRef<PointerPosition | null>(null)
  const moving = useRef(false)
  const [hover, setHover] = useState<Hover | null>(null)
  const [open, setOpen] = useState(() => new URLSearchParams(window.location.search).has('tune'))
  const nodes = graph.nodes
  const branchNodes = useMemo(() => new Map(nodes.map(node => [node.data.branch.branchId, node])), [nodes])
  const hovered = hover ? branchNodes.get(hover.branchId)?.data : undefined
  const hoverGroup = hovered?.geometry.calls.find(call => call.id === hover?.callId)
  const hoverFork = hovered?.childForks.find(fork => fork.chance?.chanceId === hover?.chanceId)
  const activeBranch = hovered?.branch.branchId
  const displayNodes = useMemo(() => {
    if (activeBranch === undefined) return nodes
    const children = new Map<string, string[]>()
    for (const { data: { branch } } of nodes) {
      if (branch.parentBranchId === null) continue
      const siblings = children.get(branch.parentBranchId) ?? []
      siblings.push(branch.branchId)
      children.set(branch.parentBranchId, siblings)
    }
    const pending = [activeBranch], active = new Set<string>()
    while (pending.length > 0) {
      const branchId = pending.pop()!
      if (active.has(branchId)) continue
      active.add(branchId)
      for (const child of children.get(branchId) ?? []) pending.push(child)
    }
    return nodes.map(node => active.has(node.data.branch.branchId)
      ? { ...node, className: 'tree-subtree-active', zIndex: (node.zIndex ?? 0) + nodes.length }
      : node)
  }, [nodes, activeBranch])

  function setHit(next: Hover | null) {
    setHover(current => sameInk(current, next) ? current : next)
  }

  function refreshHit() {
    const point = pointer.current
    if (!point || moving.current) return
    const target = document.elementFromPoint(point.x, point.y)
    setHit(canvasRef.current?.contains(target) ? hitInk(target, point) : null)
  }

  useEffect(() => {
    const updateZoom = (zoom: number) => canvasRef.current?.style.setProperty('--tree-zoom', String(zoom))
    updateZoom(flowStore.getState().transform[2])
    return flowStore.subscribe((state, previous) => {
      if (state.transform[2] !== previous.transform[2]) updateZoom(state.transform[2])
    })
  }, [flowStore])

  useEffect(() => {
    // Recheck the hit only on observation/tuning changes to avoid restacking feedback.
    const frame = requestAnimationFrame(refreshHit)
    return () => cancelAnimationFrame(frame)
  }, [nodes])

  useEffect(() => {
    const canvas = canvasRef.current
    if (initialFitHandled.current || !viewportInitialized || !canvas) return

    const bounds = getNodesBounds(nodes)
    const fit = () => {
      if (initialFitHandled.current) return true
      const { clientWidth: width, clientHeight: height } = canvas
      if (width === 0 || height === 0) return false
      const zoom = Math.max(minZoom, Math.min(1,
        Math.max(1, width - 88) / bounds.width,
        Math.max(1, height - 100) / bounds.height,
      ))
      initialFitHandled.current = true
      void setViewport({
        zoom,
        x: width / 2 - (bounds.x + bounds.width / 2) * zoom,
        y: height / 2 - (bounds.y + bounds.height / 2) * zoom,
      })
      return true
    }

    if (fit()) return
    const observer = new ResizeObserver(() => { if (fit()) observer.disconnect() })
    observer.observe(canvas)
    return () => observer.disconnect()
  }, [nodes, setViewport, viewportInitialized])

  return (
    <div className="tree-canvas" ref={canvasRef} data-tree-hover={activeBranch}
      onPointerMoveCapture={event => {
        if (event.pointerType === 'touch') return
        const { clientX: x, clientY: y } = event
        if (pointer.current?.x === x && pointer.current.y === y) return
        pointer.current = { x, y }
        if (!moving.current) setHit(hitInk(event.target, pointer.current))
      }}
      onPointerLeave={() => { pointer.current = null; setHit(null) }}>
      <ReactFlow
        nodes={displayNodes}
        onMoveStart={() => { moving.current = true; setHit(null) }}
        onMoveEnd={() => { moving.current = false; refreshHit() }}
        edges={[]}
        nodeTypes={nodeTypes}
        minZoom={minZoom}
        maxZoom={4}
        nodesDraggable={false}
        nodesConnectable={false}
        nodesFocusable={false}
        elementsSelectable={false}
        onlyRenderVisibleElements={false}
        panOnScroll
        zoomOnDoubleClick={false}
        proOptions={{ hideAttribution: true }}
        aria-label="投机树"
      >
        <Background variant={BackgroundVariant.Dots} gap={20} size={1} color="var(--tree-grid)" />
        <Controls showInteractive={false} />
      </ReactFlow>
      {!open ? <button
        type="button"
        className="tree-tune-toggle"
        onClick={() => setOpen(value => !value)}
        aria-expanded={open}
        aria-controls="tree-tuning-panel"
      >
        调参
      </button> : null}
      {open ? <TreeTuningPanel tuning={tuning} onChange={onTuning} onClose={() => setOpen(false)} /> : null}
      <button
        type="button"
        className="tree-latest-button"
        onClick={() => {
          if (!graph.latestNodeId) return
          void fitView({ nodes: [{ id: graph.latestNodeId }], duration: 320, padding: 2.4, maxZoom: 1.1 })
        }}
        disabled={!graph.latestNodeId}
        title="定位到最新事件"
      >
        <LatestIcon />
        最新
      </button>
      {hover && hovered ? createPortal(
        <div className="node-tooltip node-tooltip-float tooltip-below" role="tooltip"
          style={{ left: Math.max(0, Math.min(hover.x, window.innerWidth - 350)), top: hover.y + 10 }}>
          <span>{hovered.branch.branchId} · {statusLabel(hovered.branch.status)}</span>
          {hoverFork?.chance ? <span>Chance {hoverFork.chance.chanceId} · {hoverFork.chance.toolName ?? '未上报'}</span> : null}
          {hoverFork && hoverFork.candidateBranchIds.length > 1 ? <>
            <span>候选 Branch（点击枝或叶茎查看）</span>
            {hoverFork.candidateBranchIds.map(id => <span key={id}>{id}</span>)}
          </> : null}
          {hoverGroup ? callDetailLines(hoverGroup).map(c => <span key={c.sequence}>#{c.sequence} {c.text}</span>)
            : <span>{hovered.branch.modelUsages.length + hovered.branch.tools.length} 次调用</span>}
        </div>, document.body) : null}
    </div>
  )
}

export function SpeculationTree({ task }: { task: TaskState | null }) {
  const [tuning, setTuning] = useTreeTuning()
  const storedGraph = useAppStore(store => store.treeGraph)
  const selectedTask = useAppStore(store => store.selectedTaskId ? store.state.tasks[store.selectedTaskId] : null)
  const graph = task === selectedTask ? storedGraph : buildTree(task, tuning)

  if (graph.error) {
    return (
      <div className="tree-empty">
        <h3>投机树绘制失败</h3>
        <p>{graph.error}</p>
      </div>
    )
  }

  if (graph.nodes.length === 0) {
    return (
      <div className="tree-empty">
        <h3>还没有 Branch 事件</h3>
        <p>control plane 创建 Branch 后，投机树会在这里自上而下生长。</p>
        <p className="muted">枝头、模型叶茎和工具叶片可以点击进入该 Branch 的只读详情。</p>
      </div>
    )
  }

  return (
    <ReactFlowProvider key={task?.taskId}>
      <TreeCanvas graph={graph} tuning={tuning} onTuning={setTuning} />
    </ReactFlowProvider>
  )
}

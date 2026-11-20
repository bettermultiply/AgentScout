import { useEffect, useRef, useState } from 'react'
import {
  Background,
  BackgroundVariant,
  Controls,
  ReactFlow,
  ReactFlowProvider,
  getNodesBounds,
  useReactFlow,
  type NodeTypes,
} from '@xyflow/react'
import '@xyflow/react/dist/style.css'
import type { TaskState } from '../fold'
import { LatestIcon } from '../icons'
import { type TreeTuning } from '../tree'
import { buildTree, type TreeGraph } from './treeGraph'
import { useTreeTuning } from '../useTreeTuning'
import { BranchNode } from './BranchNode'
import { TreeTuningPanel } from './TreeTuningPanel'
import { useAppStore } from '../store'

const nodeTypes: NodeTypes = { branch: BranchNode }
const minZoom = 0.008

function TreeCanvas({ graph, tuning, onTuning }: { graph: TreeGraph; tuning: TreeTuning; onTuning: (next: TreeTuning) => void }) {
  const { fitView, setViewport, viewportInitialized } = useReactFlow()
  const canvasRef = useRef<HTMLDivElement>(null)
  const initialFitHandled = useRef(false)
  const [open, setOpen] = useState(() => new URLSearchParams(window.location.search).has('tune'))
  const nodes = graph.nodes

  useEffect(() => {
    const canvas = canvasRef.current
    if (initialFitHandled.current || !viewportInitialized || !canvas) return

    // Node rectangles already contain the full ink, including stems without leaves.
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
    <div className="tree-canvas" ref={canvasRef}>
      <ReactFlow
        nodes={nodes}
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

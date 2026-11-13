import { useEffect, useMemo, useState } from 'react'
import {
  Background,
  BackgroundVariant,
  Controls,
  ReactFlow,
  ReactFlowProvider,
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

const nodeTypes: NodeTypes = { branch: BranchNode }

function TreeCanvas({ graph, tuning, onTuning }: { graph: TreeGraph; tuning: TreeTuning; onTuning: (next: TreeTuning) => void }) {
  const { fitView, getViewport, setViewport } = useReactFlow()
  const [open, setOpen] = useState(() => new URLSearchParams(window.location.search).has('tune'))
  const nodes = graph.nodes
  // Refit when the set of Branches changes, not while tuning geometry.
  const fitKey = nodes.map(node => node.id).join('|')

  useEffect(() => {
    let cancelled = false
    const timer = window.setTimeout(() => {
      const canvas = document.querySelector('.tree-canvas')?.getBoundingClientRect()
      const rects = [...document.querySelectorAll('.tree-trunk, .tree-empty-bud, .tree-trunk-cap, .tree-leaf-visual, .tree-chance, .tree-leaf-petiole')]
        .map(element => element.getBoundingClientRect())
      if (!canvas || !rects.length || cancelled) return
      const left = Math.min(...rects.map(rect => rect.left))
      const right = Math.max(...rects.map(rect => rect.right))
      const top = Math.min(...rects.map(rect => rect.top))
      const bottom = Math.max(...rects.map(rect => rect.bottom))
      const current = getViewport()
      const factor = Math.min((canvas.width - 88) / (right - left), (canvas.height - 100) / (bottom - top))
      const target = Math.min(1, current.zoom * factor)
      void setViewport({
        zoom: target,
        x: canvas.width / 2 - ((left + right) / 2 - canvas.left - current.x) / current.zoom * target,
        y: canvas.height / 2 - ((top + bottom) / 2 - canvas.top - current.y) / current.zoom * target,
      })
    }, 100)
    return () => { cancelled = true; window.clearTimeout(timer) }
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [fitKey, getViewport, setViewport])

  return (
    <div className="tree-canvas">
      <ReactFlow
        nodes={nodes}
        edges={[]}
        nodeTypes={nodeTypes}
        fitView
        fitViewOptions={{ padding: 0.12, maxZoom: 1 }}
        minZoom={0.008}
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
  const graph = useMemo(() => buildTree(task, tuning), [task, tuning])

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
        <p>控制面创建 Branch 后，投机树会在这里自上而下生长。</p>
        <p className="muted">枝头、模型叶茎和工具叶片可以点击进入该 Branch 的只读详情。</p>
      </div>
    )
  }

  return (
    <ReactFlowProvider>
      <TreeCanvas graph={graph} tuning={tuning} onTuning={setTuning} />
    </ReactFlowProvider>
  )
}

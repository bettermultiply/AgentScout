import { useEffect } from 'react'
import { useAppStore } from '../store'
import { shortId } from '../format'
import { BackIcon } from '../icons'
import { BranchDetails } from './BranchDetails'
import { Legend } from './Legend'
import { SpeculationPanelHeader } from './SpeculationPanelHeader'
import { SpeculationTree } from './SpeculationTree'
import { StatsBar } from './StatsBar'

export function SpeculationPanel() {
  const state = useAppStore((store) => store.state)
  const selectedTaskId = useAppStore((store) => store.selectedTaskId)
  const panelView = useAppStore((store) => store.panelView)
  const selectedBranchId = useAppStore((store) => store.selectedBranchId)
  const showTree = useAppStore((store) => store.showTree)
  const setPanelOpen = useAppStore((store) => store.setPanelOpen)

  const task = selectedTaskId ? state.tasks[selectedTaskId] ?? null : null
  const branch = selectedBranchId ? task?.branches[selectedBranchId] ?? null : null
  const detailBranch = panelView === 'detail' ? branch : null
  const detailChance =
    detailBranch?.chanceId ? task?.chances[detailBranch.chanceId] ?? null : null

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== 'Escape') return
      const target = event.target
      if (
        target instanceof HTMLElement &&
        (target.tagName === 'TEXTAREA' ||
          target.tagName === 'INPUT' ||
          target.tagName === 'SELECT')
      ) {
        return
      }
      if (panelView === 'detail') {
        showTree()
      } else {
        setPanelOpen(false)
      }
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [panelView, showTree, setPanelOpen])

  return (
    <aside className="speculation-panel" id="speculation-panel" aria-label="投机面板">
      <SpeculationPanelHeader
        onClose={() => setPanelOpen(false)}
        taskId={selectedTaskId}
      />
      <StatsBar task={task} />
      <div className="panel-toolbar">
        {detailBranch ? (
          <>
            <button type="button" className="icon-button" onClick={showTree}>
              <BackIcon />
              返回树
            </button>
            <span className="panel-toolbar-title">
              分支详情 · <code>{shortId(detailBranch.branchId)}</code>
            </span>
          </>
        ) : (
          <span className="panel-toolbar-title">
            投机树
            {task ? (
              <span className="muted"> · {task.branchOrder.length} 个 Branch</span>
            ) : null}
          </span>
        )}
        <span className="panel-toolbar-spacer" />
        <Legend />
      </div>
      <div className={detailBranch ? 'panel-content panel-content-detail' : 'panel-content'}>
        {detailBranch ? (
          <BranchDetails branch={detailBranch} chance={detailChance} />
        ) : (
          <SpeculationTree task={task} />
        )}
      </div>
    </aside>
  )
}

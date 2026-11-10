import { shortId } from '../format'
import { CloseIcon } from '../icons'

export function SpeculationPanelHeader({
  onClose,
  taskId,
}: {
  onClose: () => void
  taskId: string | null
}) {
  return (
    <header className="panel-header">
      <div>
        <h2>投机面板</h2>
        <p>
          只读观察
          {taskId ? (
            <>
              {' '}
              · 任务 <code>{shortId(taskId)}</code>
            </>
          ) : null}
        </p>
      </div>
      <button
        type="button"
        className="icon-button"
        onClick={onClose}
        aria-label="隐藏投机面板"
        title="隐藏投机面板"
      >
        <CloseIcon />
      </button>
    </header>
  )
}

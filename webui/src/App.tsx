import { useEffect } from 'react'
import { ChatPane } from './components/ChatPane'
import { SpeculationPanel } from './components/SpeculationPanel'
import { useAppStore } from './store'

export function App() {
  const panelOpen = useAppStore((store) => store.panelOpen)
  const start = useAppStore((store) => store.start)
  const stop = useAppStore((store) => store.stop)
  const setPanelOpen = useAppStore((store) => store.setPanelOpen)

  useEffect(() => {
    void start()
    return () => stop()
  }, [start, stop])

  return (
    <div className={panelOpen ? 'app app-panel-open' : 'app'}>
      <ChatPane />
      {panelOpen ? <SpeculationPanel /> : null}
      {panelOpen ? (
        <button
          type="button"
          className="panel-scrim"
          aria-label="隐藏投机面板"
          onClick={() => setPanelOpen(false)}
        />
      ) : null}
    </div>
  )
}

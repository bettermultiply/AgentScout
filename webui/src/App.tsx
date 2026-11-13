import { useEffect } from 'react'
import { ChatPane } from './components/ChatPane'
import { SpeculationPanel } from './components/SpeculationPanel'
import { useAppStore } from './store'
import { NleMapPage } from './nleMap'
import { usePanelResize } from './usePanelResize'

export function App() {
  if (window.location.pathname === '/map') return <NleMapPage />
  const panelOpen = useAppStore((store) => store.panelOpen)
  const start = useAppStore((store) => store.start)
  const stop = useAppStore((store) => store.stop)
  const setPanelOpen = useAppStore((store) => store.setPanelOpen)
  const panelResize = usePanelResize(panelOpen)

  useEffect(() => {
    void start()
    return () => stop()
  }, [start, stop])

  return (
    <div
      ref={panelResize.appRef}
      className={`app${panelOpen ? ' app-panel-open' : ''}${panelResize.resizing ? ' app-resizing' : ''}`}
      style={panelResize.style}
    >
      <ChatPane />
      {panelOpen ? <div className="panel-resizer" {...panelResize.separatorProps} /> : null}
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

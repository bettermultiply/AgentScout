import { useEffect, useMemo, useRef, useState } from 'react'
import { eventStreamUrl, fetchStateEventSet } from './api'
import type { ObservationEvent } from './types'
import { eventHistory, type MapCell as Cell, type MapHistoryResult as HistoryResult, type MapSnapshot } from './nleMapHistory'

type SourceState = 'loading' | 'real' | 'demo' | 'unavailable'

const DEMO_HISTORY: MapSnapshot[] = [
  { action: '起点观测', actionNumber: 0, player: { x: 0, y: 0 }, message: '公开观测已建立；地图来源：演示数据。', visible: [
    ...row(-2, -1, '####'), ...row(-2, 0, '#.@.'), ...row(-2, 1, '####'),
  ] },
  { action: '向东移动', actionNumber: 1, player: { x: 1, y: 0 }, message: 'moved · 发现东侧走廊。', visible: [
    ...row(0, -1, '###'), ...row(0, 0, '.@.'), ...row(0, 1, '###'),
  ] },
  { action: '向东移动', actionNumber: 2, player: { x: 2, y: 0 }, message: 'moved · 公开视野继续扩大。', visible: [
    ...row(1, -1, '###'), ...row(1, 0, '.@>'), ...row(1, 1, '###'),
  ] },
  { action: '向北移动', actionNumber: 3, player: { x: 2, y: -1 }, message: 'moved · 当前可见区域更新。', visible: [
    ...row(1, -2, '###'), ...row(1, -1, '.@#'), ...row(1, 0, '..>'),
  ] },
]

function row(x: number, y: number, text: string): Cell[] {
  return [...text].map((glyph, index) => ({ x: x + index, y, glyph }))
}

export function NleMapPage() {
  const [step, setStep] = useState(0)
  const [source, setSource] = useState<SourceState>('loading')
  const [history, setHistory] = useState<MapSnapshot[]>([])
  const [error, setError] = useState<string | null>(null)
  const [branchId, setBranchId] = useState<string | null>(null)
  const [quality, setQuality] = useState<HistoryResult['quality']>('invalid')
  const [qualityMessage, setQualityMessage] = useState<string | null>(null)
  const stepRef = useRef(0)
  const followRef = useRef(true)

  useEffect(() => {
    let active = true
    let events: ObservationEvent[] = []
    let sequence = 0
    let stream: EventSource | null = null
    let resyncing = false

    const applyEvents = (nextEvents: ObservationEvent[], initial = false) => {
      const result = eventHistory(nextEvents)
      setBranchId(result.branchSources.join(' → ') || result.branchId)
      setQuality(result.quality)
      setQualityMessage(result.qualityMessage)
      if (!result.history.length) {
        setHistory(DEMO_HISTORY)
        setSource('demo')
        return
      }
      setHistory(result.history)
      setSource('real')
      const oldStep = stepRef.current
      const nextStep = initial || followRef.current
        ? result.history.length - 1
        : Math.min(oldStep, result.history.length - 1)
      stepRef.current = nextStep
      setStep(nextStep)
    }

    const resync = async (initial = false) => {
      if (resyncing && !initial) return
      resyncing = true
      stream?.close()
      stream = null
      try {
        const state = await fetchStateEventSet()
        if (!active) return
        events = state.events.slice()
        sequence = state.sequence
        applyEvents(events, initial)
        stream = new EventSource(eventStreamUrl(sequence))
        stream.onmessage = (message) => {
            if (!active) return
            let event: ObservationEvent
            try { event = JSON.parse(message.data) as ObservationEvent } catch { return }
            if (event.sequence <= sequence) return
            if (event.sequence !== sequence + 1) {
              void resync(false)
              return
            }
            sequence = event.sequence
            events = [...events, event]
            applyEvents(events)
          }
      } catch (reason: unknown) {
        if (!active) return
        setError(reason instanceof Error ? reason.message : String(reason))
        if (initial) { setHistory(DEMO_HISTORY); setSource('unavailable') }
      } finally {
        resyncing = false
      }
    }

    void resync(true)
    return () => { active = false; stream?.close() }
  }, [])

  const snapshot = history[step] ?? DEMO_HISTORY[0]
  const changeStep = (next: number) => {
    const bounded = Math.max(0, Math.min(history.length - 1, next))
    stepRef.current = bounded
    followRef.current = bounded >= history.length - 1
    setStep(bounded)
  }
  const currentVisible = useMemo(() => new Map(snapshot.visible.map((cell) => [`${cell.x},${cell.y}`, cell])), [snapshot])
  const explored = useMemo(() => {
    const cells = new Map<string, Cell>()
    history.slice(0, step + 1).forEach((item) => item.visible.forEach((cell) => cells.set(`${cell.x},${cell.y}`, cell)))
    return cells
  }, [history, step])
  const bounds = useMemo(() => {
    const all = history.flatMap((item) => item.visible)
    const xs = all.map((cell) => cell.x).concat(snapshot.player.x)
    const ys = all.map((cell) => cell.y).concat(snapshot.player.y)
    return { minX: Math.min(...xs, -3) - 1, maxX: Math.max(...xs, 4) + 1, minY: Math.min(...ys, -3) - 1, maxY: Math.max(...ys, 2) + 1 }
  }, [history, snapshot])
  const cells = []
  for (let y = bounds.minY; y <= bounds.maxY; y += 1) for (let x = bounds.minX; x <= bounds.maxX; x += 1) cells.push({ x, y })
  const sourceLabel = source === 'real' ? '真实 Runtime 公开观测' : source === 'loading' ? '正在读取 Runtime 公开事件…' : source === 'demo' ? '演示公开观测（无地图事件）' : '无运行实例 · 演示 fallback'

  return <main className="nle-map-page">
    <header className="nle-map-header"><div><p className="eyebrow">ASCOUT / NLE RUNTIME</p><h1>NetHack 地图回放</h1><p className="nle-map-subtitle">固定绝对坐标 · 只读历史浏览 · 不会发送 NLE 动作</p></div><div className={`nle-map-source source-${source}`}><span className="status-dot" />{sourceLabel}<br /><small>{source === 'real' ? '来自控制面公开观察事件' : '真实 Runtime 地图通道未提供本次运行数据'}</small>{error ? <small className="map-error">{error}</small> : null}</div></header>
     <section className="nle-map-layout"><div className="nle-map-card"><div className="nle-map-toolbar"><strong>公开地图</strong><span>东 →　北 ↑</span><span className="coordinate-readout">当前位置 ({snapshot.player.x}, {snapshot.player.y})</span></div><div className="nle-map-viewport" aria-label="NLE 固定绝对坐标地图"><div className="nle-map-grid" style={{ gridTemplateColumns: `repeat(${bounds.maxX - bounds.minX + 1}, minmax(32px, 1fr))`, gridTemplateRows: `repeat(${bounds.maxY - bounds.minY + 1}, 1fr)` }}>{cells.map(({ x, y }) => { const key = `${x},${y}`; const exploredCell = explored.get(key); const visibleCell = currentVisible.get(key); const isPlayer = snapshot.player.x === x && snapshot.player.y === y; return <div key={key} className={`nle-cell ${visibleCell ? 'is-visible' : exploredCell ? 'is-explored' : 'is-unknown'} ${isPlayer ? 'is-player' : ''}`} title={`绝对坐标 (${x}, ${y})`}><span>{visibleCell?.glyph ?? (exploredCell ? '·' : '')}</span></div> })}</div><div className="nle-map-axis axis-x">X 轴（东）</div><div className="nle-map-axis axis-y">Y 轴（北）</div></div><div className="nle-map-legend"><span><b className="legend-swatch visible" />当前可见</span><span><b className="legend-swatch explored" />已知但当前不可见</span><span><b className="legend-swatch unknown" />未知（不绘制内容）</span><span><b className="legend-player">@</b>玩家</span></div></div>
       <aside className="nle-map-sidebar"><div className="nle-history-card"><div className="card-label">权威窗口动作历史</div>{branchId ? <div className="map-meta">动作来源：<code>{branchId}</code> · {quality === 'complete' ? '完整' : '不完整'}</div> : null}{qualityMessage ? <p className="readonly-note">{qualityMessage}</p> : null}<div className="step-count"><strong>{snapshot.actionNumber}</strong><span>/ {Math.max(0, history.length - 1)} 个已发生动作</span></div><div className="history-track"><div style={{ width: `${history.length <= 1 ? 0 : (step / (history.length - 1)) * 100}%` }} /></div><div className="history-actions"><button type="button" onClick={() => changeStep(0)} disabled={step === 0}>起点</button><button type="button" onClick={() => changeStep(step - 1)} disabled={step === 0}>← 后退</button><button type="button" onClick={() => changeStep(step + 1)} disabled={step >= history.length - 1}>前进 →</button><button type="button" onClick={() => changeStep(history.length - 1)} disabled={step >= history.length - 1}>终点</button></div><p className="readonly-note">这些按钮只重建已发生动作的历史状态，不会触发、重试或撤销 NLE 动作。{followRef.current ? ' 当前跟随最新事件。' : ' 当前正在回看，不会被新事件跳转。'}</p></div><div className="nle-action-card"><div className="card-label">当前历史位置</div><h2>{snapshot.action}</h2><p>{snapshot.message}</p><dl><dt>动作序号</dt><dd>{snapshot.actionNumber}</dd><dt>玩家绝对坐标</dt><dd>({snapshot.player.x}, {snapshot.player.y})</dd><dt>已知单元格</dt><dd>{explored.size}</dd><dt>当前可见单元格</dt><dd>{currentVisible.size}</dd></dl></div><div className="nle-boundary-card"><strong>只读接入状态</strong><p>{source === 'real' ? '地图数据已从控制面发布的 NLE 公开观察事件读取。页面只使用 Runtime 已公开的 chars/blstats，不读取隐藏地图。' : '当前页面没有可用的真实 NLE 地图事件，已使用明确标注的演示数据。页面不会因 fallback 而声称 Runtime 已接通。'}</p></div></aside>
    </section>
  </main>
}

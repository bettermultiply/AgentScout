import { useLayoutEffect, useRef, useState } from 'react'
import type { CSSProperties, KeyboardEvent, PointerEvent } from 'react'

const MIN_PANEL_WIDTH = 410
const MIN_CHAT_WIDTH = 480
const SEPARATOR_WIDTH = 8

export function usePanelResize(panelOpen: boolean) {
  const appRef = useRef<HTMLDivElement>(null)
  const drag = useRef<{
    element: HTMLDivElement
    pointerId: number
    startX: number
    startWidth: number
  } | null>(null)
  const [containerWidth, setContainerWidth] = useState(window.innerWidth)
  const [requestedWidth, setRequestedWidth] = useState<number | null>(null)
  const [resizing, setResizing] = useState(false)
  const maxWidth = Math.max(MIN_PANEL_WIDTH, containerWidth - MIN_CHAT_WIDTH - SEPARATOR_WIDTH)
  const width = Math.min(maxWidth, Math.max(MIN_PANEL_WIDTH,
    requestedWidth ?? Math.min(620, window.innerWidth * 0.34)))

  function finishDrag() {
    const current = drag.current
    drag.current = null
    if (current?.element.hasPointerCapture(current.pointerId)) {
      current.element.releasePointerCapture(current.pointerId)
    }
    setResizing(false)
  }

  useLayoutEffect(() => {
    const app = appRef.current
    if (!app) return
    const observer = new ResizeObserver(() => {
      finishDrag()
      setContainerWidth(app.clientWidth)
    })
    observer.observe(app)
    return () => {
      observer.disconnect()
      finishDrag()
    }
  }, [])

  useLayoutEffect(() => {
    if (!panelOpen) finishDrag()
  }, [panelOpen])

  function onPointerDown(event: PointerEvent<HTMLDivElement>) {
    if (event.button !== 0 || !event.isPrimary || drag.current) return
    if (window.matchMedia('(max-width: 1020px)').matches) return
    event.preventDefault()
    event.currentTarget.focus()
    event.currentTarget.setPointerCapture(event.pointerId)
    drag.current = {
      element: event.currentTarget,
      pointerId: event.pointerId,
      startX: event.clientX,
      startWidth: width,
    }
    setResizing(true)
  }

  function onPointerMove(event: PointerEvent<HTMLDivElement>) {
    const current = drag.current
    if (!current || current.pointerId !== event.pointerId) return
    setRequestedWidth(Math.min(maxWidth, Math.max(MIN_PANEL_WIDTH,
      current.startWidth + current.startX - event.clientX)))
  }

  function onPointerEnd(event: PointerEvent<HTMLDivElement>) {
    if (drag.current?.pointerId === event.pointerId) finishDrag()
  }

  function onKeyDown(event: KeyboardEvent<HTMLDivElement>) {
    let next: number
    switch (event.key) {
      case 'ArrowLeft': next = width + (event.shiftKey ? 50 : 10); break
      case 'ArrowRight': next = width - (event.shiftKey ? 50 : 10); break
      case 'Home': next = MIN_PANEL_WIDTH; break
      case 'End': next = maxWidth; break
      default: return
    }
    event.preventDefault()
    setRequestedWidth(Math.min(maxWidth, Math.max(MIN_PANEL_WIDTH, next)))
  }

  return {
    appRef,
    resizing,
    style: { '--panel-width': `${width}px` } as CSSProperties,
    separatorProps: {
      role: 'separator',
      tabIndex: 0,
      'aria-label': '调整投机面板宽度',
      'aria-orientation': 'vertical' as const,
      'aria-controls': 'speculation-panel',
      'aria-valuemin': MIN_PANEL_WIDTH,
      'aria-valuemax': Math.round(maxWidth),
      'aria-valuenow': Math.round(width),
      'aria-valuetext': `${Math.round(width)} 像素`,
      onPointerDown,
      onPointerMove,
      onPointerUp: onPointerEnd,
      onPointerCancel: onPointerEnd,
      onLostPointerCapture: onPointerEnd,
      onKeyDown,
    },
  }
}

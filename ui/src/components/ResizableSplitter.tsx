import { useCallback, useEffect, useRef, useState } from 'react'

export function useResizableWidth(initial: number, min = 100, max = 400) {
  const [width, setWidth] = useState(initial)
  const dragState = useRef<{ startX: number; startWidth: number } | null>(null)
  const [dragging, setDragging] = useState(false)

  const onMouseDown = useCallback(
    (e: React.MouseEvent) => {
      e.preventDefault()
      dragState.current = { startX: e.clientX, startWidth: width }
      setDragging(true)
    },
    [width],
  )

  useEffect(() => {
    if (!dragging) return
    function onMove(e: MouseEvent) {
      const s = dragState.current
      if (!s) return
      const next = Math.max(min, Math.min(max, s.startWidth + (e.clientX - s.startX)))
      setWidth(next)
    }
    function onUp() {
      dragState.current = null
      setDragging(false)
    }
    window.addEventListener('mousemove', onMove)
    window.addEventListener('mouseup', onUp)
    const prevCursor = document.body.style.cursor
    const prevSelect = document.body.style.userSelect
    document.body.style.cursor = 'col-resize'
    document.body.style.userSelect = 'none'
    return () => {
      window.removeEventListener('mousemove', onMove)
      window.removeEventListener('mouseup', onUp)
      document.body.style.cursor = prevCursor
      document.body.style.userSelect = prevSelect
    }
  }, [dragging, min, max])

  return { width, onMouseDown, dragging }
}

export function Splitter({ onMouseDown, dragging }: { onMouseDown: (e: React.MouseEvent) => void; dragging: boolean }) {
  const [hover, setHover] = useState(false)
  const active = hover || dragging
  return (
    <div
      onMouseDown={onMouseDown}
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => setHover(false)}
      style={{
        width: 8,
        flexShrink: 0,
        cursor: 'col-resize',
        display: 'flex',
        justifyContent: 'center',
        alignItems: 'stretch',
      }}
    >
      <div
        style={{
          width: active ? 4 : 1,
          background: active ? 'var(--mantine-color-gray-5)' : 'var(--mantine-color-gray-3)',
          borderRadius: 2,
          transition: dragging ? 'none' : 'width 120ms, background 120ms',
        }}
      />
    </div>
  )
}

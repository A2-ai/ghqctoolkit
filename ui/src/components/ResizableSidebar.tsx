import { useEffect, useRef, useState } from 'react'
import { ActionIcon } from '@mantine/core'
import { IconChevronLeft, IconChevronRight } from '@tabler/icons-react'
import type { ReactNode } from 'react'

interface Props {
  children: ReactNode
  defaultWidth?: number
  minWidth?: number
  maxWidth?: number
}

const COLLAPSED_WIDTH = 28

export function ResizableSidebar({
  children,
  defaultWidth = 260,
  minWidth = 160,
  maxWidth = 520,
}: Props) {
  const [width, setWidth] = useState(defaultWidth)
  const [collapsed, setCollapsed] = useState(false)
  const lastWidth = useRef(defaultWidth)
  const isDragging = useRef(false)
  const dragStartX = useRef(0)
  const dragStartWidth = useRef(0)

  useEffect(() => {
    const onMove = (e: MouseEvent) => {
      if (!isDragging.current) return
      setWidth(Math.max(minWidth, Math.min(maxWidth, dragStartWidth.current + e.clientX - dragStartX.current)))
    }
    const onUp = () => {
      if (!isDragging.current) return
      isDragging.current = false
      document.body.style.cursor = ''
      document.body.style.userSelect = ''
    }
    document.addEventListener('mousemove', onMove)
    document.addEventListener('mouseup', onUp)
    return () => {
      document.removeEventListener('mousemove', onMove)
      document.removeEventListener('mouseup', onUp)
    }
  }, [minWidth, maxWidth])

  const onHandleMouseDown = (e: React.MouseEvent) => {
    if (collapsed) return
    isDragging.current = true
    dragStartX.current = e.clientX
    dragStartWidth.current = width
    document.body.style.cursor = 'col-resize'
    document.body.style.userSelect = 'none'
    e.preventDefault()
  }

  const toggleCollapse = () => {
    if (collapsed) {
      setWidth(lastWidth.current)
    } else {
      lastWidth.current = width
    }
    setCollapsed(c => !c)
  }

  const visibleWidth = collapsed ? COLLAPSED_WIDTH : width

  return (
    <div style={{
      display: 'flex',
      width: visibleWidth,
      flexShrink: 0,
      height: '100%',
      borderRight: '1px solid var(--mantine-color-gray-3)',
      transition: collapsed ? 'width 150ms ease' : undefined,
    }}>
      {/* Content — hidden when collapsed */}
      {!collapsed && (
        <div style={{
          flex: 1,
          overflowY: 'auto',
          overflowX: 'hidden',
          padding: 'var(--mantine-spacing-md)',
        }}>
          {children}
        </div>
      )}

      {/* Right edge: collapse button stacked above drag handle */}
      <div style={{
        width: COLLAPSED_WIDTH,
        flexShrink: 0,
        display: 'flex',
        flexDirection: 'column',
        alignItems: 'center',
      }}>
        <ActionIcon
          variant="subtle"
          size="sm"
          onClick={toggleCollapse}
          style={{ marginTop: 8 }}
          title={collapsed ? 'Expand' : 'Collapse'}
        >
          {collapsed ? <IconChevronRight size={14} /> : <IconChevronLeft size={14} />}
        </ActionIcon>
        {/* drag zone fills the rest of the height */}
        {!collapsed && (
          <div
            onMouseDown={onHandleMouseDown}
            style={{ flex: 1, width: '100%', cursor: 'col-resize' }}
          />
        )}
      </div>
    </div>
  )
}

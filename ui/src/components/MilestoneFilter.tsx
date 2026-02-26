import {
  ActionIcon,
  Combobox,
  InputBase,
  Loader,
  Stack,
  Switch,
  Text,
  Tooltip,
  useCombobox,
} from '@mantine/core'
import {
  IconAlertCircle,
  IconAlertTriangle,
  IconChevronDown,
  IconChevronRight,
  IconExclamationMark,
  IconX,
} from '@tabler/icons-react'
import { useEffect, useRef, useState } from 'react'
import { useMilestones, type Milestone } from '~/api/milestones'
import { type MilestoneStatusInfo } from '~/api/issues'

const MIN_ISSUES_HEIGHT = 80
const COLLAPSED_ISSUES_HEIGHT = 36

interface Props {
  selected: number[]
  onSelect: (numbers: number[]) => void
  includeClosedIssues: boolean
  onIncludeClosedIssuesChange: (include: boolean) => void
  milestoneStatusByMilestone: Record<number, MilestoneStatusInfo>
}

export function MilestoneFilter({
  selected,
  onSelect,
  includeClosedIssues,
  onIncludeClosedIssuesChange,
  milestoneStatusByMilestone,
}: Props) {
  const [includeClosedMilestones, setIncludeClosedMilestones] = useState(false)
  const [search, setSearch] = useState('')
  const { data, isLoading, isError } = useMilestones()
  const combobox = useCombobox({ onDropdownClose: () => setSearch('') })

  const containerRef = useRef<HTMLDivElement>(null)
  const [issuesHeight, setIssuesHeight] = useState(300)
  const [issuesCollapsed, setIssuesCollapsed] = useState(false)
  const lastIssuesHeightRef = useRef(300)
  const isDragging = useRef(false)
  const dragStartY = useRef(0)
  const dragStartHeight = useRef(0)

  // Set issues section to 50% of container height on mount
  useEffect(() => {
    if (containerRef.current) {
      const h = containerRef.current.clientHeight
      if (h > 0) {
        const half = Math.round(h * 0.5)
        setIssuesHeight(half)
        lastIssuesHeightRef.current = half
      }
    }
  }, [])

  // Vertical drag-to-resize
  useEffect(() => {
    const onMove = (e: MouseEvent) => {
      if (!isDragging.current) return
      const delta = dragStartY.current - e.clientY
      const maxH = (containerRef.current?.clientHeight ?? 600) - MIN_ISSUES_HEIGHT
      setIssuesHeight(Math.max(MIN_ISSUES_HEIGHT, Math.min(maxH, dragStartHeight.current + delta)))
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
  }, [])

  const onDragHandleMouseDown = (e: React.MouseEvent) => {
    isDragging.current = true
    dragStartY.current = e.clientY
    dragStartHeight.current = issuesHeight
    document.body.style.cursor = 'row-resize'
    document.body.style.userSelect = 'none'
    e.preventDefault()
  }

  function toggleIssuesCollapse() {
    if (issuesCollapsed) {
      setIssuesHeight(lastIssuesHeightRef.current)
    } else {
      lastIssuesHeightRef.current = issuesHeight
    }
    setIssuesCollapsed((c) => !c)
  }

  const available = (data ?? []).filter(
    (m) => (includeClosedMilestones || m.state === 'open') && !selected.includes(m.number),
  )
  const filtered = available.filter((m) =>
    m.title.toLowerCase().includes(search.toLowerCase()),
  )
  const selectedMilestones = (data ?? []).filter((m) => selected.includes(m.number))

  function add(number: number) {
    onSelect([...selected, number])
    combobox.closeDropdown()
    setSearch('')
  }

  function remove(number: number) {
    onSelect(selected.filter((n) => n !== number))
  }

  function handleIncludeClosedMilestonesChange(checked: boolean) {
    setIncludeClosedMilestones(checked)
    if (!checked) {
      const closedNumbers = (data ?? [])
        .filter((m) => m.state === 'closed' && selected.includes(m.number))
        .map((m) => m.number)
      if (closedNumbers.length > 0) {
        onSelect(selected.filter((n) => !closedNumbers.includes(n)))
      }
    }
  }

  const defaultStatusInfo: MilestoneStatusInfo = {
    listFailed: false,
    listError: null,
    loadingCount: 0,
    statusErrorCount: 0,
    statusErrors: [],
    statusAttemptedCount: 0,
  }

  const displayIssuesHeight = issuesCollapsed ? COLLAPSED_ISSUES_HEIGHT : issuesHeight

  return (
    <div ref={containerRef} style={{ display: 'flex', flexDirection: 'column', height: '100%' }}>

      {/* ── Milestones section ────────────────────────────────────────────── */}
      <div style={{ flex: 1, overflowY: 'auto', minHeight: 0, padding: 'var(--mantine-spacing-md)' }}>
        <Stack gap="sm">
          <Text fw={600} size="sm">Milestones</Text>
          <Switch
            label="Include Closed Milestones"
            size="xs"
            checked={includeClosedMilestones}
            onChange={(e) => handleIncludeClosedMilestonesChange(e.currentTarget.checked)}
          />
          <Combobox store={combobox} onOptionSubmit={(val) => add(Number(val))}>
            <Combobox.Target>
              <InputBase
                placeholder="Search milestones…"
                size="xs"
                value={search}
                rightSection={isLoading ? <Loader size={12} /> : <Combobox.Chevron />}
                onChange={(e) => { setSearch(e.currentTarget.value); combobox.openDropdown() }}
                onClick={() => combobox.openDropdown()}
                onFocus={() => combobox.openDropdown()}
              />
            </Combobox.Target>
            <Combobox.Dropdown>
              <Combobox.Options>
                {isError && <Combobox.Empty>Failed to load</Combobox.Empty>}
                {!isLoading && !isError && filtered.length === 0 && (
                  <Combobox.Empty>No milestones found</Combobox.Empty>
                )}
                {[...filtered].reverse().map((m) => (
                  <Combobox.Option key={m.number} value={String(m.number)}>
                    <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                      <Text size="sm">{m.title}</Text>
                      {m.state === 'closed' && <ClosedPill />}
                    </div>
                    <Text size="xs" c="dimmed">
                      {m.open_issues} open · {m.closed_issues} closed
                    </Text>
                  </Combobox.Option>
                ))}
              </Combobox.Options>
            </Combobox.Dropdown>
          </Combobox>
          <Stack gap={6}>
            {selectedMilestones.map((m) => (
              <SelectedMilestoneCard
                key={m.number}
                milestone={m}
                onRemove={() => remove(m.number)}
                statusInfo={milestoneStatusByMilestone[m.number] ?? defaultStatusInfo}
              />
            ))}
          </Stack>
        </Stack>
      </div>

      {/* ── Issues section ────────────────────────────────────────────────── */}
      <div style={{ height: displayIssuesHeight, flexShrink: 0, display: 'flex', flexDirection: 'column' }}>

        {/* Drag handle — doubles as the section border */}
        {!issuesCollapsed && (
          <div
            onMouseDown={onDragHandleMouseDown}
            style={{
              height: 6,
              flexShrink: 0,
              cursor: 'row-resize',
              borderTop: '1px solid var(--mantine-color-gray-3)',
            }}
          />
        )}

        {/* Header row */}
        <div style={{
          display: 'flex',
          alignItems: 'center',
          gap: 4,
          padding: '0 var(--mantine-spacing-md)',
          height: COLLAPSED_ISSUES_HEIGHT,
          flexShrink: 0,
          borderTop: issuesCollapsed ? '1px solid var(--mantine-color-gray-3)' : undefined,
          cursor: 'pointer',
        }}
        onClick={toggleIssuesCollapse}
        title={issuesCollapsed ? 'Expand' : 'Collapse'}
        >
          <ActionIcon
            size="xs"
            variant="subtle"
            tabIndex={-1}
            style={{ pointerEvents: 'none' }}
          >
            {issuesCollapsed ? <IconChevronRight size={14} /> : <IconChevronDown size={14} />}
          </ActionIcon>
          <Text fw={600} size="sm">Issues</Text>
        </div>

        {/* Scrollable content */}
        {!issuesCollapsed && (
          <div style={{ flex: 1, overflowY: 'auto', padding: '0 var(--mantine-spacing-md) var(--mantine-spacing-md)' }}>
            <Stack gap="sm">
              <Switch
                label="Include Closed Issues"
                size="xs"
                checked={includeClosedIssues}
                onChange={(e) => onIncludeClosedIssuesChange(e.currentTarget.checked)}
              />
            </Stack>
          </div>
        )}
      </div>

    </div>
  )
}

function ClosedPill() {
  return (
    <span style={{
      fontSize: 10,
      fontWeight: 600,
      padding: '1px 5px',
      borderRadius: 4,
      backgroundColor: '#868e96',
      color: 'white',
      lineHeight: '16px',
      flexShrink: 0,
    }}>
      closed
    </span>
  )
}

function SelectedMilestoneCard({
  milestone,
  onRemove,
  statusInfo,
}: {
  milestone: Milestone
  onRemove: () => void
  statusInfo: MilestoneStatusInfo
}) {
  const isAllFailed =
    !statusInfo.listFailed &&
    statusInfo.statusAttemptedCount > 0 &&
    statusInfo.statusErrorCount >= statusInfo.statusAttemptedCount

  const isPartial =
    !statusInfo.listFailed &&
    statusInfo.statusErrorCount > 0 &&
    statusInfo.statusErrorCount < statusInfo.statusAttemptedCount

  const isRed = statusInfo.listFailed || isAllFailed
  const isYellow = isPartial

  const bgColor = isRed ? '#ffe3e3' : isYellow ? '#fff3bf' : '#d7e7d3'
  const borderColor = isRed ? '#ff8787' : isYellow ? '#fcc419' : '#aacca6'

  const errorLines = statusInfo.statusErrors.length > 0 ? (
    <div>
      {statusInfo.statusErrors.map((e) => (
        <div key={e.issue_number}>#{e.issue_number}: {e.error}</div>
      ))}
    </div>
  ) : null

  return (
    <div style={{
      display: 'flex',
      alignItems: 'center',
      justifyContent: 'space-between',
      gap: 6,
      padding: '6px 8px',
      borderRadius: 6,
      backgroundColor: bgColor,
      border: `1px solid ${borderColor}`,
    }}>
      <div style={{ minWidth: 0 }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 6, minWidth: 0 }}>
          <Text size="sm" fw={600} truncate="end">{milestone.title}</Text>
          {milestone.state === 'closed' && <ClosedPill />}
          {statusInfo.listFailed && statusInfo.listError && (
            <Tooltip label={statusInfo.listError} withArrow>
              <IconExclamationMark size={14} color="#c92a2a" style={{ flexShrink: 0 }} />
            </Tooltip>
          )}
          {isAllFailed && errorLines && (
            <Tooltip label={errorLines} withArrow multiline>
              <span style={{ color: '#c92a2a', display: 'flex', alignItems: 'center', gap: 2, flexShrink: 0 }}>
                <IconAlertCircle size={14} />
                {statusInfo.statusErrorCount}
              </span>
            </Tooltip>
          )}
          {isPartial && errorLines && (
            <Tooltip label={errorLines} withArrow multiline>
              <span data-testid="partial-warning" style={{ color: '#e67700', display: 'flex', alignItems: 'center', gap: 2, flexShrink: 0 }}>
                <IconAlertTriangle size={14} />
                {statusInfo.statusErrorCount}
              </span>
            </Tooltip>
          )}
        </div>
        <Text size="xs" c="dimmed">
          {milestone.open_issues} open · {milestone.closed_issues} closed
        </Text>
        {statusInfo.loadingCount > 0 && (
          <>
            <style>{`
              @keyframes glisten {
                0%, 100% { opacity: 1; }
                50% { opacity: 0.35; }
              }
            `}</style>
            <Text size="xs" c="dimmed" style={{ animation: 'glisten 1.4s ease-in-out infinite' }}>
              {statusInfo.loadingCount} {statusInfo.loadingCount === 1 ? 'issue' : 'issues'} loading…
            </Text>
          </>
        )}
      </div>
      <ActionIcon
        size="xs"
        variant="transparent"
        color="dark"
        onClick={onRemove}
        style={{ flexShrink: 0 }}
        aria-label={`Remove ${milestone.title}`}
      >
        <IconX size={12} />
      </ActionIcon>
    </div>
  )
}

import {
  ActionIcon,
  Combobox,
  Divider,
  InputBase,
  Loader,
  Stack,
  Switch,
  Text,
  Tooltip,
  useCombobox,
} from '@mantine/core'
import { IconAlertCircle, IconAlertTriangle, IconExclamationMark, IconX } from '@tabler/icons-react'
import { useState } from 'react'
import { useMilestones, type Milestone } from '~/api/milestones'
import { type MilestoneStatusInfo } from '~/api/issues'

interface Props {
  selected: number[]
  onSelect: (numbers: number[]) => void
  includeClosedIssues: boolean
  onIncludeClosedIssuesChange: (include: boolean) => void
  milestoneStatusByMilestone: Record<number, MilestoneStatusInfo>
}

export function MilestoneFilter({ selected, onSelect, includeClosedIssues, onIncludeClosedIssuesChange, milestoneStatusByMilestone }: Props) {
  const [includeClosedMilestones, setIncludeClosedMilestones] = useState(false)
  const [search, setSearch] = useState('')
  const { data, isLoading, isError } = useMilestones()
  const combobox = useCombobox({ onDropdownClose: () => setSearch('') })

  const available = (data ?? []).filter(
    (m) => (includeClosedMilestones || m.state === 'open') && !selected.includes(m.number)
  )

  const filtered = available.filter((m) =>
    m.title.toLowerCase().includes(search.toLowerCase())
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

  return (
    <Stack gap="sm">
      <Text fw={600} size="sm">Milestones</Text>

      <Combobox store={combobox} onOptionSubmit={(val) => add(Number(val))}>
        <Combobox.Target>
          <InputBase
            placeholder="Search milestones…"
            size="xs"
            value={search}
            rightSection={isLoading ? <Loader size={12} /> : <Combobox.Chevron />}
            onChange={(e) => {
              setSearch(e.currentTarget.value)
              combobox.openDropdown()
            }}
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

      <Switch
        label="Include closed milestones"
        size="xs"
        checked={includeClosedMilestones}
        onChange={(e) => handleIncludeClosedMilestonesChange(e.currentTarget.checked)}
      />

      <Divider />

      <Switch
        label="Include closed issues"
        size="xs"
        checked={includeClosedIssues}
        onChange={(e) => onIncludeClosedIssuesChange(e.currentTarget.checked)}
      />

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
              <span style={{ color: '#e67700', display: 'flex', alignItems: 'center', gap: 2, flexShrink: 0 }}>
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

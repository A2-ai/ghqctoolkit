import { useEffect, useMemo, useRef, useState } from 'react'
import {
  ActionIcon,
  Alert,
  Anchor,
  Button,
  Combobox,
  Divider,
  InputBase,
  Loader,
  Stack,
  Switch,
  Text,
  TextInput,
  Tooltip,
  useCombobox,
} from '@mantine/core'
import {
  IconAlertCircle,
  IconAlertTriangle,
  IconExclamationMark,
  IconArrowBackUp,
  IconX,
} from '@tabler/icons-react'
import { useMilestones } from '~/api/milestones'
import { type MilestoneStatusInfo, useMilestoneIssues } from '~/api/issues'
import { type ArchiveFileRequest, generateArchive } from '~/api/archive'
import { useRepoInfo } from '~/api/repo'
import { ResizableSidebar } from './ResizableSidebar'

// ─── ArchiveTab ───────────────────────────────────────────────────────────────

export function ArchiveTab() {
  const [selectedMilestones, setSelectedMilestones] = useState<number[]>([])
  const [showOpenMilestones, setShowOpenMilestones] = useState(false)
  const [flatten, setFlatten] = useState(false)
  const [outputPath, setOutputPath] = useState('')
  const [generateLoading, setGenerateLoading] = useState(false)
  const [generateError, setGenerateError] = useState<string | null>(null)
  const [generateSuccess, setGenerateSuccess] = useState<string | null>(null)
  const outputPathUserEdited = useRef(false)
  const [outputPathIsCustom, setOutputPathIsCustom] = useState(false)

  const { data: repoData } = useRepoInfo()
  const { data: milestonesData } = useMilestones()

  // Include closed issues for archive
  const { statuses, milestoneStatusByMilestone, isLoadingStatuses } =
    useMilestoneIssues(selectedMilestones, true)

  const milestoneStatusRef = useRef(milestoneStatusByMilestone)
  milestoneStatusRef.current = milestoneStatusByMilestone

  function hasErrors(n: number): boolean {
    const info = milestoneStatusRef.current[n]
    return !info || info.listFailed || info.statusErrorCount > 0
  }

  // Count unapproved issues per milestone
  const unapprovedByMilestone = useMemo(() => {
    const result: Record<number, number> = {}
    for (const n of selectedMilestones) {
      const milestoneName = (milestonesData ?? []).find((m) => m.number === n)?.title
      const milestoneStatuses = statuses.filter((s) => s.issue.milestone === milestoneName)
      result[n] = milestoneStatuses.filter(
        (s) =>
          s.qc_status.status !== 'approved' &&
          s.qc_status.status !== 'changes_after_approval',
      ).length
    }
    return result
  }, [selectedMilestones, statuses, milestonesData])

  const erroredKey = selectedMilestones
    .filter(
      (n) =>
        milestoneStatusByMilestone[n]?.listFailed ||
        (milestoneStatusByMilestone[n]?.statusErrorCount ?? 0) > 0,
    )
    .sort((a, b) => a - b)
    .join(',')

  function resetOutputPath() {
    outputPathUserEdited.current = false
    setOutputPathIsCustom(false)
    if (!repoData) {
      setOutputPath('')
      return
    }
    const names = selectedMilestones
      .map((n) => (milestonesData ?? []).find((m) => m.number === n)?.title ?? String(n))
      .join('-')
      .replace(/\s+/g, '-')
    setOutputPath(names ? `${repoData.repo}-${names}.tar.gz` : '')
  }

  // Auto-populate output path
  useEffect(() => {
    if (outputPathUserEdited.current) return
    if (!repoData) return
    if (selectedMilestones.length === 0) {
      setOutputPath('')
      return
    }
    const names = selectedMilestones
      .map((n) => (milestonesData ?? []).find((m) => m.number === n)?.title ?? String(n))
      .join('-')
      .replace(/\s+/g, '-')
    setOutputPath(`${repoData.repo}-${names}.tar.gz`)
  }, [selectedMilestones, milestonesData, repoData, erroredKey])

  async function handleGenerate() {
    setGenerateError(null)
    setGenerateSuccess(null)
    setGenerateLoading(true)
    try {
      const files: ArchiveFileRequest[] = statuses.map((s) => ({
        repository_file: s.issue.title,
        commit: s.qc_status.approved_commit ?? s.qc_status.latest_commit,
        milestone: s.issue.milestone ?? undefined,
        approved:
          s.qc_status.status === 'approved' ||
          s.qc_status.status === 'changes_after_approval',
      }))

      const result = await generateArchive({ output_path: outputPath, flatten, files })
      setGenerateSuccess(result.output_path)
    } catch (err) {
      setGenerateError((err as Error).message)
    } finally {
      setGenerateLoading(false)
    }
  }

  const canGenerate =
    selectedMilestones.length > 0 && outputPath.trim().length > 0 && !isLoadingStatuses

  return (
    <div style={{ display: 'flex', height: '100%', overflow: 'hidden' }}>
      {/* ── Left sidebar ─────────────────────────────────────────────────── */}
      <ResizableSidebar defaultWidth={320} minWidth={280} maxWidth={560}>
        <Stack gap="sm">
          <ArchiveMilestoneSelector
            selectedMilestones={selectedMilestones}
            onSelectedMilestonesChange={setSelectedMilestones}
            showOpenMilestones={showOpenMilestones}
            onShowOpenMilestonesChange={setShowOpenMilestones}
            statusByMilestone={milestoneStatusByMilestone}
            unapprovedByMilestone={unapprovedByMilestone}
          />

          <Divider />

          <TextInput
            label="Output Path"
            placeholder="archive.tar.gz"
            size="xs"
            value={outputPath}
            onChange={(e) => {
              const val = e.currentTarget.value
              outputPathUserEdited.current = val !== ''
              setOutputPathIsCustom(val !== '')
              setOutputPath(val)
            }}
            rightSection={
              outputPathIsCustom && selectedMilestones.length > 0 ? (
                <Tooltip label="Reset to default" withArrow position="top">
                  <ActionIcon
                    size="xs"
                    variant="transparent"
                    color="gray"
                    onClick={resetOutputPath}
                    aria-label="Reset output path to default"
                  >
                    <IconArrowBackUp size={13} />
                  </ActionIcon>
                </Tooltip>
              ) : undefined
            }
          />

          <Switch
            label="Flatten directory structure"
            size="xs"
            checked={flatten}
            onChange={(e) => setFlatten(e.currentTarget.checked)}
          />

          {generateError && (
            <Alert color="red" p="xs">
              <Text size="xs">{generateError}</Text>
            </Alert>
          )}
          {generateSuccess && (
            <Alert color="green" p="xs">
              <Text size="xs">Archive written to {generateSuccess}</Text>
            </Alert>
          )}

          <Button
            fullWidth
            size="sm"
            color="green"
            onClick={handleGenerate}
            loading={generateLoading}
            disabled={!canGenerate}
          >
            Generate Archive
          </Button>
        </Stack>
      </ResizableSidebar>

      {/* ── Right panel: issue cards ──────────────────────────────────────── */}
      <div style={{ flex: 1, overflowY: 'auto', padding: 'var(--mantine-spacing-md)' }}>
        {selectedMilestones.length === 0 ? (
          <div
            style={{
              display: 'flex',
              alignItems: 'center',
              justifyContent: 'center',
              height: '100%',
            }}
          >
            <Text c="dimmed" size="sm">
              Select a milestone to see issues
            </Text>
          </div>
        ) : (
          <div
            style={{
              display: 'grid',
              gridTemplateColumns: 'repeat(auto-fill, minmax(160px, 1fr))',
              gap: 12,
            }}
          >
            {statuses.map((s) => {
              const isApproved =
                s.qc_status.status === 'approved' ||
                s.qc_status.status === 'changes_after_approval'
              const commit = s.qc_status.approved_commit ?? s.qc_status.latest_commit
              const shortCommit = commit.slice(0, 7)
              const statusLabel = s.qc_status.status.replace(/_/g, ' ')

              const bgColor = isApproved ? 'white' : '#fff3bf'
              const borderColor = isApproved ? 'var(--mantine-color-gray-3)' : '#fcc419'

              const card = (
                <Stack
                  key={s.issue.number}
                  gap={5}
                  style={{
                    padding: '10px 12px',
                    borderRadius: 6,
                    border: `1px solid ${borderColor}`,
                    backgroundColor: bgColor,
                    minWidth: 0,
                  }}
                >
                  <Anchor
                    href={s.issue.html_url}
                    target="_blank"
                    size="sm"
                    fw={700}
                    style={{ wordBreak: 'break-all' }}
                  >
                    {s.issue.title}
                  </Anchor>
                  {s.issue.milestone && (
                    <Text size="xs" c="dimmed"><b>Milestone:</b> {s.issue.milestone}</Text>
                  )}
                  <Text size="xs" c="dimmed"><b>Commit:</b> {shortCommit}</Text>
                  <Text size="xs" c="dimmed"><b>Status:</b> {statusLabel}</Text>
                </Stack>
              )

              if (!isApproved) {
                return (
                  <Tooltip
                    key={s.issue.number}
                    label="QC Issue Not Approved"
                    withArrow
                  >
                    {card}
                  </Tooltip>
                )
              }

              return card
            })}
          </div>
        )}
      </div>
    </div>
  )
}

// ─── ArchiveMilestoneSelector ─────────────────────────────────────────────────

interface ArchiveMilestoneSelectorProps {
  selectedMilestones: number[]
  onSelectedMilestonesChange: (v: number[]) => void
  showOpenMilestones: boolean
  onShowOpenMilestonesChange: (v: boolean) => void
  statusByMilestone: Record<number, MilestoneStatusInfo>
  unapprovedByMilestone: Record<number, number>
}

function ArchiveMilestoneSelector({
  selectedMilestones,
  onSelectedMilestonesChange,
  showOpenMilestones,
  onShowOpenMilestonesChange,
  statusByMilestone,
  unapprovedByMilestone,
}: ArchiveMilestoneSelectorProps) {
  const { data, isLoading, isError } = useMilestones()
  const [search, setSearch] = useState('')
  const combobox = useCombobox({ onDropdownClose: () => setSearch('') })

  const available = (data ?? []).filter(
    (m) =>
      (showOpenMilestones || m.state === 'closed') &&
      !selectedMilestones.includes(m.number),
  )
  const filtered = available.filter((m) =>
    m.title.toLowerCase().includes(search.toLowerCase()),
  )
  const selectedItems = (data ?? []).filter((m) => selectedMilestones.includes(m.number))

  function add(number: number) {
    onSelectedMilestonesChange([...selectedMilestones, number])
    combobox.closeDropdown()
    setSearch('')
  }

  function remove(number: number) {
    onSelectedMilestonesChange(selectedMilestones.filter((n) => n !== number))
  }

  return (
    <Stack gap="sm">
      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', flexWrap: 'wrap', gap: 4 }}>
        <Text fw={600} size="sm" style={{ whiteSpace: 'nowrap' }}>Milestones</Text>
        <Switch
          label="include open"
          size="xs"
          checked={showOpenMilestones}
          onChange={(e) => onShowOpenMilestonesChange(e.currentTarget.checked)}
        />
      </div>

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
                  {m.state !== 'closed' && (
                    <Tooltip label="This milestone is not closed" withArrow>
                      <IconAlertTriangle size={13} color="#f59f00" style={{ flexShrink: 0 }} />
                    </Tooltip>
                  )}
                </div>
                <Text size="xs" c="dimmed">
                  {m.open_issues} open · {m.closed_issues} closed
                </Text>
              </Combobox.Option>
            ))}
          </Combobox.Options>
        </Combobox.Dropdown>
      </Combobox>

      {selectedItems.length > 0 && (
        <Stack gap={4}>
          {selectedItems.map((m) => (
            <ArchiveMilestoneCard
              key={m.number}
              milestone={m}
              statusInfo={
                statusByMilestone[m.number] ?? {
                  listFailed: false,
                  listError: null,
                  loadingCount: 0,
                  statusErrorCount: 0,
                  statusErrors: [],
                  statusAttemptedCount: 0,
                }
              }
              unapprovedCount={unapprovedByMilestone[m.number] ?? 0}
              onRemove={() => remove(m.number)}
            />
          ))}
        </Stack>
      )}
    </Stack>
  )
}

// ─── ArchiveMilestoneCard ─────────────────────────────────────────────────────

function ArchiveMilestoneCard({
  milestone,
  statusInfo,
  unapprovedCount,
  onRemove,
}: {
  milestone: import('~/api/milestones').Milestone
  statusInfo: MilestoneStatusInfo
  unapprovedCount: number
  onRemove: () => void
}) {
  // Red = total failure: list couldn't load, or every status fetch failed
  const isRed =
    statusInfo.listFailed ||
    (statusInfo.statusErrorCount > 0 &&
      statusInfo.statusErrorCount >= statusInfo.statusAttemptedCount)
  // Yellow = partial failure: some issues loaded, some didn't (206-style)
  const isYellow = !isRed && statusInfo.statusErrorCount > 0
  const bgColor = isRed ? '#ffe3e3' : isYellow ? '#fff3bf' : '#d7e7d3'
  const borderColor = isRed ? '#ff8787' : isYellow ? '#fcc419' : '#aacca6'

  const errorLines =
    statusInfo.statusErrors.length > 0 ? (
      <div>
        {statusInfo.statusErrors.map((e) => (
          <div key={e.issue_number}>
            #{e.issue_number}: {e.error}
          </div>
        ))}
      </div>
    ) : null

  return (
    <div
      style={{
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'space-between',
        gap: 6,
        padding: '6px 8px',
        borderRadius: 6,
        backgroundColor: bgColor,
        border: `1px solid ${borderColor}`,
      }}
    >
      <div style={{ minWidth: 0 }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 6, minWidth: 0 }}>
          <Text size="sm" fw={600} truncate="end">
            {milestone.title}
          </Text>

          {/* List fetch error */}
          {statusInfo.listFailed && statusInfo.listError && (
            <Tooltip label={`${statusInfo.listError}`} withArrow>
              <IconExclamationMark
                data-testid="list-error-indicator"
                size={14}
                color="#e67700"
                style={{ flexShrink: 0 }}
              />
            </Tooltip>
          )}

          {/* Status fetch errors */}
          {statusInfo.statusErrorCount > 0 && errorLines && (
            <Tooltip label={errorLines} withArrow multiline>
              <span
                data-testid="status-error-count"
                style={{
                  color: '#e67700',
                  display: 'flex',
                  alignItems: 'center',
                  gap: 2,
                  flexShrink: 0,
                }}
              >
                <IconAlertCircle size={14} />
                {statusInfo.statusErrorCount}
              </span>
            </Tooltip>
          )}
        </div>

        <Text size="xs" c="dimmed">
          {milestone.open_issues} open · {milestone.closed_issues} closed
        </Text>

        {unapprovedCount > 0 && (
          <Text size="xs" c="dimmed">
            {unapprovedCount} unapproved
          </Text>
        )}

        {statusInfo.loadingCount > 0 && (
          <Text
            size="xs"
            c="dimmed"
            style={{ animation: 'glisten 1.4s ease-in-out infinite' }}
          >
            {statusInfo.loadingCount}{' '}
            {statusInfo.loadingCount === 1 ? 'issue' : 'issues'} loading…
          </Text>
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

import { useEffect, useMemo, useRef, useState } from 'react'
import {
  ActionIcon,
  Alert,
  Anchor,
  Button,
  Combobox,
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
  IconArrowBackUp,
  IconChevronDown,
  IconChevronRight,
  IconExclamationMark,
  IconLockOpen,
  IconX,
} from '@tabler/icons-react'
import { useQueries } from '@tanstack/react-query'
import { useMilestones } from '~/api/milestones'
import {
  type IssueStatusError,
  type IssueStatusResponse,
  type IssueStatusResult,
  type MilestoneStatusInfo,
  fetchSingleIssueStatus,
  useMilestoneIssues,
} from '~/api/issues'
import { type ArchiveFileRequest, generateArchive } from '~/api/archive'
import { useRepoInfo } from '~/api/repo'
import { ResizableSidebar } from './ResizableSidebar'
import { type BareFileResolution, BareFileResolveModal } from './BareFileResolveModal'

// ─── Constants ────────────────────────────────────────────────────────────────

const COLLAPSED_HEIGHT = 36
const MIN_ISSUES_HEIGHT = 80

// ─── Types ────────────────────────────────────────────────────────────────────

type RelevantEntry =
  | { type: 'qc'; file_name: string; issue_number: number }
  | { type: 'bare'; file_name: string }
  | { type: 'conflict'; file_name: string; reason: string }

function extractIssueNumber(url: string): number | null {
  const match = url.match(/\/issues\/(\d+)(?:[^/]*)$/)
  return match ? parseInt(match[1], 10) : null
}

function isApprovedStatus(s: IssueStatusResponse): boolean {
  return s.qc_status.status === 'approved' || s.qc_status.status === 'changes_after_approval'
}

// ─── ArchiveTab ───────────────────────────────────────────────────────────────

export function ArchiveTab() {
  const [selectedMilestones, setSelectedMilestones] = useState<number[]>([])
  const [showOpenMilestones, setShowOpenMilestones] = useState(false)
  const [includeNonApproved, setIncludeNonApproved] = useState(false)
  const [includeRelevantFiles, setIncludeRelevantFiles] = useState(false)
  const [outputPath, setOutputPath] = useState('')
  const [generateLoading, setGenerateLoading] = useState(false)
  const [generateError, setGenerateError] = useState<string | null>(null)
  const [generateSuccess, setGenerateSuccess] = useState<string | null>(null)
  const outputPathUserEdited = useRef(false)
  const [outputPathIsCustom, setOutputPathIsCustom] = useState(false)

  // Bare file resolution state
  const [resolvedBareFiles, setResolvedBareFiles] = useState<Map<string, BareFileResolution>>(
    new Map(),
  )
  const [bareFileModalFile, setBareFileModalFile] = useState<string | null>(null)
  const [dismissedRelevantFiles, setDismissedRelevantFiles] = useState<Set<string>>(new Set())

  function dismissRelevantFile(fileName: string) {
    setDismissedRelevantFiles(prev => new Set([...prev, fileName]))
    setResolvedBareFiles(prev => {
      if (!prev.has(fileName)) return prev
      const next = new Map(prev)
      next.delete(fileName)
      return next
    })
  }

  // Sidebar section layout state
  const [milestoneCollapsed, setMilestoneCollapsed] = useState(false)
  const [issuesCollapsed, setIssuesCollapsed] = useState(false)
  const [issuesHeight, setIssuesHeight] = useState(120)
  const [outputHeight, setOutputHeight] = useState<number | null>(null)
  const sidebarRef = useRef<HTMLDivElement>(null)
  const outputSectionRef = useRef<HTMLDivElement>(null)
  const minOutputHeightRef = useRef(0)
  const currentOutputHeightRef = useRef(0)
  const lastIssuesHeightRef = useRef(120)
  // Output drag refs
  const isDraggingOutput = useRef(false)
  const dragStartYOutput = useRef(0)
  const dragStartHeightOutput = useRef(0)
  // Issues drag refs
  const isDraggingIssues = useRef(false)
  const dragStartYIssues = useRef(0)
  const dragStartHeightIssues = useRef(0)
  if (outputHeight !== null) currentOutputHeightRef.current = outputHeight

  const bothCollapsed = milestoneCollapsed && issuesCollapsed

  const { data: repoData } = useRepoInfo()
  const { data: milestonesData } = useMilestones()

  const { statuses, milestoneStatusByMilestone, isLoadingStatuses } =
    useMilestoneIssues(selectedMilestones, true)

  const milestoneStatusRef = useRef(milestoneStatusByMilestone)
  milestoneStatusRef.current = milestoneStatusByMilestone

  function hasErrors(n: number): boolean {
    const info = milestoneStatusRef.current[n]
    return !info || info.listFailed || info.statusErrorCount > 0
  }

  const unapprovedByMilestone = useMemo(() => {
    const result: Record<number, number> = {}
    for (const n of selectedMilestones) {
      const milestoneName = (milestonesData ?? []).find((m) => m.number === n)?.title
      const milestoneStatuses = statuses.filter((s) => s.issue.milestone === milestoneName)
      result[n] = milestoneStatuses.filter((s) => !isApprovedStatus(s)).length
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

  // Milestones that have at least one issue visible in the right panel
  const milestonesWithVisibleIssues = useMemo(() => {
    const visibleTitles = new Set(
      statuses.filter(s => includeNonApproved || isApprovedStatus(s)).map(s => s.issue.milestone),
    )
    return selectedMilestones.filter((n) => {
      const title = (milestonesData ?? []).find((m) => m.number === n)?.title
      return title !== undefined && visibleTitles.has(title)
    })
  }, [selectedMilestones, statuses, milestonesData, includeNonApproved])

  function buildOutputPathName(milestoneNumbers: number[]) {
    if (!repoData || milestoneNumbers.length === 0) return ''
    const names = milestoneNumbers
      .map((n) => (milestonesData ?? []).find((m) => m.number === n)?.title ?? String(n))
      .join('-')
      .replace(/\s+/g, '-')
    return `${repoData.repo}-${names}.tar.gz`
  }

  function resetOutputPath() {
    outputPathUserEdited.current = false
    setOutputPathIsCustom(false)
    setOutputPath(buildOutputPathName(milestonesWithVisibleIssues))
  }

  // Measure output section height on mount
  useEffect(() => {
    if (sidebarRef.current && outputSectionRef.current) {
      const outH = outputSectionRef.current.clientHeight
      if (outH > 0) {
        minOutputHeightRef.current = outH
        currentOutputHeightRef.current = outH
        setOutputHeight(outH)
      }
    }
  }, [])

  // Unified drag-to-resize for output and issues handles
  useEffect(() => {
    const onMove = (e: MouseEvent) => {
      if (isDraggingOutput.current) {
        const delta = e.clientY - dragStartYOutput.current
        const min = minOutputHeightRef.current
        const max = (sidebarRef.current?.clientHeight ?? 600) - COLLAPSED_HEIGHT * 2
        setOutputHeight(Math.max(min, Math.min(max, dragStartHeightOutput.current + delta)))
      }
      if (isDraggingIssues.current) {
        const delta = dragStartYIssues.current - e.clientY
        const totalH = sidebarRef.current?.clientHeight ?? 600
        const maxH = totalH - currentOutputHeightRef.current - COLLAPSED_HEIGHT
        setIssuesHeight(Math.max(MIN_ISSUES_HEIGHT, Math.min(maxH, dragStartHeightIssues.current + delta)))
      }
    }
    const onUp = () => {
      isDraggingOutput.current = false
      isDraggingIssues.current = false
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

  const onOutputDragHandleMouseDown = (e: React.MouseEvent) => {
    isDraggingOutput.current = true
    dragStartYOutput.current = e.clientY
    dragStartHeightOutput.current = outputHeight ?? minOutputHeightRef.current
    document.body.style.cursor = 'row-resize'
    document.body.style.userSelect = 'none'
    e.preventDefault()
  }

  const onIssuesDragHandleMouseDown = (e: React.MouseEvent) => {
    isDraggingIssues.current = true
    dragStartYIssues.current = e.clientY
    dragStartHeightIssues.current = issuesHeight
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

  // Auto-populate output path — only names milestones that have visible issues
  useEffect(() => {
    if (outputPathUserEdited.current) return
    setOutputPath(buildOutputPathName(milestonesWithVisibleIssues))
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [milestonesWithVisibleIssues, repoData])

  // ─── Relevant files derived state ────────────────────────────────────────

  const relevantFileEntries = useMemo<RelevantEntry[]>(() => {
    if (!includeRelevantFiles || statuses.length === 0) return []

    const archiveIssueNumbers = new Set(statuses.map(s => s.issue.number))
    const archiveFileNames = new Set(statuses.map(s => s.issue.title))

    // file_name → array of sources
    const fileSourceMap = new Map<string, Array<{ kind: 'qc' | 'bare'; issue_number: number | null }>>()

    for (const status of statuses) {
      for (const rf of status.issue.relevant_files) {
        const isQcKind = rf.kind === 'blocking_qc' || rf.kind === 'relevant_qc'
        const issueNumber = rf.issue_url ? extractIssueNumber(rf.issue_url) : null

        if (isQcKind && issueNumber !== null) {
          if (archiveIssueNumbers.has(issueNumber)) continue // true de-dup
          const existing = fileSourceMap.get(rf.file_name)
          if (!existing) {
            fileSourceMap.set(rf.file_name, [{ kind: 'qc', issue_number: issueNumber }])
          } else if (!existing.some(e => e.issue_number === issueNumber)) {
            existing.push({ kind: 'qc', issue_number: issueNumber })
          }
        } else {
          // Bare file (file kind or QC kind with null issue_url)
          const existing = fileSourceMap.get(rf.file_name)
          if (!existing) {
            fileSourceMap.set(rf.file_name, [{ kind: 'bare', issue_number: null }])
          } else if (!existing.some(e => e.kind === 'bare')) {
            existing.push({ kind: 'bare', issue_number: null })
          }
        }
      }
    }

    const entries: RelevantEntry[] = []

    for (const [file_name, sources] of fileSourceMap) {
      if (archiveFileNames.has(file_name)) {
        entries.push({ type: 'conflict', file_name, reason: 'File already covered by milestone issues' })
        continue
      }
      if (sources.length > 1) {
        entries.push({ type: 'conflict', file_name, reason: 'Multiple QC issues reference this file' })
        continue
      }
      const source = sources[0]
      if (source.kind === 'qc' && source.issue_number !== null) {
        entries.push({ type: 'qc', file_name, issue_number: source.issue_number })
      } else {
        entries.push({ type: 'bare', file_name })
      }
    }

    return entries
  }, [includeRelevantFiles, statuses])

  // QC relevant issue numbers not already in statuses
  const qcIssueNumbersToFetch = useMemo(() => {
    const existing = new Set(statuses.map(s => s.issue.number))
    return relevantFileEntries
      .filter((e): e is { type: 'qc'; file_name: string; issue_number: number } => e.type === 'qc')
      .map(e => e.issue_number)
      .filter(n => !existing.has(n))
      .filter((n, i, arr) => arr.indexOf(n) === i)
  }, [relevantFileEntries, statuses])

  const relevantQcQueries = useQueries({
    queries: qcIssueNumbersToFetch.map(n => ({
      queryKey: ['issue', 'status', n],
      queryFn: async (): Promise<IssueStatusResult> => {
        try {
          const data = await fetchSingleIssueStatus(n)
          return { ok: true, data }
        } catch (e) {
          const err: IssueStatusError = {
            issue_number: n,
            kind: 'fetch_failed',
            error: (e as Error).message,
          }
          return { ok: false, error: err }
        }
      },
      staleTime: 5 * 60 * 1000,
      enabled: includeRelevantFiles,
    })),
  })

  // Map issue number → status for QC relevant cards
  const relevantQcStatusMap = useMemo(() => {
    const map = new Map<number, IssueStatusResponse>()
    for (const s of statuses) map.set(s.issue.number, s)
    for (const q of relevantQcQueries) {
      if (q.data?.ok) map.set(q.data.data.issue.number, q.data.data)
    }
    return map
  }, [statuses, relevantQcQueries])

  // Counts for canGenerate
  const unresolvedBareFileCount = useMemo(
    () =>
      relevantFileEntries.filter(e => e.type === 'bare' && !resolvedBareFiles.has(e.file_name))
        .length,
    [relevantFileEntries, resolvedBareFiles],
  )
  const conflictCount = useMemo(
    () => relevantFileEntries.filter(e => e.type === 'conflict').length,
    [relevantFileEntries],
  )

  // ─── Generation ──────────────────────────────────────────────────────────

  async function handleGenerate() {
    setGenerateError(null)
    setGenerateSuccess(null)
    setGenerateLoading(true)
    try {
      const milestoneIssueFiles: ArchiveFileRequest[] = statuses
        .filter(s => includeNonApproved || isApprovedStatus(s))
        .map(s => ({
          repository_file: s.issue.title,
          commit: s.qc_status.approved_commit ?? s.qc_status.latest_commit,
          milestone: s.issue.milestone ?? undefined,
          approved: isApprovedStatus(s),
        }))

      const bareFileFiles: ArchiveFileRequest[] = Array.from(resolvedBareFiles.values()).map(r => ({
        repository_file: r.file_name,
        commit: r.commit,
        approved: false,
      }))

      const qcRelevantFiles: ArchiveFileRequest[] = relevantFileEntries
        .filter((e): e is { type: 'qc'; file_name: string; issue_number: number } => e.type === 'qc')
        .flatMap(e => {
          const s = relevantQcStatusMap.get(e.issue_number)
          if (!s) return []
          return [{
            repository_file: e.file_name,
            commit: s.qc_status.approved_commit ?? s.qc_status.latest_commit,
            approved: isApprovedStatus(s),
          }]
        })

      const files = [...milestoneIssueFiles, ...bareFileFiles, ...qcRelevantFiles]
      const result = await generateArchive({ output_path: outputPath, flatten: false, files })
      setGenerateSuccess(result.output_path)
    } catch (err) {
      setGenerateError((err as Error).message)
    } finally {
      setGenerateLoading(false)
    }
  }

  const visibleFileCount =
    statuses.filter(s => includeNonApproved || isApprovedStatus(s)).length +
    (includeRelevantFiles
      ? relevantFileEntries.filter(e => e.type !== 'conflict').length
      : 0)

  const canGenerate =
    visibleFileCount > 0 &&
    outputPath.trim().length > 0 &&
    !isLoadingStatuses &&
    unresolvedBareFileCount === 0 &&
    conflictCount === 0

  // Referencing statuses for a bare file (for modal)
  const referencingStatusesForFile = useMemo(() => {
    if (!bareFileModalFile) return []
    return statuses.filter(s =>
      s.issue.relevant_files.some(rf => rf.file_name === bareFileModalFile),
    )
  }, [bareFileModalFile, statuses])

  return (
    <div style={{ display: 'flex', height: '100%', overflow: 'hidden' }}>

      {/* ── Left sidebar ─────────────────────────────────────────────────── */}
      <ResizableSidebar defaultWidth={320} minWidth={280} maxWidth={560} noPadding>
        <div ref={sidebarRef} style={{ display: 'flex', flexDirection: 'column', height: '100%' }}>

          {/* ── Output Path + Generate ───────────────────────────────────── */}
          <div
            ref={outputSectionRef}
            style={
              bothCollapsed
                ? { flex: 1, minHeight: 0, overflowY: 'auto', padding: 'var(--mantine-spacing-md)' }
                : { height: outputHeight ?? 'auto', flexShrink: 0, overflowY: 'auto', padding: 'var(--mantine-spacing-md)' }
            }
          >
            <Stack gap="sm">
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
                rightSection={outputPathIsCustom && selectedMilestones.length > 0 ? (
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
                ) : undefined}
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
          </div>

          {/* ── Drag handle A: between Output and Milestones ─────────────── */}
          {!bothCollapsed && outputHeight !== null && (
            <div
              onMouseDown={onOutputDragHandleMouseDown}
              style={{
                height: 6,
                flexShrink: 0,
                cursor: 'row-resize',
                borderTop: '1px solid var(--mantine-color-gray-3)',
              }}
            />
          )}

          {/* ── Milestones (collapsible) ─────────────────────────────────── */}
          <div style={
            milestoneCollapsed
              ? {
                  height: COLLAPSED_HEIGHT,
                  flexShrink: 0,
                  borderTop: bothCollapsed ? '1px solid var(--mantine-color-gray-3)' : undefined,
                }
              : { flex: 1, minHeight: 0, display: 'flex', flexDirection: 'column' }
          }>
            <div
              style={{
                display: 'flex',
                alignItems: 'center',
                gap: 4,
                padding: '0 var(--mantine-spacing-md)',
                height: COLLAPSED_HEIGHT,
                flexShrink: 0,
                borderBottom: milestoneCollapsed ? undefined : '1px solid var(--mantine-color-gray-3)',
                cursor: 'pointer',
              }}
              onClick={() => setMilestoneCollapsed((c) => !c)}
              title={milestoneCollapsed ? 'Expand' : 'Collapse'}
            >
              <ActionIcon size="xs" variant="subtle" tabIndex={-1} style={{ pointerEvents: 'none' }}>
                {milestoneCollapsed ? <IconChevronRight size={14} /> : <IconChevronDown size={14} />}
              </ActionIcon>
              <Text fw={600} size="sm">Milestones</Text>
            </div>
            {!milestoneCollapsed && (
              <div style={{ flex: 1, overflowY: 'auto', padding: 'var(--mantine-spacing-md)' }}>
                <Stack gap="sm">
                  <Switch
                    label="Include open milestones"
                    size="xs"
                    checked={showOpenMilestones}
                    onChange={(e) => setShowOpenMilestones(e.currentTarget.checked)}
                  />
                  <ArchiveMilestoneCombobox
                    selectedMilestones={selectedMilestones}
                    onSelectedMilestonesChange={setSelectedMilestones}
                    showOpenMilestones={showOpenMilestones}
                    statusByMilestone={milestoneStatusByMilestone}
                    unapprovedByMilestone={unapprovedByMilestone}
                  />
                </Stack>
              </div>
            )}
          </div>

          {/* ── Issues (collapsible) ─────────────────────────────────────── */}
          <div style={
            issuesCollapsed
              ? { height: COLLAPSED_HEIGHT, flexShrink: 0 }
              : milestoneCollapsed
                ? { flex: 1, minHeight: 0, display: 'flex', flexDirection: 'column' }
                : { height: issuesHeight, flexShrink: 0, display: 'flex', flexDirection: 'column' }
          }>
            {/* Issues drag handle — only when both milestones and issues are expanded */}
            {!milestoneCollapsed && !issuesCollapsed && (
              <div
                onMouseDown={onIssuesDragHandleMouseDown}
                style={{
                  height: 6,
                  flexShrink: 0,
                  cursor: 'row-resize',
                  borderTop: '1px solid var(--mantine-color-gray-3)',
                }}
              />
            )}

            {/* Issues header — top border when drag handle is absent */}
            <div
              style={{
                display: 'flex',
                alignItems: 'center',
                gap: 4,
                padding: '0 var(--mantine-spacing-md)',
                height: COLLAPSED_HEIGHT,
                flexShrink: 0,
                borderTop: (issuesCollapsed || milestoneCollapsed)
                  ? '1px solid var(--mantine-color-gray-3)'
                  : undefined,
                cursor: 'pointer',
              }}
              onClick={toggleIssuesCollapse}
              title={issuesCollapsed ? 'Expand' : 'Collapse'}
            >
              <ActionIcon size="xs" variant="subtle" tabIndex={-1} style={{ pointerEvents: 'none' }}>
                {issuesCollapsed ? <IconChevronRight size={14} /> : <IconChevronDown size={14} />}
              </ActionIcon>
              <Text fw={600} size="sm">Issues</Text>
            </div>

            {!issuesCollapsed && (
              <div style={{ flex: 1, overflowY: 'auto', padding: '0 var(--mantine-spacing-md) var(--mantine-spacing-md)' }}>
                <Stack gap={6} mt={4}>
                  <Switch
                    label="Include non-approved issues"
                    size="xs"
                    checked={includeNonApproved}
                    onChange={(e) => setIncludeNonApproved(e.currentTarget.checked)}
                  />
                  <Switch
                    label="Include relevant files"
                    size="xs"
                    checked={includeRelevantFiles}
                    onChange={(e) => setIncludeRelevantFiles(e.currentTarget.checked)}
                  />
                </Stack>
              </div>
            )}
          </div>

        </div>
      </ResizableSidebar>

      {/* ── Right panel: issue cards ──────────────────────────────────────── */}
      <div style={{ flex: 1, overflowY: 'auto', padding: 'var(--mantine-spacing-md)' }}>
        {selectedMilestones.length === 0 ? (
          <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'center', height: '100%' }}>
            <Text c="dimmed" size="sm">Select a milestone to see issues</Text>
          </div>
        ) : (
          <div style={{
            display: 'grid',
            gridTemplateColumns: 'repeat(auto-fill, minmax(160px, 1fr))',
            gap: 12,
          }}>
            {/* ── Milestone issue cards ──────────────────────────────────── */}
            {statuses.filter(s => includeNonApproved || isApprovedStatus(s)).map((s) => {
              const approved = isApprovedStatus(s)
              const commit = s.qc_status.approved_commit ?? s.qc_status.latest_commit
              const shortCommit = commit.slice(0, 7)
              const statusLabel = s.qc_status.status.replace(/_/g, ' ')

              return (
                <Stack
                  key={s.issue.number}
                  gap={5}
                  style={{
                    padding: '10px 12px',
                    borderRadius: 6,
                    border: '1px solid var(--mantine-color-gray-3)',
                    backgroundColor: 'white',
                    minWidth: 0,
                  }}
                >
                  <div style={{ display: 'flex', alignItems: 'flex-start', gap: 4, minWidth: 0 }}>
                    <Anchor
                      href={s.issue.html_url}
                      target="_blank"
                      size="sm"
                      fw={700}
                      style={{ wordBreak: 'break-all', flex: 1, minWidth: 0 }}
                    >
                      {s.issue.title}
                    </Anchor>
                    {!approved && (
                      <Tooltip label="Not yet approved" withArrow>
                        <span style={{ flexShrink: 0, marginTop: 2 }}>
                          <IconAlertTriangle size={12} color="#f59f00" />
                        </span>
                      </Tooltip>
                    )}
                  </div>
                  {s.issue.milestone && (
                    <Text size="xs" c="dimmed"><b>Milestone:</b> {s.issue.milestone}</Text>
                  )}
                  <Text size="xs" c="dimmed"><b>Commit:</b> {shortCommit}</Text>
                  <Text size="xs" c="dimmed"><b>Status:</b> {statusLabel}</Text>
                </Stack>
              )
            })}

            {/* ── Relevant file cards ────────────────────────────────────── */}
            {includeRelevantFiles && relevantFileEntries
              .filter(e => !dismissedRelevantFiles.has(e.file_name))
              .map((entry) => {
              if (entry.type === 'conflict') {
                return (
                  <Stack
                    key={`conflict-${entry.file_name}`}
                    gap={5}
                    style={{
                      padding: '10px 12px',
                      borderRadius: 6,
                      border: '1px solid #ff8787',
                      backgroundColor: '#ffe3e3',
                      minWidth: 0,
                    }}
                  >
                    <div style={{ display: 'flex', alignItems: 'flex-start', gap: 4 }}>
                      <Text size="sm" fw={700} style={{ wordBreak: 'break-all', flex: 1 }}>
                        {entry.file_name}
                      </Text>
                      <ActionIcon size="xs" variant="transparent" color="dark" style={{ flexShrink: 0, marginTop: 1 }} onClick={() => dismissRelevantFile(entry.file_name)} aria-label="Remove">
                        <IconX size={11} />
                      </ActionIcon>
                    </div>
                    <Text size="xs" c="dimmed"><b>Type:</b> Conflict</Text>
                    <Text size="xs" c="red.7">{entry.reason}</Text>
                  </Stack>
                )
              }

              if (entry.type === 'bare') {
                const resolution = resolvedBareFiles.get(entry.file_name)
                const bgColor = resolution ? '#d7e7d3' : '#fff3bf'
                const borderColor = resolution ? '#aacca6' : '#fcc419'

                return (
                  <Stack
                    key={`bare-${entry.file_name}`}
                    gap={5}
                    style={{
                      padding: '10px 12px',
                      borderRadius: 6,
                      border: `1px solid ${borderColor}`,
                      backgroundColor: bgColor,
                      minWidth: 0,
                    }}
                  >
                    <div style={{ display: 'flex', alignItems: 'flex-start', gap: 4 }}>
                      <Text
                        size="sm"
                        fw={700}
                        style={{ wordBreak: 'break-all', flex: 1, cursor: 'pointer' }}
                        onClick={() => setBareFileModalFile(entry.file_name)}
                      >
                        {entry.file_name}
                      </Text>
                      <ActionIcon size="xs" variant="transparent" color="dark" style={{ flexShrink: 0, marginTop: 1 }} onClick={() => dismissRelevantFile(entry.file_name)} aria-label="Remove">
                        <IconX size={11} />
                      </ActionIcon>
                    </div>
                    <Text size="xs" c="dimmed"><b>Type:</b> Bare file</Text>
                    {resolution ? (
                      <Text size="xs" c="dimmed" style={{ cursor: 'pointer' }} onClick={() => setBareFileModalFile(entry.file_name)}>
                        <b>Commit:</b> {resolution.commit.slice(0, 7)}
                      </Text>
                    ) : (
                      <Text size="xs" c="orange.7" style={{ cursor: 'pointer' }} onClick={() => setBareFileModalFile(entry.file_name)}>Click to resolve</Text>
                    )}
                  </Stack>
                )
              }

              // QC relevant card
              const qcStatus = relevantQcStatusMap.get(entry.issue_number)
              const isLoadingQc = relevantQcQueries.some(
                q => q.isPending && q.fetchStatus !== 'idle',
              )

              if (!qcStatus) {
                return (
                  <Stack
                    key={`qc-${entry.file_name}`}
                    gap={5}
                    style={{
                      padding: '10px 12px',
                      borderRadius: 6,
                      border: '1px solid var(--mantine-color-gray-3)',
                      backgroundColor: 'white',
                      minWidth: 0,
                    }}
                  >
                    <div style={{ display: 'flex', alignItems: 'flex-start', gap: 4 }}>
                      <Text size="sm" fw={700} style={{ wordBreak: 'break-all', flex: 1 }}>
                        {entry.file_name}
                      </Text>
                      <ActionIcon size="xs" variant="transparent" color="dark" style={{ flexShrink: 0, marginTop: 1 }} onClick={() => dismissRelevantFile(entry.file_name)} aria-label="Remove">
                        <IconX size={11} />
                      </ActionIcon>
                    </div>
                    {isLoadingQc ? (
                      <div style={{ display: 'flex', alignItems: 'center', gap: 4 }}>
                        <Loader size={10} />
                        <Text size="xs" c="dimmed">Loading…</Text>
                      </div>
                    ) : (
                      <Text size="xs" c="dimmed">Issue #{entry.issue_number}</Text>
                    )}
                  </Stack>
                )
              }

              const approved = isApprovedStatus(qcStatus)
              const commit = qcStatus.qc_status.approved_commit ?? qcStatus.qc_status.latest_commit
              const statusLabel = qcStatus.qc_status.status.replace(/_/g, ' ')

              return (
                <Stack
                  key={`qc-${entry.file_name}`}
                  gap={5}
                  style={{
                    padding: '10px 12px',
                    borderRadius: 6,
                    border: '1px solid var(--mantine-color-gray-3)',
                    backgroundColor: 'white',
                    minWidth: 0,
                  }}
                >
                  <div style={{ display: 'flex', alignItems: 'flex-start', gap: 4, minWidth: 0 }}>
                    <Anchor
                      href={qcStatus.issue.html_url}
                      target="_blank"
                      size="sm"
                      fw={700}
                      style={{ wordBreak: 'break-all', flex: 1, minWidth: 0 }}
                    >
                      {qcStatus.issue.title}
                    </Anchor>
                    {!approved && (
                      <Tooltip label="Not yet approved" withArrow>
                        <span style={{ flexShrink: 0, marginTop: 2 }}>
                          <IconAlertTriangle size={12} color="#f59f00" />
                        </span>
                      </Tooltip>
                    )}
                    <ActionIcon size="xs" variant="transparent" color="dark" style={{ flexShrink: 0, marginTop: 1 }} onClick={() => dismissRelevantFile(entry.file_name)} aria-label="Remove">
                      <IconX size={11} />
                    </ActionIcon>
                  </div>
                  {qcStatus.issue.milestone && (
                    <Text size="xs" c="dimmed"><b>Milestone:</b> {qcStatus.issue.milestone}</Text>
                  )}
                  <Text size="xs" c="dimmed"><b>Commit:</b> {commit.slice(0, 7)}</Text>
                  <Text size="xs" c="dimmed"><b>Status:</b> {statusLabel}</Text>
                  <Text size="xs" c="dimmed" style={{ fontStyle: 'italic' }}>Relevant file</Text>
                </Stack>
              )
            })}
          </div>
        )}
      </div>

      {/* ── Bare file resolve modal ───────────────────────────────────────── */}
      {bareFileModalFile && (
        <BareFileResolveModal
          opened={bareFileModalFile !== null}
          onClose={() => setBareFileModalFile(null)}
          fileName={bareFileModalFile}
          referencingStatuses={referencingStatusesForFile}
          allStatuses={statuses}
          onResolve={(resolution) => {
            setResolvedBareFiles(prev => {
              const next = new Map(prev)
              next.set(resolution.file_name, resolution)
              return next
            })
          }}
        />
      )}
    </div>
  )
}

// ─── ArchiveMilestoneCombobox ─────────────────────────────────────────────────

interface ArchiveMilestoneComboboxProps {
  selectedMilestones: number[]
  onSelectedMilestonesChange: (v: number[]) => void
  showOpenMilestones: boolean
  statusByMilestone: Record<number, MilestoneStatusInfo>
  unapprovedByMilestone: Record<number, number>
}

function ArchiveMilestoneCombobox({
  selectedMilestones,
  onSelectedMilestonesChange,
  showOpenMilestones,
  statusByMilestone,
  unapprovedByMilestone,
}: ArchiveMilestoneComboboxProps) {
  const { data, isLoading, isError } = useMilestones()
  const [search, setSearch] = useState('')
  const combobox = useCombobox({ onDropdownClose: () => setSearch('') })

  const available = (data ?? []).filter(
    (m) => (showOpenMilestones || m.state === 'closed') && !selectedMilestones.includes(m.number),
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
          <Combobox.Options style={{ maxHeight: 360, overflowY: 'auto' }}>
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
              statusInfo={statusByMilestone[m.number] ?? { listFailed: false, listError: null, loadingCount: 0, statusErrorCount: 0, statusErrors: [], statusAttemptedCount: 0 }}
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
  const isRed = statusInfo.listFailed || statusInfo.statusErrorCount > 0
  const isYellow = !isRed && unapprovedCount > 0

  const bgColor = isRed ? '#ffe3e3' : isYellow ? '#fff3bf' : '#d7e7d3'
  const borderColor = isRed ? '#ff8787' : isYellow ? '#fcc419' : '#aacca6'

  const errorLines =
    statusInfo.statusErrors.length > 0 ? (
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
          {milestone.state !== 'closed' && (
            <Tooltip label="Milestone is not yet closed — archive may be incomplete" withArrow>
              <IconLockOpen data-testid="open-milestone-indicator" size={14} color="#e67700" style={{ flexShrink: 0 }} />
            </Tooltip>
          )}
          {statusInfo.listFailed && statusInfo.listError && (
            <Tooltip label={`${statusInfo.listError} — excluded from archive`} withArrow>
              <IconExclamationMark data-testid="list-error-indicator" size={14} color="#c92a2a" style={{ flexShrink: 0 }} />
            </Tooltip>
          )}
          {statusInfo.statusErrorCount > 0 && errorLines && (
            <Tooltip label={errorLines} withArrow multiline>
              <span data-testid="status-error-count" style={{ color: '#c92a2a', display: 'flex', alignItems: 'center', gap: 2, flexShrink: 0 }}>
                <IconAlertCircle size={14} />
                {statusInfo.statusErrorCount}
              </span>
            </Tooltip>
          )}
          {isYellow && (
            <Tooltip
              label={`${unapprovedCount} issue${unapprovedCount !== 1 ? 's' : ''} not yet approved`}
              withArrow
            >
              <span data-testid="unapproved-warning" style={{ color: '#e67700', display: 'flex', alignItems: 'center', gap: 2, flexShrink: 0 }}>
                <IconAlertTriangle size={14} />
                {unapprovedCount}
              </span>
            </Tooltip>
          )}
        </div>
        <Text size="xs" c="dimmed">
          {milestone.open_issues} open · {milestone.closed_issues} closed
        </Text>
        {statusInfo.loadingCount > 0 && (
          <Text size="xs" c="dimmed" style={{ animation: 'glisten 1.4s ease-in-out infinite' }}>
            {statusInfo.loadingCount} {statusInfo.loadingCount === 1 ? 'issue' : 'issues'} loading…
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

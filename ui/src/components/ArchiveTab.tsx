import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
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
  IconExclamationMark,
  IconX,
} from '@tabler/icons-react'
import { useQueries } from '@tanstack/react-query'
import { useMilestones } from '~/api/milestones'
import {
  type IssueStatusResponse,
  type RelevantFileInfo,
  type MilestoneStatusInfo,
  fetchMilestoneIssues,
  issueStatusBatcher,
  useMilestoneIssues,
} from '~/api/issues'
import { type ArchiveFileRequest, generateArchive } from '~/api/archive'
import { useRepoInfo } from '~/api/repo'
import { OpenPill } from './MilestoneFilter'
import { ResizableSidebar } from './ResizableSidebar'
import { type FileResolution, FileResolveModal } from './FileResolveModal'
import { RelevantFilesList } from './RelevantFilesList'
import { extractIssueNumber } from '~/utils'

// ─── Constants ────────────────────────────────────────────────────────────────

const CARD_HEIGHT = 185


function isApprovedStatus(s: IssueStatusResponse): boolean {
  return s.qc_status.status === 'approved' || s.qc_status.status === 'changes_after_approval'
}

function basename(path: string): string {
  return path.split('/').pop() ?? path
}

// ─── ArchiveTab ───────────────────────────────────────────────────────────────

export function ArchiveTab() {
  const [selectedMilestones, setSelectedMilestones] = useState<number[]>([])
  const [showOpenMilestones, setShowOpenMilestones] = useState(false)
  const [includeNonApproved, setIncludeNonApproved] = useState<Record<number, boolean>>({})
  const [outputPath, setOutputPath] = useState('')
  const [flatten, setFlatten] = useState(false)
  const [generateLoading, setGenerateLoading] = useState(false)
  const [generateError, setGenerateError] = useState<string | null>(null)
  const [generateSuccess, setGenerateSuccess] = useState<string | null>(null)
  const outputPathUserEdited = useRef(false)
  const [outputPathIsCustom, setOutputPathIsCustom] = useState(false)

  const [addedFiles, setAddedFiles] = useState<Map<string, FileResolution>>(new Map())

  // Single modal state for editing any resolved/bare file
  const [editFileModal, setEditFileModal] = useState<string | null>(null)
  const [addFileModalOpen, setAddFileModalOpen] = useState(false)

  function handleEditResolve(resolution: FileResolution) {
    const { file_name } = resolution
    setAddedFiles(prev => new Map([...prev, [file_name, resolution]]))
  }


  const { data: repoData } = useRepoInfo()
  const { data: milestonesData } = useMilestones()

  // Pre-fetch all milestone issue lists to warm the cache and detect conflicts
  const allMilestoneNumbers = useMemo(
    () => (milestonesData ?? []).map(m => m.number),
    [milestonesData],
  )

  const allMilestoneIssueQueries = useQueries({
    queries: allMilestoneNumbers.map(n => ({
      queryKey: ['milestones', n, 'issues'],
      queryFn: () => fetchMilestoneIssues(n),
    })),
  })

  // Per-milestone file sets: "all" = every issue title, "approvedOnly" = closed issues only
  const milestoneFileSets = useMemo(() => {
    const map = new Map<number, { all: Set<string>; approvedOnly: Set<string> }>()
    for (let i = 0; i < allMilestoneNumbers.length; i++) {
      const issues = allMilestoneIssueQueries[i]?.data ?? []
      map.set(allMilestoneNumbers[i], {
        all: new Set(issues.map(iss => iss.title)),
        approvedOnly: new Set(issues.filter(iss => iss.state === 'closed').map(iss => iss.title)),
      })
    }
    return map
  }, [allMilestoneNumbers, allMilestoneIssueQueries])

  const { statuses, milestoneStatusByMilestone, isLoadingStatuses } =
    useMilestoneIssues(selectedMilestones, true)

  // Fetch statuses for manually added files that came from an issue
  const addedFileIssueNums = useMemo(
    () => [...addedFiles.values()]
      .filter((r) => r.source_issue_number != null)
      .map((r) => r.source_issue_number!)
      .filter((n, i, arr) => arr.indexOf(n) === i),
    [addedFiles],
  )

  const addedFileStatusQueries = useQueries({
    queries: addedFileIssueNums.map((num) => ({
      queryKey: ['issue', 'status', num],
      queryFn: () => issueStatusBatcher.load(num),
      staleTime: 5 * 60 * 1000,
    })),
  })

  const addedFileStatusMap = useMemo(() => {
    const m = new Map<number, IssueStatusResponse>()
    for (let i = 0; i < addedFileIssueNums.length; i++) {
      const q = addedFileStatusQueries[i]
      if (q.data?.ok) m.set(addedFileIssueNums[i], q.data.data)
    }
    return m
  }, [addedFileIssueNums, addedFileStatusQueries])

  // Map milestone title → number for per-milestone visibility checks
  const milestoneTitleToNumber = useMemo(() => {
    const map = new Map<string, number>()
    for (const m of milestonesData ?? []) map.set(m.title, m.number)
    return map
  }, [milestonesData])

  // Whether a status is visible given per-milestone includeNonApproved settings
  const isStatusVisible = useCallback((s: IssueStatusResponse): boolean => {
    if (isApprovedStatus(s)) return true
    const msNum = s.issue.milestone ? milestoneTitleToNumber.get(s.issue.milestone) : undefined
    return msNum !== undefined && !!includeNonApproved[msNum]
  }, [milestoneTitleToNumber, includeNonApproved])

  const unapprovedByMilestone = useMemo(() => {
    const result: Record<number, number> = {}
    for (const n of selectedMilestones) {
      const milestoneName = (milestonesData ?? []).find((m) => m.number === n)?.title
      const milestoneStatuses = statuses.filter((s) => s.issue.milestone === milestoneName)
      result[n] = milestoneStatuses.filter((s) => !isApprovedStatus(s)).length
    }
    return result
  }, [selectedMilestones, statuses, milestonesData])

  // Per-milestone: detect if enabling non-approved for a milestone would conflict with other milestones
  const nonApprovedOverlapByMilestone = useMemo(() => {
    const result: Record<number, string[]> = {}
    if (selectedMilestones.length < 2) return result
    for (const candidate of selectedMilestones) {
      const candidateAll = milestoneFileSets.get(candidate)?.all ?? new Set<string>()
      const candidateApproved = milestoneFileSets.get(candidate)?.approvedOnly ?? new Set<string>()
      // Files that are non-approved-only in this milestone
      const nonApprovedFiles = [...candidateAll].filter(f => !candidateApproved.has(f))
      // Build set of files from other milestones (using basenames when flatten is ON)
      const otherFiles = new Set<string>()
      for (const other of selectedMilestones) {
        if (other === candidate) continue
        const otherSet = includeNonApproved[other]
          ? milestoneFileSets.get(other)?.all
          : milestoneFileSets.get(other)?.approvedOnly
        if (otherSet) {
          for (const f of otherSet) otherFiles.add(flatten ? basename(f) : f)
        }
      }
      // Check if any non-approved files conflict (using basenames when flatten is ON)
      const conflicts: string[] = []
      for (const f of nonApprovedFiles) {
        if (otherFiles.has(flatten ? basename(f) : f)) {
          conflicts.push(f)
        }
      }
      if (conflicts.length > 0) result[candidate] = conflicts
    }
    return result
  }, [selectedMilestones, milestoneFileSets, includeNonApproved, flatten])

  // Force off any milestone's includeNonApproved that now conflicts
  useEffect(() => {
    const toDisable: number[] = []
    for (const [msNum, conflicts] of Object.entries(nonApprovedOverlapByMilestone)) {
      const n = Number(msNum)
      if (conflicts.length > 0 && includeNonApproved[n]) toDisable.push(n)
    }
    if (toDisable.length > 0) {
      setIncludeNonApproved(prev => {
        const next = { ...prev }
        for (const n of toDisable) next[n] = false
        return next
      })
    }
  }, [nonApprovedOverlapByMilestone, includeNonApproved])

  // Milestones that have at least one issue visible in the right panel
  const milestonesWithVisibleIssues = useMemo(() => {
    const visibleTitles = new Set(
      statuses.filter(s => isStatusVisible(s)).map(s => s.issue.milestone),
    )
    return selectedMilestones.filter((n) => {
      const title = (milestonesData ?? []).find((m) => m.number === n)?.title
      return title !== undefined && visibleTitles.has(title)
    })
  }, [selectedMilestones, statuses, milestonesData, isStatusVisible])

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

  // Auto-populate output path — only names milestones that have visible issues
  useEffect(() => {
    if (outputPathUserEdited.current) return
    setOutputPath(buildOutputPathName(milestonesWithVisibleIssues))
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [milestonesWithVisibleIssues, repoData])

  // Conflict state for manually added files
  const addedFileConflicts = useMemo(() => {
    const map = new Map<string, { reason: string; blocking: boolean; dedup: boolean }>()
    const visibleIssuesByTitle = new Map<string, number>()
    statuses
      .filter(s => isStatusVisible(s))
      .forEach(s => visibleIssuesByTitle.set(s.issue.title, s.issue.number))

    for (const [fn, res] of addedFiles) {
      const milestoneIssueNumber = visibleIssuesByTitle.get(fn)
      if (milestoneIssueNumber !== undefined) {
        if (res.source_issue_number !== undefined && res.source_issue_number === milestoneIssueNumber) {
          map.set(fn, { reason: '', blocking: false, dedup: true })
        } else {
          map.set(fn, { reason: 'File already covered by milestone issues', blocking: false, dedup: false })
        }
      }
    }
    return map
  }, [addedFiles, statuses, isStatusVisible])

  // Check for unresolved added files (bare files with empty commit and no backing issue)
  const unresolvedAddedFileCount = useMemo(
    () => Array.from(addedFiles.values()).filter(r => r.commit === '' && r.source_issue_number == null).length,
    [addedFiles],
  )
  const conflictCount = useMemo(
    () => Array.from(addedFileConflicts.values()).filter(c => c.blocking).length,
    [addedFileConflicts],
  )

  // ─── Relevant file selection handlers ──────────────────────────────────

  function handleSelectRelevantFile(rf: RelevantFileInfo) {
    const isQc = rf.kind === 'blocking_qc' || rf.kind === 'relevant_qc'
    if (isQc && rf.issue_url) {
      const issueNumber = extractIssueNumber(rf.issue_url)
      if (issueNumber !== null) {
        setAddedFiles(prev => {
          const next = new Map(prev)
          next.set(rf.file_name, { file_name: rf.file_name, commit: '', source_issue_number: issueNumber })
          return next
        })
        return
      }
    }
    // Bare file — add as unresolved and open modal
    setAddedFiles(prev => {
      const next = new Map(prev)
      next.set(rf.file_name, { file_name: rf.file_name, commit: '' })
      return next
    })
    setEditFileModal(rf.file_name)
  }

  function handleSelectAllRelevant(files: RelevantFileInfo[]) {
    setAddedFiles(prev => {
      const next = new Map(prev)
      for (const rf of files) {
        const isQc = rf.kind === 'blocking_qc' || rf.kind === 'relevant_qc'
        if (isQc && rf.issue_url) {
          const issueNumber = extractIssueNumber(rf.issue_url)
          if (issueNumber !== null) {
            next.set(rf.file_name, { file_name: rf.file_name, commit: '', source_issue_number: issueNumber })
            continue
          }
        }
        next.set(rf.file_name, { file_name: rf.file_name, commit: '' })
      }
      return next
    })
  }

  // ─── Generation ──────────────────────────────────────────────────────────

  async function handleGenerate() {
    setGenerateError(null)
    setGenerateSuccess(null)
    setGenerateLoading(true)
    try {
      const milestoneIssueFiles: ArchiveFileRequest[] = statuses
        .filter(s => isStatusVisible(s))
        .map(s => ({
          repository_file: s.issue.title,
          commit: s.qc_status.approved_commit ?? s.qc_status.latest_commit,
          milestone: s.issue.milestone ?? undefined,
          approved: isApprovedStatus(s),
        }))

      const addedFilesRequests: ArchiveFileRequest[] = Array.from(addedFiles.values())
        .filter(r => !addedFileConflicts.has(r.file_name))
        .map(r => {
          // For QC files added via relevant files, prefer the status-derived commit
          const statusCommit = r.source_issue_number != null
            ? addedFileStatusMap.get(r.source_issue_number)
            : undefined
          const commit = statusCommit
            ? (statusCommit.qc_status.approved_commit ?? statusCommit.qc_status.latest_commit)
            : r.commit
          return {
            repository_file: r.file_name,
            commit,
            approved: false,
          }
        })

      const files = [...milestoneIssueFiles, ...addedFilesRequests]
      const result = await generateArchive({ output_path: outputPath, flatten, files })
      setGenerateSuccess(result.output_path)
    } catch (err) {
      setGenerateError((err as Error).message)
    } finally {
      setGenerateLoading(false)
    }
  }

  const visibleFileCount =
    statuses.filter(s => isStatusVisible(s)).length +
    addedFiles.size

  const canGenerate =
    visibleFileCount > 0 &&
    outputPath.trim().length > 0 &&
    !isLoadingStatuses &&
    unresolvedAddedFileCount === 0 &&
    conflictCount === 0

  // Files already occupying the right panel — unselectable in AddFileModal
  const claimedFiles = useMemo(() => {
    const s = new Set<string>()
    statuses.filter(st => isStatusVisible(st)).forEach(st => s.add(st.issue.title))
    addedFiles.forEach((_, fn) => s.add(fn))
    return s
  }, [statuses, isStatusVisible, addedFiles])

  // ─── Flatten collision detection ─────────────────────────────────────

  const allArchiveFiles = useMemo(() => {
    const files: string[] = []
    statuses.filter(s => isStatusVisible(s)).forEach(s => files.push(s.issue.title))
    for (const [fn] of addedFiles) {
      if (!addedFileConflicts.get(fn)?.dedup) files.push(fn)
    }
    return files
  }, [statuses, isStatusVisible, addedFiles, addedFileConflicts])

  const basenameCollisions = useMemo(() => {
    const seen = new Map<string, string>()
    const collisions: string[] = []
    for (const f of allArchiveFiles) {
      const base = basename(f)
      const existing = seen.get(base)
      if (existing !== undefined && existing !== f) {
        if (!collisions.includes(base)) collisions.push(base)
      } else {
        seen.set(base, f)
      }
    }
    return collisions
  }, [allArchiveFiles])

  const canFlatten = basenameCollisions.length === 0

  useEffect(() => {
    if (!canFlatten && flatten) setFlatten(false)
  }, [canFlatten, flatten])

  const claimedBasenames = useMemo(() => {
    if (!flatten) return new Set<string>()
    const s = new Set<string>()
    for (const f of claimedFiles) s.add(basename(f))
    return s
  }, [flatten, claimedFiles])

  const isFileClaimed = useCallback((fileName: string) => {
    if (claimedFiles.has(fileName)) return true
    if (flatten && claimedBasenames.has(basename(fileName))) return true
    return false
  }, [claimedFiles, flatten, claimedBasenames])

  // Referencing statuses for the file being edited (relevant for bare files; empty for added files)
  const referencingStatusesForFile = useMemo(() => {
    if (!editFileModal) return []
    return statuses.filter(s =>
      (s.issue.relevant_files ?? []).some(rf => rf.file_name === editFileModal),
    )
  }, [editFileModal, statuses])

  return (
    <div style={{ display: 'flex', height: '100%', overflow: 'hidden' }}>

      {/* ── Left sidebar ─────────────────────────────────────────────────── */}
      <ResizableSidebar defaultWidth={320} minWidth={280} maxWidth={560} noPadding>
        <div style={{ display: 'flex', flexDirection: 'column', height: '100%' }}>

          {/* ── Output Path + Generate ───────────────────────────────────── */}
          <div style={{ flexShrink: 0, padding: 'var(--mantine-spacing-md)' }}>
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
              <Tooltip
                label={`Basename conflicts: ${basenameCollisions.join(', ')}`}
                disabled={canFlatten}
                withArrow
                multiline
                maw={300}
              >
                <Switch
                  label="Flatten directory structure"
                  size="xs"
                  checked={flatten}
                  disabled={!canFlatten}
                  onChange={(e) => setFlatten(e.currentTarget.checked)}
                />
              </Tooltip>
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

          {/* ── Milestones ────────────────────────────────────────────────── */}
          <div style={{ flex: 1, minHeight: 0, display: 'flex', flexDirection: 'column', borderTop: '1px solid var(--mantine-color-gray-3)' }}>
            <div style={{ padding: '8px var(--mantine-spacing-md) 0', flexShrink: 0 }}>
              <Text fw={600} size="sm">Milestones</Text>
            </div>
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
                  milestoneFileSets={milestoneFileSets}
                  includeNonApproved={includeNonApproved}
                  onIncludeNonApprovedChange={(n, v) => setIncludeNonApproved(prev => ({ ...prev, [n]: v }))}
                  nonApprovedOverlapByMilestone={nonApprovedOverlapByMilestone}
                  flatten={flatten}
                />
              </Stack>
            </div>
          </div>

        </div>
      </ResizableSidebar>

      {/* ── Right panel: issue cards ──────────────────────────────────────── */}
      <div style={{ flex: 1, overflowY: 'auto', padding: 'var(--mantine-spacing-md)' }}>
          <div style={{
            display: 'grid',
            gridTemplateColumns: 'repeat(auto-fill, minmax(200px, 1fr))',
            gap: 12,
          }}>
            {/* ── Add file card (always first) ──────────────────────────── */}
            <div
              onClick={() => setAddFileModalOpen(true)}
              style={{
                height: CARD_HEIGHT,
                borderRadius: 6,
                border: '2px dashed #74c69d',
                backgroundColor: '#f0faf4',
                minWidth: 0,
                cursor: 'pointer',
                display: 'flex',
                flexDirection: 'column',
                alignItems: 'center',
                justifyContent: 'center',
                gap: 4,
                color: '#2f7a3b',
                transition: 'background-color 0.15s, border-color 0.15s',
              }}
              onMouseEnter={e => {
                e.currentTarget.style.backgroundColor = '#d3f0df'
                e.currentTarget.style.borderColor = '#2f7a3b'
              }}
              onMouseLeave={e => {
                e.currentTarget.style.backgroundColor = '#f0faf4'
                e.currentTarget.style.borderColor = '#74c69d'
              }}
            >
              <span style={{ fontSize: 28, lineHeight: 1, fontWeight: 300 }}>+</span>
              <Text size="xs" fw={600} style={{ color: 'inherit' }}>Add file</Text>
            </div>

            {/* ── Manually added file cards ─────────────────────────────── */}
            {Array.from(addedFiles.entries()).map(([fileName, res]) => {
              const conflict = addedFileConflicts.get(fileName)
              if (conflict?.dedup) return null // silently hidden — milestone covers same issue

              if (conflict) {
                return (
                  <Tooltip key={`added-${fileName}`} label={conflict.reason} withArrow>
                    <Stack
                      gap={5}
                      style={{
                        padding: '10px 12px',
                        borderRadius: 6,
                        border: '1px solid #ff8787',
                        backgroundColor: '#ffe3e3',
                        height: CARD_HEIGHT,
                        overflowY: 'auto',
                        minWidth: 0,
                      }}
                    >
                      <div style={{ display: 'flex', alignItems: 'flex-start', gap: 4 }}>
                        <Text size="sm" fw={700} style={{ wordBreak: 'break-all', flex: 1 }}>{fileName}</Text>
                        <ActionIcon size="xs" variant="transparent" color="dark" style={{ flexShrink: 0, marginTop: 1 }} onClick={() => setAddedFiles(prev => { const n = new Map(prev); n.delete(fileName); return n })} aria-label="Remove">
                          <IconX size={11} />
                        </ActionIcon>
                      </div>
                      <Text size="xs" c="dimmed"><b>Commit:</b> {res.commit ? res.commit.slice(0, 7) : '—'}</Text>
                    </Stack>
                  </Tooltip>
                )
              }

              // Rich card when the file was added via an issue and status is available
              const issueStatus = res.source_issue_number != null
                ? addedFileStatusMap.get(res.source_issue_number)
                : undefined

              if (issueStatus) {
                const approved = isApprovedStatus(issueStatus)
                const commit = issueStatus.qc_status.approved_commit ?? issueStatus.qc_status.latest_commit
                const shortCommit = commit.slice(0, 7)
                const statusLabel = issueStatus.qc_status.status.replace(/_/g, ' ')

                return (
                  <Stack
                    key={`added-${fileName}`}
                    gap={5}
                    style={{
                      padding: '10px 12px',
                      borderRadius: 6,
                      border: '1px solid var(--mantine-color-gray-3)',
                      backgroundColor: 'white',
                      height: CARD_HEIGHT,
                      overflowY: 'auto',
                      minWidth: 0,
                      cursor: 'pointer',
                    }}
                    onClick={() => setEditFileModal(fileName)}
                  >
                    <div style={{ display: 'flex', alignItems: 'flex-start', gap: 4, minWidth: 0 }}>
                      <Anchor
                        href={issueStatus.issue.html_url}
                        target="_blank"
                        size="sm"
                        fw={700}
                        style={{ wordBreak: 'break-all', flex: 1, minWidth: 0 }}
                        onClick={e => e.stopPropagation()}
                      >
                        {issueStatus.issue.title}
                      </Anchor>
                      {!approved && (
                        <Tooltip label="Not yet approved" withArrow>
                          <span style={{ flexShrink: 0, marginTop: 2 }}>
                            <IconAlertTriangle size={12} color="#f59f00" />
                          </span>
                        </Tooltip>
                      )}
                      <ActionIcon
                        size="xs"
                        variant="transparent"
                        color="dark"
                        style={{ flexShrink: 0, marginTop: 1 }}
                        onClick={e => { e.stopPropagation(); setAddedFiles(prev => { const n = new Map(prev); n.delete(fileName); return n }) }}
                        aria-label="Remove"
                      >
                        <IconX size={11} />
                      </ActionIcon>
                    </div>
                    {issueStatus.issue.milestone && (
                      <Text size="xs" c="dimmed"><b>Milestone:</b> {issueStatus.issue.milestone}</Text>
                    )}
                    <Text size="xs" c="dimmed"><b>Commit:</b> {shortCommit}</Text>
                    <Text size="xs" c="dimmed"><b>Status:</b> {statusLabel}</Text>
                    <RelevantFilesList
                      relevantFiles={issueStatus.issue.relevant_files ?? []}
                      claimedFiles={claimedFiles}
                      isFileClaimed={isFileClaimed}
                      onSelectFile={handleSelectRelevantFile}
                      onSelectAll={handleSelectAllRelevant}
                    />
                  </Stack>
                )
              }

              // Unresolved bare file — yellow card
              if (res.commit === '') {
                return (
                  <Tooltip key={`added-${fileName}`} label="Click to resolve" withArrow>
                    <Stack
                      gap={5}
                      style={{
                        padding: '10px 12px',
                        borderRadius: 6,
                        border: '1px solid #fcc419',
                        backgroundColor: '#fff3bf',
                        height: CARD_HEIGHT,
                        overflowY: 'auto',
                        minWidth: 0,
                        cursor: 'pointer',
                      }}
                      onClick={() => setEditFileModal(fileName)}
                    >
                      <div style={{ display: 'flex', alignItems: 'flex-start', gap: 4 }}>
                        <Text size="sm" fw={700} style={{ wordBreak: 'break-all', flex: 1 }}>{fileName}</Text>
                        <ActionIcon size="xs" variant="transparent" color="dark" style={{ flexShrink: 0, marginTop: 1 }} onClick={e => { e.stopPropagation(); setAddedFiles(prev => { const n = new Map(prev); n.delete(fileName); return n }) }} aria-label="Remove">
                          <IconX size={11} />
                        </ActionIcon>
                      </div>
                      <Text size="xs" c="dimmed">Commit not yet selected</Text>
                    </Stack>
                  </Tooltip>
                )
              }

              return (
                <ResolvedFileCard
                  key={`added-${fileName}`}
                  fileName={fileName}
                  commit={res.commit}
                  onEdit={() => setEditFileModal(fileName)}
                  onRemove={() => setAddedFiles(prev => { const n = new Map(prev); n.delete(fileName); return n })}
                />
              )
            })}

            {selectedMilestones.length > 0 && (<>
            {/* ── Milestone issue cards ──────────────────────────────────── */}
            {statuses.filter(s => isStatusVisible(s)).map((s) => {
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
                    height: CARD_HEIGHT,
                    overflowY: 'auto',
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
                  <RelevantFilesList
                    relevantFiles={s.issue.relevant_files ?? []}
                    claimedFiles={claimedFiles}
                    isFileClaimed={isFileClaimed}
                    onSelectFile={handleSelectRelevantFile}
                    onSelectAll={handleSelectAllRelevant}
                  />
                </Stack>
              )
            })}
            </>)}
          </div>
      </div>

      {/* ── Edit / resolve modal (bare files + added files) ──────────────── */}
      <FileResolveModal
        opened={editFileModal !== null}
        onClose={() => setEditFileModal(null)}
        fileName={editFileModal ?? undefined}
        referencingStatuses={referencingStatusesForFile}
        editMode={editFileModal !== null && addedFiles.has(editFileModal) ? 'edit' : 'resolve'}
        onResolve={handleEditResolve}
      />

      {/* ── Add-file modal (file picker + commit/issue step) ─────────────── */}
      <FileResolveModal
        opened={addFileModalOpen}
        onClose={() => setAddFileModalOpen(false)}
        claimedFiles={claimedFiles}
        isFileClaimed={isFileClaimed}
        onResolve={(resolution) => {
          setAddedFiles(prev => {
            const next = new Map(prev)
            next.set(resolution.file_name, resolution)
            return next
          })
        }}
      />
    </div>
  )
}

// ─── ResolvedFileCard ─────────────────────────────────────────────────────────
// Shared card for resolved bare files and manually added files.

function ResolvedFileCard({
  fileName,
  commit,
  via,
  onEdit,
  onRemove,
}: {
  fileName: string
  commit: string
  via?: { title: string; html_url: string }
  onEdit: () => void
  onRemove: () => void
}) {
  return (
    <Stack
      gap={5}
      style={{
        padding: '10px 12px',
        borderRadius: 6,
        border: '1px solid #aacca6',
        backgroundColor: '#d7e7d3',
        height: CARD_HEIGHT,
        overflowY: 'auto',
        minWidth: 0,
        cursor: 'pointer',
      }}
      onClick={onEdit}
    >
      <div style={{ display: 'flex', alignItems: 'flex-start', gap: 4 }}>
        <Text size="sm" fw={700} style={{ wordBreak: 'break-all', flex: 1 }}>
          {fileName}
        </Text>
        <ActionIcon
          size="xs"
          variant="transparent"
          color="dark"
          style={{ flexShrink: 0, marginTop: 1 }}
          onClick={e => { e.stopPropagation(); onRemove() }}
          aria-label="Remove"
        >
          <IconX size={11} />
        </ActionIcon>
      </div>
      {via && (
        <Text size="xs" c="dimmed">
          <b>Via:</b>{' '}
          <Anchor href={via.html_url} target="_blank" size="xs" onClick={e => e.stopPropagation()}>
            {via.title}
          </Anchor>
        </Text>
      )}
      <Text size="xs" c="dimmed"><b>Commit:</b> {commit.slice(0, 7)}</Text>
    </Stack>
  )
}

// ─── ArchiveMilestoneCombobox ─────────────────────────────────────────────────

interface ArchiveMilestoneComboboxProps {
  selectedMilestones: number[]
  onSelectedMilestonesChange: (v: number[]) => void
  showOpenMilestones: boolean
  statusByMilestone: Record<number, MilestoneStatusInfo>
  unapprovedByMilestone: Record<number, number>
  milestoneFileSets: Map<number, { all: Set<string>; approvedOnly: Set<string> }>
  includeNonApproved: Record<number, boolean>
  onIncludeNonApprovedChange: (milestoneNumber: number, value: boolean) => void
  nonApprovedOverlapByMilestone: Record<number, string[]>
  flatten: boolean
}

function ArchiveMilestoneCombobox({
  selectedMilestones,
  onSelectedMilestonesChange,
  showOpenMilestones,
  statusByMilestone,
  unapprovedByMilestone,
  milestoneFileSets,
  includeNonApproved,
  onIncludeNonApprovedChange,
  nonApprovedOverlapByMilestone,
  flatten,
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

  // Build the union of file names from currently selected milestones
  const selectedFileUnion = useMemo(() => {
    const union = new Set<string>()
    for (const n of selectedMilestones) {
      const fileSet = milestoneFileSets.get(n)
      if (!fileSet) continue
      const set = includeNonApproved[n] ? fileSet.all : fileSet.approvedOnly
      for (const f of set) union.add(flatten ? basename(f) : f)
    }
    return union
  }, [selectedMilestones, milestoneFileSets, includeNonApproved, flatten])

  // For each candidate milestone, compute conflicts with the selected set
  const milestoneConflicts = useMemo(() => {
    const map = new Map<number, { conflicts: string[]; milestones: string[] }>()
    for (const m of filtered) {
      const candidateFiles = milestoneFileSets.get(m.number)
      if (!candidateFiles) continue
      // Candidate defaults to approvedOnly since it's not yet selected
      const candidateSet = candidateFiles.approvedOnly
      const conflicts: string[] = []
      for (const f of candidateSet) {
        if (selectedFileUnion.has(flatten ? basename(f) : f)) conflicts.push(f)
      }
      if (conflicts.length > 0) {
        const owningMilestones = new Set<string>()
        for (const n of selectedMilestones) {
          const fileSet = milestoneFileSets.get(n)
          if (!fileSet) continue
          const set = includeNonApproved[n] ? fileSet.all : fileSet.approvedOnly
          for (const f of conflicts) {
            if (set.has(f)) {
              const title = (data ?? []).find(ms => ms.number === n)?.title ?? String(n)
              owningMilestones.add(title)
            }
          }
        }
        map.set(m.number, { conflicts, milestones: [...owningMilestones] })
      }
    }
    return map
  }, [filtered, milestoneFileSets, includeNonApproved, selectedFileUnion, selectedMilestones, data, flatten])

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
            {[...filtered].reverse().map((m) => {
              const conflict = milestoneConflicts.get(m.number)
              const isDisabled = !!conflict
              const tooltipLabel = conflict
                ? conflict.milestones.map(ms =>
                    `Conflicts with ${ms}: ${conflict.conflicts.join(', ')}`
                  ).join('\n')
                : ''
              const option = (
                <Combobox.Option key={m.number} value={String(m.number)} disabled={isDisabled}>
                  <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                    <Text size="sm" c={isDisabled ? 'dimmed' : undefined}>{m.title}</Text>
                    {m.state !== 'closed' && <OpenPill />}
                  </div>
                  <Text size="xs" c="dimmed">
                    {m.open_issues} open · {m.closed_issues} closed
                  </Text>
                </Combobox.Option>
              )
              return isDisabled ? (
                <Tooltip key={m.number} label={tooltipLabel} withArrow multiline maw={300}>
                  <div>{option}</div>
                </Tooltip>
              ) : option
            })}
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
              includeNonApproved={!!includeNonApproved[m.number]}
              onIncludeNonApprovedChange={(v) => onIncludeNonApprovedChange(m.number, v)}
              nonApprovedOverlap={nonApprovedOverlapByMilestone[m.number] ?? null}
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
  includeNonApproved,
  onIncludeNonApprovedChange,
  nonApprovedOverlap,
}: {
  milestone: import('~/api/milestones').Milestone
  statusInfo: MilestoneStatusInfo
  unapprovedCount: number
  onRemove: () => void
  includeNonApproved: boolean
  onIncludeNonApprovedChange: (v: boolean) => void
  nonApprovedOverlap: string[] | null
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
              <span data-testid="open-milestone-indicator"><OpenPill /></span>
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
        <Tooltip
          label={nonApprovedOverlap ? `File conflicts: ${nonApprovedOverlap.join(', ')}` : ''}
          disabled={!nonApprovedOverlap}
          withArrow
          multiline
          maw={300}
        >
          <Switch
            label="Include non-approved"
            size="xs"
            checked={includeNonApproved}
            disabled={!!nonApprovedOverlap}
            onChange={(e) => onIncludeNonApprovedChange(e.currentTarget.checked)}
            styles={{ root: { marginTop: 4 } }}
          />
        </Tooltip>
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

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
  IconX,
} from '@tabler/icons-react'
import { useQueries } from '@tanstack/react-query'
import { useMilestones } from '~/api/milestones'
import {
  type IssueStatusError,
  type IssueStatusResponse,
  type IssueStatusResult,
  type MilestoneStatusInfo,
  fetchMilestoneIssues,
  fetchSingleIssueStatus,
  issueStatusBatcher,
  useMilestoneIssues,
} from '~/api/issues'
import { type ArchiveFileRequest, generateArchive } from '~/api/archive'
import { useRepoInfo } from '~/api/repo'
import { OpenPill } from './MilestoneFilter'
import { ResizableSidebar } from './ResizableSidebar'
import { type FileResolution, FileResolveModal } from './FileResolveModal'

// ─── Constants ────────────────────────────────────────────────────────────────

const COLLAPSED_HEIGHT = 36
const MIN_ISSUES_HEIGHT = 80

// ─── Types ────────────────────────────────────────────────────────────────────

interface SourceIssue { number: number; title: string; html_url: string }

type RelevantEntry =
  | { type: 'qc'; file_name: string; issue_number: number; via: SourceIssue }
  | { type: 'bare'; file_name: string; via: SourceIssue }
  | { type: 'conflict'; file_name: string; reason: string; via: SourceIssue[]; blocking: boolean; issue_numbers?: number[] }

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
  const [resolvedBareFiles, setResolvedBareFiles] = useState<Map<string, FileResolution>>(
    new Map(),
  )
  const [dismissedRelevantFiles, setDismissedRelevantFiles] = useState<Set<string>>(new Set())
  const [addedFiles, setAddedFiles] = useState<Map<string, FileResolution>>(new Map())

  // Single modal state for editing any resolved/bare file
  const [editFileModal, setEditFileModal] = useState<string | null>(null)
  const [addFileModalOpen, setAddFileModalOpen] = useState(false)

  function dismissRelevantFile(fileName: string) {
    setDismissedRelevantFiles(prev => new Set([...prev, fileName]))
    setResolvedBareFiles(prev => {
      if (!prev.has(fileName)) return prev
      const next = new Map(prev)
      next.delete(fileName)
      return next
    })
  }

  function handleEditResolve(resolution: FileResolution) {
    const { file_name } = resolution
    if (addedFiles.has(file_name)) {
      setAddedFiles(prev => new Map([...prev, [file_name, resolution]]))
    } else {
      setResolvedBareFiles(prev => new Map([...prev, [file_name, resolution]]))
    }
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

  // Detect whether enabling non-approved issues would introduce cross-milestone file conflicts
  const nonApprovedOverlap = useMemo(() => {
    if (selectedMilestones.length < 2) return null
    const fileCount = new Map<string, number>()
    for (const n of selectedMilestones) {
      for (const f of milestoneFileSets.get(n)?.all ?? []) {
        fileCount.set(f, (fileCount.get(f) ?? 0) + 1)
      }
    }
    const approvedCount = new Map<string, number>()
    for (const n of selectedMilestones) {
      for (const f of milestoneFileSets.get(n)?.approvedOnly ?? []) {
        approvedCount.set(f, (approvedCount.get(f) ?? 0) + 1)
      }
    }
    const conflicts = [...fileCount.entries()]
      .filter(([f, c]) => c >= 2 && (approvedCount.get(f) ?? 0) < 2)
      .map(([f]) => f)
    return conflicts.length > 0 ? conflicts : null
  }, [selectedMilestones, milestoneFileSets])

  // Force includeNonApproved off when it would cause cross-milestone file conflicts
  useEffect(() => {
    if (nonApprovedOverlap && includeNonApproved) {
      setIncludeNonApproved(false)
    }
  }, [nonApprovedOverlap, includeNonApproved])

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

    // Only consider issues that are currently visible in the right panel.
    // Relevant files of hidden (unapproved) issues are not surfaced.
    const visibleStatuses = statuses.filter(s => includeNonApproved || isApprovedStatus(s))

    const archiveIssueNumbers = new Set(visibleStatuses.map(s => s.issue.number))
    const archiveFileNames = new Set(visibleStatuses.map(s => s.issue.title))

    // file_name → array of sources (each source includes the milestone issue that listed it)
    const fileSourceMap = new Map<string, Array<{
      kind: 'qc' | 'bare'
      issue_number: number | null
      via: SourceIssue
    }>>()

    for (const status of visibleStatuses) {
      const via: SourceIssue = {
        number: status.issue.number,
        title: status.issue.title,
        html_url: status.issue.html_url,
      }
      for (const rf of (status.issue.relevant_files ?? [])) {
        const isQcKind = rf.kind === 'blocking_qc' || rf.kind === 'relevant_qc'
        const issueNumber = rf.issue_url ? extractIssueNumber(rf.issue_url) : null

        if (isQcKind && issueNumber !== null) {
          // De-dup at source level: if this QC issue is already a visible milestone
          // issue, the reference is covered — don't add this source at all.
          // This ensures the referencing issue (via) is excluded from the conflict card.
          if (archiveIssueNumbers.has(issueNumber)) continue
          const existing = fileSourceMap.get(rf.file_name)
          if (!existing) {
            fileSourceMap.set(rf.file_name, [{ kind: 'qc', issue_number: issueNumber, via }])
          } else if (!existing.some(e => e.issue_number === issueNumber)) {
            existing.push({ kind: 'qc', issue_number: issueNumber, via })
          }
        } else {
          // Bare file (file kind or QC kind with null issue_url)
          const existing = fileSourceMap.get(rf.file_name)
          if (!existing) {
            fileSourceMap.set(rf.file_name, [{ kind: 'bare', issue_number: null, via }])
          } else if (!existing.some(e => e.kind === 'bare')) {
            existing.push({ kind: 'bare', issue_number: null, via })
          }
        }
      }
    }

    const entries: RelevantEntry[] = []

    for (const [file_name, sources] of fileSourceMap) {
      const viaAll = sources.map(s => s.via).filter(
        (v, i, arr) => arr.findIndex(x => x.number === v.number) === i,
      )

      const qcIssueNums = sources
        .filter(s => s.kind === 'qc' && s.issue_number !== null)
        .map(s => s.issue_number!)

      // File name already covered by a visible milestone issue — informational, non-blocking.
      // viaAll here only contains sources whose QC issue was NOT already in the archive
      // (the others were skipped above), so the via list is already correct.
      if (archiveFileNames.has(file_name)) {
        entries.push({ type: 'conflict', file_name, reason: 'File already covered by milestone issues', via: viaAll, blocking: false, issue_numbers: qcIssueNums.length > 0 ? qcIssueNums : undefined })
        continue
      }

      // Genuine ambiguity: multiple different non-archived sources claim this file — blocking
      if (sources.length > 1) {
        entries.push({ type: 'conflict', file_name, reason: 'Multiple QC issues reference this file', via: viaAll, blocking: true, issue_numbers: qcIssueNums.length > 0 ? qcIssueNums : undefined })
        continue
      }

      const source = sources[0]

      // Conflict with a manually added file claiming the same name
      if (addedFiles.has(file_name)) {
        entries.push({ type: 'conflict', file_name, reason: 'Conflicts with a manually added file', via: viaAll, blocking: true, issue_numbers: qcIssueNums.length > 0 ? qcIssueNums : undefined })
        continue
      }

      if (source.kind === 'qc' && source.issue_number !== null) {
        entries.push({ type: 'qc', file_name, issue_number: source.issue_number, via: source.via })
      } else {
        entries.push({ type: 'bare', file_name, via: source.via })
      }
    }

    return entries
  }, [includeRelevantFiles, statuses, includeNonApproved, addedFiles])

  // QC relevant issue numbers not already in statuses
  const qcIssueNumbersToFetch = useMemo(() => {
    const existing = new Set(statuses.map(s => s.issue.number))
    const nums: number[] = []
    for (const e of relevantFileEntries) {
      if (e.type === 'qc') nums.push(e.issue_number)
      if (e.type === 'conflict' && e.issue_numbers) nums.push(...e.issue_numbers)
    }
    return nums
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

  // Conflict state for manually added files
  const addedFileConflicts = useMemo(() => {
    const map = new Map<string, { reason: string; blocking: boolean; dedup: boolean }>()
    const visibleIssuesByTitle = new Map<string, number>() // file_name → issue number
    statuses
      .filter(s => includeNonApproved || isApprovedStatus(s))
      .forEach(s => visibleIssuesByTitle.set(s.issue.title, s.issue.number))

    for (const [fn, res] of addedFiles) {
      const milestoneIssueNumber = visibleIssuesByTitle.get(fn)
      if (milestoneIssueNumber !== undefined) {
        if (res.source_issue_number !== undefined && res.source_issue_number === milestoneIssueNumber) {
          // Same issue — silently de-dup, milestone card already covers it
          map.set(fn, { reason: '', blocking: false, dedup: true })
        } else {
          map.set(fn, { reason: 'File already covered by milestone issues', blocking: false, dedup: false })
        }
        continue
      }
      // Conflict with a non-dismissed relevant file entry
      const hasRelevantConflict = relevantFileEntries.some(
        e => e.file_name === fn && !dismissedRelevantFiles.has(fn),
      )
      if (hasRelevantConflict) {
        map.set(fn, { reason: 'Conflicts with a relevant file', blocking: true, dedup: false })
      }
    }
    return map
  }, [addedFiles, statuses, includeNonApproved, relevantFileEntries, dismissedRelevantFiles])

  // Counts for canGenerate
  const unresolvedBareFileCount = useMemo(
    () =>
      relevantFileEntries.filter(e => e.type === 'bare' && !resolvedBareFiles.has(e.file_name))
        .length,
    [relevantFileEntries, resolvedBareFiles],
  )
  const conflictCount = useMemo(
    () =>
      relevantFileEntries.filter(e => e.type === 'conflict').length +
      Array.from(addedFileConflicts.values()).filter(c => c.blocking).length,
    [relevantFileEntries, addedFileConflicts],
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
        .filter(e => e.type === 'qc' as const)
        .flatMap(e => {
          const s = relevantQcStatusMap.get(e.issue_number)
          if (!s) return []
          return [{
            repository_file: e.file_name,
            commit: s.qc_status.approved_commit ?? s.qc_status.latest_commit,
            approved: isApprovedStatus(s),
          }]
        })

      const addedFilesRequests: ArchiveFileRequest[] = Array.from(addedFiles.values())
        .filter(r => !addedFileConflicts.has(r.file_name)) // skip conflicted / de-duped
        .map(r => ({
          repository_file: r.file_name,
          commit: r.commit,
          approved: false,
        }))

      const files = [...milestoneIssueFiles, ...bareFileFiles, ...qcRelevantFiles, ...addedFilesRequests]
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
      : 0) +
    addedFiles.size

  const canGenerate =
    visibleFileCount > 0 &&
    outputPath.trim().length > 0 &&
    !isLoadingStatuses &&
    unresolvedBareFileCount === 0 &&
    conflictCount === 0

  // Files already occupying the right panel — unselectable in AddFileModal
  const claimedFiles = useMemo(() => {
    const s = new Set<string>()
    statuses.filter(st => includeNonApproved || isApprovedStatus(st)).forEach(st => s.add(st.issue.title))
    relevantFileEntries.filter(e => !dismissedRelevantFiles.has(e.file_name)).forEach(e => s.add(e.file_name))
    addedFiles.forEach((_, fn) => s.add(fn))
    return s
  }, [statuses, includeNonApproved, relevantFileEntries, dismissedRelevantFiles, addedFiles])

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
                    milestoneFileSets={milestoneFileSets}
                    includeNonApproved={includeNonApproved}
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
                  <Tooltip
                    label={nonApprovedOverlap ? `Cross-milestone file conflicts: ${nonApprovedOverlap.join(', ')}` : ''}
                    disabled={!nonApprovedOverlap}
                    withArrow
                    multiline
                    maw={300}
                  >
                    <Switch
                      label="Include non-approved issues"
                      size="xs"
                      checked={includeNonApproved}
                      disabled={!!nonApprovedOverlap}
                      onChange={(e) => setIncludeNonApproved(e.currentTarget.checked)}
                    />
                  </Tooltip>
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
          <div style={{
            display: 'grid',
            gridTemplateColumns: 'repeat(auto-fill, minmax(160px, 1fr))',
            gap: 12,
          }}>
            {/* ── Add file card (always first) ──────────────────────────── */}
            <div
              onClick={() => setAddFileModalOpen(true)}
              style={{
                minHeight: 80,
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
                        minWidth: 0,
                      }}
                    >
                      <div style={{ display: 'flex', alignItems: 'flex-start', gap: 4 }}>
                        <Text size="sm" fw={700} style={{ wordBreak: 'break-all', flex: 1 }}>{fileName}</Text>
                        <ActionIcon size="xs" variant="transparent" color="dark" style={{ flexShrink: 0, marginTop: 1 }} onClick={() => setAddedFiles(prev => { const n = new Map(prev); n.delete(fileName); return n })} aria-label="Remove">
                          <IconX size={11} />
                        </ActionIcon>
                      </div>
                      <Text size="xs" c="dimmed"><b>Commit:</b> {res.commit.slice(0, 7)}</Text>
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
                  </Stack>
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
                // Try to find a QC issue status for richer rendering
                const qcIssueNum = entry.issue_numbers?.[0]
                const qcStatus = qcIssueNum != null ? relevantQcStatusMap.get(qcIssueNum) : undefined

                return (
                  <Tooltip key={`conflict-${entry.file_name}`} label={entry.reason} withArrow>
                    <Stack
                      gap={5}
                      style={{
                        padding: '10px 12px',
                        borderRadius: 6,
                        border: '1px solid #ff8787',
                        backgroundColor: '#ffe3e3',
                        minWidth: 0,
                      }}
                    >
                      <div style={{ display: 'flex', alignItems: 'flex-start', gap: 4, minWidth: 0 }}>
                        {qcStatus ? (
                          <Anchor
                            href={qcStatus.issue.html_url}
                            target="_blank"
                            size="sm"
                            fw={700}
                            style={{ wordBreak: 'break-all', flex: 1, minWidth: 0 }}
                          >
                            {entry.file_name}
                          </Anchor>
                        ) : (
                          <Text size="sm" fw={700} style={{ wordBreak: 'break-all', flex: 1 }}>
                            {entry.file_name}
                          </Text>
                        )}
                        <ActionIcon size="xs" variant="transparent" color="dark" style={{ flexShrink: 0, marginTop: 1 }} onClick={() => dismissRelevantFile(entry.file_name)} aria-label="Remove">
                          <IconX size={11} />
                        </ActionIcon>
                      </div>
                      {qcStatus ? (<>
                        {qcStatus.issue.milestone && (
                          <Text size="xs" c="dimmed"><b>Milestone:</b> {qcStatus.issue.milestone}</Text>
                        )}
                        <Text size="xs" c="dimmed"><b>Commit:</b> {(qcStatus.qc_status.approved_commit ?? qcStatus.qc_status.latest_commit).slice(0, 7)}</Text>
                        <Text size="xs" c="dimmed"><b>Status:</b> {qcStatus.qc_status.status.replace(/_/g, ' ')}</Text>
                      </>) : null}
                      {entry.via.map(v => (
                        <Text key={v.number} size="xs" c="dimmed">
                          <b>Via:</b>{' '}
                          <Anchor href={v.html_url} target="_blank" size="xs">{v.title}</Anchor>
                        </Text>
                      ))}
                    </Stack>
                  </Tooltip>
                )
              }

              if (entry.type === 'bare') {
                const resolution = resolvedBareFiles.get(entry.file_name)
                if (resolution) {
                  return (
                    <Stack
                      key={`bare-${entry.file_name}`}
                      gap={5}
                      style={{
                        padding: '10px 12px',
                        borderRadius: 6,
                        border: '1px solid var(--mantine-color-gray-3)',
                        backgroundColor: 'white',
                        minWidth: 0,
                        cursor: 'pointer',
                      }}
                      onClick={() => setEditFileModal(entry.file_name)}
                    >
                      <div style={{ display: 'flex', alignItems: 'flex-start', gap: 4 }}>
                        <Text size="sm" fw={700} style={{ wordBreak: 'break-all', flex: 1 }}>
                          {entry.file_name}
                        </Text>
                        <ActionIcon size="xs" variant="transparent" color="dark" style={{ flexShrink: 0, marginTop: 1 }} onClick={e => { e.stopPropagation(); dismissRelevantFile(entry.file_name) }} aria-label="Remove">
                          <IconX size={11} />
                        </ActionIcon>
                      </div>
                      <Text size="xs" c="dimmed"><b>Commit:</b> {resolution.commit.slice(0, 7)}</Text>
                      <Text size="xs" c="dimmed">
                        <b>Via:</b>{' '}
                        <Anchor href={entry.via.html_url} target="_blank" size="xs" onClick={e => e.stopPropagation()}>
                          {entry.via.title}
                        </Anchor>
                      </Text>
                    </Stack>
                  )
                }
                // Unresolved — yellow card with tooltip
                return (
                  <Tooltip key={`bare-${entry.file_name}`} label="Click to resolve" withArrow>
                    <Stack
                      gap={5}
                      style={{
                        padding: '10px 12px',
                        borderRadius: 6,
                        border: '1px solid #fcc419',
                        backgroundColor: '#fff3bf',
                        minWidth: 0,
                        cursor: 'pointer',
                      }}
                      onClick={() => setEditFileModal(entry.file_name)}
                    >
                      <div style={{ display: 'flex', alignItems: 'flex-start', gap: 4 }}>
                        <Text size="sm" fw={700} style={{ wordBreak: 'break-all', flex: 1 }}>
                          {entry.file_name}
                        </Text>
                        <ActionIcon size="xs" variant="transparent" color="dark" style={{ flexShrink: 0, marginTop: 1 }} onClick={e => { e.stopPropagation(); dismissRelevantFile(entry.file_name) }} aria-label="Remove">
                          <IconX size={11} />
                        </ActionIcon>
                      </div>
                      <Text size="xs" c="dimmed">
                        <b>Via:</b>{' '}
                        <Anchor href={entry.via.html_url} target="_blank" size="xs" onClick={e => e.stopPropagation()}>
                          {entry.via.title}
                        </Anchor>
                      </Text>
                    </Stack>
                  </Tooltip>
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
                  <Text size="xs" c="dimmed">
                    <b>Via:</b>{' '}
                    <Anchor href={entry.via.html_url} target="_blank" size="xs">
                      {entry.via.title}
                    </Anchor>
                  </Text>
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
  includeNonApproved: boolean
}

function ArchiveMilestoneCombobox({
  selectedMilestones,
  onSelectedMilestonesChange,
  showOpenMilestones,
  statusByMilestone,
  unapprovedByMilestone,
  milestoneFileSets,
  includeNonApproved,
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
      const set = includeNonApproved ? fileSet.all : fileSet.approvedOnly
      for (const f of set) union.add(f)
    }
    return union
  }, [selectedMilestones, milestoneFileSets, includeNonApproved])

  // For each candidate milestone, compute conflicts with the selected set
  const milestoneConflicts = useMemo(() => {
    const map = new Map<number, { conflicts: string[]; milestones: string[] }>()
    for (const m of filtered) {
      const candidateFiles = milestoneFileSets.get(m.number)
      if (!candidateFiles) continue
      const candidateSet = includeNonApproved ? candidateFiles.all : candidateFiles.approvedOnly
      const conflicts: string[] = []
      for (const f of candidateSet) {
        if (selectedFileUnion.has(f)) conflicts.push(f)
      }
      if (conflicts.length > 0) {
        // Find which selected milestones own the conflicting files
        const owningMilestones = new Set<string>()
        for (const n of selectedMilestones) {
          const fileSet = milestoneFileSets.get(n)
          if (!fileSet) continue
          const set = includeNonApproved ? fileSet.all : fileSet.approvedOnly
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
  }, [filtered, milestoneFileSets, includeNonApproved, selectedFileUnion, selectedMilestones, data])

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

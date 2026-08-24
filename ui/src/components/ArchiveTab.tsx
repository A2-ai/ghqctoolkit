import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import {
  ActionIcon,
  Alert,
  Anchor,
  Badge,
  Button,
  Combobox,
  InputBase,
  Loader,
  Modal,
  Select,
  Stack,
  Text,
  TextInput,
  Tooltip,
  useCombobox,
} from '@mantine/core'
import {
  IconAlertTriangle,
  IconArrowBackUp,
  IconEye,
  IconExclamationMark,
  IconX,
} from '@tabler/icons-react'
import { useQueries } from '@tanstack/react-query'
import { useMilestones } from '~/api/milestones'
import {
  type IssueStatusResponse,
  type RelevantFileInfo,
  type MilestoneStatusInfo,
  type RoundInfo,
  archiveBlockedReason,
  fetchMilestoneIssues,
  issueStatusBatcher,
  latestRound,
  roundByIndex,
  useMilestoneIssues,
} from '~/api/issues'
import { type ArchiveFileRequest, type SkippedFileRequest, generateArchive } from '~/api/archive'
import { useRepoInfo } from '~/api/repo'
import { OpenPill } from './MilestoneFilter'
import { StatusErrorDisplay } from './StatusErrorDisplay'
import { ResizableSidebar } from './ResizableSidebar'
import { type FileResolution, FileResolveModal } from './FileResolveModal'
import { RelevantFilesList } from './RelevantFilesList'
import { extractIssueNumber } from '~/utils'
import { ToggleField } from './ToggleField'
import { useUiSession } from '~/state/uiSession'
import { buildFileRawUrl, fetchFileContent, getFileExtensionLabel, getFilePreviewKind } from '~/api/preview'
import { DocPreview } from './DocPreview'

// ─── Constants ────────────────────────────────────────────────────────────────

const CARD_HEIGHT = 185


function isApprovedStatus(s: IssueStatusResponse): boolean {
  return s.qc_status.status === 'approved' || s.qc_status.status === 'changes_after_approval'
}

function basename(path: string): string {
  return path.split('/').pop() ?? path
}

/**
 * U5: a round `<Select>` per QC-attached file, defaulting to the latest, showing the
 * resolved commit and warning when file-changing commits exist after it.
 *
 * The resolved commit is `rounds[i].archive_commit` (M8) — which is exactly what the
 * old `approved_commit ?? latest_commit` was approximating (A3).
 */
function ArchiveRoundSelect({
  status,
  round,
  onChange,
}: {
  status: IssueStatusResponse
  round: RoundInfo
  onChange: (roundIndex: number) => void
}) {
  return (
    // Mantine portals `Combobox.Dropdown` out of this subtree in the DOM, but its options
    // stay React-tree descendants of this element — and React events bubble the fiber
    // tree, not the DOM tree. Without a guard at a common React ancestor of both the
    // target and the dropdown, choosing a round fires the enclosing card's `onClick`
    // and opens the edit modal. Guarding the `<Select>` alone only covers the input.
    <div style={{ display: 'grid', gap: 3 }} onClick={(e) => e.stopPropagation()}>
      <Select
        size="xs"
        label="Round"
        styles={{ label: { fontSize: 11, fontWeight: 700 } }}
        data={status.rounds.map((r) => ({ value: String(r.index), label: `Round ${r.index}` }))}
        value={String(round.index)}
        onChange={(value) => { if (value !== null) onChange(Number(value)) }}
        allowDeselect={false}
        comboboxProps={{ withinPortal: true }}
        data-testid={`archive-round-select-${status.issue.number}`}
      />
      {/* D54/D55: with no resolved commit there is nothing to show and nothing to
          freeze — name the branch to fetch. A substituted hash here is exactly the
          audit lie §18 removes: the archive would claim approved content over
          content that was never approved. */}
      {round.archive_commit === null ? (
        <Text size="xs" c="orange.7" data-testid={`archive-round-unresolved-${status.issue.number}`}>
          {archiveBlockedReason(round)}
        </Text>
      ) : (
        <Text size="xs" c="dimmed"><b>Commit:</b> {round.archive_commit.slice(0, 7)}</Text>
      )}
      {round.subsequent_file_changes && (
        <Tooltip label="The file has committed changes after this commit" withArrow>
          <Badge color="yellow" variant="light" size="xs" data-testid={`archive-subsequent-changes-${status.issue.number}`}>
            changed since
          </Badge>
        </Tooltip>
      )}
    </div>
  )
}

/**
 * D62: one file that will be omitted from the archive, in the shape the wire wants
 * (`SkippedFileRequest`) plus the two fields the notice needs to point the user at the
 * card whose round they can change.
 */
type RoundSkip = SkippedFileRequest & { issueNumber: number; issueTitle: string }

/** The four frozen facts a QC-attached file carries, or `null` when it carries none.
 *  D28.3: the quartet is all-or-nothing — a partial one is a 400. */
function archiveQcFields(status: IssueStatusResponse, round: RoundInfo) {
  // Without a milestone there is nothing to freeze the QC against, so the file is
  // archived as a plain file rather than with a milestone we invented.
  if (!status.issue.milestone) return null
  return {
    milestone: status.issue.milestone,
    // D36: approvedness of *this* round, read off the tagged union.
    approved: round.state.kind === 'approved',
    round: round.index,
    subsequent_file_changes: round.subsequent_file_changes,
  }
}

// ─── ArchiveTab ───────────────────────────────────────────────────────────────

export function ArchiveTab() {
  const { archive, setArchive } = useUiSession()
  const [previewLoading, setPreviewLoading] = useState(false)
  const [previewOpen, setPreviewOpen] = useState(false)
  const [previewTitle, setPreviewTitle] = useState<string | null>(null)
  const [previewContent, setPreviewContent] = useState<string | null>(null)
  const [previewUrl, setPreviewUrl] = useState<string | null>(null)
  const [previewKind, setPreviewKind] = useState<'text' | 'doc' | 'unsupported'>('text')

  function handleEditResolve(resolution: FileResolution) {
    const { file_name } = resolution
    setArchive(prev => ({
      ...prev,
      addedFiles: new Map([...prev.addedFiles, [file_name, resolution]]),
    }))
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
    useMilestoneIssues(archive.selectedMilestones, true)

  // Fetch statuses for manually added files that came from an issue
  const addedFileIssueNums = useMemo(
    () => [...archive.addedFiles.values()]
      .filter((r) => r.source_issue_number != null)
      .map((r) => r.source_issue_number!)
      .filter((n, i, arr) => arr.indexOf(n) === i),
    [archive.addedFiles],
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

  // U5: the selected round per issue, defaulting to the latest. Selection is stored;
  // the round itself is read from `rounds[]` — the UI derives no boundaries (U7).
  const roundFor = useCallback(
    (s: IssueStatusResponse): RoundInfo =>
      roundByIndex(s, archive.selectedRounds[s.issue.number] ?? latestRound(s).index),
    [archive.selectedRounds],
  )

  const selectRound = useCallback(
    (issueNumber: number, roundIndex: number) =>
      setArchive(prev => ({
        ...prev,
        selectedRounds: { ...prev.selectedRounds, [issueNumber]: roundIndex },
      })),
    [setArchive],
  )

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
    return msNum !== undefined && !!archive.includeNonApproved[msNum]
  }, [milestoneTitleToNumber, archive.includeNonApproved])

  const unapprovedByMilestone = useMemo(() => {
    const result: Record<number, number> = {}
    for (const n of archive.selectedMilestones) {
      const milestoneName = (milestonesData ?? []).find((m) => m.number === n)?.title
      const milestoneStatuses = statuses.filter((s) => s.issue.milestone === milestoneName)
      result[n] = milestoneStatuses.filter((s) => !isApprovedStatus(s)).length
    }
    return result
  }, [archive.selectedMilestones, statuses, milestonesData])

  // Per-milestone: detect if enabling non-approved for a milestone would conflict with other milestones
  const nonApprovedOverlapByMilestone = useMemo(() => {
    const result: Record<number, string[]> = {}
    if (archive.selectedMilestones.length < 2) return result
    for (const candidate of archive.selectedMilestones) {
      const candidateAll = milestoneFileSets.get(candidate)?.all ?? new Set<string>()
      const candidateApproved = milestoneFileSets.get(candidate)?.approvedOnly ?? new Set<string>()
      // Files that are non-approved-only in this milestone
      const nonApprovedFiles = [...candidateAll].filter(f => !candidateApproved.has(f))
      // Build set of files from other milestones (using basenames when flatten is ON)
      const otherFiles = new Set<string>()
      for (const other of archive.selectedMilestones) {
        if (other === candidate) continue
        const otherSet = archive.includeNonApproved[other]
          ? milestoneFileSets.get(other)?.all
          : milestoneFileSets.get(other)?.approvedOnly
        if (otherSet) {
          for (const f of otherSet) otherFiles.add(archive.flatten ? basename(f) : f)
        }
      }
      // Check if any non-approved files conflict (using basenames when flatten is ON)
      const conflicts: string[] = []
      for (const f of nonApprovedFiles) {
        if (otherFiles.has(archive.flatten ? basename(f) : f)) {
          conflicts.push(f)
        }
      }
      if (conflicts.length > 0) result[candidate] = conflicts
    }
    return result
  }, [archive.selectedMilestones, milestoneFileSets, archive.includeNonApproved, archive.flatten])

  // Force off any milestone's includeNonApproved that now conflicts
  useEffect(() => {
    const toDisable: number[] = []
    for (const [msNum, conflicts] of Object.entries(nonApprovedOverlapByMilestone)) {
      const n = Number(msNum)
      if (conflicts.length > 0 && archive.includeNonApproved[n]) toDisable.push(n)
    }
    if (toDisable.length > 0) {
      setArchive(prev => {
        const next = { ...prev, includeNonApproved: { ...prev.includeNonApproved } }
        for (const n of toDisable) next.includeNonApproved[n] = false
        return next
      })
    }
  }, [nonApprovedOverlapByMilestone, archive.includeNonApproved, setArchive])

  // Milestones that have at least one issue visible in the right panel
  const milestonesWithVisibleIssues = useMemo(() => {
    const visibleTitles = new Set(
      statuses.filter(s => isStatusVisible(s)).map(s => s.issue.milestone),
    )
    return archive.selectedMilestones.filter((n) => {
      const title = (milestonesData ?? []).find((m) => m.number === n)?.title
      return title !== undefined && visibleTitles.has(title)
    })
  }, [archive.selectedMilestones, statuses, milestonesData, isStatusVisible])

  function buildOutputPathName(milestoneNumbers: number[]) {
    if (!repoData || milestoneNumbers.length === 0) return ''
    const names = milestoneNumbers
      .map((n) => (milestonesData ?? []).find((m) => m.number === n)?.title ?? String(n))
      .join('-')
      .replace(/\s+/g, '-')
    return `${repoData.repo}-${names}.tar.gz`
  }

  function resetOutputPath() {
    const nextOutputPath = buildOutputPathName(milestonesWithVisibleIssues)
    setArchive(prev => {
      if (
        !prev.outputPathUserEdited &&
        !prev.outputPathIsCustom &&
        prev.outputPath === nextOutputPath
      ) {
        return prev
      }
      return {
        ...prev,
        outputPathUserEdited: false,
        outputPathIsCustom: false,
        outputPath: nextOutputPath,
      }
    })
  }

  // Auto-populate output path — only names milestones that have visible issues
  useEffect(() => {
    const nextOutputPath = buildOutputPathName(milestonesWithVisibleIssues)
    setArchive(prev => {
      if (prev.outputPathUserEdited || prev.outputPath === nextOutputPath) {
        return prev
      }
      return { ...prev, outputPath: nextOutputPath }
    })
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [milestonesWithVisibleIssues, repoData, archive.outputPathUserEdited, setArchive])

  // Conflict state for manually added files
  const addedFileConflicts = useMemo(() => {
    const map = new Map<string, { reason: string; blocking: boolean; dedup: boolean }>()
    const visibleIssuesByTitle = new Map<string, number>()
    statuses
      .filter(s => isStatusVisible(s))
      .forEach(s => visibleIssuesByTitle.set(s.issue.title, s.issue.number))

    for (const [fn, res] of archive.addedFiles) {
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
  }, [archive.addedFiles, statuses, isStatusVisible])

  // Check for unresolved added files (bare files with empty commit and no backing issue)
  const unresolvedAddedFileCount = useMemo(
    () => Array.from(archive.addedFiles.values()).filter(r => r.commit === '' && r.source_issue_number == null).length,
    [archive.addedFiles],
  )
  const conflictCount = useMemo(
    () => Array.from(addedFileConflicts.values()).filter(c => c.blocking).length,
    [addedFileConflicts],
  )

  // D62: a selected round whose commit does not resolve no longer blocks the whole
  // archive — that left the UI refusing where the CLI skipped one file and archived
  // the rest (D61). The file is dropped from `files`, declared in `skipped`, and shown
  // here first so the omission is never silent. The user's remedy is often to pick a
  // different round for that one file, which is a click away on the card itself.
  //
  // What is unchanged and absolute: a substituted commit is never sent. A file is in
  // `files` with its own resolved commit, or it is in `skipped`.
  const roundSkips = useMemo(() => {
    const seen = new Map<string, RoundSkip>()
    const add = (repositoryFile: string, s: IssueStatusResponse) => {
      const round = roundFor(s)
      const reason = archiveBlockedReason(round)
      if (reason === null) return
      seen.set(repositoryFile, {
        repository_file: repositoryFile,
        round: round.index,
        branch: round.branch,
        reason,
        issueNumber: s.issue.number,
        issueTitle: s.issue.title,
      })
    }
    statuses.filter(s => isStatusVisible(s)).forEach(s => add(s.issue.title, s))
    for (const r of archive.addedFiles.values()) {
      // Mirror the generation filter exactly: a conflicting added file is not posted,
      // so it is not skipped either — it was never going into the archive.
      if (addedFileConflicts.has(r.file_name)) continue
      const s = r.source_issue_number != null ? addedFileStatusMap.get(r.source_issue_number) : undefined
      if (s) add(r.file_name, s)
    }
    return [...seen.values()]
  }, [statuses, isStatusVisible, archive.addedFiles, addedFileConflicts, addedFileStatusMap, roundFor])

  // ─── Relevant file selection handlers ──────────────────────────────────

  function handleSelectRelevantFile(rf: RelevantFileInfo) {
    const isQc = rf.kind === 'blocking_qc' || rf.kind === 'previous_qc' || rf.kind === 'relevant_qc'
    if (isQc && rf.issue_url) {
      const issueNumber = extractIssueNumber(rf.issue_url)
      if (issueNumber !== null) {
        setArchive(prev => {
          const next = new Map(prev.addedFiles)
          next.set(rf.file_name, { file_name: rf.file_name, commit: '', source_issue_number: issueNumber })
          return { ...prev, addedFiles: next }
        })
        return
      }
    }
    // Bare file — add as unresolved and open modal
    setArchive(prev => {
      const next = new Map(prev.addedFiles)
      next.set(rf.file_name, { file_name: rf.file_name, commit: '' })
      return { ...prev, addedFiles: next, editFileModal: rf.file_name }
    })
  }

  function handleSelectAllRelevant(files: RelevantFileInfo[]) {
    setArchive(prev => {
      const next = new Map(prev.addedFiles)
      for (const rf of files) {
        const isQc = rf.kind === 'blocking_qc' || rf.kind === 'previous_qc' || rf.kind === 'relevant_qc'
        if (isQc && rf.issue_url) {
          const issueNumber = extractIssueNumber(rf.issue_url)
          if (issueNumber !== null) {
            next.set(rf.file_name, { file_name: rf.file_name, commit: '', source_issue_number: issueNumber })
            continue
          }
        }
        next.set(rf.file_name, { file_name: rf.file_name, commit: '' })
      }
      return { ...prev, addedFiles: next }
    })
  }

  async function handlePreviewFile(fileName: string, commit: string) {
    setPreviewLoading(true)
    setPreviewTitle(fileName)
    try {
      const kind = getFilePreviewKind(fileName)
      setPreviewKind(kind)
      if (kind === 'doc') {
        setPreviewUrl(buildFileRawUrl(fileName, commit))
        setPreviewContent(null)
        setPreviewOpen(true)
        return
      }
      if (kind !== 'text') {
        setPreviewUrl(null)
        setPreviewContent(`Preview is not available for ${getFileExtensionLabel(fileName)} files at a specific commit.`)
        setPreviewOpen(true)
        return
      }
      setPreviewUrl(null)
      const content = await fetchFileContent({ path: fileName, commit })
      setPreviewContent(content)
      setPreviewOpen(true)
    } catch (err) {
      setPreviewUrl(null)
      setPreviewKind('text')
      setPreviewContent(`Error: ${(err as Error).message}`)
      setPreviewOpen(true)
    } finally {
      setPreviewLoading(false)
    }
  }

  /** Scroll the affected file's card into view and open its round `<Select>` (D62/D61:
   *  picking a resolvable round is often the real remedy, so keep it one click away). */
  function focusRoundSelect(issueNumber: number) {
    const el = document.querySelector<HTMLInputElement>(`[data-testid="archive-round-select-${issueNumber}"]`)
    if (!el) return
    el.scrollIntoView({ block: 'center' })
    el.focus()
  }

  // ─── Generation ──────────────────────────────────────────────────────────

  async function handleGenerate() {
    setArchive(prev => ({ ...prev, generateError: null, generateSuccess: null, generateSkipped: [], generateLoading: true }))
    try {
      // D28.3: every QC-attached file sends all four of milestone / approved / round /
      // subsequent_file_changes. A partial quartet is a 400 — there is no safe default
      // for the round, so the backend refuses to guess one.
      const milestoneIssueFiles: ArchiveFileRequest[] = statuses
        .filter(s => isStatusVisible(s))
        .flatMap(s => {
          const round = roundFor(s)
          // D62: no resolved commit, no entry in `files` — the file is declared in
          // `skipped` below instead. The null is dropped rather than coerced, because
          // the only alternative is a substituted commit.
          if (round.archive_commit === null) return []
          return [{
            repository_file: s.issue.title,
            commit: round.archive_commit,
            ...(archiveQcFields(s, round) ?? {}),
          }]
        })

      const addedFilesRequests: ArchiveFileRequest[] = Array.from(archive.addedFiles.values())
        .filter(r => !addedFileConflicts.has(r.file_name))
        .flatMap(r => {
          // For QC files added via relevant files, the backing issue's selected round
          // supplies both the commit and the four frozen facts.
          const issueStatus = r.source_issue_number != null
            ? addedFileStatusMap.get(r.source_issue_number)
            : undefined
          if (!issueStatus) {
            // A plain file: none of the four, so `qc` is None server-side.
            return [{ repository_file: r.file_name, commit: r.commit }]
          }
          const round = roundFor(issueStatus)
          // D62: same omission as above — no resolved commit, no entry in `files`.
          if (round.archive_commit === null) return []
          return [{
            repository_file: r.file_name,
            commit: round.archive_commit,
            ...(archiveQcFields(issueStatus, round) ?? {}),
          }]
        })

      const files = [...milestoneIssueFiles, ...addedFilesRequests]
      // D62: declare the omissions on the wire. The server writes them verbatim into
      // the archive's metadata, so a partial archive says it is partial — a warning in
      // the browser scrolls away, the manifest outlives it.
      const skipped = roundSkips.map(({ repository_file, round, branch, reason }) => ({
        repository_file,
        round,
        branch,
        reason,
      }))
      const result = await generateArchive({
        output_path: archive.outputPath,
        flatten: archive.flatten,
        files,
        ...(skipped.length > 0 ? { skipped } : {}),
      })
      // D62: report back what the server *recorded*, not what we asked for — the
      // response echoes the manifest's `skipped` list.
      setArchive(prev => ({ ...prev, generateSuccess: result.output_path, generateSkipped: result.skipped }))
    } catch (err) {
      setArchive(prev => ({ ...prev, generateError: (err as Error).message }))
    } finally {
      setArchive(prev => ({ ...prev, generateLoading: false }))
    }
  }

  const visibleFileCount =
    statuses.filter(s => isStatusVisible(s)).length +
    archive.addedFiles.size

  const canGenerate =
    visibleFileCount > 0 &&
    archive.outputPath.trim().length > 0 &&
    !isLoadingStatuses &&
    unresolvedAddedFileCount === 0 &&
    conflictCount === 0

  // Files already occupying the right panel — unselectable in AddFileModal
  const claimedFiles = useMemo(() => {
    const s = new Set<string>()
    statuses.filter(st => isStatusVisible(st)).forEach(st => s.add(st.issue.title))
    archive.addedFiles.forEach((_, fn) => s.add(fn))
    return s
  }, [statuses, isStatusVisible, archive.addedFiles])

  // ─── Flatten collision detection ─────────────────────────────────────

  const allArchiveFiles = useMemo(() => {
    const files: string[] = []
    statuses.filter(s => isStatusVisible(s)).forEach(s => files.push(s.issue.title))
    for (const [fn] of archive.addedFiles) {
      if (!addedFileConflicts.get(fn)?.dedup) files.push(fn)
    }
    return files
  }, [statuses, isStatusVisible, archive.addedFiles, addedFileConflicts])

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
    if (!canFlatten && archive.flatten) {
      setArchive(prev => ({ ...prev, flatten: false }))
    }
  }, [canFlatten, archive.flatten, setArchive])

  const claimedBasenames = useMemo(() => {
    if (!archive.flatten) return new Set<string>()
    const s = new Set<string>()
    for (const f of claimedFiles) s.add(basename(f))
    return s
  }, [archive.flatten, claimedFiles])

  const isFileClaimed = useCallback((fileName: string) => {
    if (claimedFiles.has(fileName)) return true
    if (archive.flatten && claimedBasenames.has(basename(fileName))) return true
    return false
  }, [claimedFiles, archive.flatten, claimedBasenames])

  // Referencing statuses for the file being edited (relevant for bare files; empty for added files)
  const referencingStatusesForFile = useMemo(() => {
    if (!archive.editFileModal) return []
    return statuses.filter(s =>
      (s.issue.relevant_files ?? []).some(rf => rf.file_name === archive.editFileModal),
    )
  }, [archive.editFileModal, statuses])

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
                value={archive.outputPath}
                onChange={(e) => {
                  const val = e.currentTarget.value
                  setArchive(prev => ({
                    ...prev,
                    outputPathUserEdited: val !== '',
                    outputPathIsCustom: val !== '',
                    outputPath: val,
                  }))
                }}
                rightSection={archive.outputPathIsCustom && archive.selectedMilestones.length > 0 ? (
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
              {/* D62: a notice, not a blocker. Naming the branch and the round keeps the
                  remedy in view — fetch the branch, or pick a round that resolves. */}
              {roundSkips.length > 0 && (
                <Alert color="orange" p="xs" data-testid="archive-round-skips">
                  <Text size="xs" fw={600}>
                    {roundSkips.length === 1 ? '1 file will be skipped' : `${roundSkips.length} files will be skipped`}
                  </Text>
                  {roundSkips.map(skip => (
                    <Text key={skip.repository_file} size="xs">
                      #{skip.issueNumber} {skip.repository_file}: {skip.reason} —{' '}
                      <Anchor
                        component="button"
                        type="button"
                        size="xs"
                        onClick={() => focusRoundSelect(skip.issueNumber)}
                        data-testid={`archive-skip-choose-round-${skip.issueNumber}`}
                      >
                        choose another round
                      </Anchor>
                    </Text>
                  ))}
                  <Text size="xs" c="dimmed">
                    Skipped files are omitted from the archive and recorded in its metadata.
                  </Text>
                </Alert>
              )}
              {archive.generateError && (
                <Alert color="red" p="xs">
                  <Text size="xs">{archive.generateError}</Text>
                </Alert>
              )}
              {archive.generateSuccess && (
                <Alert color="green" p="xs">
                  <Text size="xs">Archive written to {archive.generateSuccess}</Text>
                  {archive.generateSkipped.length > 0 && (
                    <Text size="xs" data-testid="archive-generate-skipped">
                      {archive.generateSkipped.length === 1
                        ? '1 file was skipped and recorded in the archive metadata:'
                        : `${archive.generateSkipped.length} files were skipped and recorded in the archive metadata:`}
                      {' '}
                      {archive.generateSkipped.map(sk => sk.repository_file).join(', ')}
                    </Text>
                  )}
                </Alert>
              )}
              <Tooltip
                label={`Basename conflicts: ${basenameCollisions.join(', ')}`}
                disabled={canFlatten}
                withArrow
                multiline
                maw={300}
              >
                <div>
                  <ToggleField
                    label="Flatten directory structure"
                    checked={archive.flatten}
                    disabled={!canFlatten}
                    onChange={(checked) => setArchive(prev => ({ ...prev, flatten: checked }))}
                  />
                </div>
              </Tooltip>
              <Button
                fullWidth
                size="sm"
                color="green"
                onClick={handleGenerate}
                loading={archive.generateLoading}
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
                <ToggleField
                  label="Include open milestones"
                  checked={archive.showOpenMilestones}
                  onChange={(checked) => setArchive(prev => ({ ...prev, showOpenMilestones: checked }))}
                />
                <ArchiveMilestoneCombobox
                  selectedMilestones={archive.selectedMilestones}
                  onSelectedMilestonesChange={(selectedMilestones) => setArchive(prev => ({ ...prev, selectedMilestones }))}
                  showOpenMilestones={archive.showOpenMilestones}
                  statusByMilestone={milestoneStatusByMilestone}
                  unapprovedByMilestone={unapprovedByMilestone}
                  milestoneFileSets={milestoneFileSets}
                  includeNonApproved={archive.includeNonApproved}
                  onIncludeNonApprovedChange={(n, v) => setArchive(prev => ({
                    ...prev,
                    includeNonApproved: { ...prev.includeNonApproved, [n]: v },
                  }))}
                  nonApprovedOverlapByMilestone={nonApprovedOverlapByMilestone}
                  flatten={archive.flatten}
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
              data-testid="archive-add-file-card"
              onClick={() => setArchive(prev => ({ ...prev, addFileModalOpen: true }))}
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
            {Array.from(archive.addedFiles.entries()).map(([fileName, res]) => {
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
                      justifyContent: 'space-between',
                    }}
                  >
                      <div style={{ display: 'grid', gap: 5 }}>
                        <div style={{ display: 'flex', alignItems: 'flex-start', gap: 4 }}>
                        <Text size="sm" fw={700} style={{ wordBreak: 'break-all', flex: 1 }}>{fileName}</Text>
                        <ActionIcon size="xs" variant="transparent" color="dark" style={{ flexShrink: 0, marginTop: 1 }} onClick={() => setArchive(prev => { const n = new Map(prev.addedFiles); n.delete(fileName); return { ...prev, addedFiles: n } })} aria-label="Remove">
                          <IconX size={11} />
                        </ActionIcon>
                        </div>
                        <Text size="xs" c="dimmed"><b>Commit:</b> {res.commit ? res.commit.slice(0, 7) : '—'}</Text>
                      </div>
                      <div>
                        {res.commit && (
                          <Button
                            size="compact-xs"
                            variant="light"
                            leftSection={<IconEye size={12} />}
                            onClick={e => { e.stopPropagation(); void handlePreviewFile(fileName, res.commit) }}
                          >
                            Preview
                          </Button>
                        )}
                      </div>
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
                const round = roundFor(issueStatus)
                const commit = round.archive_commit
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
                      justifyContent: 'space-between',
                    }}
                    onClick={() => setArchive(prev => ({ ...prev, editFileModal: fileName }))}
                  >
                    <div style={{ display: 'grid', gap: 5 }}>
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
                          onClick={e => { e.stopPropagation(); setArchive(prev => { const n = new Map(prev.addedFiles); n.delete(fileName); return { ...prev, addedFiles: n } }) }}
                          aria-label="Remove"
                        >
                          <IconX size={11} />
                        </ActionIcon>
                      </div>
                      {issueStatus.issue.milestone && (
                        <Text size="xs" c="dimmed"><b>Milestone:</b> {issueStatus.issue.milestone}</Text>
                      )}
                      <ArchiveRoundSelect
                        status={issueStatus}
                        round={round}
                        onChange={(roundIndex) => selectRound(issueStatus.issue.number, roundIndex)}
                      />
                      <Text size="xs" c="dimmed"><b>Status:</b> {statusLabel}</Text>
                      <RelevantFilesList
                        relevantFiles={issueStatus.issue.relevant_files ?? []}
                        claimedFiles={claimedFiles}
                        isFileClaimed={isFileClaimed}
                        onSelectFile={handleSelectRelevantFile}
                        onSelectAll={handleSelectAllRelevant}
                      />
                    </div>
                    {/* D55: no resolved commit, nothing to preview at — the round
                        select above names the branch to fetch instead. */}
                    {commit !== null && (
                      <div>
                        <Button
                          size="compact-xs"
                          variant="light"
                          leftSection={<IconEye size={12} />}
                          onClick={e => { e.stopPropagation(); void handlePreviewFile(fileName, commit) }}
                        >
                          Preview
                        </Button>
                      </div>
                    )}
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
                      onClick={() => setArchive(prev => ({ ...prev, editFileModal: fileName }))}
                    >
                      <div style={{ display: 'flex', alignItems: 'flex-start', gap: 4 }}>
                        <Text size="sm" fw={700} style={{ wordBreak: 'break-all', flex: 1 }}>{fileName}</Text>
                        <ActionIcon size="xs" variant="transparent" color="dark" style={{ flexShrink: 0, marginTop: 1 }} onClick={e => { e.stopPropagation(); setArchive(prev => { const n = new Map(prev.addedFiles); n.delete(fileName); return { ...prev, addedFiles: n } }) }} aria-label="Remove">
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
                  onPreview={() => void handlePreviewFile(fileName, res.commit)}
                  onEdit={() => setArchive(prev => ({ ...prev, editFileModal: fileName }))}
                  onRemove={() => setArchive(prev => { const n = new Map(prev.addedFiles); n.delete(fileName); return { ...prev, addedFiles: n } })}
                />
              )
            })}

            {archive.selectedMilestones.length > 0 && (<>
            {/* ── Milestone issue cards ──────────────────────────────────── */}
            {statuses.filter(s => isStatusVisible(s)).map((s) => {
              const approved = isApprovedStatus(s)
              const round = roundFor(s)
              const commit = round.archive_commit
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
                    justifyContent: 'space-between',
                  }}
                >
                  <div style={{ display: 'grid', gap: 5 }}>
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
                    <ArchiveRoundSelect
                      status={s}
                      round={round}
                      onChange={(roundIndex) => selectRound(s.issue.number, roundIndex)}
                    />
                    <Text size="xs" c="dimmed"><b>Status:</b> {statusLabel}</Text>
                    <RelevantFilesList
                      relevantFiles={s.issue.relevant_files ?? []}
                      claimedFiles={claimedFiles}
                      isFileClaimed={isFileClaimed}
                      onSelectFile={handleSelectRelevantFile}
                      onSelectAll={handleSelectAllRelevant}
                    />
                  </div>
                  {/* D55: no resolved commit, nothing to preview at. */}
                  {commit !== null && (
                    <div>
                      <Button
                        size="compact-xs"
                        variant="light"
                        leftSection={<IconEye size={12} />}
                        onClick={() => void handlePreviewFile(s.issue.title, commit)}
                      >
                        Preview
                      </Button>
                    </div>
                  )}
                </Stack>
              )
            })}
            </>)}
          </div>
      </div>

      {/* ── Edit / resolve modal (bare files + added files) ──────────────── */}
      {archive.editFileModal !== null && (
        <FileResolveModal
          opened
          onClose={() => setArchive(prev => ({ ...prev, editFileModal: null }))}
          fileName={archive.editFileModal}
          referencingStatuses={referencingStatusesForFile}
          editMode={archive.addedFiles.has(archive.editFileModal) ? 'edit' : 'resolve'}
          onResolve={handleEditResolve}
        />
      )}

      {/* ── Add-file modal (file picker + commit/issue step) ─────────────── */}
      {archive.addFileModalOpen && (
        <FileResolveModal
          opened
          onClose={() => setArchive(prev => ({ ...prev, addFileModalOpen: false }))}
          claimedFiles={claimedFiles}
          isFileClaimed={isFileClaimed}
          onResolve={(resolution) => {
            setArchive(prev => {
              const next = new Map(prev.addedFiles)
              next.set(resolution.file_name, resolution)
              return { ...prev, addedFiles: next }
            })
          }}
        />
      )}

      <Modal
        opened={previewOpen}
        onClose={() => {
          setPreviewOpen(false)
          setPreviewUrl(null)
        }}
        title={previewTitle ?? 'Archive File Preview'}
        size={800}
        centered
      >
        {previewLoading ? (
          <div style={{ minHeight: 180, display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
            <Loader size="sm" />
          </div>
        ) : previewKind === 'doc' && previewUrl && previewTitle ? (
          <DocPreview url={previewUrl} fileName={previewTitle} />
        ) : (
          <pre style={{
            margin: 0,
            maxHeight: 500,
            overflow: 'auto',
            padding: '12px 16px',
            borderRadius: 6,
            background: '#e9ecef',
            color: '#212529',
            fontFamily: 'monospace',
            fontSize: 12,
            lineHeight: 1.6,
            whiteSpace: 'pre-wrap',
            wordBreak: 'break-all',
          }}>
            {previewContent ?? ''}
          </pre>
        )}
      </Modal>
    </div>
  )
}

// ─── ResolvedFileCard ─────────────────────────────────────────────────────────
// Shared card for resolved bare files and manually added files.

function ResolvedFileCard({
  fileName,
  commit,
  via,
  onPreview,
  onEdit,
  onRemove,
}: {
  fileName: string
  commit: string
  via?: { title: string; html_url: string }
  onPreview: () => void
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
        justifyContent: 'space-between',
      }}
      onClick={onEdit}
    >
      <div style={{ display: 'grid', gap: 5 }}>
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
      </div>
      <div>
        <Button
          size="compact-xs"
          variant="light"
          leftSection={<IconEye size={12} />}
          onClick={e => { e.stopPropagation(); onPreview() }}
        >
          Preview
        </Button>
      </div>
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
          {statusInfo.statusErrorCount > 0 && (
            <StatusErrorDisplay errors={statusInfo.statusErrors} variant="icon-red" />
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
          <ToggleField
            label="Include non-approved"
            checked={includeNonApproved}
            disabled={!!nonApprovedOverlap}
            onChange={onIncludeNonApprovedChange}
            rootStyle={{ marginTop: 4 }}
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

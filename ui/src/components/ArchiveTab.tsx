import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import {
  ActionIcon,
  Alert,
  Anchor,
  Button,
  Chip,
  Combobox,
  InputBase,
  Loader,
  Modal,
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
  fetchMilestoneIssues,
  issueStatusBatcher,
  useMilestoneIssues,
} from '~/api/issues'
import { type ArchiveFileRequest, generateArchive } from '~/api/archive'
import type { Segment, UnplaceableReason } from '~/api/rounds'
import {
  ARCHIVE_FILTERS,
  type ArchiveFilterKey,
  type ArchiveSelection,
  archiveSelectionOf,
  isApprovedAndCurrent,
  matchesArchiveFilter,
  neverApproved,
  provenanceLine,
} from '~/utils/archiveSelection'
import { ArchiveRoundPicker } from './ArchiveRoundPicker'
import { useRepoInfo } from '~/api/repo'
import { OpenPill } from './MilestoneFilter'
import { StatusErrorDisplay } from './StatusErrorDisplay'
import { ResizableSidebar } from './ResizableSidebar'
import { type FileResolution, FileResolveModal } from './FileResolveModal'
import { RelevantFilesList } from './RelevantFilesList'
import { extractIssueNumber } from '~/utils'
import { shortHash, unplaceableReasonText } from '~/utils/rounds'
import { ToggleField } from './ToggleField'
import { useUiSession } from '~/state/uiSession'
import { buildFileRawUrl, fetchFileContent, getFileExtensionLabel, getFilePreviewKind } from '~/api/preview'
import { DocPreview } from './DocPreview'

// ─── Constants ────────────────────────────────────────────────────────────────

const CARD_HEIGHT = 185


// ─── What this tab no longer decides ─────────────────────────────────────────
/*
 * The three predicates that used to live here — `isApprovedStatus`
 * (`status ∈ {approved, changes_after_approval}`), `archiveCommitOf`
 * (`last_approved_commit ?? latest_commit`) and `addedFileCommitOf`'s status branch —
 * are **deleted** (U7/D6). They were two of the four disagreeing definitions of
 * "approved": between them the UI sent a reopened file's *approved* bytes labelled
 * `approved: false`, while the CLI wrote `approved: true` for the same issue. The tab now
 * sends `{issue_number, round}` and renders what the segments say; it computes no
 * approval, no supersession, and no commit beyond reading the selected round's own
 * `closing_commit` / `latest_actioned_commit`, which A3 put on the wire for exactly this.
 */

function basename(path: string): string {
  return path.split('/').pop() ?? path
}

/**
 * Why a round cannot be archived, in the user's terms.
 *
 * The reason comes from `placement.reason`, which is where the wire keeps it — the round's
 * `archive_preview` is `null` and deliberately carries no copy of it. `null` is unreachable
 * on a conforming response (a null preview means unplaceable, and an unplaceable placement
 * has a reason); it is rendered rather than asserted so a shape the server says cannot occur
 * cannot take the tab down either.
 */
function refusalText(reason: UnplaceableReason | null): string {
  return reason === null ? 'its commits could not be located' : unplaceableReasonText(reason)
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

  /*
   * Per-milestone file sets — every issue title, and nothing else.
   *
   * The `approvedOnly` partition this used to carry was
   * `issues.filter(iss => iss.state === 'closed')`: **GitHub issue state** standing in for
   * approval. It is deleted (U7/§0.6). It read `false` for every approved-then-reopened
   * file, so the conflict predictor built on it mispredicted for exactly the case rounds
   * exist to describe.
   *
   * A milestone the user has not selected yet has no statuses fetched, so which of its
   * files an archive *would* include is not knowable here. Its full title set is the
   * honest answer for the dropdown's collision check: a superset, so it never claims
   * "no conflict" it cannot back up. The predictor that decides what this archive
   * actually contains reads the selections instead (U6, `plannedFiles` below).
   */
  const milestoneFileSets = useMemo(() => {
    const map = new Map<number, { all: Set<string> }>()
    for (let i = 0; i < allMilestoneNumbers.length; i++) {
      const issues = allMilestoneIssueQueries[i]?.data ?? []
      map.set(allMilestoneNumbers[i], { all: new Set(issues.map(iss => iss.title)) })
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

  // Map milestone title → number for per-milestone visibility checks
  const milestoneTitleToNumber = useMemo(() => {
    const map = new Map<string, number>()
    for (const m of milestonesData ?? []) map.set(m.title, m.number)
    return map
  }, [milestonesData])

  /**
   * Where the archive would land for one file: the round this session targets, the commit
   * that round carries, and the approval on it. One call per render per file, memoized on
   * the overrides so retargeting one card does not re-derive the rest.
   *
   * D2: the round is a *selection*, not a filter. Nothing below asks whether a file is
   * "approved" in order to decide what to send — it sends the selection and renders the
   * round.
   */
  const selectionOf = useCallback(
    (s: IssueStatusResponse): ArchiveSelection =>
      archiveSelectionOf(s, archive.roundOverrides[s.issue.number]),
    [archive.roundOverrides],
  )

  /**
   * U4: the round-aware filters, OR'd — a file is kept when it matches **any** active
   * filter, so the chips read as "show me these kinds of file". No filter is the default
   * and keeps everything.
   */
  const passesFilters = useCallback(
    (s: IssueStatusResponse): boolean =>
      archive.filters.length === 0 ||
      archive.filters.some((key) => matchesArchiveFilter(key, s, selectionOf(s))),
    [archive.filters, selectionOf],
  )

  /**
   * Whether a milestone file is on screen, and so in the archive.
   *
   * S4 narrowed "include non-approved" to mean **threads where no round has ever closed**,
   * and nothing else. It no longer governs reopened files: those have a standing approval
   * a round back and a live round in front, and which of the two the archive takes is a
   * per-file selection (D2), not a milestone-wide toggle.
   */
  const isStatusVisible = useCallback((s: IssueStatusResponse): boolean => {
    if (!passesFilters(s)) return false
    if (!neverApproved(s)) return true
    const msNum = s.issue.milestone ? milestoneTitleToNumber.get(s.issue.milestone) : undefined
    return msNum !== undefined && !!archive.includeNonApproved[msNum]
  }, [milestoneTitleToNumber, archive.includeNonApproved, passesFilters])

  /**
   * Per milestone, how many of its files would be archived at **unapproved** bytes.
   *
   * Read off the selection, not off a status pill: an approved-then-reopened file defaults
   * to its open round (D9/S2), so it archives unapproved content and is counted here —
   * which is precisely what the old `status ∈ {approved, changes_after_approval}` count
   * hid.
   */
  const unapprovedByMilestone = useMemo(() => {
    const result: Record<number, number> = {}
    for (const n of archive.selectedMilestones) {
      const milestoneName = (milestonesData ?? []).find((m) => m.number === n)?.title
      const milestoneStatuses = statuses.filter((s) => s.issue.milestone === milestoneName)
      result[n] = milestoneStatuses.filter((s) => selectionOf(s).approval === null).length
    }
    return result
  }, [archive.selectedMilestones, statuses, milestonesData, selectionOf])

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

  /*
   * ─── What this archive will contain, post-selection ──────────────────────
   *
   * One list, built once, and everything downstream reads it: the request, the flatten
   * collision check, the conflict predictor (U6), the pre-generate summary (U5) and the
   * unplaceable block (U8). The predictor used to partition by GitHub issue state
   * instead, which is why it mispredicted for every approved-then-reopened file (§0.6).
   *
   * Two kinds of entry, and they are the wire's two modes:
   *
   * - **mode 1** — a file under QC. Its handle is the issue number, and nothing else
   *   travels: the server reads the thread and derives the path, the milestone, the
   *   commit, the approval and `superseded` (A1/D6). Milestone cards are always this, and
   *   so is an added file that was resolved *via its QC issue* — that file has a thread,
   *   and a round selection is the honest way to address it.
   * - **mode 2** — a bare added file the user picked a commit for, unchanged (D4/R1).
   */
  type PlannedFile =
    | {
        mode: 'issue'
        /** Repo path, for collision checks and card ordering. */
        path: string
        issueNumber: number
        /** Absent while the status query is still in flight. */
        status: IssueStatusResponse | undefined
        selection: ArchiveSelection | null
      }
    | { mode: 'file'; path: string; commit: string }

  const plannedFiles = useMemo<PlannedFile[]>(() => {
    const planned: PlannedFile[] = []

    for (const s of statuses) {
      if (!isStatusVisible(s)) continue
      planned.push({
        mode: 'issue',
        path: s.issue.title,
        issueNumber: s.issue.number,
        status: s,
        selection: selectionOf(s),
      })
    }

    for (const r of archive.addedFiles.values()) {
      // A file the milestone side already covers is deduped there (D8: one entry per file).
      if (addedFileConflicts.has(r.file_name)) continue
      if (r.source_issue_number != null) {
        const status = addedFileStatusMap.get(r.source_issue_number)
        planned.push({
          mode: 'issue',
          path: r.file_name,
          issueNumber: r.source_issue_number,
          status,
          selection: status ? selectionOf(status) : null,
        })
        continue
      }
      planned.push({ mode: 'file', path: r.file_name, commit: r.commit })
    }

    return planned
  }, [statuses, isStatusVisible, selectionOf, archive.addedFiles, addedFileConflicts, addedFileStatusMap])

  /**
   * Files whose **selected round** cannot be archived (§18.1/§20.2).
   *
   * The gate keys on the selected round, not on the active segment: a file whose trailing
   * gap could not be placed is perfectly archivable at its round's approval, so it is not
   * greyed and not dropped. What is refused is a selection whose own round owns no
   * locatable commits — there is no sha such an archive could honestly point at (§11.1).
   */
  const blockedFiles = useMemo(
    () =>
      plannedFiles.flatMap((f) =>
        f.mode === 'issue' && f.selection?.blocked
          ? [{
              path: f.path,
              issueNumber: f.issueNumber,
              roundName: f.selection.blocked.roundName,
              reason: f.selection.blocked.reason,
            }]
          : [],
      ),
    [plannedFiles],
  )

  /**
   * The signature the acknowledgement is keyed on. Acknowledging means *proceed without
   * these files* (§11.1) — never include them — so the acknowledgement must lapse the
   * moment a different file becomes blocked rather than quietly covering it too.
   */
  const blockedKey = useMemo(
    () => blockedFiles.map((f) => `${f.issueNumber}:${f.roundName}`).sort().join('|'),
    [blockedFiles],
  )
  const blockedAcknowledged =
    blockedFiles.length === 0 || archive.unplaceableAckKey === blockedKey

  /** Exactly what the request will carry: the planned files minus the blocked ones. */
  const includedFiles = useMemo(
    () => plannedFiles.filter((f) => !(f.mode === 'issue' && f.selection?.blocked)),
    [plannedFiles],
  )

  /**
   * U6: whether turning a milestone's "include non-approved" on would collide with a file
   * this archive **already contains**.
   *
   * Repointed at the post-selection set. The old version partitioned both sides by
   * `issue.state === 'closed'`, so an approved-then-reopened file counted as non-approved
   * in one milestone and as absent from the other — the mispredict §0.6 names. What
   * arrives with the toggle is now S4's set exactly (threads where no round has ever
   * closed), and what it is compared against is the list the request will carry.
   */
  const nonApprovedOverlapByMilestone = useMemo(() => {
    const result: Record<number, string[]> = {}
    if (archive.selectedMilestones.length < 2) return result
    const titleOf = (n: number) => (milestonesData ?? []).find((m) => m.number === n)?.title

    for (const candidate of archive.selectedMilestones) {
      const candidateTitle = titleOf(candidate)
      const wouldAdd = statuses
        .filter((s) => s.issue.milestone === candidateTitle && neverApproved(s))
        .map((s) => s.issue.title)
      if (wouldAdd.length === 0) continue

      const key = (path: string) => (archive.flatten ? basename(path) : path)
      const otherFiles = new Set(
        includedFiles
          .filter((f) => !(f.mode === 'issue' && f.status?.issue.milestone === candidateTitle))
          .map((f) => key(f.path)),
      )
      const conflicts = wouldAdd.filter((f) => otherFiles.has(key(f)))
      if (conflicts.length > 0) result[candidate] = conflicts
    }
    return result
  }, [archive.selectedMilestones, archive.flatten, statuses, includedFiles, milestonesData])

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

  /**
   * U5: what this archive is about to contain, stated before it is written.
   *
   * Aggregation only — the per-file answer is the server's, projected onto each round
   * (§26), and the 200 body is `{output_path}` and nothing else, so this counts the previews
   * the response already carried rather than a post-hoc report or a second derivation of
   * `superseded`. "Approved & current" is `approval !== null` with an **empty** cause list,
   * which the wire means as a positive claim of currency; every non-empty list, whatever its
   * causes, counts as superseded.
   */
  const archiveSummary = useMemo(() => {
    let approvedCurrent = 0
    let approvedSuperseded = 0
    let unapproved = 0
    let added = 0
    let pending = 0
    const openRounds = new Set<string>()

    for (const f of includedFiles) {
      if (f.mode === 'file') {
        added++
        continue
      }
      if (f.selection === null || f.selection.selected === null) {
        pending++
        continue
      }
      if (f.selection.approval === null) {
        unapproved++
        if (f.selection.selected.state === 'open') openRounds.add(f.selection.selected.name)
      } else if (isApprovedAndCurrent(f.selection)) {
        approvedCurrent++
      } else {
        approvedSuperseded++
      }
    }

    const parts: string[] = [`${includedFiles.length} file${includedFiles.length === 1 ? '' : 's'}`]
    if (approvedCurrent > 0) parts.push(`${approvedCurrent} approved & current`)
    if (approvedSuperseded > 0) parts.push(`${approvedSuperseded} approved but superseded`)
    if (unapproved > 0) {
      const rounds = [...openRounds]
      parts.push(
        rounds.length > 0 && rounds.length <= 2
          ? `${unapproved} unapproved (${rounds.join(', ')} open)`
          : `${unapproved} unapproved`,
      )
    }
    if (added > 0) parts.push(`${added} added file${added === 1 ? '' : 's'}`)
    if (pending > 0) parts.push(`${pending} still loading`)

    return { text: parts.join(' · '), unapproved, approvedSuperseded }
  }, [includedFiles])

  /**
   * U2/D7: retarget one file's round, or clear the override with `null`.
   *
   * `null` is stored as an *absence*, not as the latest round's number: the default is
   * "the latest round, whatever it becomes", and pinning today's latest by number would
   * silently turn into an override the next time a round opens.
   */
  const setRoundOverride = useCallback((issueNumber: number, round: number | null) => {
    setArchive(prev => {
      const next = { ...prev.roundOverrides }
      if (round === null) delete next[issueNumber]
      else next[issueNumber] = round
      return { ...prev, roundOverrides: next }
    })
  }, [setArchive])

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

  // ─── Generation ──────────────────────────────────────────────────────────

  async function handleGenerate() {
    setArchive(prev => ({ ...prev, generateError: null, generateSuccess: null, generateLoading: true }))
    try {
      /*
       * A1/A2: the request is built from the tagged types and never hand-assembled.
       *
       * Mode 1 carries the issue number and the round, and nothing else — no
       * `repository_file`, no `commit`, no `milestone`, and no `approved`. Each of those
       * was either a second source of truth for a fact the server already holds, or the
       * one bool that was unfalsifiable after the first approval. `round` travels as an
       * explicit `null` for the default so "latest" has exactly one encoding on the wire,
       * and as a number only where the user overrode it (D7).
       *
       * Mode 2 carries the path and the commit the user picked, as it always did — and no
       * `approved: false`, which is the key the server rejected with a 400 for every
       * manually added file (§11.5). The migration is what fixes that; it was never
       * patched against the old flat shape.
       */
      const files: ArchiveFileRequest[] = includedFiles.map((f) =>
        f.mode === 'issue'
          ? {
              mode: 'issue' as const,
              issue_number: f.issueNumber,
              round: archive.roundOverrides[f.issueNumber] ?? null,
            }
          : { mode: 'file' as const, repository_file: f.path, commit: f.commit },
      )

      const result = await generateArchive({ output_path: archive.outputPath, flatten: archive.flatten, files })
      setArchive(prev => ({ ...prev, generateSuccess: result.output_path }))
    } catch (err) {
      // The server's message, verbatim: the round-selection and unplaceable refusals name
      // the issue and the reason on purpose, and every client error on this route wears
      // the `{"error": …}` envelope, so there is nothing to translate.
      setArchive(prev => ({ ...prev, generateError: (err as Error).message }))
    } finally {
      setArchive(prev => ({ ...prev, generateLoading: false }))
    }
  }

  /**
   * How many milestone files the active filters take off screen — and so out of the
   * archive. A filter that silently shrinks an audit artifact is the omission §0.4
   * describes, so the number is shown beside the chips.
   */
  const hiddenByFilterCount = useMemo(() => {
    if (archive.filters.length === 0) return 0
    return statuses.filter((s) => !passesFilters(s)).length
  }, [statuses, archive.filters, passesFilters])

  const visibleFileCount =
    statuses.filter(s => isStatusVisible(s)).length +
    archive.addedFiles.size

  const canGenerate =
    visibleFileCount > 0 &&
    includedFiles.length > 0 &&
    archive.outputPath.trim().length > 0 &&
    !isLoadingStatuses &&
    unresolvedAddedFileCount === 0 &&
    conflictCount === 0 &&
    // U8/§11.1: a file whose selected round owns no locatable commits blocks generation
    // until it is acknowledged, and acknowledging leaves it out. The old note said only
    // how many files were dropped, at the bottom of the sidebar, where it was missed.
    blockedAcknowledged

  // Files already occupying the right panel — unselectable in AddFileModal
  const claimedFiles = useMemo(() => {
    const s = new Set<string>()
    statuses.filter(st => isStatusVisible(st)).forEach(st => s.add(st.issue.title))
    archive.addedFiles.forEach((_, fn) => s.add(fn))
    return s
  }, [statuses, isStatusVisible, archive.addedFiles])

  // ─── Flatten collision detection ─────────────────────────────────────

  // U6: the collision check reads the files this archive will actually include, which is
  // the same list the request is built from — not a set partitioned by issue state.
  const allArchiveFiles = useMemo(
    () => includedFiles.map((f) => f.path),
    [includedFiles],
  )

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
              {archive.generateError && (
                <Alert color="red" p="xs">
                  <Text size="xs">{archive.generateError}</Text>
                </Alert>
              )}
              {archive.generateSuccess && (
                <Alert color="green" p="xs">
                  <Text size="xs">Archive written to {archive.generateSuccess}</Text>
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
              {/*
                U8/§11.1: a blocking callout, naming each file and carrying the reason —
                not the footnote it replaces. The override is *acknowledge and proceed
                without these files*; it never includes them, because an unplaceable round
                owns no commits and there is no sha the archive could honestly point at. A
                user who knows the commit they want adds the file directly, which is what
                the Add-file card is for (D4).
              */}
              {blockedFiles.length > 0 && (
                <Alert color="red" p="xs" data-testid="archive-unplaceable-callout">
                  <Stack gap={4}>
                    <Text size="xs" fw={700}>
                      {blockedFiles.length} file{blockedFiles.length === 1 ? '' : 's'} cannot be
                      archived — the round selected for each owns no locatable commits:
                    </Text>
                    {blockedFiles.map((f) => (
                      <Text size="xs" key={f.issueNumber} data-testid={`archive-blocked-${f.issueNumber}`}>
                        {f.path} (#{f.issueNumber}, {f.roundName}): {refusalText(f.reason)}
                      </Text>
                    ))}
                    <Text size="xs">
                      Proceeding leaves {blockedFiles.length === 1 ? 'it' : 'them'} out of the
                      archive entirely. To archive one anyway, add it with the commit you name.
                    </Text>
                    {!blockedAcknowledged && (
                      <Button
                        size="compact-xs"
                        color="red"
                        variant="light"
                        data-testid="archive-unplaceable-acknowledge"
                        onClick={() => setArchive(prev => ({ ...prev, unplaceableAckKey: blockedKey }))}
                      >
                        Continue without {blockedFiles.length === 1 ? 'this file' : 'these files'}
                      </Button>
                    )}
                  </Stack>
                </Alert>
              )}
              {/* U5: stated before the archive is written, from the selections. */}
              {includedFiles.length > 0 && (
                <Text size="xs" c="dimmed" data-testid="archive-summary">
                  {archiveSummary.text}
                </Text>
              )}
              {/*
                U3/D9: the default is the latest round, so a reopened file archives
                unapproved content. Said loudly and nothing gates it — a user cutting an
                archive for a previous QC round retargets that file's round picker.
              */}
              {archiveSummary.unapproved > 0 && (
                <Text size="xs" c="orange.7" data-testid="archive-unapproved-note">
                  {archiveSummary.unapproved} file{archiveSummary.unapproved === 1 ? '' : 's'} will be
                  archived at unapproved bytes: the round selected for
                  {archiveSummary.unapproved === 1 ? ' it' : ' them'} is open. That is the default —
                  the latest round, not the newest approval. Target an earlier round to archive the
                  approval that stands there.
                </Text>
              )}
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
                {/*
                  U4: round-aware bulk filters, each derived from the segments or from a
                  fact `qc_status` already carries — never from a local approval predicate.
                  They narrow what is on screen, and what is on screen is what the archive
                  contains, so the count of what they hide is stated next to them rather
                  than left to be discovered in the tarball.
                */}
                <Stack gap={4}>
                  <Text size="xs" fw={600} c="dimmed">Filter files</Text>
                  <Chip.Group
                    multiple
                    value={archive.filters}
                    onChange={(value) => setArchive(prev => ({ ...prev, filters: value as ArchiveFilterKey[] }))}
                  >
                    <div style={{ display: 'flex', flexWrap: 'wrap', gap: 4 }}>
                      {ARCHIVE_FILTERS.map((f) => (
                        // The testid sits on the wrapper, not the Chip: Mantine forwards it
                        // to the visually hidden checkbox, which nothing can click.
                        <span key={f.key} data-testid={`archive-filter-${f.key}`}>
                          <Chip value={f.key} size="xs">{f.label}</Chip>
                        </span>
                      ))}
                    </div>
                  </Chip.Group>
                  {archive.filters.length > 0 && (
                    <Text size="xs" c="orange.7" data-testid="archive-filter-note">
                      {hiddenByFilterCount} file{hiddenByFilterCount === 1 ? '' : 's'} hidden by
                      {' '}{archive.filters.length === 1 ? 'this filter' : 'these filters'} — hidden
                      files are not archived.
                    </Text>
                  )}
                </Stack>
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
                // An added file with a QC issue behind it is a **mode-1** entry: it has a
                // thread, so the round is what addresses it and the server derives the
                // commit. The commit shown here is the selected round's own, read for the
                // card and the preview — never sent, and never the deleted
                // `last_approved_commit ?? latest_commit`.
                const selection = selectionOf(issueStatus)
                const commit = selection.commit
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
                        {selection.approval === null && !selection.blocked && (
                          <Tooltip label="These bytes were never approved" withArrow>
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
                      <ArchiveProvenance
                        issueNumber={issueStatus.issue.number}
                        segments={issueStatus.segments ?? []}
                        selection={selection}
                        onSelectRound={(round) => setRoundOverride(issueStatus.issue.number, round)}
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
                    <div>
                      <Button
                        size="compact-xs"
                        variant="light"
                        leftSection={<IconEye size={12} />}
                        disabled={commit === null}
                        onClick={e => { e.stopPropagation(); if (commit !== null) void handlePreviewFile(fileName, commit) }}
                      >
                        Preview
                      </Button>
                    </div>
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
              const selection = selectionOf(s)
              const commit = selection.commit
              const statusLabel = s.qc_status.status.replace(/_/g, ' ')

              return (
                <Stack
                  key={s.issue.number}
                  gap={5}
                  style={{
                    padding: '10px 12px',
                    borderRadius: 6,
                    border: `1px solid ${selection.blocked ? '#ff8787' : 'var(--mantine-color-gray-3)'}`,
                    backgroundColor: selection.blocked ? '#ffe3e3' : 'white',
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
                      {selection.approval === null && !selection.blocked && (
                        <Tooltip label="These bytes were never approved" withArrow>
                          <span style={{ flexShrink: 0, marginTop: 2 }}>
                            <IconAlertTriangle size={12} color="#f59f00" />
                          </span>
                        </Tooltip>
                      )}
                    </div>
                    {s.issue.milestone && (
                      <Text size="xs" c="dimmed"><b>Milestone:</b> {s.issue.milestone}</Text>
                    )}
                    <ArchiveProvenance
                      issueNumber={s.issue.number}
                      segments={s.segments ?? []}
                      selection={selection}
                      onSelectRound={(round) => setRoundOverride(s.issue.number, round)}
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
                  <div>
                    <Button
                      size="compact-xs"
                      variant="light"
                      leftSection={<IconEye size={12} />}
                      disabled={commit === null}
                      onClick={() => { if (commit !== null) void handlePreviewFile(s.issue.title, commit) }}
                    >
                      Preview
                    </Button>
                  </div>
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

// ─── ArchiveProvenance ────────────────────────────────────────────────────────
//
// U1/D1: **two independent facts, never one bool.** The provenance of the bytes — which
// round the selection addressed, whether those bytes were approved, by whom, when, and at
// which commit — and, separately, whether anything newer is known about the file. One bool
// compressing the two is the root cause §0 diagnoses.
//
// The two round frames are kept visibly apart. When a round's anchor is the previous
// round's approval, selecting the open round archives approved bytes under a different
// frame (I2), and this card reads *"Round 2 · bytes are Initial QC's approval …"* — never
// "Round 2 · approved", which would assert an approval of a round that is still open.

function ArchiveProvenance({
  issueNumber,
  segments,
  selection,
  onSelectRound,
}: {
  issueNumber: number
  segments: Segment[]
  selection: ArchiveSelection
  onSelectRound: (round: number | null) => void
}) {
  if (selection.blocked !== null) {
    return (
      <>
        <Text size="xs" c="red.8" fw={600} data-testid={`archive-blocked-note-${issueNumber}`}>
          {selection.blocked.roundName} cannot be archived —{' '}
          {refusalText(selection.blocked.reason)}. This file will be left out.
        </Text>
        <ArchiveRoundPicker
          segments={segments}
          selection={selection}
          issueNumber={issueNumber}
          onSelectRound={onSelectRound}
        />
      </>
    )
  }

  return (
    <>
      <Text size="xs" c="dimmed" data-testid={`archive-provenance-${issueNumber}`}>
        {provenanceLine(selection)}
      </Text>
      {selection.stale.length > 0 && (
        <Text size="xs" c="orange.7" data-testid={`archive-superseded-${issueNumber}`}>
          Not the newest QC state: {selection.stale.join('; ')}.
        </Text>
      )}
      <ArchiveRoundPicker
        segments={segments}
        selection={selection}
        issueNumber={issueNumber}
        onSelectRound={onSelectRound}
      />
    </>
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
  /**
   * Every file title per milestone. The `approvedOnly` half is gone with U7 — it was
   * `issue.state === 'closed'`, GitHub issue state standing in for approval (§0.6).
   */
  milestoneFileSets: Map<number, { all: Set<string> }>
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

  /*
   * The union of file names the selected milestones contribute.
   *
   * Both sides of this check are now the milestones' **full** file sets. It used to switch
   * between `all` and `approvedOnly` per milestone, which put a reopened file on one side
   * and not the other; the collision this predicts is between *file identities*, and a
   * milestone containing a path at all is a milestone that can collide on it. The check is
   * deliberately conservative here — it disables a dropdown option, so claiming "no
   * conflict" it cannot back up is the worse error, and the archive's actual contents are
   * predicted from the selections instead (U6).
   */
  const selectedFileUnion = useMemo(() => {
    const union = new Set<string>()
    for (const n of selectedMilestones) {
      const fileSet = milestoneFileSets.get(n)
      if (!fileSet) continue
      for (const f of fileSet.all) union.add(flatten ? basename(f) : f)
    }
    return union
  }, [selectedMilestones, milestoneFileSets, flatten])

  // For each candidate milestone, compute conflicts with the selected set
  const milestoneConflicts = useMemo(() => {
    const map = new Map<number, { conflicts: string[]; milestones: string[] }>()
    for (const m of filtered) {
      const candidateFiles = milestoneFileSets.get(m.number)
      if (!candidateFiles) continue
      const candidateSet = candidateFiles.all
      const conflicts: string[] = []
      for (const f of candidateSet) {
        if (selectedFileUnion.has(flatten ? basename(f) : f)) conflicts.push(f)
      }
      if (conflicts.length > 0) {
        const owningMilestones = new Set<string>()
        for (const n of selectedMilestones) {
          const fileSet = milestoneFileSets.get(n)
          if (!fileSet) continue
          for (const f of conflicts) {
            if (fileSet.all.has(f)) {
              const title = (data ?? []).find(ms => ms.number === n)?.title ?? String(n)
              owningMilestones.add(title)
            }
          }
        }
        map.set(m.number, { conflicts, milestones: [...owningMilestones] })
      }
    }
    return map
  }, [filtered, milestoneFileSets, selectedFileUnion, selectedMilestones, data, flatten])

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
          {/*
            Round-aware: the count is of files whose *selected* round yields unapproved
            bytes, which includes an approved-then-reopened file sitting on its open round
            by default (D9/S2). The old count read a status pill and hid exactly those.
          */}
          {isYellow && (
            <Tooltip
              label={`${unapprovedCount} file${unapprovedCount !== 1 ? 's' : ''} would be archived at unapproved bytes`}
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

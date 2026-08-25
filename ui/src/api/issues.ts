import { useMutation, useQueries, useQuery, useQueryClient } from '@tanstack/react-query'
import { API_BASE } from '../config'

export type RelevantFileKind = 'blocking_qc' | 'previous_qc' | 'relevant_qc' | 'file'

export interface RelevantFileInfo {
  file_name: string
  kind: RelevantFileKind
  issue_url: string | null
}

export interface FileRenameEvent {
  old_path: string
  new_path: string
  commit: string
}

export interface Issue {
  number: number
  title: string
  state: 'open' | 'closed'
  html_url: string
  assignees: string[]
  labels: string[]
  milestone: string | null
  created_at: string
  updated_at: string
  closed_at: string | null
  created_by: string
  /**
   * Round 1's declared branch, parsed from the issue body (D50). Valid as the
   * *current* branch only when `has_qc_rounds_marker` is false; when it is true the
   * current branch is `rounds[last].branch`, which needs the comment fetch.
   */
  branch: string | null
  /**
   * D52: whether the body carries a `## QC Rounds` marker. Presence only — it never
   * reports a round count and is never authoritative for round data (D51). It exists
   * so a cheap list path that never fetches comments can tell whether `branch` above
   * is still current.
   */
  has_qc_rounds_marker: boolean
  relevant_files: RelevantFileInfo[]
  file_history: FileRenameEvent[]
}

export interface IssueCommit {
  hash: string
  message: string
  statuses: ('initial' | 'notification' | 'approved' | 'reviewed')[]
  file_changed: boolean
}

export interface ChecklistSummary {
  completed: number
  total: number
  percentage: number
}

/**
 * A verdict, not a commit carrier (D35). `approved_commit`, `initial_commit` and
 * `latest_commit` are gone: all three were round-scoped and duplicated
 * `rounds[last].state`, `rounds[0].start_commit` and `rounds[last].archive_commit`.
 */
export interface QCStatus {
  status:
    | 'approved'
    | 'changes_after_approval'
    | 'awaiting_review'
    | 'change_requested'
    | 'in_progress'
    | 'approval_required'
    | 'changes_to_comment'
  status_detail: string
}

/**
 * A round's state (D36) — a tagged union, so an open round carrying an approval is
 * inexpressible. Switch on `kind`; there is no `approved_commit` field.
 */
export type RoundState =
  | { kind: 'open' }
  | { kind: 'approved'; commit: string; comment_id: number | null }
  | { kind: 'superseded' }

/** One shape for both gap positions — a round's `preceding_gap` and the thread's `drift` (D30). */
export interface Gap {
  /** newest-first; `[]` is normal and meaningful (the D8 overlap case). */
  commits: IssueCommit[]
  /** The anchoring approval is not in this branch's ancestry (D22/D31). */
  divergent: boolean
  /**
   * D35: the hash `ChangesAfterApproval` reports. The **only** legal source for it —
   * rescanning `commits` client-side would violate U7.
   */
  newest_file_change: string | null
}

/**
 * D53: whether the round could be placed on its branch. `unplaceable` means the
 * declared start commit could not be resolved there — usually the branch is not
 * fetched locally. The round still exists and keeps its declared index; the remedy
 * is to fetch `RoundInfo.branch`.
 */
export type RoundPlacement = 'placed' | 'unplaceable'

/** One QC round (A4). The status card reads `drift`; the round switcher reads `rounds[]`. */
export interface RoundInfo {
  /**
   * The **declared** round number (D53.2) — never a position. `rounds` can carry a
   * hole (a malformed declaration is dropped without renumbering), so `rounds[i].index`
   * is not `i + 1` and no round number may be derived from an array position or length.
   */
  index: number
  branch: string
  /**
   * D56: `true` when the round comment declared no `git branch:` and inherited this
   * one. Surfaced wherever the round is viewed — branch is load-bearing for both the
   * round's and its gap's commit walk (D7/D9), so the inheritance must not be silent.
   */
  branch_inherited: boolean
  /** D53: `unplaceable` ⇒ `commits` and `preceding_gap` are empty and
   *  `archive_commit` is `null`. */
  placement: RoundPlacement
  start_commit: string
  /** The sole encoding of approvedness (D36). */
  state: RoundState
  /** '' when the round carries no checklist. */
  checklist_name: string
  /** Excludes its `# ` heading line (D37) — never re-add it. */
  checklist_content: string
  checklist_summary: ChecklistSummary
  /** This round's commits only (D8/W5) — never the whole QC's life. */
  commits: IssueCommit[]
  preceding_gap: Gap
  /**
   * `Round::latest_commit().hash` (M8) — the commit the archive freezes.
   *
   * D54: `null` in exactly that function's two `None` cases — the round is
   * unplaceable, or it is approved and its approval is no longer among the commits it
   * owns (a force-push). D55: consumers **refuse and name the branch**; a substitute
   * here would let an archive claim approved content over content that was never
   * approved.
   */
  archive_commit: string | null
  /** D39.3: conservatively `true` whenever `archive_commit` is `null` — with no
   *  resolved commit there is nothing to measure "after". */
  subsequent_file_changes: boolean
}

export interface BlockingQCItem {
  issue_number: number
  file_name: string
}

export interface BlockingQCItemWithStatus {
  issue_number: number
  file_name: string
  status: string
}

export interface BlockingQCError {
  issue_number: number
  error: string
  kind: IssueStatusErrorKind
  /** Title of the QC'd file. Present for processing failures; absent when the issue itself failed to fetch. */
  file_name?: string
  /** Set when `kind === 'branch_not_local'` — the branch the user needs to check out. */
  branch?: string
}

export interface BlockingQCStatus {
  total: number
  approved_count: number
  summary: string
  approved: BlockingQCItem[]
  not_approved: BlockingQCItemWithStatus[]
  errors: BlockingQCError[]
}

/**
 * D24/A2/A3: no top-level field duplicates a round-scoped value. `commits`, `branch`
 * and `checklist_summary` are gone — read `rounds[rounds.length - 1]` (list indexing,
 * not derivation) and `drift`.
 */
/**
 * Which kind of segment a `SegmentRef` points at (M2). `drift` is its own kind even
 * though it is the same `Gap` shape as a preceding gap: D30 — a trailing gap is defined
 * by its position — and the pinned tail block (D80/D81) is picked by kind, never by
 * index arithmetic.
 */
export type SegmentKind = 'round' | 'gap' | 'drift'

/**
 * One row of the History dropdown (M2): a **pointer** into `rounds`/`drift`, never a
 * copy of their commits.
 */
export interface SegmentRef {
  kind: SegmentKind
  /**
   * The round this segment belongs to: itself for a round, the round it **precedes**
   * for a gap, the latest round for drift. A *declared* index (D53.2) — never a position
   * in `history` or in `rounds`.
   */
  round_index: number
}

export interface IssueStatusResponse {
  issue: Issue
  qc_status: QCStatus
  dirty: boolean
  /** Never empty (I1); `rounds[0]` comes from the issue body. */
  rounds: RoundInfo[]
  /**
   * Always present; empty when the latest round is unapproved (D17). Check
   * `latestRound(status).state.kind` to know whether it is meaningful — the same
   * dispatch the backend uses (S0).
   */
  drift: Gap
  /**
   * W6's segment order (M2) — the History dropdown's rows, and the only source of that
   * order. It encodes two positional suppression rules (round 1's preceding gap is
   * skipped; `drift` appears only once the latest round is closed) that the client must
   * not reimplement (D30/U7). Never empty: I1 guarantees a round.
   */
  history: SegmentRef[]
  blocking_qc_status?: BlockingQCStatus
}

/** The latest round — list indexing, never derivation (D24). */
export function latestRound(status: IssueStatusResponse): RoundInfo {
  return status.rounds[status.rounds.length - 1]
}

/** A round by its 1-based index, falling back to the latest. */
export function roundByIndex(status: IssueStatusResponse, index: number): RoundInfo {
  return status.rounds.find((r) => r.index === index) ?? latestRound(status)
}

/**
 * D54/D55: why this round cannot be archived, or `null` when it can. The message
 * names the branch to fetch — it never stands in for a commit, because inventing one
 * would freeze a false claim (D28.3).
 */
export function archiveBlockedReason(round: RoundInfo): string | null {
  if (round.archive_commit !== null) return null
  return `Fetch ${round.branch} to archive round ${round.index}`
}

/** The approval commit of a round, or null when it is not approved (D36). */
export function roundApprovedCommit(round: RoundInfo): string | null {
  return round.state.kind === 'approved' ? round.state.commit : null
}

/**
 * Deep-link to the approval comment (U8/D36), or null when there is none.
 *
 * D44: `comment_id` is nullable — the old `#[serde(default)]` made a missing id
 * deserialize to `0`, a valid-looking comment id that lies. `None` means "unknown",
 * and U8's deep-link is omitted rather than pointing at comment 0.
 */
export function approvalCommentUrl(issue: Issue, round: RoundInfo): string | null {
  if (round.state.kind !== 'approved' || round.state.comment_id == null) return null
  return `${issue.html_url}#issuecomment-${round.state.comment_id}`
}

/** A new round may be started only from an approved QC (D12/U1). */
export function canStartRound(status: QCStatus['status']): boolean {
  return status === 'approved' || status === 'changes_after_approval'
}

// ---------------------------------------------------------------------------
// Rounds (A5)
// ---------------------------------------------------------------------------

export interface CreateRoundRequest {
  /** From the checkout, read-only — there is no commit picker (D23). */
  start_commit: string
  /** From the checkout, read-only (D23). */
  branch: string
  /** `content` excludes the `# {name}` heading line (D37). */
  checklist: { name: string; content: string }
  notify: boolean
  note: string | null
  include_diff: boolean
}

/**
 * D45: the notification outcome is a tagged union, because a bare
 * `notification_url: null` conflated "not requested" with "requested but failed" —
 * the second is the one a user must be able to act on.
 */
export type RoundNotification =
  | { kind: 'not_requested' }
  | { kind: 'posted'; url: string }
  | { kind: 'failed'; error: string }

export interface CreateRoundResponse {
  round_index: number
  comment_url: string
  /**
   * D45: false means the issue stayed closed with an unapproved latest round, which
   * S3 reads as `ApprovalRequired` — a just-created round reporting "approval
   * required". That must be visible, not logged.
   */
  reopened: boolean
  notification: RoundNotification
}

export async function postRound(issueNumber: number, request: CreateRoundRequest): Promise<CreateRoundResponse> {
  const res = await fetch(`${API_BASE}/issues/${issueNumber}/rounds`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(request),
  })
  if (!res.ok) {
    const data = await res.json().catch(() => null)
    throw new Error(data?.error ?? `Failed to start round: ${res.status}`)
  }
  return res.json()
}

export interface CreateCommentRequest {
  current_commit: string
  previous_commit: string | null
  note: string | null
  include_diff: boolean
}

export interface ReviewRequest {
  commit: string
  note: string | null
  include_diff: boolean
  auto_stash: boolean
}

export interface ApproveRequest {
  commit: string
  note: string | null
}

export interface ApprovalResponse {
  approval_url: string
  skipped_unapproved: number[]
  skipped_errors: BlockingQCError[]
  closed: boolean
}

export interface UnapproveRequest {
  reason: string
}

export interface UnapprovalResponse {
  unapproval_url: string
  opened: boolean
}

export interface CommentResponse {
  comment_url: string
}

export interface ReviewStashResult {
  status: 'stashed' | 'no_changes' | 'skipped' | 'failed'
  message: string | null
}

export interface ReviewResponse {
  comment_url: string
  stash: ReviewStashResult
}

export async function postComment(issueNumber: number, request: CreateCommentRequest): Promise<CommentResponse> {
  const res = await fetch(`${API_BASE}/issues/${issueNumber}/comment`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(request),
  })
  if (!res.ok) {
    const data = await res.json().catch(() => null)
    throw new Error(data?.error ?? `Failed to post comment: ${res.status}`)
  }
  return res.json()
}

export type IssueStatusErrorKind = 'fetch_failed' | 'processing_failed' | 'branch_not_local'

export interface IssueStatusError {
  issue_number: number
  kind: IssueStatusErrorKind
  error: string
  /** Set when `kind === 'branch_not_local'`. */
  branch?: string
}

export interface BatchIssueStatusResponse {
  results: IssueStatusResponse[]
  errors: IssueStatusError[]
}

// Returned by each per-issue query. Backend application-level errors come back
// as { ok: false } so React Query doesn't treat them as retryable failures.
export type IssueStatusResult =
  | { ok: true; data: IssueStatusResponse }
  | { ok: false; error: IssueStatusError }

export interface MilestoneStatusInfo {
  listFailed: boolean
  listError: string | null
  loadingCount: number
  statusErrorCount: number
  statusErrors: IssueStatusError[]
  statusAttemptedCount: number
}

export async function fetchMilestoneIssues(milestoneNumber: number): Promise<Issue[]> {
  const res = await fetch(`${API_BASE}/milestones/${milestoneNumber}/issues`)
  if (!res.ok) throw new Error(`Failed to fetch issues for milestone ${milestoneNumber}: ${res.status}`)
  return res.json()
}

async function fetchIssueStatuses(issueNumbers: number[]): Promise<BatchIssueStatusResponse> {
  const res = await fetch(`${API_BASE}/issues/status?issues=${issueNumbers.join(',')}`)
  const data = await res.json()
  if ('results' in data && 'errors' in data) return data as BatchIssueStatusResponse
  throw new Error(`Failed to fetch issue statuses: ${res.status}`)
}

export async function postReview(issueNumber: number, request: ReviewRequest): Promise<ReviewResponse> {
  const res = await fetch(`${API_BASE}/issues/${issueNumber}/review`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(request),
  })
  if (!res.ok) {
    const data = await res.json().catch(() => null)
    throw new Error(data?.error ?? `Failed to post review: ${res.status}`)
  }
  return res.json()
}

export async function postUnapprove(issueNumber: number, request: UnapproveRequest): Promise<UnapprovalResponse> {
  const res = await fetch(`${API_BASE}/issues/${issueNumber}/unapprove`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(request),
  })
  if (!res.ok) {
    const data = await res.json().catch(() => null)
    throw new Error(data?.error ?? `Failed to unapprove: ${res.status}`)
  }
  return res.json()
}

export async function postApprove(issueNumber: number, request: ApproveRequest, force = false): Promise<ApprovalResponse> {
  const url = force ? `${API_BASE}/issues/${issueNumber}/approve?force=true` : `${API_BASE}/issues/${issueNumber}/approve`
  const res = await fetch(url, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(request),
  })
  if (!res.ok) {
    const data = await res.json().catch(() => null)
    throw new Error(data?.error ?? `Failed to approve: ${res.status}`)
  }
  return res.json()
}

export async function fetchSingleIssueStatus(issueNumber: number): Promise<IssueStatusResponse> {
  const batch = await fetchIssueStatuses([issueNumber])
  const result = batch.results.find((r) => r.issue.number === issueNumber)
  if (result) return result
  const err = batch.errors.find((e) => e.issue_number === issueNumber)
  throw new Error(err?.error ?? `No status returned for issue ${issueNumber}`)
}

// ---------------------------------------------------------------------------
// Blocking QC inverse map
// ---------------------------------------------------------------------------
// blockingQcInverseMap: blocking-QC issue number → set of issue numbers that
// list it as a blocking QC. Used to invalidate only the affected dependent
// issues when a blocker is approved or unapproved.
//
// issueToBlockingQcs: issue number → set of its blocking QC issue numbers.
// Kept in sync so stale entries are removed when a status is re-fetched.
const blockingQcInverseMap = new Map<number, Set<number>>()
const issueToBlockingQcs  = new Map<number, Set<number>>()

function updateBlockingQcMaps(issueNumber: number, status: BlockingQCStatus | undefined) {
  // Remove this issue from whichever sets it was previously in.
  const prev = issueToBlockingQcs.get(issueNumber)
  if (prev) {
    for (const blockerNum of prev) {
      blockingQcInverseMap.get(blockerNum)?.delete(issueNumber)
    }
  }
  // Rebuild from the freshly-fetched status.
  const next = new Set<number>()
  if (status) {
    for (const item of [...status.approved, ...status.not_approved]) {
      next.add(item.issue_number)
    }
  }
  issueToBlockingQcs.set(issueNumber, next)
  for (const blockerNum of next) {
    if (!blockingQcInverseMap.has(blockerNum)) blockingQcInverseMap.set(blockerNum, new Set())
    blockingQcInverseMap.get(blockerNum)!.add(issueNumber)
  }
}

/** Returns a stable callback that invalidates every issue whose blocking-QC
 *  status depends on `issueNumber` (i.e. issues that list it as a blocker). */
export function useInvalidateBlockingDependents() {
  const queryClient = useQueryClient()
  return (issueNumber: number) => {
    const dependents = blockingQcInverseMap.get(issueNumber)
    if (dependents) {
      for (const num of dependents) {
        void queryClient.invalidateQueries({ queryKey: ['issue', 'status', num] })
      }
    }
  }
}

// Module-level tick batcher. All batcher.load() calls within a single synchronous
// render pass land before setTimeout fires, so they're coalesced into one HTTP
// request. React Query's own cache means already-fetched issues never reach here.
export const issueStatusBatcher = (() => {
  let pending = new Map<number, { resolve: (v: IssueStatusResult) => void; reject: (e: Error) => void }>()
  let timer: ReturnType<typeof setTimeout> | null = null

  function dispatch() {
    const batch = pending
    pending = new Map()
    timer = null

    fetchIssueStatuses([...batch.keys()])
      .then((response) => {
        const handled = new Set<number>()
        for (const r of response.results) {
          batch.get(r.issue.number)?.resolve({ ok: true, data: r })
          handled.add(r.issue.number)
          updateBlockingQcMaps(r.issue.number, r.blocking_qc_status)
        }
        for (const e of response.errors) {
          batch.get(e.issue_number)?.resolve({ ok: false, error: e })
          handled.add(e.issue_number)
        }
        for (const [num, { reject }] of batch) {
          if (!handled.has(num)) reject(new Error(`No status returned for issue ${num}`))
        }
      })
      .catch((err: Error) => {
        for (const { reject } of batch.values()) reject(err)
      })
  }

  return {
    load(num: number): Promise<IssueStatusResult> {
      return new Promise((resolve, reject) => {
        pending.set(num, { resolve, reject })
        if (!timer) timer = setTimeout(dispatch, 0)
      })
    },
  }
})()

export function useMilestoneIssues(milestoneNumbers: number[], includeClosedIssues: boolean | Record<number, boolean>) {
  // Normalize to per-milestone lookup
  const closedByMilestone = typeof includeClosedIssues === 'boolean'
    ? Object.fromEntries(milestoneNumbers.map(n => [n, includeClosedIssues]))
    : includeClosedIssues

  // Step 1: fetch issue lists per milestone (each independently cached)
  const milestoneQueries = useQueries({
    queries: milestoneNumbers.map((n) => ({
      queryKey: ['milestones', n, 'issues'],
      queryFn: () => fetchMilestoneIssues(n),
    })),
  })

  // Step 2: one query per issue, no open/closed or per-milestone distinction.
  // The batcher coalesces all queryFn calls from one render into a single HTTP
  // request. React Query's cache prevents re-fetching already-seen issues.
  const allNeededNums = milestoneQueries
    .flatMap((q, idx) =>
      (q.data ?? [])
        .filter((i) => i.state === 'open' || closedByMilestone[milestoneNumbers[idx]])
        .map((i) => i.number),
    )
    .filter((n, i, arr) => arr.indexOf(n) === i) // deduplicate
    .sort((a, b) => a - b)

  const statusQueries = useQueries({
    queries: allNeededNums.map((num) => ({
      queryKey: ['issue', 'status', num],
      queryFn: () => issueStatusBatcher.load(num),
      staleTime: 5 * 60 * 1000,
    })),
  })

  const allIssues = milestoneQueries.flatMap((q) => q.data ?? [])
  const deduped = [...new Map(allIssues.map((i) => [i.number, i])).values()]
  // Keep an issue if ANY of its milestones have closed issues enabled
  const anyIncludeClosed = Object.values(closedByMilestone).some(Boolean)
  const issues = anyIncludeClosed ? deduped : deduped.filter((i) => i.state === 'open')

  const statuses = statusQueries.flatMap((q) => (q.data?.ok ? [q.data.data] : []))

  const milestoneStatusByMilestone: Record<number, MilestoneStatusInfo> = {}
  milestoneNumbers.forEach((milestoneNum, milestoneIdx) => {
    const listQuery = milestoneQueries[milestoneIdx]
    const milestoneIssues = listQuery?.data ?? []

    const listFailed = listQuery?.isError ?? false
    const listError = listFailed ? ((listQuery.error as Error)?.message ?? 'Failed to fetch issues') : null

    const relevantNums = new Set(
      milestoneIssues
        .filter((i) => i.state === 'open' || closedByMilestone[milestoneNum])
        .map((i) => i.number),
    )

    let loadingCount = 0
    const statusErrors: IssueStatusError[] = []

    for (let i = 0; i < allNeededNums.length; i++) {
      const num = allNeededNums[i]
      if (!relevantNums.has(num)) continue
      const q = statusQueries[i]
      if (q.isPending && q.fetchStatus !== 'idle') loadingCount++
      if (q.data && !q.data.ok) statusErrors.push(q.data.error)
      else if (q.isError)
        statusErrors.push({
          issue_number: num,
          kind: 'fetch_failed',
          error: (q.error as Error)?.message ?? 'Failed to fetch status',
        })
    }

    milestoneStatusByMilestone[milestoneNum] = {
      listFailed,
      listError,
      loadingCount,
      statusErrorCount: statusErrors.length,
      statusErrors,
      statusAttemptedCount: relevantNums.size,
    }
  })

  return {
    issues,
    statuses,
    milestoneStatusByMilestone,
    isLoadingIssues: milestoneQueries.some((q) => q.isPending),
    isLoadingStatuses: statusQueries.some((q) => q.isPending && q.fetchStatus !== 'idle'),
    isError: milestoneQueries.some((q) => q.isError) || statusQueries.some((q) => q.isError),
  }
}

export function useIssuesForMilestone(milestoneNumber: number | null) {
  return useQuery({
    queryKey: ['milestones', milestoneNumber, 'issues'],
    queryFn: () => fetchMilestoneIssues(milestoneNumber!),
    enabled: milestoneNumber !== null,
  })
}

/**
 * Returns a function that forces a fresh fetch for a specific milestone's issue list.
 * Call after creating or closing issues to keep the cache in sync.
 *
 * Usage:
 *   const invalidate = useInvalidateMilestoneIssues()
 *   await invalidate(milestoneNumber)
 */
export function useInvalidateMilestoneIssues() {
  const queryClient = useQueryClient()
  return (milestoneNumber: number) =>
    queryClient.invalidateQueries({ queryKey: ['milestones', milestoneNumber, 'issues'] })
}

export interface BlockedIssueStatus {
  issue: Issue
  qc_status: QCStatus
}

export class ApiRequestError extends Error {
  status: number

  constructor(message: string, status: number) {
    super(message)
    this.name = 'ApiRequestError'
    this.status = status
  }
}

export async function fetchBlockedIssues(issueNumber: number): Promise<BlockedIssueStatus[]> {
  const res = await fetch(`${API_BASE}/issues/${issueNumber}/blocked`)
  if (!res.ok) {
    const data = await res.json().catch(() => null)
    throw new ApiRequestError(
      data?.error ?? `Failed to fetch blocked issues: ${res.status}`,
      res.status,
    )
  }
  return res.json()
}

export function useAllMilestoneIssues(milestoneNumbers: number[], enabled = true) {
  const queries = useQueries({
    queries: milestoneNumbers.map((n) => ({
      queryKey: ['milestones', n, 'issues'],
      queryFn: () => fetchMilestoneIssues(n),
      enabled,
    })),
  })
  return {
    issues: queries.flatMap((q) => q.data ?? []),
    isLoading: enabled && queries.some((q) => q.isPending && q.fetchStatus !== 'idle'),
  }
}

// ---------------------------------------------------------------------------
// File rename detection and confirmation
// ---------------------------------------------------------------------------

export interface DetectedRename {
  issue_number: number
  old_path: string
  new_path: string
}

export interface DetectedRenameWithMilestone extends DetectedRename {
  milestone_number: number
}

export async function fetchMilestoneRenames(milestoneNumber: number): Promise<DetectedRename[]> {
  const res = await fetch(`${API_BASE}/milestones/${milestoneNumber}/renames`)
  if (!res.ok) throw new Error(`Failed to fetch renames for milestone ${milestoneNumber}: ${res.status}`)
  return res.json()
}

export async function postRenameIssue(issueNumber: number, newPath: string): Promise<void> {
  const res = await fetch(`${API_BASE}/issues/${issueNumber}/rename`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ new_path: newPath }),
  })
  if (!res.ok) {
    const data = await res.json().catch(() => null)
    throw new Error(data?.error ?? `Failed to confirm rename: ${res.status}`)
  }
}

/** Fetch detected renames for all provided milestone numbers, merged into one list.
 *  Each entry is annotated with the milestone_number it came from. */
export function useRenames(milestoneNumbers: number[]) {
  const queries = useQueries({
    queries: milestoneNumbers.map((n) => ({
      queryKey: ['milestones', n, 'renames'],
      queryFn: () => fetchMilestoneRenames(n),
      // Rename detection is an expensive git-history walk (~1s per milestone), so it runs
      // once per milestone selection and is never refetched on remount, focus or reconnect.
      // Freshness comes from explicit invalidation: after confirming a rename, and when the
      // repo's local commit changes (see useRepoInfo).
      staleTime: Infinity,
      refetchOnMount: false,
      refetchOnWindowFocus: false,
      refetchOnReconnect: false,
    })),
  })
  return {
    renames: queries.flatMap((q, i) =>
      (q.data ?? []).map((r): DetectedRenameWithMilestone => ({
        ...r,
        milestone_number: milestoneNumbers[i],
      })),
    ),
    isLoading: queries.some((q) => q.isPending && q.fetchStatus !== 'idle'),
  }
}

/** Mutation that confirms a rename and invalidates the affected milestone queries. */
export function useConfirmRename(milestoneNumbers: number[]) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: ({ issueNumber, newPath }: { issueNumber: number; newPath: string }) =>
      postRenameIssue(issueNumber, newPath),
    onSuccess: (_, { issueNumber }) => {
      // Invalidate the issue's own status cache so its card title refreshes immediately.
      void queryClient.invalidateQueries({ queryKey: ['issue', 'status', issueNumber] })
      // Invalidate milestone issues and renames so the banner clears and issue list updates.
      for (const n of milestoneNumbers) {
        void queryClient.invalidateQueries({ queryKey: ['milestones', n, 'issues'] })
        void queryClient.invalidateQueries({ queryKey: ['milestones', n, 'renames'] })
      }
    },
  })
}

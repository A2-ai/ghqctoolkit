import { useMutation, useQueries, useQuery, useQueryClient } from '@tanstack/react-query'
import { API_BASE } from '../config'
import type { RoundRepairStatus, Segment } from './rounds'

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
  branch: string | null
  checklist_name: string | null
  relevant_files: RelevantFileInfo[]
  file_history: FileRenameEvent[]
}

export type CommitStatus = 'initial' | 'notification' | 'approved' | 'reviewed'

export interface IssueCommit {
  hash: string
  message: string
  /**
   * API-computed projection of the owning segment's events and state — not a
   * stored parse. Wire shape unchanged. Emitted in the fixed order
   * initial, notification, approved, reviewed; may be empty.
   */
  statuses: CommitStatus[]
  file_changed: boolean
}

export interface ChecklistSummary {
  completed: number
  total: number
  percentage: number
}

export interface QCStatus {
  status:
    | 'approved'
    | 'changes_after_approval'
    | 'awaiting_review'
    | 'change_requested'
    | 'in_progress'
    | 'approval_required'
    | 'changes_to_comment'
    /**
     * No status could be determined: the active segment is `unplaceable` (S4).
     * Distinct from `in_progress`, which asserts the round is open and understood —
     * this asserts nothing. Render it as an absence, not an activity, and read
     * `segments.at(-1).placement.reason` for the cause.
     */
    | 'unknown'
  status_detail: string
  /**
   * The approval currently standing: the previous round's closing commit when the
   * last segment is a Gap. null while a round is open — i.e. null exactly when the
   * issue is back under review.
   */
  standing_approval: string | null
  /** Newest closing commit across all rounds, ungated. null when nothing was ever approved. */
  last_approved_commit: string | null
  /**
   * Round 1's anchor. null when Round 1 is unplaceable — an unplaceable segment
   * owns no commits, so its anchor cannot be resolved to a sha.
   */
  initial_commit: string | null
  /**
   * Newest commit of the active segment. null whenever that segment owns no
   * commits: the steady approved state is `[…, Round(closed), Gap(empty)]` and an
   * empty trailing Gap has no newest commit; an unplaceable active segment owns
   * none either.
   */
  latest_commit: string | null
  /**
   * **The newest file-changing commit of the trailing Gap** — the commit
   * `changes_after_approval` is reported *about*. null in every other state,
   * including `approved`: a trailing Gap whose commits never touched the file is
   * approved, not changed.
   *
   * Distinct from `latest_commit`, deliberately. S1 picks the newest commit of the
   * Gap that *touched the file*, so for a Gap `[X(file_changed: false),
   * Y(file_changed: true)]` this names `Y` while `latest_commit` is `X`. Rendering
   * "changed at" from `latest_commit` would name a commit that never touched it.
   */
  changed_commit: string | null
  /**
   * The newest commit the **active round** reviewed. null when the active segment is
   * a Gap, or when that round has posted no review.
   *
   * Round-scoped deliberately: `latest_commit` now means the newest commit of the
   * active segment (≈ the branch tip), so rendering it under a *Reviewed* label
   * would report unreviewed drift as reviewed.
   */
  last_reviewed_commit: string | null
  /**
   * The newest commit the **active round** notified, on the same terms as
   * `last_reviewed_commit` — the card's *Last Posted* row.
   */
  last_notified_commit: string | null
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

export interface IssueStatusResponse {
  issue: Issue
  qc_status: QCStatus
  dirty: boolean
  /**
   * The branch of the active (last) segment — what the status was computed on.
   * Replaces the old `branch`, which was the issue body's branch. Compare it
   * against `/api/repo`'s branch at render time to decide whether to gray a card.
   */
  active_branch: string
  checklist_summary: ChecklistSummary
  /**
   * Always present: the Rust field is a plain (non-`Option`) struct with no
   * `skip_serializing_if`, so the key is emitted on every response.
   */
  blocking_qc_status: BlockingQCStatus
  /**
   * The thread as a strictly alternating segment list, oldest first. Replaces both
   * `rounds` and the top-level `commits`: every known commit is owned by exactly one
   * segment. `segments[0]` is always the Initial QC round; the last segment is an
   * open Round or a Gap. A round is open iff `segments.at(-1).kind === 'round'`.
   */
  segments: Segment[]
  /**
   * The commit a new notification would diff against — the default comparison
   * base. null when the active segment cannot supply one: an unplaceable active
   * segment owns no commits, so there is no newest event commit and no standing
   * approval to fall back to.
   */
  next_notification_from: string | null
  /**
   * Which of the open round's follow-up steps are incomplete, so a surface can
   * offer a repair without a second request. null when no round is open, or when
   * the open round is `Initial QC` (which no start-round action opened).
   */
  round_repair: RoundRepairStatus | null
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
      // Refresh on window focus so renames are detected promptly after a git operation.
      staleTime: 30 * 1000,
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

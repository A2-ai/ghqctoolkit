import { useQueries, useQuery, useQueryClient } from '@tanstack/react-query'

export type RelevantFileKind = 'blocking_qc' | 'relevant_qc' | 'file'

export interface RelevantFileInfo {
  file_name: string
  kind: RelevantFileKind
  issue_url: string | null
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
  approved_commit: string | null
  initial_commit: string
  latest_commit: string
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
  branch: string
  commits: IssueCommit[]
  checklist_summary: ChecklistSummary
  blocking_qc_status?: BlockingQCStatus
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

export async function postComment(issueNumber: number, request: CreateCommentRequest): Promise<CommentResponse> {
  const res = await fetch(`/api/issues/${issueNumber}/comment`, {
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

export type IssueStatusErrorKind = 'fetch_failed' | 'processing_failed'

export interface IssueStatusError {
  issue_number: number
  kind: IssueStatusErrorKind
  error: string
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

async function fetchMilestoneIssues(milestoneNumber: number): Promise<Issue[]> {
  const res = await fetch(`/api/milestones/${milestoneNumber}/issues`)
  if (!res.ok) throw new Error(`Failed to fetch issues for milestone ${milestoneNumber}: ${res.status}`)
  return res.json()
}

async function fetchIssueStatuses(issueNumbers: number[]): Promise<BatchIssueStatusResponse> {
  const res = await fetch(`/api/issues/status?issues=${issueNumbers.join(',')}`)
  const data = await res.json()
  if ('results' in data && 'errors' in data) return data as BatchIssueStatusResponse
  throw new Error(`Failed to fetch issue statuses: ${res.status}`)
}

export async function postReview(issueNumber: number, request: ReviewRequest): Promise<CommentResponse> {
  const res = await fetch(`/api/issues/${issueNumber}/review`, {
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
  const res = await fetch(`/api/issues/${issueNumber}/unapprove`, {
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
  const url = force ? `/api/issues/${issueNumber}/approve?force=true` : `/api/issues/${issueNumber}/approve`
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

// Module-level tick batcher. All batcher.load() calls within a single synchronous
// render pass land before setTimeout fires, so they're coalesced into one HTTP
// request. React Query's own cache means already-fetched issues never reach here.
const issueStatusBatcher = (() => {
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

export function useMilestoneIssues(milestoneNumbers: number[], includeClosedIssues: boolean) {
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
    .flatMap((q) =>
      (q.data ?? [])
        .filter((i) => i.state === 'open' || includeClosedIssues)
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
  const issues = includeClosedIssues ? deduped : deduped.filter((i) => i.state === 'open')

  const statuses = statusQueries.flatMap((q) => (q.data?.ok ? [q.data.data] : []))

  const milestoneStatusByMilestone: Record<number, MilestoneStatusInfo> = {}
  milestoneNumbers.forEach((milestoneNum, milestoneIdx) => {
    const listQuery = milestoneQueries[milestoneIdx]
    const milestoneIssues = listQuery?.data ?? []

    const listFailed = listQuery?.isError ?? false
    const listError = listFailed ? ((listQuery.error as Error)?.message ?? 'Failed to fetch issues') : null

    const relevantNums = new Set(
      milestoneIssues
        .filter((i) => i.state === 'open' || includeClosedIssues)
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

export async function fetchBlockedIssues(issueNumber: number): Promise<BlockedIssueStatus[]> {
  const res = await fetch(`/api/issues/${issueNumber}/blocked`)
  if (!res.ok) {
    const data = await res.json().catch(() => null)
    throw new Error(data?.error ?? `Failed to fetch blocked issues: ${res.status}`)
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

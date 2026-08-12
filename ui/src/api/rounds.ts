import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { API_BASE } from '../config'
import { ApiRequestError, useInvalidateBlockingDependents } from './issues'

// ---------------------------------------------------------------------------
// Types — mirror src/api/types/{responses,requests}.rs
// ---------------------------------------------------------------------------

/** Whether a round is still under review, or was closed by an approval. */
export type RoundState = 'open' | 'closed'

/** Where a round's checklist lives. */
export type ChecklistSourceKind = 'issue_body' | 'comment'

export interface RoundChecklistSource {
  kind: ChecklistSourceKind
  /** null for `issue_body`, and for cache-loaded comments. */
  comment_id: number | null
  /** null for the same reasons as `comment_id`. */
  comment_url: string | null
}

/** A single derived QC round. `index` is 1-based; 1 is Initial QC. */
export interface RoundInfo {
  index: number
  /** `"Initial QC"` or `"Round N"`. */
  name: string
  /** The commit this round opened at (its anchor). */
  opened_at: string
  /** The approval this round builds on; null for Initial QC. */
  previous_approval: string | null
  checklist_name: string | null
  checklist_source: RoundChecklistSource
  state: RoundState
  /** Set only when `state === 'closed'`: the approved commit. */
  closing_commit: string | null
  /** Set only when `state === 'closed'`: who approved. */
  closed_by: string | null
  /** Set only when `state === 'closed'`: RFC 3339 timestamp. */
  closed_at: string | null
  /** Notifications and reviews inside this round. */
  event_count: number
  /** `# QC Un-Approval` comments that took back this round's approval. */
  retraction_count: number
  /** `# QC New Round` comments that extended this round instead of opening one. */
  extension_count: number
}

/** How much of a notification comment to post when a round opens. */
export type NotificationMode = 'full' | 'metadata_only' | 'none'

export type StepStatus = 'done' | 'skipped' | 'failed'

/** Outcome of one recoverable step of a round start. */
export interface StepOutcome {
  status: StepStatus
  /** Present only when `status === 'failed'`. */
  error?: string
}

/** A downstream issue a new round may have invalidated. Display only. */
export interface ImpactedIssueItem {
  issue_number: number
  file_name: string
  milestone: string
  /** Human-readable relationship, e.g. `"previous QC"`. */
  relationship: string
}

export interface ImpactedIssues {
  /** false when the dependency API is unavailable, in which case `issues` is empty. */
  api_available: boolean
  issues: ImpactedIssueItem[]
}

/**
 * Request body for POST /api/issues/{number}/rounds.
 *
 * The anchor is deliberately absent: it is always HEAD of the issue's branch at
 * open time, read from the repository rather than accepted from the caller.
 */
export interface StartRoundRequest {
  checklist_content: string
  checklist_name?: string | null
  note?: string | null
  notification: NotificationMode
}

/**
 * 201 body of POST /api/issues/{number}/rounds.
 *
 * The round exists as soon as `round_comment_url` is set; the three step fields
 * report what else landed. A `failed` step is NOT an error: each step is
 * idempotent and independently retryable, which is what `needs_repair` flags.
 */
export interface StartRoundResponse {
  /** Derived index of the round that was opened (always >= 2). */
  round: number
  /** e.g. `"Round 2"`. */
  round_name: string
  /** URL of the `# QC New Round` comment: the round's identity. */
  round_comment_url: string
  /** The commit the round opened at (HEAD at open time). */
  anchor: string
  /** Step 2: reopening the issue. */
  reopened: StepOutcome
  /** Step 3: refreshing the `## QC Round` block in the issue body. */
  body_marker: StepOutcome
  /** Step 4: the `# QC Notification` comment. */
  notification: StepOutcome
  /** True when any step above failed and wants a retry. */
  needs_repair: boolean
  impacted_issues: ImpactedIssues
}

/**
 * Which of the open round's follow-up steps are incomplete, carried on every
 * `IssueStatusResponse` so a surface can offer a repair without a second request.
 * null there when no round is open, or when the open round is Initial QC.
 */
export interface RoundRepairStatus {
  /** Index of the open round these flags describe. */
  round: number
  /** e.g. `"Round 2"`. */
  round_name: string
  /** The issue is closed while this round is open. */
  reopen: boolean
  /** The `## QC Round` body marker is missing or disagrees with this round. */
  body_marker: boolean
  /**
   * This round carries no QC Notification. Informational, NOT a defect — opening a
   * round without notifying is a legitimate choice — so it is excluded from
   * `needs_repair` and must never on its own drive an affordance.
   */
  notification_missing: boolean
  /**
   * Whether something is actually wrong (`reopen || body_marker`). The only field
   * to branch on when deciding whether to offer a repair.
   */
  needs_repair: boolean
}

/**
 * Request body for POST /api/issues/{number}/rounds/repair.
 *
 * Nothing else is accepted: which steps are incomplete is derived server-side.
 * `notification` defaults to `'none'` — a repair never pings a reviewer unless
 * asked, because a round opened without notifying is in the state its author chose.
 */
export interface RepairRoundRequest {
  notification: NotificationMode
}

/**
 * 200 body of POST /api/issues/{number}/rounds/repair.
 *
 * Mirrors `StartRoundResponse`'s per-step shape. A `failed` step is NOT an error:
 * every step is idempotent, so `needs_repair` just says one wants another attempt.
 */
export interface RepairRoundResponse {
  /** Index of the open round that was repaired. */
  round: number
  /** e.g. `"Round 2"`. */
  round_name: string
  /** null when the round comment came from the disk cache and carries no identity. */
  round_comment_url: string | null
  /** Reopening the issue. */
  reopened: StepOutcome
  /** Refreshing the `## QC Round` block in the issue body. */
  body_marker: StepOutcome
  /** The QC Notification comment; `skipped` unless requested and missing. */
  notification: StepOutcome
  /** Whether anything was actually written. */
  repaired: boolean
  /** Whether a step that was attempted failed and still wants a retry. */
  needs_repair: boolean
}

/** Everything a "start new round" form needs. Side-effect free. */
export interface RoundSeedResponse {
  next_round: number
  /** e.g. `"Round 2"`. */
  next_round_name: string
  /** Seeded checklist markdown, boxes reset. null when no checklist was found. */
  checklist_content: string | null
  checklist_name: string | null
  /** HEAD of the issue's branch; null when it could not be resolved. */
  anchor: string | null
  /** The last round's closing commit; null while that round is still open. */
  previous_approval: string | null
  can_start: boolean
  /** Why not, when `can_start` is false. */
  blocked_reason: string | null
}

// ---------------------------------------------------------------------------
// Fetch functions
// ---------------------------------------------------------------------------

/**
 * True when the error is the backend's 409: the last round is still open, so no
 * new round could be opened. An expected precondition failure the UI renders as
 * a message rather than a generic failure.
 */
export function isRoundStillOpenError(error: unknown): error is ApiRequestError {
  return error instanceof ApiRequestError && error.status === 409
}

/**
 * True when the error is the repair endpoint's 409: there is nothing to repair,
 * because no round is open or the open round is Initial QC. The mirror image of
 * `isRoundStillOpenError` — an expected precondition, not a failure.
 */
export function isNothingToRepairError(error: unknown): error is ApiRequestError {
  return error instanceof ApiRequestError && error.status === 409
}

export async function fetchRoundSeed(issueNumber: number): Promise<RoundSeedResponse> {
  const res = await fetch(`${API_BASE}/issues/${issueNumber}/rounds/seed`)
  if (!res.ok) {
    const data = await res.json().catch(() => null)
    throw new ApiRequestError(
      data?.error ?? `Failed to fetch round seed: ${res.status}`,
      res.status,
    )
  }
  return res.json()
}

/**
 * Starts a new QC round. Resolves on 201 even when `needs_repair` is true — the
 * round exists on GitHub and the per-step outcomes say what still needs a retry.
 * Rejects with an `ApiRequestError`; `isRoundStillOpenError` identifies the 409.
 */
export async function postStartRound(
  issueNumber: number,
  request: StartRoundRequest,
): Promise<StartRoundResponse> {
  const res = await fetch(`${API_BASE}/issues/${issueNumber}/rounds`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(request),
  })
  if (!res.ok) {
    const data = await res.json().catch(() => null)
    throw new ApiRequestError(
      data?.error ?? `Failed to start round: ${res.status}`,
      res.status,
    )
  }
  return res.json()
}

/**
 * Repairs the issue's open round. Resolves on 200 even when `needs_repair` is
 * true, and even when nothing needed doing — both are successes, and the per-step
 * outcomes say which. Rejects with an `ApiRequestError`;
 * `isNothingToRepairError` identifies the 409 precondition.
 */
export async function postRepairRound(
  issueNumber: number,
  request: RepairRoundRequest,
): Promise<RepairRoundResponse> {
  const res = await fetch(`${API_BASE}/issues/${issueNumber}/rounds/repair`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(request),
  })
  if (!res.ok) {
    const data = await res.json().catch(() => null)
    throw new ApiRequestError(
      data?.error ?? `Failed to repair round: ${res.status}`,
      res.status,
    )
  }
  return res.json()
}

// ---------------------------------------------------------------------------
// Hooks
// ---------------------------------------------------------------------------

export const roundSeedQueryKey = (issueNumber: number) =>
  ['issue', 'rounds', 'seed', issueNumber] as const

/** Seed for the next round. Not cached across opens: `can_start` is a live precondition. */
export function useRoundSeed(issueNumber: number | null, enabled = true) {
  return useQuery({
    queryKey: roundSeedQueryKey(issueNumber ?? -1),
    queryFn: () => fetchRoundSeed(issueNumber!),
    enabled: enabled && issueNumber !== null,
    staleTime: 0,
    retry: false,
  })
}

/**
 * Mutation that starts a new round and refreshes the issue's status, matching
 * what the approve / unapprove flows invalidate.
 *
 * A 201 with a failed step resolves: read `needs_repair` and the per-step
 * outcomes off the returned `StartRoundResponse`.
 */
export function useStartRound(issueNumber: number) {
  const queryClient = useQueryClient()
  const invalidateBlockingDependents = useInvalidateBlockingDependents()
  return useMutation<StartRoundResponse, ApiRequestError, StartRoundRequest>({
    mutationFn: (request) => postStartRound(issueNumber, request),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ['issue', 'status', issueNumber] })
      void queryClient.invalidateQueries({ queryKey: roundSeedQueryKey(issueNumber) })
      invalidateBlockingDependents(issueNumber)
    },
  })
}

/**
 * Mutation that repairs the issue's open round, invalidating the same queries as
 * `useStartRound`: a repair changes the issue's open/closed state and its body.
 *
 * A 200 with a failed step resolves: read `needs_repair` and the per-step
 * outcomes off the returned `RepairRoundResponse`.
 */
export function useRepairRound(issueNumber: number) {
  const queryClient = useQueryClient()
  return useMutation<RepairRoundResponse, ApiRequestError, RepairRoundRequest>({
    mutationFn: (request) => postRepairRound(issueNumber, request),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ['issue', 'status', issueNumber] })
      void queryClient.invalidateQueries({ queryKey: roundSeedQueryKey(issueNumber) })
    },
  })
}

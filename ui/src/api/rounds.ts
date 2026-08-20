import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { API_BASE } from '../config'
import { ApiRequestError, useInvalidateBlockingDependents } from './issues'
import type { IssueCommit } from './issues'

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

// ---------------------------------------------------------------------------
// Segment model (design/segment-model.md §2, §7; contract:
// design/segment-api-contract.md). A thread is a strictly alternating list of
// segments: Round, Gap, Round, Gap … `segments[0]` is always the Initial QC
// round, and the last segment is either an open Round or a Gap.
// ---------------------------------------------------------------------------

/** Why a segment could not be placed on a branch. Its `commits` is then empty. */
export type UnplaceableReason =
  | 'branch_not_declared'
  | 'branch_unavailable'
  | 'anchor_unreachable'
  | 'merge_base_unreachable'
  | 'neighbour_unplaceable'

/** Whether a segment's commits could be resolved. Internally tagged on `kind`. */
export type Placement =
  | { kind: 'placed' }
  | { kind: 'unplaceable'; reason: UnplaceableReason }

/**
 * How a Gap's two bounding commits relate. Replaces the old `BranchDivergence`:
 * same fact, one type. Internally tagged on `kind`; only `diverged` carries data.
 */
export type GapContinuity =
  | { kind: 'linear' }
  | { kind: 'diverged'; merge_base: string }
  | { kind: 'unrelated' }

/** Whether a round was opened by creating the issue, or by a `# QC Round` comment. */
export type RoundOpenKind = 'issue_created' | 'new_round'

/** What opened a round. `kind === 'issue_created'` carries no comment metadata. */
export interface RoundOpenInfo {
  kind: RoundOpenKind
  /** null for `issue_created`, and for cache-loaded comments. */
  comment_id: number | null
  /** null for the same reasons as `comment_id`. */
  comment_url: string | null
  /** null for `issue_created`. */
  author: string | null
  /** RFC 3339; null for `issue_created`. */
  at: string | null
  /** The `note:` on the round comment, when written. */
  note: string | null
}

/** A notification or review posted inside a round. */
export interface RoundEventInfo {
  kind: 'notification' | 'review'
  /** The commit the event names. */
  commit: string
  by: string
  /** RFC 3339. */
  at: string
  comment_id: number | null
  comment_url: string | null
}

/** A `# QC Un-Approval` comment that took back a round's approval. */
export interface RetractionInfo {
  retracted_commit: string
  by: string
  /** RFC 3339. */
  at: string
  comment_id: number | null
  comment_url: string | null
}

/** A round comment that extended an open round instead of opening a new one. */
export interface ExtensionInfo {
  by: string
  /** RFC 3339. */
  at: string
  /** The written `round commit:`, when present and resolvable. */
  at_commit: string | null
  note: string | null
  comment_id: number | null
  comment_url: string | null
}

/**
 * One reason a round's archived bytes would not be provably the newest QC state — one
 * value per supersession clause (S3), in the order the wire emits them.
 *
 * Not mutually exclusive: `later_approval` and `round_open` co-occur routinely.
 */
export type SupersedingCause =
  | 'later_approval'
  | 'round_open'
  | 'changed_since'
  | 'undeterminable'

/** An approval claim about a previewed commit. */
export interface PreviewApproval {
  /**
   * The round that closed on this commit. **May be less than the enclosing segment's
   * `index`** (I2): a round's anchor may be the previous round's closing commit, so
   * previewing an open round can yield approved bytes under a different frame. Read as
   * *you are on round N; these bytes are round M's approval* — and never render the
   * enclosing round as approved on the strength of this field.
   */
  round: number
  commit: string
  by: string
  /** RFC 3339. */
  at: string
}

/**
 * What archiving **this** round would produce (spec §26, contract §12).
 *
 * Projected from the one derivation the archive itself runs, so the answer a user selects
 * on and the answer the metadata records cannot disagree. Render it; do not re-derive it —
 * the S1 content rule, the I2 approval tie-break and all four supersession clauses used to
 * live in `utils/archiveSelection.ts` as well, and one rule in two languages is the drift
 * this model keeps removing.
 */
export interface ArchivePreview {
  /**
   * The commit that would be archived: this round's `closing_commit` when it is closed, its
   * `latest_actioned_commit` when it is open. **Not** necessarily `commits[0]`.
   */
  commit: string
  /** null ⇒ those bytes were never approved. */
  approval: PreviewApproval | null
  /**
   * Empty ⇒ the bytes are provably the newest QC state — a **positive** claim, never
   * "unknown". Non-empty ⇒ every reason they are not, in clause order.
   *
   * There is deliberately no `superseded` bool beside it, on the wire or in this file:
   * `superseded` **is** `superseding_causes.length > 0`, and two fields for one fact is the
   * defect this model exists to remove.
   */
  superseding_causes: SupersedingCause[]
}

/** A Round segment. `index` is 1-based; 1 is Initial QC. */
export interface RoundSegment {
  kind: 'round'
  index: number
  /** `"Initial QC"` or `"Round N"`. */
  name: string
  /**
   * The commit this round opened at (its anchor). Owned by the round, not the Gap
   * before it.
   *
   * null when the round could not be placed: the fold leaves an unresolved anchor as
   * the all-zero OID, which is not a commit anyone can address, so the API nulls it
   * rather than putting a fake sha on the wire.
   */
  opened_at: string | null
  /** Always resolved — every round declares a branch (D5). */
  branch: string
  opened: RoundOpenInfo
  /** Where this round's checklist lives. Projects the model's `checklist` field. */
  checklist_source: RoundChecklistSource
  checklist_name: string | null
  state: RoundState
  /** Set only when `state === 'closed'`: the approved commit. */
  closing_commit: string | null
  /** Set only when `state === 'closed'`: who approved. */
  closed_by: string | null
  /** Set only when `state === 'closed'`: RFC 3339 timestamp. */
  closed_at: string | null
  /** Notifications and reviews inside this round, oldest first. */
  events: RoundEventInfo[]
  retractions: RetractionInfo[]
  extensions: ExtensionInfo[]
  /** Commits this round owns, newest first. Empty when `placement.kind === 'unplaceable'`. */
  commits: IssueCommit[]
  /**
   * M4: this round's newest commit carrying an action — its anchor, a notification, or a
   * review — by position in its own `commits`. Drift nobody acted on is not a candidate.
   *
   * null exactly when `placement.kind === 'unplaceable'`. Not `commits[0]` (S6) and not
   * `closing_commit`: a closed round's approval is not an event, so this can be older.
   *
   * It is the commit an **open** round would be archived at (S1 row 3). A closed round is
   * archived at its `closing_commit`; reading this field for one reads the wrong field.
   */
  latest_actioned_commit: string | null
  /**
   * What archiving this round would produce — the projected answer to S1, I2 and S3.
   *
   * **null exactly when `placement.kind === 'unplaceable'`**: the round cannot be archived,
   * which is the archive's own refusal, and the reason stays on `placement.reason` rather
   * than being duplicated here. It does *not* mean "unknown round" — the segment is right
   * here in the array.
   *
   * A round may carry a non-null `closing_commit` **and** a null preview (§20.2): it closed
   * at a real identified sha whose anchor is on no walked branch. So a closing commit is not
   * proof a round is archivable; a non-null preview is the only such proof.
   */
  archive_preview: ArchivePreview | null
  placement: Placement
}

/**
 * A Gap segment: the commits between two rounds, or after the last round.
 * Gaps are unnamed and positional — they carry no index and no stable id.
 * Empty Gaps are legal and expected.
 */
export interface GapSegment {
  kind: 'gap'
  /** The branch this gap was walked on: the bounding newer round's, else the older round's. */
  branch: string
  /** Commits this gap owns, newest first. Empty when placement is unplaceable, or genuinely empty. */
  commits: IssueCommit[]
  continuity: GapContinuity
  /** Older bound: the previous round's closing commit (exclusive). null when unknown. */
  lower_bound: string | null
  /** Newer bound: the next round's `opened_at` (exclusive), or the branch tip for a trailing gap. null when unknown. */
  upper_bound: string | null
  placement: Placement
}

/** One segment of a thread. Discriminated on `kind`. */
export type Segment = RoundSegment | GapSegment

/** How much of a notification comment to post when a round opens. */
export type NotificationMode = 'full' | 'metadata_only' | 'none'

export type StepStatus = 'done' | 'skipped' | 'failed'

/** Outcome of one recoverable step of a round start. */
export interface StepOutcome {
  status: StepStatus
  /** Present only when `status === 'failed'`. */
  error?: string
  /**
   * Why the step was skipped, when the reason is not simply that there was nothing
   * to do — today only a repair's notification, skipped because the round could not
   * be placed ("its branch is unavailable locally"). Present only when
   * `status === 'skipped'` and such a reason exists, exactly as `error` is present
   * only on `'failed'`.
   */
  skipped_reason?: string
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
  /** Why the round is being opened. Recorded on the `# QC Round` comment. */
  note?: string | null
  /**
   * Context for the reviewer, carried by the `# QC Notification` comment only.
   *
   * Separate from `note` on purpose, and with no fallback between them: the reason
   * a round exists and the message addressed to whoever must review it are
   * different things.
   */
  notification_note?: string | null
  /** Optional: the server defaults it (`#[serde(default)]`). */
  notification?: NotificationMode
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
  /** URL of the `# QC Round` comment: the round's identity. */
  round_comment_url: string
  /** The commit the round opened at (HEAD at open time). */
  anchor: string
  /** The branch the round was opened on, recorded in its round comment. */
  branch: string
  /** What the notification diff compared against. */
  comparison_base: string
  /**
   * How the previous approval relates to the anchor. null when there was no
   * divergence to report; never `{ kind: 'linear' }`.
   */
  divergence: GapContinuity | null
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
  /**
   * Optional: the server defaults it to `'none'` (`#[serde(default)]`), so `{}` is a
   * valid body — and is what every default caller sends.
   */
  notification?: NotificationMode
  /**
   * Context for the reviewer on the notification this repair may post. A
   * notification-only message lives nowhere but that comment, so when its post is
   * the step being repaired the text is gone and we send it again. Absent falls
   * back to the round's own note.
   */
  notification_note?: string | null
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
  /**
   * The QC Notification comment; `skipped` unless requested and missing — and
   * `skipped` with a `skipped_reason` when the round could not be placed, which is
   * the one step placement blocks: the reopen and the body marker still run.
   */
  notification: StepOutcome
  /** Whether anything was actually written. */
  repaired: boolean
  /** Whether a step that was attempted failed and still wants a retry. */
  needs_repair: boolean
}

/** Everything a "start new round" form needs. Side-effect free. */
/**
 * One selectable checklist source for a new round: an existing round, and the
 * checklist it was QC'd against with every box already reset.
 */
export interface RoundChecklistOption {
  /** Index of the round this checklist came from. */
  round: number
  /** That round's display name, e.g. `"Initial QC"` or `"Round 2"`. */
  round_name: string
  checklist_name: string | null
  content: string
}

export interface RoundSeedResponse {
  /** Repo-relative path of the QC'd file, for diffing without guessing at it. */
  file: string
  next_round: number
  /** e.g. `"Round 2"`. */
  next_round_name: string
  /** Seeded checklist markdown, boxes reset. null when no checklist was found. */
  checklist_content: string | null
  checklist_name: string | null
  /**
   * Every round's checklist, oldest first. Rounds whose checklist could not be
   * recovered are absent, so this can be shorter than the round list — and empty,
   * meaning there is nothing to seed from.
   */
  checklist_options: RoundChecklistOption[]
  /** `round` of the pre-selected option; null when there are none. */
  default_round: number | null
  /**
   * HEAD of the branch the round would open on — the checked-out one, not necessarily
   * the issue's. `null` when it could not be resolved.
   */
  anchor: string | null
  /** The last round's closing commit; null while that round is still open. */
  previous_approval: string | null
  /**
   * The branch the round would open on: whatever is currently checked out, which need
   * not be the branch the issue was created on.
   */
  branch: string | null
  /**
   * What the diff actually compares against — `previous_approval` normally, or the
   * merge-base when that approval is not an ancestor of `anchor`.
   */
  comparison_base: string | null
  /**
   * Set only when the previous approval is unreachable from `branch`: `diverged`
   * carries the merge-base, `unrelated` means no comparison is meaningful. null
   * when the approval is an ancestor; never `{ kind: 'linear' }`.
   */
  divergence: GapContinuity | null
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

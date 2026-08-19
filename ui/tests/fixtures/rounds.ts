// QC segment fixtures.
//
// `segmentFields` supplies the fields every IssueStatusResponse carries that describe
// the thread — `active_branch`, `segments`, `next_notification_from`, `round_repair` —
// so a fixture needs one spread rather than four hand-written literals. Segments own
// their commits (the top-level `commits` array is gone), so the builders here are also
// where a fixture's commit list lives.
//
// Invariants these builders respect, because the UI is allowed to rely on them
// (design/segment-api-contract.md §2): segments are oldest-first and strictly
// alternating Round, Gap, Round, Gap …; `segments[0]` is always the Initial QC round;
// the last segment is an open Round or a Gap, never a closed Round; each segment's
// `commits` is newest-first, and a round's include its own `opened_at`.

import type { Issue, IssueCommit, IssueStatusResponse } from '../../src/api/issues'
import type {
  GapSegment,
  RepairRoundResponse,
  RoundEventInfo,
  RoundRepairStatus,
  RoundSeedResponse,
  RoundSegment,
  Segment,
  StartRoundResponse,
} from '../../src/api/rounds'

/** Full 40-char hashes, since the UI abbreviates them. */
export const ROUND1_OPENED = 'a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1'
export const ROUND1_CLOSED = 'b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2'
/** A commit after Initial QC's approval that belongs to no round (the draft gap). */
export const DRAFT_GAP_COMMIT = 'c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3'
export const ROUND2_OPENED = 'd4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4'
/** Drift inside Round 2: newer than anything a comment named. */
export const ROUND2_DRIFT = 'e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5'

/**
 * A commit as the wire carries it. `file_changed` defaults to true.
 *
 * The default message deliberately contains no hash: several specs locate a commit by
 * its abbreviated hash, and a message echoing it makes those locators ambiguous.
 */
export function commit(hash: string, overrides: Partial<IssueCommit> = {}): IssueCommit {
  return {
    hash,
    message: 'a change',
    statuses: [],
    file_changed: true,
    ...overrides,
  }
}

/** A notification event naming `commit`. */
export function notificationEvent(
  commitHash: string,
  overrides: Partial<RoundEventInfo> = {},
): RoundEventInfo {
  return {
    kind: 'notification',
    commit: commitHash,
    by: 'test-user',
    at: '2024-01-04T00:00:00Z',
    comment_id: 6001,
    comment_url: 'https://github.com/test-owner/test-repo/issues/1#issuecomment-6001',
    ...overrides,
  }
}

/** A review event naming `commit`. */
export function reviewEvent(
  commitHash: string,
  overrides: Partial<RoundEventInfo> = {},
): RoundEventInfo {
  return { ...notificationEvent(commitHash), kind: 'review', comment_id: 6002, ...overrides }
}

export function roundSegment(
  overrides: Partial<RoundSegment> & Pick<RoundSegment, 'index' | 'name' | 'opened_at'>,
): RoundSegment {
  return {
    kind: 'round',
    // D5: a round always declares a branch. Nullability is gone deliberately — it is
    // what made the card-graying bug representable.
    branch: 'main',
    opened: {
      kind: overrides.index === 1 ? 'issue_created' : 'new_round',
      comment_id: null,
      comment_url: null,
      author: null,
      at: null,
      note: null,
    },
    checklist_name: 'Code Review',
    checklist_source: { kind: 'issue_body', comment_id: null, comment_url: null },
    state: 'open',
    closing_commit: null,
    closed_by: null,
    closed_at: null,
    events: [],
    retractions: [],
    extensions: [],
    // W2: a round owns its anchor, and D10 gives the anchor to the round rather than the
    // preceding gap — so **every** round's `opened_at` carries `initial`, not just Initial
    // QC's. The round comment names it `initial qc round commit`. This mirrors
    // `IssueCommit::project`; it previously read `index === 1 ? ['initial'] : []`, which
    // left rounds from 2 on with an unmarked start.
    commits: [commit(overrides.opened_at, { statuses: ['initial'] })],
    placement: { kind: 'placed' },
    ...overrides,
  }
}

/** Round 1 — checklist lives in the issue body. */
export function initialQcRound(
  opened_at = ROUND1_OPENED,
  overrides: Partial<RoundSegment> = {},
): RoundSegment {
  return roundSegment({ index: 1, name: 'Initial QC', opened_at, ...overrides })
}

/** Round N > 1 — checklist lives in the `# QC Round` comment. */
export function laterRound(
  index: number,
  opened_at: string,
  overrides: Partial<RoundSegment> = {},
): RoundSegment {
  return roundSegment({
    index,
    name: `Round ${index}`,
    opened_at,
    opened: {
      kind: 'new_round',
      comment_id: 5000 + index,
      comment_url: `https://github.com/test-owner/test-repo/issues/1#issuecomment-${5000 + index}`,
      author: 'test-user',
      at: '2024-01-06T00:00:00Z',
      note: null,
    },
    checklist_source: {
      kind: 'comment',
      comment_id: 5000 + index,
      comment_url: `https://github.com/test-owner/test-repo/issues/1#issuecomment-${5000 + index}`,
    },
    ...overrides,
  })
}

/**
 * Closes a round with an approval, adding the closing commit to the round's own
 * commits (newest-first) when it is not already there — W2 walks a closed round from
 * its closing commit back to its anchor, so the closing commit is always a member.
 */
export function closeRound(
  round: RoundSegment,
  closing_commit: string,
  closed_by = 'reviewer1',
  closed_at = '2024-01-05T00:00:00Z',
): RoundSegment {
  const existing = round.commits.findIndex((c) => c.hash === closing_commit)
  const commits =
    existing >= 0
      ? round.commits.map((c, i) =>
          i === existing ? { ...c, statuses: [...new Set([...c.statuses, 'approved' as const])] } : c,
        )
      : [commit(closing_commit, { statuses: ['approved'] }), ...round.commits]
  return { ...round, state: 'closed', closing_commit, closed_by, closed_at, commits }
}

/**
 * A Gap. Empty gaps are legal and expected (D6) — the steady approved state is a
 * closed round followed by an empty trailing gap.
 */
export function gapSegment(overrides: Partial<GapSegment> = {}): GapSegment {
  return {
    kind: 'gap',
    branch: 'main',
    commits: [],
    continuity: { kind: 'linear' },
    lower_bound: null,
    upper_bound: null,
    placement: { kind: 'placed' },
    ...overrides,
  }
}

type SegmentFields = Pick<
  IssueStatusResponse,
  'active_branch' | 'segments' | 'next_notification_from' | 'round_repair'
>

/**
 * A `round_repair` for the open round. Nothing is wrong by default — the flags are
 * facts about the round, and only `needs_repair` should drive an affordance.
 */
export function roundRepair(overrides: Partial<RoundRepairStatus> = {}): RoundRepairStatus {
  const base = {
    round: 2,
    round_name: 'Round 2',
    reopen: false,
    body_marker: false,
    notification_missing: false,
    ...overrides,
  }
  // Mirrors the backend: `notification_missing` is deliberately excluded.
  return { ...base, needs_repair: base.reopen || base.body_marker, ...overrides }
}

/**
 * The thread fields for a given segment list.
 *
 * `active_branch` is the last segment's branch (A2) — never the viewer's checkout.
 * `next_notification_from` defaults to M8: an open round's newest event commit, else
 * its anchor; for a trailing gap, the standing approval. `round_repair` defaults to
 * null: no round open, or nothing about the open round worth reporting.
 */
export function segmentFields(
  segments: Segment[],
  nextNotificationFrom?: string,
  repair: RoundRepairStatus | null = null,
): SegmentFields {
  const last = segments[segments.length - 1]
  const previousRound = segments[segments.length - 2]
  const standingApproval =
    last.kind === 'gap' && previousRound !== undefined && previousRound.kind === 'round'
      ? previousRound.closing_commit
      : last.kind === 'round' && last.state === 'closed'
        ? last.closing_commit
        : null
  const fromOpenRound =
    last.kind === 'round'
      ? (last.events[last.events.length - 1]?.commit ?? last.opened_at)
      : null

  return {
    active_branch: last.branch,
    segments,
    next_notification_from: nextNotificationFrom ?? fromOpenRound ?? standingApproval ?? null,
    round_repair: repair,
  }
}

/**
 * Shorthand: a single open `Initial QC` round anchored at `opened_at`.
 *
 * `latest` — when given and different — is a newer commit inside that same round, so
 * the round owns both and `qc_status.latest_commit` is that newer one.
 */
export function legacyRoundFields(
  opened_at: string,
  latest?: string,
  opts: { branch?: string } = {},
): SegmentFields {
  const branch = opts.branch ?? 'main'
  const commits =
    latest !== undefined && latest !== opened_at
      ? [commit(latest, { statuses: ['notification'], file_changed: false }), commit(opened_at, { statuses: ['initial'] })]
      : [commit(opened_at, { statuses: ['initial'] })]
  return segmentFields([initialQcRound(opened_at, { branch, commits })])
}

/**
 * Shorthand: a single `Initial QC` round closed by `approvedCommit`, followed by the
 * empty trailing gap that I3 guarantees. `qc_status.latest_commit` is null for such an
 * issue — the normal fully-approved state, not an edge case.
 */
export function approvedRoundFields(
  opened_at: string,
  approvedCommit: string,
  opts: { branch?: string } = {},
): SegmentFields {
  const branch = opts.branch ?? 'main'
  return segmentFields([
    closeRound(initialQcRound(opened_at, { branch }), approvedCommit),
    gapSegment({ branch, lower_bound: approvedCommit, upper_bound: approvedCommit }),
  ])
}

// ── Scenario fixtures ────────────────────────────────────────────────────────
// Issues 110-113 are reserved for round scenarios.

function makeRoundIssue(number: number, title: string, state: Issue['state'] = 'open'): Issue {
  return {
    number,
    title,
    state,
    html_url: `https://github.com/test-owner/test-repo/issues/${number}`,
    assignees: ['reviewer1'],
    labels: ['ghqc', 'main'],
    milestone: 'Sprint 1',
    created_at: '2024-01-01T00:00:00Z',
    updated_at: '2024-01-06T00:00:00Z',
    closed_at: state === 'closed' ? '2024-01-05T00:00:00Z' : null,
    created_by: 'test-user',
    branch: 'main',
    checklist_name: 'Code Review',
    relevant_files: [],
    file_history: [],
  }
}

const emptyBlockingQCStatus = {
  total: 0, approved_count: 0, summary: '-',
  approved: [], not_approved: [], errors: [],
}

/** Legacy issue: exactly one round, `Initial QC`, still open. */
export const legacyRoundIssue = makeRoundIssue(110, 'src/legacy-round.rs')
export const legacyRoundStatus: IssueStatusResponse = {
  issue: legacyRoundIssue,
  qc_status: {
    status: 'awaiting_review',
    status_detail: 'Awaiting first review',
    standing_approval: null,
    last_approved_commit: null,
    initial_commit: ROUND1_OPENED,
    latest_commit: ROUND1_OPENED,
    // Non-null only for `changes_after_approval` (S1: the trailing gap's newest
    // *file-changing* commit), which is not the same field as `latest_commit`.
    changed_commit: null,
    // Nothing has been notified or reviewed in this round yet.
    last_reviewed_commit: null,
    last_notified_commit: null,
  },
  dirty: false,
  checklist_summary: { completed: 1, total: 3, percentage: 33.3 },
  blocking_qc_status: emptyBlockingQCStatus,
  ...legacyRoundFields(ROUND1_OPENED),
}

/** Approved single-round issue: `Initial QC` closed, so a new round may start. */
export const approvedRoundIssue = makeRoundIssue(111, 'src/approved-round.rs', 'closed')
export const approvedRoundStatus: IssueStatusResponse = {
  issue: approvedRoundIssue,
  qc_status: {
    status: 'approved',
    status_detail: 'Approved',
    standing_approval: ROUND1_CLOSED,
    last_approved_commit: ROUND1_CLOSED,
    initial_commit: ROUND1_OPENED,
    // The trailing gap is empty, so the active segment owns no newest commit.
    latest_commit: null,
    // Non-null only for `changes_after_approval` (S1: the trailing gap's newest
    // *file-changing* commit), which is not the same field as `latest_commit`.
    changed_commit: null,
    // Both are round-scoped to the *active* segment, which is a gap here.
    last_reviewed_commit: null,
    last_notified_commit: null,
  },
  dirty: false,
  checklist_summary: { completed: 3, total: 3, percentage: 100 },
  blocking_qc_status: emptyBlockingQCStatus,
  ...approvedRoundFields(ROUND1_OPENED, ROUND1_CLOSED),
}

/**
 * Multi-round issue: `Initial QC` closed at ROUND1_CLOSED, `Round 2` open at
 * ROUND2_OPENED, with DRAFT_GAP_COMMIT in the gap between them belonging to no round.
 */
export const multiRoundIssue = makeRoundIssue(112, 'src/multi-round.rs')

/** The three segments #112's scenario is built from, in one place. */
export function multiRoundSegments(overrides: { round2?: Partial<RoundSegment> } = {}): Segment[] {
  return [
    closeRound(
      initialQcRound(ROUND1_OPENED, {
        commits: [
          commit(ROUND1_CLOSED, { message: 'address review' }),
          commit(ROUND1_OPENED, { message: 'initial commit', statuses: ['initial'] }),
        ],
      }),
      ROUND1_CLOSED,
    ),
    gapSegment({
      commits: [commit(DRAFT_GAP_COMMIT, { message: 'draft work, no round' })],
      lower_bound: ROUND1_CLOSED,
      upper_bound: ROUND2_OPENED,
    }),
    laterRound(2, ROUND2_OPENED, {
      commits: [commit(ROUND2_OPENED, { message: 'round 2 changes', statuses: ['initial', 'notification'] })],
      events: [notificationEvent(ROUND2_OPENED)],
      ...overrides.round2,
    }),
  ]
}

export const multiRoundStatus: IssueStatusResponse = {
  issue: multiRoundIssue,
  qc_status: {
    status: 'awaiting_review',
    status_detail: 'Awaiting review of Round 2',
    // A round is open, so nothing stands: null exactly while the file is back under
    // review. `last_approved_commit` is ungated and still reports round 1's approval.
    standing_approval: null,
    last_approved_commit: ROUND1_CLOSED,
    initial_commit: ROUND1_OPENED,
    latest_commit: ROUND2_OPENED,
    // Non-null only for `changes_after_approval` (S1: the trailing gap's newest
    // *file-changing* commit), which is not the same field as `latest_commit`.
    changed_commit: null,
    last_reviewed_commit: null,
    // Round 2's notification named its own anchor.
    last_notified_commit: ROUND2_OPENED,
  },
  dirty: false,
  checklist_summary: { completed: 1, total: 4, percentage: 25 },
  blocking_qc_status: emptyBlockingQCStatus,
  ...segmentFields(multiRoundSegments()),
}

/**
 * Multi-round issue whose Round 2 never finished landing: the issue is still
 * closed and the body marker is stale, so `round_repair.needs_repair` is true.
 * The round itself is real — only its follow-up steps are incomplete.
 */
export const brokenRoundIssue = makeRoundIssue(113, 'src/broken-round.rs', 'closed')
export const brokenRoundStatus: IssueStatusResponse = {
  ...multiRoundStatus,
  issue: brokenRoundIssue,
  ...segmentFields(
    multiRoundSegments({ round2: { events: [] } }),
    undefined,
    roundRepair({ reopen: true, body_marker: true, notification_missing: true }),
  ),
}

/**
 * The same issue with an open Round 2 that is perfectly fine except that nobody
 * was notified — deliberately not a defect, so no repair must be offered.
 */
export const quietRoundStatus: IssueStatusResponse = {
  ...multiRoundStatus,
  ...segmentFields(
    multiRoundSegments({ round2: { events: [] } }),
    undefined,
    roundRepair({ notification_missing: true }),
  ),
}

/**
 * Cross-branch scenario: Round 2 was QC'd on `feature/reanalysis`, which does not
 * contain Initial QC's approval, so the gap between them is `diverged`.
 *
 * This is the shape the card-graying bug lived in: `active_branch` is the round the
 * user is working in, while `issue.branch` still says `main`.
 */
export const crossBranchIssue = makeRoundIssue(114, 'src/cross-branch.rs')
export const crossBranchSegments: Segment[] = [
  closeRound(
    initialQcRound(ROUND1_OPENED, {
      commits: [
        commit(ROUND1_CLOSED, { message: 'address review' }),
        commit(ROUND1_OPENED, { message: 'initial commit', statuses: ['initial'] }),
      ],
    }),
    ROUND1_CLOSED,
  ),
  gapSegment({
    branch: 'feature/reanalysis',
    commits: [commit(DRAFT_GAP_COMMIT, { message: 'reanalysis groundwork' })],
    continuity: { kind: 'diverged', merge_base: ROUND1_OPENED },
    lower_bound: ROUND1_CLOSED,
    upper_bound: ROUND2_OPENED,
  }),
  laterRound(2, ROUND2_OPENED, {
    branch: 'feature/reanalysis',
    commits: [commit(ROUND2_OPENED, { message: 'round 2 changes', statuses: ['initial', 'notification'] })],
    events: [notificationEvent(ROUND2_OPENED)],
  }),
]
export const crossBranchStatus: IssueStatusResponse = {
  ...multiRoundStatus,
  issue: crossBranchIssue,
  ...segmentFields(crossBranchSegments),
}

/**
 * The same two rounds, but on branches that share no history at all — the gap between
 * them is `unrelated`, so no diff across it is meaningful and there is no merge base to
 * name. The rail draws this severed rather than dashed; reach across it is still offered
 * and the receipt is what declines to claim a diff.
 */
export const unrelatedHistoryStatus: IssueStatusResponse = {
  ...crossBranchStatus,
  ...segmentFields([
    crossBranchSegments[0],
    { ...(crossBranchSegments[1] as GapSegment), continuity: { kind: 'unrelated' } },
    crossBranchSegments[2],
  ]),
}

/**
 * A segment that could not be placed: Round 2's branch is unavailable locally, so
 * it owns no commits and the gap before it is unplaceable by neighbour (W6).
 */
export const unplaceableIssue = makeRoundIssue(115, 'src/unplaceable.rs')
export const unplaceableStatus: IssueStatusResponse = {
  ...multiRoundStatus,
  issue: unplaceableIssue,
  ...segmentFields([
    closeRound(
      initialQcRound(ROUND1_OPENED, {
        commits: [
          commit(ROUND1_CLOSED, { message: 'address review' }),
          commit(ROUND1_OPENED, { message: 'initial commit', statuses: ['initial'] }),
        ],
      }),
      ROUND1_CLOSED,
    ),
    gapSegment({
      branch: 'feature/gone',
      placement: { kind: 'unplaceable', reason: 'neighbour_unplaceable' },
    }),
    laterRound(2, ROUND2_OPENED, {
      branch: 'feature/gone',
      commits: [],
      placement: { kind: 'unplaceable', reason: 'branch_unavailable' },
    }),
  ]),
  // Stated explicitly rather than inherited from `multiRoundStatus`. The spread would
  // carry `awaiting_review` plus real shas for `latest_commit` / `next_notification_from`,
  // none of which the backend can emit alongside an unplaceable active segment: S4 gives
  // no status, and a segment that owns no commits supplies no newest commit and no
  // notification base. These three must come last so they win over the spread.
  qc_status: {
    ...multiRoundStatus.qc_status,
    status: 'unknown',
    status_detail: 'Unknown',
    latest_commit: null,
    initial_commit: null,
    changed_commit: null,
  },
  next_notification_from: null,
}

/**
 * D15 clause 2, the reachable grayed state.
 *
 * Initial QC is closed, but its anchor is no longer reachable (gc'd, force-pushed), so
 * the round is `unplaceable` and owns no commits; by W6 the trailing gap it bounds is
 * `neighbour_unplaceable`.
 *
 * The status is **`unknown`, not `approved`** — an earlier version of this fixture said
 * `approved`, which D15's second addendum showed is not producible: S4 short-circuits on
 * an unplaceable gap before S1's arm can return `Approved`. Status is still purely a
 * function of the record (D8); the card grays because the record cannot be taken at face
 * value (D4), and `unknown` is the honest thing to say about it.
 *
 * `active_branch` deliberately matches `defaultRepoInfo.branch`, so the only thing that
 * can gray this card is the placement.
 */
export const vanishedApprovalIssue = makeRoundIssue(116, 'src/vanished-approval.rs', 'closed')
export const vanishedApprovalStatus: IssueStatusResponse = {
  issue: vanishedApprovalIssue,
  qc_status: {
    // NOT `approved`. Per D15's second addendum, `approved` + an unplaceable active
    // segment is not producible: S4 matches `Gap(gap) if gap.is_placed()` and then
    // `Gap(_) => None`, so an unplaceable trailing gap yields no status at all —
    // pinned in Rust by "an unplaceable trailing gap must not report as approved".
    // The reachable shape is `unknown`, which is what the card must gray on.
    status: 'unknown',
    status_detail: 'Unknown',
    standing_approval: ROUND1_CLOSED,
    last_approved_commit: ROUND1_CLOSED,
    // An unplaceable round owns no commits, so its anchor resolves to no sha.
    initial_commit: null,
    latest_commit: null,
    // Non-null only for `changes_after_approval` (S1: the trailing gap's newest
    // *file-changing* commit), which is not the same field as `latest_commit`.
    changed_commit: null,
    last_reviewed_commit: null,
    last_notified_commit: null,
  },
  dirty: false,
  checklist_summary: { completed: 3, total: 3, percentage: 100 },
  blocking_qc_status: emptyBlockingQCStatus,
  ...segmentFields([
    {
      ...initialQcRound(ROUND1_OPENED, { commits: [] }),
      state: 'closed',
      closing_commit: ROUND1_CLOSED,
      closed_by: 'reviewer1',
      closed_at: '2024-01-05T00:00:00Z',
      placement: { kind: 'unplaceable', reason: 'anchor_unreachable' },
    },
    gapSegment({ placement: { kind: 'unplaceable', reason: 'neighbour_unplaceable' } }),
  ]),
}

/**
 * D12: Round 2 was notified and reviewed at its anchor, and then the file moved again.
 *
 * The drift commit is the newest commit of the active segment, so it *is*
 * `latest_commit` — and labelling it *Reviewed* or *Last Posted* would be false. These
 * two fixtures exist so the card's rows can be pinned to the fields that actually mean
 * what the labels say.
 */
function reviewedThenDriftedSegments(
  opts: { driftChangesFile?: boolean; reviewed?: boolean } = {},
): Segment[] {
  const { driftChangesFile = true, reviewed = true } = opts
  return [
    closeRound(
      initialQcRound(ROUND1_OPENED, {
        commits: [
          commit(ROUND1_CLOSED, { message: 'address review' }),
          commit(ROUND1_OPENED, { message: 'initial commit', statuses: ['initial'] }),
        ],
      }),
      ROUND1_CLOSED,
    ),
    gapSegment({
      commits: [commit(DRAFT_GAP_COMMIT, { message: 'draft work, no round' })],
      lower_bound: ROUND1_CLOSED,
      upper_bound: ROUND2_OPENED,
    }),
    laterRound(2, ROUND2_OPENED, {
      commits: [
        commit(ROUND2_DRIFT, { message: 'unreviewed drift', file_changed: driftChangesFile }),
        commit(ROUND2_OPENED, {
          message: 'round 2 changes',
          statuses: reviewed ? ['notification', 'reviewed'] : ['notification'],
        }),
      ],
      events: reviewed
        ? [notificationEvent(ROUND2_OPENED), reviewEvent(ROUND2_OPENED)]
        : [notificationEvent(ROUND2_OPENED)],
    }),
  ]
}

export const driftedRoundIssue = makeRoundIssue(117, 'src/drifted-round.rs')

/**
 * Findings raised at Round 2's anchor, then the file moved — but the drift commit does
 * not touch the QC'd file.
 *
 * `file_changed: false` is what makes `change_requested` a state the backend can
 * actually reach. `Round::status()` compares the newest *file-changing* commit with the
 * newest covering event: with the drift changing the file, that commit (position 0) is
 * newer than the event naming the anchor (position 1), so the real status would be
 * `ChangesToComment(ROUND2_DRIFT)` and this fixture would pin a state no fold emits.
 * With the drift not touching the file, the newest file change *is* the anchor, the
 * review covers it, and `change_requested` follows. `latest_commit` is unaffected — it
 * is the newest commit of the active segment, file-changing or not, which is the whole
 * point of the D12 rows below.
 */
export const changeRequestedDriftStatus: IssueStatusResponse = {
  issue: driftedRoundIssue,
  qc_status: {
    status: 'change_requested',
    status_detail: 'Changes requested',
    standing_approval: null,
    last_approved_commit: ROUND1_CLOSED,
    initial_commit: ROUND1_OPENED,
    // The branch tip, which no comment ever named.
    latest_commit: ROUND2_DRIFT,
    // Non-null only for `changes_after_approval` (S1: the trailing gap's newest
    // *file-changing* commit), which is not the same field as `latest_commit`.
    changed_commit: null,
    last_reviewed_commit: ROUND2_OPENED,
    last_notified_commit: ROUND2_OPENED,
  },
  dirty: false,
  checklist_summary: { completed: 1, total: 4, percentage: 25 },
  blocking_qc_status: emptyBlockingQCStatus,
  ...segmentFields(reviewedThenDriftedSegments({ driftChangesFile: false }), ROUND2_OPENED),
}

/**
 * The same history, read as "there are changes nobody has been told about yet".
 *
 * Two things differ from the variant above, and both are forced by the status:
 * `changes_to_comment` needs the newest file-changing commit to be *uncovered*, so the
 * drift touches the file here; and `last_reviewed_commit: null` needs there to have
 * been no review, so the review event is dropped rather than left in the round where
 * the API's projection would emit `ROUND2_OPENED` from it.
 */
export const changesToCommentDriftStatus: IssueStatusResponse = {
  ...changeRequestedDriftStatus,
  qc_status: {
    ...changeRequestedDriftStatus.qc_status,
    status: 'changes_to_comment',
    status_detail: 'New changes since the last notification',
    last_reviewed_commit: null,
  },
  ...segmentFields(reviewedThenDriftedSegments({ reviewed: false }), ROUND2_OPENED),
}

// ── Round seed / start responses ─────────────────────────────────────────────

/** Seed for #111: the last round is closed, so a round may be started. */
export const roundSeedCanStart: RoundSeedResponse = {
  file: 'src/approved-round.rs',
  next_round: 2,
  next_round_name: 'Round 2',
  checklist_content: '- [ ] Review logic\n- [ ] Check tests',
  checklist_name: 'Code Review',
  // Starting round 2, so Initial QC is the only possible base: one option, and the
  // picker stays hidden because there is no choice to make.
  checklist_options: [
    {
      round: 1,
      round_name: 'Initial QC',
      checklist_name: 'Code Review',
      content: '- [ ] Review logic\n- [ ] Check tests',
    },
  ],
  default_round: 1,
  anchor: ROUND2_OPENED,
  previous_approval: ROUND1_CLOSED,
  branch: 'main',
  comparison_base: ROUND1_CLOSED,
  divergence: null,
  can_start: true,
  blocked_reason: null,
}

/** The trimmed checklist round 2 was QC'd against, carrying its own `## ` heading. */
export const ROUND2_CHECKLIST = '- [ ] Spot-check the refactor\n\n## Technical Review\n\n- [ ] Renders clean'

/**
 * Seed for starting round 3, where both Initial QC's checklist and round 2's are
 * selectable — the case the picker exists for.
 */
export const roundSeedMultipleChecklists: RoundSeedResponse = {
  file: 'src/approved-round.rs',
  next_round: 3,
  next_round_name: 'Round 3',
  checklist_content: ROUND2_CHECKLIST,
  checklist_name: 'Focused Re-review',
  checklist_options: [
    {
      round: 1,
      round_name: 'Initial QC',
      checklist_name: 'Code Review',
      content: '- [ ] Review logic\n- [ ] Check tests',
    },
    {
      round: 2,
      round_name: 'Round 2',
      checklist_name: 'Focused Re-review',
      content: ROUND2_CHECKLIST,
    },
  ],
  default_round: 2,
  anchor: ROUND2_OPENED,
  previous_approval: ROUND1_CLOSED,
  branch: 'main',
  comparison_base: ROUND1_CLOSED,
  divergence: null,
  can_start: true,
  blocked_reason: null,
}

/** Seed for an issue whose last round is still open. */
export const roundSeedBlocked: RoundSeedResponse = {
  file: 'src/approved-round.rs',
  next_round: 2,
  next_round_name: 'Round 2',
  checklist_content: '- [ ] Review logic\n- [ ] Check tests',
  checklist_name: 'Code Review',
  checklist_options: roundSeedCanStart.checklist_options,
  default_round: 1,
  anchor: ROUND2_OPENED,
  previous_approval: null,
  branch: 'main',
  comparison_base: ROUND1_CLOSED,
  divergence: null,
  can_start: false,
  blocked_reason:
    'Initial QC is still open. Approve it first — a `# QC Round` comment posted now would extend that round instead of opening a new one.',
}

/**
 * Seed for a round opening on a branch that does not contain the previous approval.
 *
 * The legitimate cross-branch case: analysis moved to `feature/reanalysis` and the old
 * branch was never merged in, so the diff falls back to the commit the two share.
 *
 * `previous_branch` is gone from the wire (M4) and needs no replacement: the modal
 * joins it in from the last closed Round segment of the issue status it already holds.
 */
export const roundSeedDivergentBranch: RoundSeedResponse = {
  ...roundSeedCanStart,
  branch: 'feature/reanalysis',
  comparison_base: ROUND1_OPENED,
  divergence: { kind: 'diverged', merge_base: ROUND1_OPENED },
}

/** The same, but the two branches share no history at all. */
export const roundSeedUnrelatedHistory: RoundSeedResponse = {
  ...roundSeedCanStart,
  branch: 'orphan',
  comparison_base: ROUND1_CLOSED,
  divergence: { kind: 'unrelated' },
}

/** Seed blocked because HEAD of the branch could not be resolved locally. */
export const roundSeedAnchorUnresolved: RoundSeedResponse = {
  ...roundSeedCanStart,
  anchor: null,
  can_start: false,
  blocked_reason: "Could not resolve HEAD of branch 'main' — fetch or check out the branch locally.",
}

/** Every step landed. */
export const startRoundSuccess: StartRoundResponse = {
  round: 2,
  round_name: 'Round 2',
  round_comment_url: 'https://github.com/test-owner/test-repo/issues/111#issuecomment-5002',
  anchor: ROUND2_OPENED,
  branch: 'main',
  comparison_base: ROUND1_CLOSED,
  divergence: null,
  reopened: { status: 'done' },
  body_marker: { status: 'done' },
  notification: { status: 'done' },
  needs_repair: false,
  impacted_issues: { api_available: true, issues: [] },
}

/** 201, but step 2 failed: the round exists and `needs_repair` asks for a retry. */
export const startRoundNeedsRepair: StartRoundResponse = {
  ...startRoundSuccess,
  reopened: { status: 'failed', error: 'Could not reopen the issue: 403 Forbidden' },
  notification: { status: 'skipped' },
  needs_repair: true,
}

/** 201 with downstream issues the new round may have invalidated. */
export const startRoundWithImpactedIssues: StartRoundResponse = {
  ...startRoundSuccess,
  impacted_issues: {
    api_available: true,
    issues: [
      { issue_number: 91, file_name: 'src/file_a.rs', milestone: 'Sprint 1', relationship: 'previous QC' },
    ],
  },
}

/** The 409 message the backend returns when the last round is still open. */
export const roundStillOpenError = roundSeedBlocked.blocked_reason!

// ── Repair responses ─────────────────────────────────────────────────────────

/** Every incomplete step landed on the retry. */
export const repairRoundSuccess: RepairRoundResponse = {
  round: 2,
  round_name: 'Round 2',
  round_comment_url: 'https://github.com/test-owner/test-repo/issues/111#issuecomment-5002',
  reopened: { status: 'done' },
  body_marker: { status: 'done' },
  notification: { status: 'skipped' },
  repaired: true,
  needs_repair: false,
}

/** 200, but the reopen failed again: reported per step, never as an error. */
export const repairRoundStillFailing: RepairRoundResponse = {
  ...repairRoundSuccess,
  reopened: { status: 'failed', error: 'Could not reopen the issue: 403 Forbidden' },
  repaired: true,
  needs_repair: true,
}

/** 200 where the round turned out to be complete already. */
export const repairRoundNothingDone: RepairRoundResponse = {
  ...repairRoundSuccess,
  reopened: { status: 'skipped' },
  body_marker: { status: 'skipped' },
  repaired: false,
  needs_repair: false,
}

/**
 * 200 on a round that could not be placed: the two steps that do not read placement
 * still run, and the notification is skipped **with a reason**.
 *
 * A repair no longer refuses a grayed round with a 409 — `needs_repair` is
 * `reopen || body_marker`, neither of which reads placement, so refusing would have
 * offered a repair the endpoint then rejected for the ordinary "branch not fetched
 * locally" state. `skipped_reason` carries `UnplaceableReason::describe()` verbatim,
 * the same string `ghqc issue status` prints.
 */
export const repairRoundNotificationSkipped: RepairRoundResponse = {
  ...repairRoundSuccess,
  notification: {
    status: 'skipped',
    skipped_reason: 'its branch is unavailable locally',
  },
}

/** A body marker skipped for its own reason: the round comment URL is unknown. */
export const repairRoundMarkerSkipped: RepairRoundResponse = {
  ...repairRoundSuccess,
  round_comment_url: null,
  body_marker: {
    status: 'skipped',
    skipped_reason: 'round comment URL unknown, marker left as it is',
  },
}

/** The 409 message the backend returns when there is nothing to repair. */
export const nothingToRepairError =
  'Nothing to repair: Round 2 is closed, so no round is open on this issue. A repair only ever completes the follow-up steps of a round that is currently open.'

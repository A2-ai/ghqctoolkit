// QC round fixtures.
//
// `roundFields` supplies the three additive fields every IssueStatusResponse now
// carries (`rounds`, `open_round_index`, `next_notification_from`) so existing
// fixtures only need one spread rather than three hand-written literals.

import type { Issue, IssueStatusResponse } from '../../src/api/issues'
import type {
  RepairRoundResponse,
  RoundInfo,
  RoundRepairStatus,
  RoundSeedResponse,
  StartRoundResponse,
} from '../../src/api/rounds'

/** Full 40-char hashes, since the UI abbreviates them. */
export const ROUND1_OPENED = 'a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1'
export const ROUND1_CLOSED = 'b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2'
/** A commit after Initial QC's approval that belongs to no round (the draft gap). */
export const DRAFT_GAP_COMMIT = 'c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3'
export const ROUND2_OPENED = 'd4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4'

export function makeRound(
  overrides: Partial<RoundInfo> & Pick<RoundInfo, 'index' | 'name' | 'opened_at'>,
): RoundInfo {
  return {
    previous_approval: null,
    checklist_name: 'Code Review',
    checklist_source: { kind: 'issue_body', comment_id: null, comment_url: null },
    state: 'open',
    closing_commit: null,
    closed_by: null,
    closed_at: null,
    event_count: 0,
    retraction_count: 0,
    extension_count: 0,
    ...overrides,
  }
}

/** Round 1 — checklist lives in the issue body. */
export function initialQcRound(
  opened_at = ROUND1_OPENED,
  overrides: Partial<RoundInfo> = {},
): RoundInfo {
  return makeRound({ index: 1, name: 'Initial QC', opened_at, ...overrides })
}

/** Round N > 1 — checklist lives in the `# QC New Round` comment. */
export function laterRound(
  index: number,
  opened_at: string,
  previous_approval: string,
  overrides: Partial<RoundInfo> = {},
): RoundInfo {
  return makeRound({
    index,
    name: `Round ${index}`,
    opened_at,
    previous_approval,
    checklist_source: {
      kind: 'comment',
      comment_id: 5000 + index,
      comment_url: `https://github.com/test-owner/test-repo/issues/1#issuecomment-${5000 + index}`,
    },
    ...overrides,
  })
}

/** Closes a round with an approval. */
export function closeRound(
  round: RoundInfo,
  closing_commit: string,
  closed_by = 'reviewer1',
  closed_at = '2024-01-05T00:00:00Z',
): RoundInfo {
  return { ...round, state: 'closed', closing_commit, closed_by, closed_at }
}

type RoundFields = Pick<
  IssueStatusResponse,
  'rounds' | 'open_round_index' | 'next_notification_from' | 'round_repair'
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
 * The round fields for a given round list. `open_round_index` is derived, and
 * `next_notification_from` defaults to the open round's anchor (or the last round's
 * closing commit when everything is closed). `round_repair` defaults to null: no
 * round open, or nothing about the open round worth reporting.
 */
export function roundFields(
  rounds: RoundInfo[],
  nextNotificationFrom?: string,
  repair: RoundRepairStatus | null = null,
): RoundFields {
  const open = rounds.find((r) => r.state === 'open')
  const last = rounds[rounds.length - 1]
  return {
    rounds,
    open_round_index: open?.index ?? null,
    next_notification_from:
      nextNotificationFrom ?? open?.opened_at ?? last?.closing_commit ?? last?.opened_at ?? '',
    round_repair: repair,
  }
}

/** Shorthand: a legacy single-round, still-open issue anchored at `opened_at`. */
export function legacyRoundFields(opened_at: string, nextNotificationFrom?: string): RoundFields {
  return roundFields([initialQcRound(opened_at)], nextNotificationFrom)
}

/** Shorthand: a single Initial QC round closed by `approvedCommit`. */
export function approvedRoundFields(opened_at: string, approvedCommit: string): RoundFields {
  return roundFields([closeRound(initialQcRound(opened_at), approvedCommit)])
}

// ── Scenario fixtures ────────────────────────────────────────────────────────
// Issues 110-112 are reserved for round scenarios.

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
    approved_commit: null,
    initial_commit: ROUND1_OPENED,
    latest_commit: ROUND1_OPENED,
  },
  dirty: false,
  branch: 'main',
  commits: [
    { hash: ROUND1_OPENED, message: 'initial commit', statuses: ['initial'], file_changed: true },
  ],
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
    approved_commit: ROUND1_CLOSED,
    initial_commit: ROUND1_OPENED,
    latest_commit: ROUND1_CLOSED,
  },
  dirty: false,
  branch: 'main',
  commits: [
    { hash: ROUND1_CLOSED, message: 'address review', statuses: ['approved'], file_changed: true },
    { hash: ROUND1_OPENED, message: 'initial commit', statuses: ['initial'], file_changed: true },
  ],
  checklist_summary: { completed: 3, total: 3, percentage: 100 },
  blocking_qc_status: emptyBlockingQCStatus,
  ...approvedRoundFields(ROUND1_OPENED, ROUND1_CLOSED),
}

/**
 * Multi-round issue: `Initial QC` closed at ROUND1_CLOSED, `Round 2` open at
 * ROUND2_OPENED, with DRAFT_GAP_COMMIT in between belonging to no round.
 */
export const multiRoundIssue = makeRoundIssue(112, 'src/multi-round.rs')
export const multiRoundStatus: IssueStatusResponse = {
  issue: multiRoundIssue,
  qc_status: {
    status: 'awaiting_review',
    status_detail: 'Awaiting review of Round 2',
    approved_commit: ROUND1_CLOSED,
    initial_commit: ROUND1_OPENED,
    latest_commit: ROUND2_OPENED,
  },
  dirty: false,
  branch: 'main',
  commits: [
    { hash: ROUND2_OPENED, message: 'round 2 changes', statuses: ['notification'], file_changed: true },
    { hash: DRAFT_GAP_COMMIT, message: 'draft work, no round', statuses: [], file_changed: true },
    { hash: ROUND1_CLOSED, message: 'address review', statuses: ['approved'], file_changed: true },
    { hash: ROUND1_OPENED, message: 'initial commit', statuses: ['initial'], file_changed: true },
  ],
  checklist_summary: { completed: 1, total: 4, percentage: 25 },
  blocking_qc_status: emptyBlockingQCStatus,
  ...roundFields([
    closeRound(initialQcRound(ROUND1_OPENED), ROUND1_CLOSED),
    laterRound(2, ROUND2_OPENED, ROUND1_CLOSED, { event_count: 1 }),
  ]),
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
  ...roundFields(
    [
      closeRound(initialQcRound(ROUND1_OPENED), ROUND1_CLOSED),
      laterRound(2, ROUND2_OPENED, ROUND1_CLOSED),
    ],
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
  ...roundFields(
    [
      closeRound(initialQcRound(ROUND1_OPENED), ROUND1_CLOSED),
      laterRound(2, ROUND2_OPENED, ROUND1_CLOSED),
    ],
    undefined,
    roundRepair({ notification_missing: true }),
  ),
}

// ── Round seed / start responses ─────────────────────────────────────────────

/** Seed for #111: the last round is closed, so a round may be started. */
export const roundSeedCanStart: RoundSeedResponse = {
  next_round: 2,
  next_round_name: 'Round 2',
  checklist_content: '- [ ] Review logic\n- [ ] Check tests',
  checklist_name: 'Code Review',
  anchor: ROUND2_OPENED,
  previous_approval: ROUND1_CLOSED,
  can_start: true,
  blocked_reason: null,
}

/** Seed for an issue whose last round is still open. */
export const roundSeedBlocked: RoundSeedResponse = {
  next_round: 2,
  next_round_name: 'Round 2',
  checklist_content: '- [ ] Review logic\n- [ ] Check tests',
  checklist_name: 'Code Review',
  anchor: ROUND2_OPENED,
  previous_approval: null,
  can_start: false,
  blocked_reason:
    'Initial QC is still open. Approve it first — a `# QC New Round` comment posted now would extend that round instead of opening a new one.',
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

/** The 409 message the backend returns when there is nothing to repair. */
export const nothingToRepairError =
  'Nothing to repair: Round 2 is closed, so no round is open on this issue. A repair only ever completes the follow-up steps of a round that is currently open.'

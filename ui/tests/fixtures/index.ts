import type { RepoInfo } from '../../src/api/repo'
import type { Milestone } from '../../src/api/milestones'
import type {
  SegmentRef,
  Gap,
  Issue,
  IssueCommit,
  IssueStatusResponse,
  BatchIssueStatusResponse,
  BlockedIssueStatus,
  QCStatus,
  RoundInfo,
} from '../../src/api/issues'
import type { Assignee } from '../../src/api/assignees'
import type { Checklist } from '../../src/api/checklists'
import type { FileTreeResponse } from '../../src/api/files'
import type { CreateIssueResponse } from '../../src/api/create'
import type { ConfigGitRepository } from '../../src/api/configuration'

export const defaultRepoInfo: RepoInfo = {
  owner: 'test-owner',
  repo: 'test-repo',
  remote: 'origin',
  branch: 'main',
  local_commit: 'abc1234',
  remote_commit: 'abc1234',
  git_status: 'clean',
  git_status_detail: 'Up to date',
  dirty_files: [],
  current_user: 'test-user',
}

export const openMilestone: Milestone = {
  number: 1,
  title: 'Sprint 1',
  state: 'open',
  description: 'First sprint',
  open_issues: 2,
  closed_issues: 0,
}

export const closedMilestone: Milestone = {
  number: 2,
  title: 'Sprint 0',
  state: 'closed',
  description: 'Initial sprint',
  open_issues: 0,
  closed_issues: 3,
}

function makeIssue(overrides: Partial<Issue> & Pick<Issue, 'number' | 'title'>): Issue {
  return {
    state: 'open',
    html_url: `https://github.com/test-owner/test-repo/issues/${overrides.number}`,
    assignees: [],
    labels: ['ghqc', 'main'],
    milestone: 'Sprint 1',
    created_at: '2024-01-01T00:00:00Z',
    updated_at: '2024-01-01T00:00:00Z',
    closed_at: null,
    created_by: 'test-user',
    branch: 'main',
    // D52: no `## QC Rounds` marker ⇒ single round ⇒ `branch` above is current.
    has_qc_rounds_marker: false,
    relevant_files: [],
    file_history: [],
    ...overrides,
  }
}

/** A gap with nothing in it — normal and meaningful (D8/D26). */
export function emptyGap(): Gap {
  return { commits: [], divergent: false, newest_file_change: null }
}

/**
 * A round on the wire (A4). `state` is the tagged union (D36) — there is no
 * `approved_commit` field — and `checklist_content` excludes its `# ` heading (D37).
 */
export function makeRound(overrides: Partial<RoundInfo> = {}): RoundInfo {
  const commits: IssueCommit[] = overrides.commits ?? [
    { hash: 'ccc3333', message: 'latest commit', statuses: ['notification'], file_changed: false },
  ]
  return {
    index: 1,
    branch: 'main',
    // D56: an inherited branch is the exception, so the default is a declared one.
    branch_inherited: false,
    // D53: the default round is placed — `unplaceableRound()` builds the other case.
    placement: 'placed',
    start_commit: commits[commits.length - 1]?.hash ?? '',
    state: { kind: 'open' },
    checklist_name: 'Code Review',
    checklist_content: '- [ ] Review logic',
    checklist_summary: { completed: 0, total: 0, percentage: 0 },
    commits,
    preceding_gap: emptyGap(),
    archive_commit: commits[0]?.hash ?? '',
    subsequent_file_changes: false,
    ...overrides,
  }
}

/**
 * D53: a round that could not be placed on its branch. It is **never dropped and
 * never re-indexed** — it keeps its declared index — and it owns no commits, so its
 * `commits` and `preceding_gap` are empty and `archive_commit` is `null` (D54).
 * `subsequent_file_changes` is `true` because a null archive commit gives nothing to
 * measure "after" (D39.3).
 */
export function unplaceableRound(overrides: Partial<RoundInfo> = {}): RoundInfo {
  return makeRound({
    placement: 'unplaceable',
    commits: [],
    // The start commit is *declared* — it just cannot be resolved on `branch`.
    start_commit: 'deadbeefdeadbeefdeadbeefdeadbeefdeadbeef',
    preceding_gap: emptyGap(),
    archive_commit: null,
    subsequent_file_changes: true,
    ...overrides,
  })
}

/** An approved single round: `state.kind === 'approved'` carries the commit and the
 *  comment id the approved-commit row deep-links (U8). */
export function makeApprovedRound(commit: string, overrides: Partial<RoundInfo> = {}): RoundInfo {
  return makeRound({
    state: { kind: 'approved', commit, comment_id: 1234 },
    commits: [{ hash: commit, message: 'initial commit', statuses: ['initial'], file_changed: true }],
    archive_commit: commit,
    ...overrides,
  })
}

/**
 * W6's segment order, for fixtures only. The **server** owns this projection (M2) — this
 * exists so a fixture cannot silently disagree with it, and its output is pinned against
 * the same concrete two-round case `test_history_projects_w6_order_with_gaps_naming_the_round_they_precede`
 * asserts on the Rust side.
 *
 * Two positional rules: round 1's preceding gap is skipped (W6.1), and `drift` is
 * emitted only when the latest round is closed (W6.2). A last round is closed exactly
 * when it is `approved` — `superseded` means a later round exists, so it can never be
 * last.
 */
export function historyOf(rounds: RoundInfo[]): SegmentRef[] {
  const history: SegmentRef[] = []
  rounds.forEach((round, position) => {
    if (position > 0) history.push({ kind: 'gap', round_index: round.index })
    history.push({ kind: 'round', round_index: round.index })
  })
  const latest = rounds[rounds.length - 1]
  if (latest?.state.kind === 'approved') {
    history.push({ kind: 'drift', round_index: latest.index })
  }
  return history
}

/**
 * Fills in `history` from `rounds` so a fixture can never disagree with the server's M2
 * projection. An explicit `history` still wins — a fixture that wants a shape the
 * projection would not produce says so deliberately.
 */
export function withHistory(
  status: Omit<IssueStatusResponse, 'history'> & { history?: SegmentRef[] },
): IssueStatusResponse {
  // Always **recomputed**, never inherited. These fixtures are built by spreading one
  // another, and a spread carries the source's `history` — which would keep a `drift` row
  // (W6.2) on a fixture whose latest round is open, i.e. a segment the server would never
  // project.
  return { ...status, history: historyOf(status.rounds) }
}

function makeStatusResponse(
  issue: Issue,
  status: QCStatus['status'],
  overrides: Partial<IssueStatusResponse> = {},
): IssueStatusResponse {
  const approved = status === 'approved' || status === 'changes_after_approval'
  return {
    issue,
    // D35: a verdict, not a commit carrier.
    qc_status: { status, status_detail: '' },
    dirty: false,
    rounds: [approved ? makeApprovedRound('aaa1111') : makeRound()],
    drift: emptyGap(),
    history: historyOf([approved ? makeApprovedRound('aaa1111') : makeRound()]),
    ...overrides,
  }
}

// One issue per swimlane category
export const awaitingReviewIssue = makeIssue({ number: 10, title: 'src/awaiting.rs' })
export const changeRequestedIssue = makeIssue({ number: 11, title: 'src/change.rs' })
export const inProgressIssue = makeIssue({ number: 12, title: 'src/inprogress.rs' })
export const approvedIssue = makeIssue({ number: 13, title: 'src/approved.rs' })

export const awaitingReviewStatus = makeStatusResponse(awaitingReviewIssue, 'awaiting_review')
export const changeRequestedStatus = makeStatusResponse(changeRequestedIssue, 'change_requested')
export const inProgressStatus = makeStatusResponse(inProgressIssue, 'in_progress')
export const approvedStatus = makeStatusResponse(approvedIssue, 'approved')

// Milestone 2 issue (for multi-milestone test)
export const milestone2Issue = makeIssue({ number: 20, title: 'src/milestone2.rs', milestone: 'Sprint 0' })
export const milestone2Status = makeStatusResponse(milestone2Issue, 'awaiting_review')

// Closed issue
export const closedIssue = makeIssue({ number: 30, title: 'src/closed.rs', state: 'closed', closed_at: '2024-01-02T00:00:00Z' })
export const closedIssueStatus = makeStatusResponse(closedIssue, 'approved')

// Dirty issue
export const dirtyIssue = makeIssue({ number: 40, title: 'src/dirty.rs' })
export const dirtyStatus = makeStatusResponse(dirtyIssue, 'awaiting_review', { dirty: true })

export const cleanIssue = makeIssue({ number: 41, title: 'src/clean.rs' })
export const cleanStatus = makeStatusResponse(cleanIssue, 'awaiting_review', { dirty: false })

// Issues for partial 206 test
export const partialIssue1 = makeIssue({ number: 50, title: 'src/partial1.rs' })
export const partialIssue2 = makeIssue({ number: 51, title: 'src/partial2.rs' })
export const partialIssue3 = makeIssue({ number: 52, title: 'src/partial3.rs' })

export const partialStatus1 = makeStatusResponse(partialIssue1, 'awaiting_review')
export const partialStatus2 = makeStatusResponse(partialIssue2, 'in_progress')

export const partialBatchResponse: BatchIssueStatusResponse = {
  results: [partialStatus1, partialStatus2],
  errors: [{ issue_number: 52, kind: 'fetch_failed', error: 'not found' }],
}

// ── IssueDetailModal fixtures ─────────────────────────────────────────────────

const emptyBlockingQCStatus = {
  total: 0, approved_count: 0, summary: '-',
  approved: [], not_approved: [], errors: [],
}

// Single commit: one file-changing initial commit. Slider should center it.
export const singleCommitIssue = makeIssue({ number: 70, title: 'src/single.rs', branch: 'feature-branch', assignees: ['alice'] })
export const singleCommitStatus: IssueStatusResponse = withHistory({
  issue: singleCommitIssue,
  qc_status: { status: 'awaiting_review', status_detail: 'Awaiting first review' },
  dirty: false,
  rounds: [
    makeRound({
      branch: 'feature-branch',
      checklist_summary: { completed: 2, total: 7, percentage: 28.6 },
      commits: [
        { hash: 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa', message: 'initial commit', statuses: ['initial'], file_changed: true },
      ],
    }),
  ],
  drift: emptyGap(),
  blocking_qc_status: emptyBlockingQCStatus,
})

// Multi-commit: 4 commits, one hidden by default (ccccccc: no file change, no statuses).
// Newest-first order as the API returns them.
//   ddddddd – file_changed=true,  statuses=[]             ← latest, TO default
//   ccccccc – file_changed=false, statuses=[]             ← hidden unless showAll
//   bbbbbbb – file_changed=true,  statuses=['notification'] ← FROM default
//   aaaaaaa – file_changed=true,  statuses=['initial']
export const multiCommitIssue = makeIssue({ number: 71, title: 'src/multi.rs' })
export const multiCommitStatus: IssueStatusResponse = withHistory({
  issue: multiCommitIssue,
  qc_status: { status: 'changes_to_comment', status_detail: 'New changes since last notification' },
  dirty: false,
  rounds: [
    makeRound({
      commits: [
        { hash: 'ddddddddddddddddddddddddddddddddddddddd1', message: 'new changes', statuses: [], file_changed: true },
        { hash: 'ccccccccccccccccccccccccccccccccccccccc1', message: 'bump version', statuses: [], file_changed: false },
        { hash: 'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb1', message: 'push notification', statuses: ['notification'], file_changed: true },
        { hash: 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa', message: 'initial commit', statuses: ['initial'], file_changed: true },
      ],
      archive_commit: 'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb1',
    }),
  ],
  drift: emptyGap(),
  blocking_qc_status: emptyBlockingQCStatus,
})

// Notification landed on a non-file-changing commit after the last file change.
// FROM and TO both default to bbbbbbb (FROM is already the last commit).
//   bbbbbbb – file_changed=false, statuses=['notification'] ← FROM=TO default, also exceptionIdx
//   aaaaaaa – file_changed=true,  statuses=['initial']
export const notifOnNonFileIssue = makeIssue({ number: 72, title: 'src/notif-nofile.rs' })
export const notifOnNonFileStatus: IssueStatusResponse = withHistory({
  issue: notifOnNonFileIssue,
  qc_status: { status: 'awaiting_review', status_detail: 'Awaiting review' },
  dirty: false,
  rounds: [
    makeRound({
      commits: [
        { hash: 'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb1', message: 'notification on non-file commit', statuses: ['notification'], file_changed: false },
        { hash: 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa', message: 'initial commit', statuses: ['initial'], file_changed: true },
      ],
      archive_commit: 'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb1',
    }),
  ],
  drift: emptyGap(),
  blocking_qc_status: emptyBlockingQCStatus,
})

// Approved modal issue — used to test the unapprove tab (defaults to 'unapprove' tab)
export const approvedModalIssue = makeIssue({ number: 74, title: 'src/approved-modal.rs', branch: 'feature-branch' })
export const approvedModalStatus: IssueStatusResponse = withHistory({
  issue: approvedModalIssue,
  qc_status: { status: 'approved', status_detail: 'Approved' },
  dirty: false,
  rounds: [
    makeApprovedRound('aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa', {
      branch: 'feature-branch',
      commits: [
        { hash: 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa', message: 'initial commit', statuses: ['initial', 'approved'], file_changed: true },
      ],
    }),
  ],
  drift: emptyGap(),
  blocking_qc_status: { total: 0, approved_count: 0, summary: '-', approved: [], not_approved: [], errors: [] },
})

// Dirty modal issue — used to test the asterisk in the modal status card
export const dirtyModalIssue = makeIssue({ number: 73, title: 'src/dirty-modal.rs' })
export const dirtyModalStatus: IssueStatusResponse = withHistory({
  ...singleCommitStatus,
  issue: dirtyModalIssue,
  dirty: true,
})
export const cleanModalStatus: IssueStatusResponse = withHistory({
  ...singleCommitStatus,
  issue: dirtyModalIssue,
  dirty: false,
})

// ── Unapprove / blocked fixtures ─────────────────────────────────────────────

// Approved child issue blocked by approvedModalIssue (#74)
export const approvedChildIssue = makeIssue({ number: 80, title: 'src/child-approved.rs', state: 'closed', milestone: 'Sprint 1' })
export const approvedChildBlocked: BlockedIssueStatus = {
  issue: approvedChildIssue,
  qc_status: { status: 'approved', status_detail: 'Approved' },
}

// Not-approved child issue blocked by approvedModalIssue (#74)
export const notApprovedChildIssue = makeIssue({ number: 81, title: 'src/child-pending.rs', milestone: 'Sprint 1' })
export const notApprovedChildBlocked: BlockedIssueStatus = {
  issue: notApprovedChildIssue,
  qc_status: { status: 'awaiting_review', status_detail: 'Awaiting review' },
}

// Grandchild — returned when approvedChildIssue's /blocked is fetched
export const grandchildIssue = makeIssue({ number: 83, title: 'src/grandchild.rs', milestone: 'Sprint 1' })
export const grandchildBlocked: BlockedIssueStatus = {
  issue: grandchildIssue,
  qc_status: { status: 'awaiting_review', status_detail: 'Awaiting review' },
}

// Full IssueStatusResponse for approvedChildIssue (used in unapproval cache tests)
export const approvedChildStatus: IssueStatusResponse = withHistory({
  issue: approvedChildIssue,
  qc_status: { status: 'approved', status_detail: 'Approved' },
  dirty: false,
  rounds: [
    makeApprovedRound('cccccccccccccccccccccccccccccccccccccccc', {
      commits: [
        { hash: 'cccccccccccccccccccccccccccccccccccccccc', message: 'initial commit', statuses: ['initial', 'approved'], file_changed: true },
      ],
    }),
  ],
  drift: emptyGap(),
  blocking_qc_status: { total: 0, approved_count: 0, summary: '-', approved: [], not_approved: [], errors: [] },
})

// Non-approved issue for tab-disabled tests (defaults to Notify tab)
export const inProgressModalIssue = makeIssue({ number: 82, title: 'src/in-progress-modal.rs', branch: 'feature-branch' })
export const inProgressModalStatus: IssueStatusResponse = withHistory({
  issue: inProgressModalIssue,
  qc_status: { status: 'in_progress', status_detail: 'In progress' },
  dirty: false,
  rounds: [
    makeRound({
      branch: 'feature-branch',
      commits: [
        { hash: 'ffffffffffffffffffffffffffffffffffffffff', message: 'initial commit', statuses: ['initial'], file_changed: true },
      ],
    }),
  ],
  drift: emptyGap(),
  blocking_qc_status: { total: 0, approved_count: 0, summary: '-', approved: [], not_approved: [], errors: [] },
})

// ── Blocking QC inverse-map / cache-invalidation fixtures ────────────────────
// helperIssue (#90) is a blocking QC for fileAIssue (#91).
// Before helper is approved: fileA shows 0/1 blocking QC progress.
// After helper is approved:  fileA shows 1/1.

export const helperIssue = makeIssue({ number: 90, title: 'src/helper.rs', branch: 'feature-branch' })
export const fileAIssue  = makeIssue({ number: 91, title: 'src/file_a.rs' })

export const helperStatusInitial: IssueStatusResponse = withHistory({
  issue: helperIssue,
  qc_status: { status: 'awaiting_review', status_detail: 'Awaiting first review' },
  dirty: false,
  rounds: [
    makeRound({
      branch: 'feature-branch',
      commits: [
        { hash: 'aaa0000000000000000000000000000000000000', message: 'initial commit', statuses: ['initial'], file_changed: true },
      ],
    }),
  ],
  drift: emptyGap(),
  blocking_qc_status: emptyBlockingQCStatus,
})

export const helperStatusApproved: IssueStatusResponse = withHistory({
  ...helperStatusInitial,
  qc_status: { status: 'approved', status_detail: 'Approved' },
  rounds: [
    makeApprovedRound('aaa0000000000000000000000000000000000000', {
      branch: 'feature-branch',
      commits: [
        { hash: 'aaa0000000000000000000000000000000000000', message: 'initial commit', statuses: ['initial', 'approved'], file_changed: true },
      ],
    }),
  ],
})

export const fileAStatusBlocked: IssueStatusResponse = withHistory({
  issue: fileAIssue,
  qc_status: { status: 'awaiting_review', status_detail: 'Awaiting first review' },
  dirty: false,
  rounds: [
    makeRound({
      commits: [
        { hash: 'bbb0000000000000000000000000000000000000', message: 'initial commit', statuses: ['initial'], file_changed: true },
      ],
    }),
  ],
  drift: emptyGap(),
  blocking_qc_status: {
    total: 1,
    approved_count: 0,
    summary: '0/1 blocking QCs approved',
    approved: [],
    not_approved: [{ issue_number: 90, file_name: 'src/helper.rs', status: 'awaiting_review' }],
    errors: [],
  },
})

export const fileAStatusUnblocked: IssueStatusResponse = withHistory({
  ...fileAStatusBlocked,
  blocking_qc_status: {
    total: 1,
    approved_count: 1,
    summary: '1/1 blocking QCs approved',
    approved: [{ issue_number: 90, file_name: 'src/helper.rs' }],
    not_approved: [],
    errors: [],
  },
})

// ── Create tab fixtures ───────────────────────────────────────────────────────

export const defaultAssignees: Assignee[] = [
  { login: 'reviewer1', name: 'Reviewer One' },
]

export const defaultChecklists: Checklist[] = [
  { name: 'Code Review', content: '- [ ] Review logic\n- [ ] Check tests' },
  { name: 'Custom', content: '' },
]

export const rootFileTree: FileTreeResponse = {
  path: '',
  entries: [{ name: 'src', kind: 'directory' }],
}

export const srcFileTree: FileTreeResponse = {
  path: 'src',
  entries: [
    { name: 'main.rs', kind: 'file' },
    { name: 'lib.rs', kind: 'file' },
    { name: 'utils.rs', kind: 'file' },
    { name: 'external.rs', kind: 'file' },
  ],
}

// Issues used by rel-file picker (src/utils.rs is intentionally absent → hasIssues=false)
export const mainRsIssue = makeIssue({ number: 5, title: 'src/main.rs' })
export const libIssue = makeIssue({ number: 10, title: 'src/lib.rs' })
export const externalIssue = makeIssue({ number: 11, title: 'src/external.rs' })

export const createdMilestone: Milestone = {
  number: 100,
  title: 'My Milestone',
  state: 'open',
  description: null,
  open_issues: 0,
  closed_issues: 0,
}

export const createIssueResponses: CreateIssueResponse[] = [
  {
    issue_url: 'https://github.com/test-owner/test-repo/issues/201',
    issue_number: 201,
    blocking_created: [],
    blocking_errors: [],
  },
  {
    issue_url: 'https://github.com/test-owner/test-repo/issues/202',
    issue_number: 202,
    blocking_created: [],
    blocking_errors: [],
  },
]

// ---------------------------------------------------------------------------
// Configuration repository git state
// ---------------------------------------------------------------------------

export function makeConfigGitRepo(
  overrides: Partial<ConfigGitRepository> = {},
): ConfigGitRepository {
  return {
    owner: 'myorg',
    repo: 'config-repo',
    status: 'clean',
    status_detail: 'Configuration repository is up to date',
    ahead_commits: [],
    behind_commits: [],
    dirty_files: [],
    ...overrides,
  }
}

export const configRepoClean = makeConfigGitRepo()

export const configRepoBehind = makeConfigGitRepo({
  status: 'behind',
  status_detail: 'Repository is behind by 3 commits',
  behind_commits: ['aaa1111', 'bbb2222', 'ccc3333'],
})

export const configRepoAhead = makeConfigGitRepo({
  status: 'ahead',
  status_detail: 'Repository is ahead by 1 commit',
  ahead_commits: ['ddd4444'],
})

export const configRepoDiverged = makeConfigGitRepo({
  status: 'diverged',
  status_detail: 'Repository has diverged from its remote',
  ahead_commits: ['eee5555', 'fff6666'],
  behind_commits: ['aaa1111', 'bbb2222', 'ccc3333'],
})

// ── QC rounds fixtures (§9 U1–U8) ────────────────────────────────────────────
// A two-round approved issue. Each round owns its own commits (D8), so the
// slider is scoped to whichever round the switcher selects (U4/W5) — it never
// spans the whole QC's life again.

export const R1_START = '1a11111111111111111111111111111111111111'
export const R1_APPROVAL = '1b11111111111111111111111111111111111111'
export const GAP_COMMIT = '9999999999999999999999999999999999999999'
export const R2_START = '2a22222222222222222222222222222222222222'
export const R2_APPROVAL = '2b22222222222222222222222222222222222222'
/** A file-changing drift commit that is **not** `drift.newest_file_change`. It sits
 *  first in `drift.commits`, so `commits.find(c => c.file_changed)` — the client-side
 *  rescan U7 forbids — returns this instead of the hash the server reports. Legal
 *  data: D39.3 says a divergent gap's ordering is unsound, which is exactly why the
 *  UI must not re-derive the hash from the walk. */
export const DRIFT_DECOY = '3c33333333333333333333333333333333333333'

// Two rounds exist, so the body carries the `## QC Rounds` marker (D51/D52) and
// `issue.branch` is round 1's.
export const twoRoundIssue = makeIssue({ number: 75, title: 'src/two-rounds.rs', has_qc_rounds_marker: true })

export const twoRoundRounds: RoundInfo[] = [
  makeRound({
    index: 1,
    state: { kind: 'approved', commit: R1_APPROVAL, comment_id: 111 },
    checklist_name: 'Round One',
    // D37: no `# ` heading in the content, and the round-1 record keeps its ticks.
    checklist_content: '- [x] r1 item',
    checklist_summary: { completed: 1, total: 1, percentage: 1 },
    commits: [
      { hash: R1_APPROVAL, message: 'round 1 approval', statuses: ['reviewed'], file_changed: false },
      { hash: R1_START, message: 'round 1 initial', statuses: ['initial'], file_changed: true },
    ],
    start_commit: R1_START,
    archive_commit: R1_APPROVAL,
    subsequent_file_changes: true,
  }),
  makeRound({
    index: 2,
    state: { kind: 'approved', commit: R2_APPROVAL, comment_id: 222 },
    checklist_name: 'Round Two',
    checklist_content: '- [x] r2 item one\n- [ ] r2 item two',
    checklist_summary: { completed: 1, total: 2, percentage: 0.5 },
    commits: [
      { hash: R2_APPROVAL, message: 'round 2 approval', statuses: ['notification'], file_changed: false },
      { hash: R2_START, message: 'round 2 initial', statuses: ['initial'], file_changed: true },
    ],
    start_commit: R2_START,
    archive_commit: R2_APPROVAL,
    // The D8 overlap case is absent here: a real gap commit sits between the
    // round-1 approval and the round-2 start.
    preceding_gap: { commits: [{ hash: GAP_COMMIT, message: 'gap commit', statuses: [], file_changed: true }], divergent: false, newest_file_change: GAP_COMMIT },
  }),
]

export const twoRoundStatus: IssueStatusResponse = withHistory({
  issue: twoRoundIssue,
  qc_status: { status: 'approved', status_detail: 'Approved' },
  dirty: false,
  rounds: twoRoundRounds,
  drift: emptyGap(),
  blocking_qc_status: { total: 0, approved_count: 0, summary: '-', approved: [], not_approved: [], errors: [] },
})

/** D44: an approval whose `comment_id` is unknown. `comment_id` is nullable precisely
 *  because a `0` sentinel is indistinguishable from a real comment id, so the
 *  approved-commit row must drop U8's deep-link rather than point at comment 0. */
export const nullCommentIdIssue = makeIssue({ number: 76, title: 'src/no-comment-id.rs' })

export const nullCommentIdStatus: IssueStatusResponse = withHistory({
  issue: nullCommentIdIssue,
  qc_status: { status: 'approved', status_detail: 'Approved' },
  dirty: false,
  rounds: [
    makeApprovedRound(R2_APPROVAL, {
      state: { kind: 'approved', commit: R2_APPROVAL, comment_id: null },
    }),
  ],
  drift: emptyGap(),
})

/** A divergent preceding gap (D22): the round-2 start is not descended from the
 *  round-1 approval, so the two rounds share no cohesive history (U6). */
export const divergentGapStatus: IssueStatusResponse = withHistory({
  ...twoRoundStatus,
  rounds: [
    twoRoundRounds[0],
    { ...twoRoundRounds[1], preceding_gap: { ...twoRoundRounds[1].preceding_gap, divergent: true } },
  ],
})

/**
 * Two rounds with the latest one **open**, so the Approve tab is reachable. I14 keeps
 * `drift` empty here, which is why D83's tail collapses to the latest round on that tab.
 */
export const approvableTwoRoundStatus: IssueStatusResponse = withHistory({
  ...twoRoundStatus,
  qc_status: { status: 'awaiting_review', status_detail: 'Awaiting first review' },
  rounds: [twoRoundRounds[0], { ...twoRoundRounds[1], state: { kind: 'open' } }],
})

/**
 * The shape from the reported miscount: round 1's initial commit **is** its approval (one
 * commit carrying both statuses), and the gap before round 2 holds a commit that changed
 * nothing and carries no status — one the slider hides. Two commits are of interest; a raw
 * count would claim three.
 */
export const QUIET_GAP_COMMIT = '7c77777777777777777777777777777777777777'

export const quietGapStatus: IssueStatusResponse = withHistory({
  ...twoRoundStatus,
  qc_status: { status: 'awaiting_review', status_detail: 'Awaiting first review' },
  rounds: [
    makeRound({
      index: 1,
      state: { kind: 'approved', commit: R1_START, comment_id: 111 },
      // One commit, both roles: the initial commit is the approved commit.
      commits: [
        { hash: R1_START, message: 'round 1 initial', statuses: ['initial', 'approved'], file_changed: true },
      ],
      start_commit: R1_START,
      archive_commit: R1_START,
    }),
    makeRound({
      index: 2,
      state: { kind: 'open' },
      commits: [
        { hash: R2_START, message: 'round 2 initial', statuses: ['initial'], file_changed: true },
      ],
      start_commit: R2_START,
      archive_commit: R2_START,
      preceding_gap: {
        commits: [
          { hash: QUIET_GAP_COMMIT, message: 'unrelated change', statuses: [], file_changed: false },
        ],
        divergent: false,
        newest_file_change: null,
      },
    }),
  ],
})

/**
 * D8's overlap case *plus* divergence: a divergent gap that owns **no commits**. The
 * break marker attaches to the next commit that actually appears, so a fixture with
 * nothing in the gap is what proves the marker is carried across rather than lost.
 */
export const divergentEmptyGapStatus: IssueStatusResponse = withHistory({
  ...twoRoundStatus,
  rounds: [
    twoRoundRounds[0],
    {
      ...twoRoundRounds[1],
      preceding_gap: { commits: [], divergent: true, newest_file_change: null },
    },
  ],
})

/**
 * A break with **no file-changing commit inside the crossed range**, so D75's
 * conservative-`true` is observable in the payload: the order-derived walk says "nothing
 * changed", and D75 says that walk is not a fact across a break.
 */
export const breakNoFileChangeStatus: IssueStatusResponse = withHistory({
  ...approvableTwoRoundStatus,
  rounds: [
    twoRoundRounds[0],
    {
      ...twoRoundRounds[1],
      state: { kind: 'open' },
      commits: [
        { hash: R2_APPROVAL, message: 'round 2 approval', statuses: ['notification'], file_changed: false },
        { hash: R2_START, message: 'round 2 initial', statuses: ['initial'], file_changed: false },
      ],
    },
  ],
})

/** A divergent drift (D31): a force-push removed the approval from its branch, so
 *  the reported ChangesAfterApproval hash is not meaningful (U6). */
export const divergentDriftStatus: IssueStatusResponse = withHistory({
  ...twoRoundStatus,
  qc_status: { status: 'changes_after_approval', status_detail: 'Changes after approval' },
  drift: {
    // Two file-changing commits, and the first one is *not* the reported hash — so a
    // client-side `commits.find(c => c.file_changed)` disagrees with the server (U7).
    commits: [
      { hash: DRIFT_DECOY, message: 'older post-approval change', statuses: [], file_changed: true },
      { hash: GAP_COMMIT, message: 'post-approval change', statuses: [], file_changed: true },
    ],
    divergent: true,
    newest_file_change: GAP_COMMIT,
  },
})

/** A drift-touching commit and two that leave the file alone. The two counts the Round
 *  tab reports must therefore **differ** — a fixture where they agree cannot tell
 *  `drift.commits.length` from the file-changing filter. */
export const DRIFT_TOUCHING = '4d44444444444444444444444444444444444444'
const DRIFT_QUIET_A = '5e55555555555555555555555555555555555555'
const DRIFT_QUIET_B = '6f66666666666666666666666666666666666666'

/** A **non-divergent** drift that actually holds commits — the ordinary
 *  `changes_after_approval` shape a new round starts from. */
export const driftingStatus: IssueStatusResponse = withHistory({
  ...twoRoundStatus,
  qc_status: { status: 'changes_after_approval', status_detail: 'Changes after approval' },
  drift: {
    commits: [
      { hash: DRIFT_TOUCHING, message: 'touched the qc file', statuses: [], file_changed: true },
      { hash: DRIFT_QUIET_A, message: 'unrelated change', statuses: [], file_changed: false },
      { hash: DRIFT_QUIET_B, message: 'another unrelated change', statuses: [], file_changed: false },
    ],
    divergent: false,
    newest_file_change: DRIFT_TOUCHING,
  },
})

/** D96: round 1 lived on a different branch than the round the QC is on now, so its
 *  commits were walked against that branch. Round 2 is on `main`, the reference. */
export const otherBranchStatus: IssueStatusResponse = withHistory({
  ...twoRoundStatus,
  rounds: [
    { ...twoRoundRounds[0], branch: 'feature/round-one' },
    { ...twoRoundRounds[1], branch: 'main' },
  ],
})

// ── §18 (D53–D56) fixtures ───────────────────────────────────────────────────

/** The branch a round was declared on but which is not fetched locally (D53). */
export const UNFETCHED_BRANCH = 'feature/not-fetched'
/** Round 1's approval commit. It is real — it is in the comment log — but it cannot
 *  be placed, so no surface may print it as though it were located (D54/D55). */
export const R1_UNPLACEABLE_APPROVAL = '1c11111111111111111111111111111111111111'

/**
 * D53: round 1 is declared on a branch that is not fetched, so it is `unplaceable`:
 * it keeps its declared index, owns no commits, and has `archive_commit: null`.
 * Round 2 is on a fetched branch and places normally — which is why this issue is in
 * `results[]` at all (D55/D10: only the *latest* round gates status).
 */
export const unplaceableRoundIssue = makeIssue({
  number: 77,
  title: 'src/unplaceable.rs',
  has_qc_rounds_marker: true,
})

export const unplaceableRoundStatus: IssueStatusResponse = withHistory({
  issue: unplaceableRoundIssue,
  qc_status: { status: 'approved', status_detail: 'Approved' },
  dirty: false,
  rounds: [
    unplaceableRound({
      index: 1,
      branch: UNFETCHED_BRANCH,
      state: { kind: 'approved', commit: R1_UNPLACEABLE_APPROVAL, comment_id: 777 },
      checklist_name: 'Round One',
      checklist_content: '- [x] r1 item',
      checklist_summary: { completed: 1, total: 1, percentage: 1 },
    }),
    makeRound({
      index: 2,
      state: { kind: 'approved', commit: R2_APPROVAL, comment_id: 222 },
      checklist_name: 'Round Two',
      checklist_content: '- [x] r2 item one\n- [ ] r2 item two',
      commits: [
        { hash: R2_APPROVAL, message: 'round 2 approval', statuses: ['reviewed'], file_changed: false },
        { hash: R2_START, message: 'round 2 initial', statuses: ['initial'], file_changed: true },
      ],
      start_commit: R2_START,
      archive_commit: R2_APPROVAL,
    }),
  ],
  drift: emptyGap(),
  blocking_qc_status: { total: 0, approved_count: 0, summary: '-', approved: [], not_approved: [], errors: [] },
})

/**
 * D55: an issue whose **latest** round is unresolvable arrives in `errors[]` as
 * `branch_not_local` rather than in `results[]` — the status endpoint refuses to
 * invent a `QCStatus` for it (S5: no new status variants).
 */
export const latestRoundUnplaceableError: BatchIssueStatusResponse = {
  results: [],
  errors: [
    {
      issue_number: unplaceableRoundIssue.number,
      kind: 'branch_not_local',
      error: `Branch not found: ${UNFETCHED_BRANCH}`,
      branch: UNFETCHED_BRANCH,
    },
  ],
}

/** D56: round 2's comment declared no `git branch:`, so it inherited round 1's. */
export const branchInheritedStatus: IssueStatusResponse = withHistory({
  ...twoRoundStatus,
  rounds: [twoRoundRounds[0], { ...twoRoundRounds[1], branch_inherited: true }],
})

export const R3_START = '3a33333333333333333333333333333333333333'
export const R3_APPROVAL = '3b33333333333333333333333333333333333333'

/**
 * D53.2 / item 4: round 2's declaration was malformed, so it was dropped **without
 * renumbering** — `rounds` is `[1, 3]`. Nothing may derive a round number from an
 * array position or from `rounds.length`: position 1 holds round *3*, and the next
 * round to start is *4*, not 3.
 */
export const holeRoundsIssue = makeIssue({
  number: 78,
  title: 'src/round-hole.rs',
  has_qc_rounds_marker: true,
})

export const holeRoundsStatus: IssueStatusResponse = withHistory({
  issue: holeRoundsIssue,
  qc_status: { status: 'approved', status_detail: 'Approved' },
  dirty: false,
  rounds: [
    twoRoundRounds[0],
    makeRound({
      index: 3,
      state: { kind: 'approved', commit: R3_APPROVAL, comment_id: 333 },
      checklist_name: 'Round Three',
      checklist_content: '- [x] r3 item',
      commits: [
        { hash: R3_APPROVAL, message: 'round 3 approval', statuses: ['reviewed'], file_changed: false },
        { hash: R3_START, message: 'round 3 initial', statuses: ['initial'], file_changed: true },
      ],
      start_commit: R3_START,
      archive_commit: R3_APPROVAL,
    }),
  ],
  drift: emptyGap(),
  blocking_qc_status: { total: 0, approved_count: 0, summary: '-', approved: [], not_approved: [], errors: [] },
})

/**
 * D60: the *other* `None` case of `Round::latest_commit()` (D54.2) — the branch is
 * local, but the approval was force-pushed or rebased off it. It arrives as
 * `branch_not_local` on purpose (one wire kind, two messages), so the message is the
 * only signal that "fetch the branch" is the wrong remedy here.
 */
export const approvalNotOnBranchError: BatchIssueStatusResponse = {
  results: [],
  errors: [
    {
      issue_number: unplaceableRoundIssue.number,
      kind: 'branch_not_local',
      error: `Approval commit ${R1_UNPLACEABLE_APPROVAL} is no longer reachable on branch '${UNFETCHED_BRANCH}' — its history was likely rewritten`,
      branch: UNFETCHED_BRANCH,
    },
  ],
}

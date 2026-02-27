import type { RepoInfo } from '../../src/api/repo'
import type { Milestone } from '../../src/api/milestones'
import type {
  Issue,
  IssueStatusResponse,
  BatchIssueStatusResponse,
  BlockedIssueStatus,
  QCStatus,
} from '../../src/api/issues'
import type { Assignee } from '../../src/api/assignees'
import type { Checklist } from '../../src/api/checklists'
import type { FileTreeResponse } from '../../src/api/files'
import type { CreateIssueResponse } from '../../src/api/create'

export const defaultRepoInfo: RepoInfo = {
  owner: 'test-owner',
  repo: 'test-repo',
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
    checklist_name: 'Code Review',
    relevant_files: [],
    ...overrides,
  }
}

function makeStatusResponse(
  issue: Issue,
  status: QCStatus['status'],
  overrides: Partial<IssueStatusResponse> = {},
): IssueStatusResponse {
  return {
    issue,
    qc_status: {
      status,
      status_detail: '',
      approved_commit: status === 'approved' ? 'aaa1111' : null,
      initial_commit: 'bbb2222',
      latest_commit: 'ccc3333',
    },
    dirty: false,
    branch: 'main',
    commits: [
      {
        hash: 'ccc3333',
        message: 'latest commit',
        statuses: ['notification'],
        file_changed: false,
      },
    ],
    checklist_summary: { completed: 0, total: 0, percentage: 0 },
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
export const singleCommitStatus: IssueStatusResponse = {
  issue: singleCommitIssue,
  qc_status: { status: 'awaiting_review', status_detail: 'Awaiting first review', approved_commit: null, initial_commit: 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa', latest_commit: 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa' },
  dirty: false,
  branch: 'feature-branch',
  commits: [
    { hash: 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa', message: 'initial commit', statuses: ['initial'], file_changed: true },
  ],
  checklist_summary: { completed: 2, total: 7, percentage: 28.6 },
  blocking_qc_status: emptyBlockingQCStatus,
}

// Multi-commit: 4 commits, one hidden by default (ccccccc: no file change, no statuses).
// Newest-first order as the API returns them.
//   ddddddd – file_changed=true,  statuses=[]             ← latest, TO default
//   ccccccc – file_changed=false, statuses=[]             ← hidden unless showAll
//   bbbbbbb – file_changed=true,  statuses=['notification'] ← FROM default
//   aaaaaaa – file_changed=true,  statuses=['initial']
export const multiCommitIssue = makeIssue({ number: 71, title: 'src/multi.rs' })
export const multiCommitStatus: IssueStatusResponse = {
  issue: multiCommitIssue,
  qc_status: { status: 'changes_to_comment', status_detail: 'New changes since last notification', approved_commit: null, initial_commit: 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa', latest_commit: 'ddddddddddddddddddddddddddddddddddddddd1' },
  dirty: false,
  branch: 'main',
  commits: [
    { hash: 'ddddddddddddddddddddddddddddddddddddddd1', message: 'new changes', statuses: [], file_changed: true },
    { hash: 'ccccccccccccccccccccccccccccccccccccccc1', message: 'bump version', statuses: [], file_changed: false },
    { hash: 'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb1', message: 'push notification', statuses: ['notification'], file_changed: true },
    { hash: 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa', message: 'initial commit', statuses: ['initial'], file_changed: true },
  ],
  checklist_summary: { completed: 0, total: 0, percentage: 0 },
  blocking_qc_status: emptyBlockingQCStatus,
}

// Notification landed on a non-file-changing commit after the last file change.
// FROM and TO both default to bbbbbbb (FROM is already the last commit).
//   bbbbbbb – file_changed=false, statuses=['notification'] ← FROM=TO default, also exceptionIdx
//   aaaaaaa – file_changed=true,  statuses=['initial']
export const notifOnNonFileIssue = makeIssue({ number: 72, title: 'src/notif-nofile.rs' })
export const notifOnNonFileStatus: IssueStatusResponse = {
  issue: notifOnNonFileIssue,
  qc_status: { status: 'awaiting_review', status_detail: 'Awaiting review', approved_commit: null, initial_commit: 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa', latest_commit: 'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb1' },
  dirty: false,
  branch: 'main',
  commits: [
    { hash: 'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb1', message: 'notification on non-file commit', statuses: ['notification'], file_changed: false },
    { hash: 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa', message: 'initial commit', statuses: ['initial'], file_changed: true },
  ],
  checklist_summary: { completed: 0, total: 0, percentage: 0 },
  blocking_qc_status: emptyBlockingQCStatus,
}

// Approved modal issue — used to test the unapprove tab (defaults to 'unapprove' tab)
export const approvedModalIssue = makeIssue({ number: 74, title: 'src/approved-modal.rs', branch: 'feature-branch' })
export const approvedModalStatus: IssueStatusResponse = {
  issue: approvedModalIssue,
  qc_status: { status: 'approved', status_detail: 'Approved', approved_commit: 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa', initial_commit: 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa', latest_commit: 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa' },
  dirty: false,
  branch: 'feature-branch',
  commits: [
    { hash: 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa', message: 'initial commit', statuses: ['initial', 'approved'], file_changed: true },
  ],
  checklist_summary: { completed: 0, total: 0, percentage: 0 },
  blocking_qc_status: { total: 0, approved_count: 0, summary: '-', approved: [], not_approved: [], errors: [] },
}

// Dirty modal issue — used to test the asterisk in the modal status card
export const dirtyModalIssue = makeIssue({ number: 73, title: 'src/dirty-modal.rs' })
export const dirtyModalStatus: IssueStatusResponse = {
  ...singleCommitStatus,
  issue: dirtyModalIssue,
  dirty: true,
}
export const cleanModalStatus: IssueStatusResponse = {
  ...singleCommitStatus,
  issue: dirtyModalIssue,
  dirty: false,
}

// ── Unapprove / blocked fixtures ─────────────────────────────────────────────

// Approved child issue blocked by approvedModalIssue (#74)
export const approvedChildIssue = makeIssue({ number: 80, title: 'src/child-approved.rs', state: 'closed', milestone: 'Sprint 1' })
export const approvedChildBlocked: BlockedIssueStatus = {
  issue: approvedChildIssue,
  qc_status: {
    status: 'approved',
    status_detail: 'Approved',
    approved_commit: 'cccccccccccccccccccccccccccccccccccccccc',
    initial_commit: 'cccccccccccccccccccccccccccccccccccccccc',
    latest_commit: 'cccccccccccccccccccccccccccccccccccccccc',
  },
}

// Not-approved child issue blocked by approvedModalIssue (#74)
export const notApprovedChildIssue = makeIssue({ number: 81, title: 'src/child-pending.rs', milestone: 'Sprint 1' })
export const notApprovedChildBlocked: BlockedIssueStatus = {
  issue: notApprovedChildIssue,
  qc_status: {
    status: 'awaiting_review',
    status_detail: 'Awaiting review',
    approved_commit: null,
    initial_commit: 'dddddddddddddddddddddddddddddddddddddddd',
    latest_commit: 'dddddddddddddddddddddddddddddddddddddddd',
  },
}

// Grandchild — returned when approvedChildIssue's /blocked is fetched
export const grandchildIssue = makeIssue({ number: 83, title: 'src/grandchild.rs', milestone: 'Sprint 1' })
export const grandchildBlocked: BlockedIssueStatus = {
  issue: grandchildIssue,
  qc_status: {
    status: 'awaiting_review',
    status_detail: 'Awaiting review',
    approved_commit: null,
    initial_commit: 'eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee',
    latest_commit: 'eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee',
  },
}

// Non-approved issue for tab-disabled tests (defaults to Notify tab)
export const inProgressModalIssue = makeIssue({ number: 82, title: 'src/in-progress-modal.rs', branch: 'feature-branch' })
export const inProgressModalStatus: IssueStatusResponse = {
  issue: inProgressModalIssue,
  qc_status: {
    status: 'in_progress',
    status_detail: 'In progress',
    approved_commit: null,
    initial_commit: 'ffffffffffffffffffffffffffffffffffffffff',
    latest_commit: 'ffffffffffffffffffffffffffffffffffffffff',
  },
  dirty: false,
  branch: 'feature-branch',
  commits: [
    { hash: 'ffffffffffffffffffffffffffffffffffffffff', message: 'initial commit', statuses: ['initial'], file_changed: true },
  ],
  checklist_summary: { completed: 0, total: 0, percentage: 0 },
  blocking_qc_status: { total: 0, approved_count: 0, summary: '-', approved: [], not_approved: [], errors: [] },
}

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

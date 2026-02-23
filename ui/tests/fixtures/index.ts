import type { RepoInfo } from '../../src/api/repo'
import type { Milestone } from '../../src/api/milestones'
import type {
  Issue,
  IssueStatusResponse,
  BatchIssueStatusResponse,
  QCStatus,
} from '../../src/api/issues'

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

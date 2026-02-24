import type { Page } from 'playwright/test'
import {
  defaultRepoInfo,
  openMilestone,
  awaitingReviewStatus,
  libIssue,
  externalIssue,
  defaultAssignees,
  defaultChecklists,
  rootFileTree,
  srcFileTree,
  createdMilestone,
  createIssueResponses,
} from '../fixtures/index'
import type { BatchIssueStatusResponse } from '../../src/api/issues'
import type { Milestone } from '../../src/api/milestones'
import type { Issue } from '../../src/api/issues'
import type { RepoInfo } from '../../src/api/repo'
import type { Assignee } from '../../src/api/assignees'
import type { Checklist } from '../../src/api/checklists'
import type { FileTreeResponse } from '../../src/api/files'
import type { CreateIssueResponse } from '../../src/api/create'
import type { CommentResponse } from '../../src/api/issues'

export interface RouteOverrides {
  repo: RepoInfo
  milestones: Milestone[]
  /** Map from milestone number to issue list */
  milestoneIssues: Record<number, Issue[]>
  /** Full batch response returned for /api/issues/status */
  issueStatuses: BatchIssueStatusResponse
  /** HTTP status code for /api/issues/status (default 200) */
  issueStatusesCode: number
  /** Checklists returned by GET /api/configuration */
  checklists: Checklist[]
  /** Assignees returned by /api/assignees */
  assignees: Assignee[]
  /** File tree responses keyed by path ('' for root, 'src' for src/, etc.) */
  fileTree: Record<string, FileTreeResponse>
  /** Milestone returned by POST /api/milestones */
  createMilestone: Milestone
  /** Issue responses returned by POST /api/milestones/:n/issues */
  createIssues: CreateIssueResponse[]
  /** Response for POST /api/issues/:n/comment; null → 500 error */
  postCommentResponse: CommentResponse | null
}

const defaultOverrides: RouteOverrides = {
  repo: defaultRepoInfo,
  milestones: [openMilestone],
  milestoneIssues: {
    1: [libIssue, externalIssue],
  },
  issueStatuses: {
    results: [awaitingReviewStatus],
    errors: [],
  },
  issueStatusesCode: 200,
  checklists: defaultChecklists,
  assignees: defaultAssignees,
  fileTree: { '': rootFileTree, src: srcFileTree },
  createMilestone: createdMilestone,
  createIssues: createIssueResponses,
  postCommentResponse: { comment_url: 'https://github.com/test-owner/test-repo/issues/71#issuecomment-99999' },
}

export async function setupRoutes(page: Page, overrides: Partial<RouteOverrides> = {}): Promise<void> {
  const cfg: RouteOverrides = { ...defaultOverrides, ...overrides }

  await page.route('/api/repo', (route) => {
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify(cfg.repo),
    })
  })

  await page.route('/api/milestones', (route, request) => {
    if (request.method() === 'POST') {
      route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify(cfg.createMilestone),
      })
    } else {
      route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify(cfg.milestones),
      })
    }
  })

  // Handle /api/milestones/:n/issues — GET returns issue list, POST returns create responses
  await page.route(/\/api\/milestones\/(\d+)\/issues/, (route, request) => {
    if (request.method() === 'POST') {
      route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify(cfg.createIssues),
      })
    } else {
      const url = request.url()
      const match = url.match(/\/api\/milestones\/(\d+)\/issues/)
      const milestoneNum = match ? Number(match[1]) : -1
      const issues = cfg.milestoneIssues[milestoneNum] ?? []
      route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify(issues),
      })
    }
  })

  await page.route(/\/api\/issues\/status/, (route) => {
    route.fulfill({
      status: cfg.issueStatusesCode,
      contentType: 'application/json',
      body: JSON.stringify(cfg.issueStatuses),
    })
  })

  await page.route('/api/configuration', (route, request) => {
    const configStatus = {
      directory: '/mock/config',
      exists: true,
      git_repository: null,
      options: {
        prepended_checklist_note: null,
        checklist_display_name: 'Code Review',
        logo_path: 'logo.png',
        logo_found: false,
        checklist_directory: 'checklists/',
        record_path: 'records/',
      },
      checklists: cfg.checklists,
      config_repo_env: null,
    }
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify(configStatus),
    })
  })

  await page.route('/api/assignees', (route) => {
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify(cfg.assignees),
    })
  })

  await page.route(/\/api\/files\/tree/, (route, request) => {
    const url = new URL(request.url())
    const path = url.searchParams.get('path') ?? ''
    const treeResponse = cfg.fileTree[path] ?? { path, entries: [] }
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify(treeResponse),
    })
  })

  await page.route(/\/api\/files\/content/, (route) => {
    route.fulfill({
      status: 200,
      contentType: 'text/plain',
      body: '// mock file content',
    })
  })

  await page.route(/\/api\/preview\/issue/, (route) => {
    route.fulfill({
      status: 200,
      contentType: 'text/html',
      body: '<p>Preview</p>',
    })
  })

  await page.route(/\/api\/preview\/\d+\/comment/, (route) => {
    route.fulfill({
      status: 200,
      contentType: 'text/html',
      body: '<p>Comment preview</p>',
    })
  })

  await page.route(/\/api\/issues\/\d+\/comment/, (route, request) => {
    if (request.method() === 'POST') {
      if (cfg.postCommentResponse) {
        route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify(cfg.postCommentResponse),
        })
      } else {
        route.fulfill({
          status: 500,
          contentType: 'application/json',
          body: JSON.stringify({ error: 'Internal server error' }),
        })
      }
    } else {
      route.continue()
    }
  })
}

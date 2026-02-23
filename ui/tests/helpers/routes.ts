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

export interface RouteOverrides {
  repo: RepoInfo
  milestones: Milestone[]
  /** Map from milestone number to issue list */
  milestoneIssues: Record<number, Issue[]>
  /** Full batch response returned for /api/issues/status */
  issueStatuses: BatchIssueStatusResponse
  /** HTTP status code for /api/issues/status (default 200) */
  issueStatusesCode: number
  /** Checklists returned by /api/configuration/checklists */
  checklists: Checklist[]
  /** Assignees returned by /api/assignees */
  assignees: Assignee[]
  /** File tree responses keyed by path ('' for root, 'src' for src/, etc.) */
  fileTree: Record<string, FileTreeResponse>
  /** Milestone returned by POST /api/milestones */
  createMilestone: Milestone
  /** Issue responses returned by POST /api/milestones/:n/issues */
  createIssues: CreateIssueResponse[]
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

  await page.route('/api/configuration/checklists', (route) => {
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify(cfg.checklists),
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
}

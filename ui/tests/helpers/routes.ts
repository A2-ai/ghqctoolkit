import type { Page } from 'playwright/test'
import {
  defaultRepoInfo,
  openMilestone,
  awaitingReviewIssue,
  awaitingReviewStatus,
} from '../fixtures/index'
import type { BatchIssueStatusResponse } from '../../src/api/issues'
import type { Milestone } from '../../src/api/milestones'
import type { Issue } from '../../src/api/issues'
import type { RepoInfo } from '../../src/api/repo'

export interface RouteOverrides {
  repo: RepoInfo
  milestones: Milestone[]
  /** Map from milestone number to issue list */
  milestoneIssues: Record<number, Issue[]>
  /** Full batch response returned for /api/issues/status */
  issueStatuses: BatchIssueStatusResponse
  /** HTTP status code for /api/issues/status (default 200) */
  issueStatusesCode: number
}

const defaultOverrides: RouteOverrides = {
  repo: defaultRepoInfo,
  milestones: [openMilestone],
  milestoneIssues: {
    1: [awaitingReviewIssue],
  },
  issueStatuses: {
    results: [awaitingReviewStatus],
    errors: [],
  },
  issueStatusesCode: 200,
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

  await page.route('/api/milestones', (route) => {
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify(cfg.milestones),
    })
  })

  // Handle /api/milestones/:n/issues for each configured milestone
  await page.route(/\/api\/milestones\/(\d+)\/issues/, (route, request) => {
    const url = request.url()
    const match = url.match(/\/api\/milestones\/(\d+)\/issues/)
    const milestoneNum = match ? Number(match[1]) : -1
    const issues = cfg.milestoneIssues[milestoneNum] ?? []
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify(issues),
    })
  })

  await page.route(/\/api\/issues\/status/, (route) => {
    route.fulfill({
      status: cfg.issueStatusesCode,
      contentType: 'application/json',
      body: JSON.stringify(cfg.issueStatuses),
    })
  })
}

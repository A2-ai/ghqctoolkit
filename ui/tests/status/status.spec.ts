import { test, expect } from 'playwright/test'
import { setupRoutes } from '../helpers/routes'
import {
  defaultRepoInfo,
  openMilestone,
  closedMilestone,
  awaitingReviewIssue,
  awaitingReviewStatus,
  changeRequestedIssue,
  changeRequestedStatus,
  inProgressIssue,
  inProgressStatus,
  approvedIssue,
  approvedStatus,
  milestone2Issue,
  milestone2Status,
  closedIssue,
  closedIssueStatus,
  dirtyIssue,
  dirtyStatus,
  cleanIssue,
  cleanStatus,
  partialIssue1,
  partialIssue2,
  partialIssue3,
  partialBatchResponse,
} from '../fixtures/index'

// ---------------------------------------------------------------------------
// Helper: select a milestone from the combobox in the sidebar
// ---------------------------------------------------------------------------
async function selectMilestone(page: import('playwright/test').Page, milestoneTitle: string) {
  await page.getByPlaceholder('Search milestones…').click()
  await page.getByRole('option', { name: new RegExp(milestoneTitle) }).click()
}

// ---------------------------------------------------------------------------
// Test 1: Issues placed in correct swimlanes
// ---------------------------------------------------------------------------
test('issues placed in correct swimlanes', async ({ page }) => {
  await setupRoutes(page, {
    milestoneIssues: {
      1: [awaitingReviewIssue, changeRequestedIssue, inProgressIssue, approvedIssue],
    },
    issueStatuses: {
      results: [awaitingReviewStatus, changeRequestedStatus, inProgressStatus, approvedStatus],
      errors: [],
    },
  })

  await page.goto('/')
  await selectMilestone(page, 'Sprint 1')

  // Locate each lane by its heading, then assert the card link is inside it
  const readyLane = page.locator('*').filter({ has: page.getByRole('heading', { name: 'Ready for Review' }) })
  await expect(readyLane.getByRole('link', { name: /src\/awaiting\.rs/ })).toBeVisible()

  const findingsLane = page.locator('*').filter({ has: page.getByRole('heading', { name: 'Findings to Address' }) })
  await expect(findingsLane.getByRole('link', { name: /src\/change\.rs/ })).toBeVisible()

  const changesLane = page.locator('*').filter({ has: page.getByRole('heading', { name: 'Changes to Notify' }) })
  await expect(changesLane.getByRole('link', { name: /src\/inprogress\.rs/ })).toBeVisible()

  const approvedLane = page.locator('*').filter({ has: page.getByRole('heading', { name: 'Approved' }) })
  await expect(approvedLane.getByRole('link', { name: /src\/approved\.rs/ })).toBeVisible()
})

// ---------------------------------------------------------------------------
// Test 2: Multi-milestone — issues from both appear
// ---------------------------------------------------------------------------
test('multi-milestone — issues from both appear', async ({ page }) => {
  await setupRoutes(page, {
    milestones: [openMilestone, { ...closedMilestone, state: 'open', title: 'Sprint 2', number: 2 }],
    milestoneIssues: {
      1: [awaitingReviewIssue],
      2: [milestone2Issue],
    },
    issueStatuses: {
      results: [awaitingReviewStatus, milestone2Status],
      errors: [],
    },
  })

  await page.goto('/')

  // Select milestone 1
  await selectMilestone(page, 'Sprint 1')

  // Select milestone 2
  await selectMilestone(page, 'Sprint 2')

  // Both issue links should be visible
  await expect(page.getByRole('link', { name: /src\/awaiting\.rs/ })).toBeVisible()
  await expect(page.getByRole('link', { name: /src\/milestone2\.rs/ })).toBeVisible()
})

// ---------------------------------------------------------------------------
// Test 3: Include closed milestones toggle
// ---------------------------------------------------------------------------
test('include closed milestones toggle shows closed milestone in dropdown', async ({ page }) => {
  await setupRoutes(page, {
    milestones: [openMilestone, closedMilestone],
  })

  await page.goto('/')

  // Open dropdown — closed milestone should NOT appear initially
  await page.getByPlaceholder('Search milestones…').click()
  await expect(page.getByRole('option', { name: /Sprint 0/ })).not.toBeVisible()
  // Close dropdown
  await page.keyboard.press('Escape')

  // Toggle "Include closed milestones"
  await page.getByRole('switch', { name: 'Include closed milestones' }).click()

  // Open dropdown again — closed milestone should now appear
  await page.getByPlaceholder('Search milestones…').click()
  await expect(page.getByRole('option', { name: /Sprint 0/ })).toBeVisible()
})

// ---------------------------------------------------------------------------
// Test 4: Include closed issues toggle
// ---------------------------------------------------------------------------
test('include closed issues toggle shows closed issue after toggle', async ({ page }) => {
  await setupRoutes(page, {
    milestoneIssues: {
      1: [awaitingReviewIssue, closedIssue],
    },
    issueStatuses: {
      results: [awaitingReviewStatus, closedIssueStatus],
      errors: [],
    },
  })

  await page.goto('/')
  await selectMilestone(page, 'Sprint 1')

  // Closed issue should NOT be visible (state === 'closed', toggle off)
  await expect(page.getByRole('link', { name: /src\/closed\.rs/ })).not.toBeVisible()

  // Toggle on
  await page.getByRole('switch', { name: 'Include closed issues' }).click()

  // Closed issue should now appear
  await expect(page.getByRole('link', { name: /src\/closed\.rs/ })).toBeVisible()
})

// ---------------------------------------------------------------------------
// Test 5: Dirty indicator from IssueStatusResponse.dirty
// ---------------------------------------------------------------------------
test('dirty indicator shown for dirty issue, not for clean issue', async ({ page }) => {
  await setupRoutes(page, {
    milestoneIssues: {
      1: [dirtyIssue, cleanIssue],
    },
    issueStatuses: {
      results: [dirtyStatus, cleanStatus],
      errors: [],
    },
  })

  await page.goto('/')
  await selectMilestone(page, 'Sprint 1')

  // Both cards are visible
  await expect(page.getByRole('link', { name: /src\/dirty\.rs/ })).toBeVisible()
  await expect(page.getByRole('link', { name: /src\/clean\.rs/ })).toBeVisible()

  // Dirty card should have the asterisk indicator; hover it to trigger the tooltip
  await page.getByRole('link', { name: /src\/dirty\.rs/ }).locator('xpath=../..').locator('[data-testid="dirty-indicator"]').hover()
  await expect(page.getByText('This file has uncommitted local changes')).toBeVisible()

  // Clean card should not have the dirty indicator at all
  await expect(page.getByRole('link', { name: /src\/clean\.rs/ }).locator('xpath=../..').locator('[data-testid="dirty-indicator"]')).not.toBeAttached()
})

// ---------------------------------------------------------------------------
// Test 6: Dirty indicator from RepoInfo.dirty_files
// ---------------------------------------------------------------------------
test('dirty indicator from RepoInfo.dirty_files marks matching issue', async ({ page }) => {
  const mainIssue = { ...awaitingReviewIssue, number: 60, title: 'src/main.rs' }
  const libIssue = { ...cleanIssue, number: 61, title: 'src/lib.rs' }

  const mainStatus = { ...awaitingReviewStatus, issue: mainIssue, dirty: false }
  const libStatus = { ...cleanStatus, issue: libIssue, dirty: false }

  await setupRoutes(page, {
    repo: { ...defaultRepoInfo, dirty_files: ['src/main.rs'] },
    milestoneIssues: {
      1: [mainIssue, libIssue],
    },
    issueStatuses: {
      results: [mainStatus, libStatus],
      errors: [],
    },
  })

  await page.goto('/')
  await selectMilestone(page, 'Sprint 1')

  // main.rs card: hover the dirty indicator to trigger tooltip
  await page.getByRole('link', { name: /src\/main\.rs/ }).locator('xpath=../..').locator('[data-testid="dirty-indicator"]').hover()
  await expect(page.getByText('This file has uncommitted local changes')).toBeVisible()

  // lib.rs card should not have the dirty indicator at all
  await expect(page.getByRole('link', { name: /src\/lib\.rs/ }).locator('xpath=../..').locator('[data-testid="dirty-indicator"]')).not.toBeAttached()
})

// ---------------------------------------------------------------------------
// Test 7: 206 partial status response
// ---------------------------------------------------------------------------
test('206 partial response — partial issues shown and milestone shows warning', async ({ page }) => {
  await setupRoutes(page, {
    milestoneIssues: {
      1: [partialIssue1, partialIssue2, partialIssue3],
    },
    issueStatuses: partialBatchResponse,
    issueStatusesCode: 206,
  })

  await page.goto('/')
  await selectMilestone(page, 'Sprint 1')

  // Issues 1 and 2 should appear
  await expect(page.getByRole('link', { name: /src\/partial1\.rs/ })).toBeVisible()
  await expect(page.getByRole('link', { name: /src\/partial2\.rs/ })).toBeVisible()

  // Issue 3 should NOT appear (it returned an error)
  await expect(page.getByRole('link', { name: /src\/partial3\.rs/ })).not.toBeVisible()

  // The selected milestone pill should show the partial warning icon
  await expect(page.locator('[data-testid="partial-warning"]')).toBeVisible()
})

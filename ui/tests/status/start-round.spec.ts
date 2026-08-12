// Start-new-round modal (S4) and the status-surface affordance (S6).
//
// Everything is driven through the Status tab: the affordance lives on the issue
// card, which is also the only place the modal is opened from in this slice.

import { test, expect, type Page } from 'playwright/test'
import { setupRoutes } from '../helpers/routes'
import {
  approvedRoundIssue,
  approvedRoundStatus,
  legacyRoundIssue,
  legacyRoundStatus,
  multiRoundIssue,
  multiRoundStatus,
  roundSeedCanStart,
  roundSeedBlocked,
  roundSeedAnchorUnresolved,
  startRoundSuccess,
  startRoundNeedsRepair,
  startRoundWithImpactedIssues,
  roundStillOpenError,
} from '../fixtures/index'
import type { Issue, IssueStatusResponse } from '../../src/api/issues'
import type { RouteOverrides } from '../helpers/routes'
import type { StartRoundRequest } from '../../src/api/rounds'

// ---------------------------------------------------------------------------
// Fixtures local to this spec: #111 approved, then the file changed again.
// ---------------------------------------------------------------------------

const changedIssue: Issue = { ...approvedRoundIssue, state: 'open', closed_at: null }

const changedAfterApprovalStatus: IssueStatusResponse = {
  ...approvedRoundStatus,
  issue: changedIssue,
  qc_status: {
    ...approvedRoundStatus.qc_status,
    status: 'changes_after_approval',
    status_detail: 'Approved; subsequent file changes',
  },
}

async function selectMilestone(page: Page, milestoneTitle: string) {
  await page.getByPlaceholder('Search milestones…').click()
  await page.getByRole('option', { name: new RegExp(milestoneTitle) }).click()
}

/** Loads the Status tab with #111 in `changes_after_approval` and opens the modal. */
async function openStartRoundModal(page: Page, overrides: Partial<RouteOverrides> = {}) {
  await setupRoutes(page, {
    milestoneIssues: { 1: [changedIssue] },
    issueStatuses: { results: [changedAfterApprovalStatus], errors: [] },
    ...overrides,
  })
  await page.goto('/')
  await selectMilestone(page, 'Sprint 1')
  await page.getByTestId('start-round-action-111').click()
  await expect(page.getByRole('heading', { name: 'Start New QC Round' })).toBeVisible()
}

/** Collects the bodies POSTed to /api/issues/:n/rounds. */
function captureStartRoundRequests(page: Page): StartRoundRequest[] {
  const bodies: StartRoundRequest[] = []
  page.on('request', (request) => {
    if (request.method() === 'POST' && /\/api\/issues\/\d+\/rounds$/.test(request.url())) {
      bodies.push(request.postDataJSON() as StartRoundRequest)
    }
  })
  return bodies
}

// ---------------------------------------------------------------------------
// S6: the status-surface affordance
// ---------------------------------------------------------------------------

test('S6: changes_after_approval shows a Start new round affordance that opens the modal', async ({ page }) => {
  await setupRoutes(page, {
    milestoneIssues: { 1: [changedIssue] },
    issueStatuses: { results: [changedAfterApprovalStatus], errors: [] },
  })

  await page.goto('/')
  await selectMilestone(page, 'Sprint 1')

  const action = page.getByTestId('start-round-action-111')
  await expect(action).toBeVisible()
  await expect(action).toHaveText('Start new round')

  await action.click()
  await expect(page.getByRole('heading', { name: 'Start New QC Round' })).toBeVisible()
  // Seeded from the round seed endpoint, for the issue that was clicked.
  await expect(page.getByTestId('next-round-name')).toHaveText('Round 2')
  await expect(page.getByTestId('start-round-submit')).toBeVisible()
})

test('S6: a single Initial QC round shows no round badge and no start-round action', async ({ page }) => {
  await setupRoutes(page, {
    milestoneIssues: { 1: [legacyRoundIssue] },
    issueStatuses: { results: [legacyRoundStatus], errors: [] },
  })

  await page.goto('/')
  await selectMilestone(page, 'Sprint 1')

  await expect(page.getByTestId('issue-card-110')).toBeVisible()
  await expect(page.getByTestId('round-badge-110')).toHaveCount(0)
  await expect(page.getByTestId('start-round-action-110')).toHaveCount(0)
})

test('S6: a multi-round issue shows a Round N badge', async ({ page }) => {
  await setupRoutes(page, {
    milestoneIssues: { 1: [multiRoundIssue] },
    issueStatuses: { results: [multiRoundStatus], errors: [] },
  })

  await page.goto('/')
  await selectMilestone(page, 'Sprint 1')

  await expect(page.getByTestId('round-badge-112')).toHaveText('Round 2')
})

// ---------------------------------------------------------------------------
// S4: blocked seeds — the action is not offered
// ---------------------------------------------------------------------------

test('blocked: last round still open — reason shown, no submit offered', async ({ page }) => {
  await openStartRoundModal(page, { roundSeedResponse: roundSeedBlocked })

  const blocked = page.getByTestId('round-blocked')
  await expect(blocked).toBeVisible()
  await expect(blocked).toContainText('Initial QC is still open')
  await expect(page.getByTestId('start-round-submit')).toHaveCount(0)
})

test('blocked: anchor could not be resolved — reason shown, no submit offered', async ({ page }) => {
  await openStartRoundModal(page, { roundSeedResponse: roundSeedAnchorUnresolved })

  const blocked = page.getByTestId('round-blocked')
  await expect(blocked).toBeVisible()
  await expect(blocked).toContainText('Could not resolve HEAD of branch')
  await expect(page.getByTestId('start-round-submit')).toHaveCount(0)
  // A null anchor renders as unavailable rather than crashing.
  await expect(page.getByTestId('round-anchor')).toContainText('not available')
})

// ---------------------------------------------------------------------------
// S4: notification mode reaches the request body
// ---------------------------------------------------------------------------

test('notification mode defaults to full and reaches the request body', async ({ page }) => {
  const bodies = captureStartRoundRequests(page)
  await openStartRoundModal(page)

  await page.getByTestId('start-round-submit').click()
  await expect(page.getByTestId('start-round-result')).toBeVisible()

  expect(bodies).toHaveLength(1)
  expect(bodies[0].notification).toBe('full')
  expect(bodies[0].checklist_content).toBe(roundSeedCanStart.checklist_content)
  expect(bodies[0].checklist_name).toBe('Code Review')
})

test('notification mode metadata_only reaches the request body', async ({ page }) => {
  const bodies = captureStartRoundRequests(page)
  await openStartRoundModal(page)

  await page.getByTestId('notification-mode').getByText('Metadata only').click()
  await page.getByTestId('start-round-submit').click()
  await expect(page.getByTestId('start-round-result')).toBeVisible()

  expect(bodies[0].notification).toBe('metadata_only')
})

test('notification mode none warns that the reviewer is not notified and reaches the request body', async ({ page }) => {
  const bodies = captureStartRoundRequests(page)
  await openStartRoundModal(page)

  await expect(page.getByTestId('notification-none-warning')).toHaveCount(0)
  await page.getByTestId('notification-mode').getByText('No notification').click()
  await expect(page.getByTestId('notification-none-warning')).toContainText('reviewer will not be notified')

  await page.getByTestId('start-round-submit').click()
  await expect(page.getByTestId('start-round-result')).toBeVisible()

  expect(bodies[0].notification).toBe('none')
})

// ---------------------------------------------------------------------------
// S4: the checklist editor
// ---------------------------------------------------------------------------

test('checklist editor is seeded, freely editable, and the edit reaches the request body', async ({ page }) => {
  const bodies = captureStartRoundRequests(page)
  await openStartRoundModal(page)

  const editor = page.locator('.mantine-Modal-body textarea')
  await expect(editor).toHaveValue(roundSeedCanStart.checklist_content!)

  await editor.fill('- [ ] Rewritten item\n- [ ] Second item')
  await page.getByTestId('start-round-submit').click()
  await expect(page.getByTestId('start-round-result')).toBeVisible()

  expect(bodies[0].checklist_content).toBe('- [ ] Rewritten item\n- [ ] Second item')
})

test('null checklist_content starts an empty editor with a note, and blocks submission', async ({ page }) => {
  await openStartRoundModal(page, {
    roundSeedResponse: { ...roundSeedCanStart, checklist_content: null, checklist_name: null },
  })

  await expect(page.getByTestId('no-prior-checklist')).toContainText('No prior checklist was found')
  await expect(page.locator('.mantine-Modal-body textarea')).toHaveValue('')
  // Nothing to review against yet, so the action stays unavailable until filled.
  await expect(page.getByTestId('start-round-submit')).toBeDisabled()
})

// ---------------------------------------------------------------------------
// S4: result handling
// ---------------------------------------------------------------------------

test('needs_repair renders as a success with the failed step visible and a repair hint', async ({ page }) => {
  await openStartRoundModal(page, { startRoundResponse: startRoundNeedsRepair })

  await page.getByTestId('start-round-submit').click()

  // Success framing, never an error.
  const success = page.getByTestId('start-round-success')
  await expect(success).toContainText('Round 2 started')
  await expect(page.getByTestId('start-round-error')).toHaveCount(0)
  await expect(page.getByTestId('round-still-open')).toHaveCount(0)
  await expect(page.getByTestId('round-comment-link')).toHaveAttribute(
    'href',
    startRoundNeedsRepair.round_comment_url,
  )

  // The repair path is spelled out as safe to re-run.
  await expect(page.getByTestId('needs-repair')).toContainText('safe')

  // Failed vs skipped vs done are distinguishable per step.
  await expect(page.getByTestId('step-reopened')).toContainText('Failed')
  await expect(page.getByTestId('step-reopened')).toContainText('403 Forbidden')
  await expect(page.getByTestId('step-notification')).toContainText('Skipped')
  await expect(page.getByTestId('step-body_marker')).toContainText('Done')
})

test('a 409 renders the round-already-open precondition, not a generic failure', async ({ page }) => {
  await openStartRoundModal(page, { startRoundResponse: roundStillOpenError })

  await page.getByTestId('start-round-submit').click()

  const precondition = page.getByTestId('round-still-open')
  await expect(precondition).toBeVisible()
  await expect(precondition).toContainText('Initial QC is still open')
  await expect(page.getByTestId('start-round-error')).toHaveCount(0)
  await expect(page.getByTestId('start-round-result')).toHaveCount(0)
})

// ---------------------------------------------------------------------------
// S4: impact preview
// ---------------------------------------------------------------------------

test('impacted issues render as information only', async ({ page }) => {
  await openStartRoundModal(page, { startRoundResponse: startRoundWithImpactedIssues })

  await page.getByTestId('start-round-submit').click()

  const list = page.getByTestId('impact-list')
  await expect(list).toBeVisible()
  await expect(list).toContainText('Information only — nothing was written to these issues')
  await expect(page.getByTestId('impact-issue-91')).toContainText('src/file_a.rs')
  await expect(page.getByTestId('impact-empty')).toHaveCount(0)
  await expect(page.getByTestId('impact-unavailable')).toHaveCount(0)
})

test('no downstream issues renders an empty-list message', async ({ page }) => {
  await openStartRoundModal(page, { startRoundResponse: startRoundSuccess })

  await page.getByTestId('start-round-submit').click()

  await expect(page.getByTestId('impact-empty')).toContainText('No downstream issues appear to be affected')
  await expect(page.getByTestId('impact-unavailable')).toHaveCount(0)
})

test('api_available false renders distinctly from an empty downstream list', async ({ page }) => {
  await openStartRoundModal(page, {
    startRoundResponse: {
      ...startRoundSuccess,
      impacted_issues: { api_available: false, issues: [] },
    },
  })

  await page.getByTestId('start-round-submit').click()

  await expect(page.getByTestId('impact-unavailable')).toContainText('could not be checked')
  await expect(page.getByTestId('impact-empty')).toHaveCount(0)
})

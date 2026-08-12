// Repairing an open QC round: the status-surface affordance, and the repair action
// in the round modal.
//
// The gap this covers: once a round is open the start-round action cannot be
// re-run, so a round whose follow-up steps failed can only be finished by the
// repair endpoint. The affordance must appear exactly when — and only when —
// `round_repair.needs_repair` says a step is actually incomplete.

import { test, expect, type Page } from 'playwright/test'
import { setupRoutes } from '../helpers/routes'
import {
  brokenRoundIssue,
  brokenRoundStatus,
  legacyRoundIssue,
  legacyRoundStatus,
  multiRoundIssue,
  multiRoundStatus,
  nothingToRepairError,
  quietRoundStatus,
  repairRoundNothingDone,
  repairRoundStillFailing,
  repairRoundSuccess,
  roundSeedBlocked,
  startRoundNeedsRepair,
} from '../fixtures/index'
import type { RouteOverrides } from '../helpers/routes'
import type { RepairRoundRequest } from '../../src/api/rounds'

async function selectMilestone(page: Page, milestoneTitle: string) {
  await page.getByPlaceholder('Search milestones…').click()
  await page.getByRole('option', { name: new RegExp(milestoneTitle) }).click()
}

/**
 * Loads the Status tab with #113, whose open Round 2 has incomplete steps.
 *
 * The issue is *closed* — that is what `reopen` being needed means — so the board's
 * closed-issue filter hides it until the switch is on. That is exactly the state a
 * start-round whose reopen failed leaves behind.
 */
async function openBrokenRoundBoard(page: Page, overrides: Partial<RouteOverrides> = {}) {
  await setupRoutes(page, {
    milestoneIssues: { 1: [brokenRoundIssue] },
    issueStatuses: { results: [brokenRoundStatus], errors: [] },
    // A round is open, so starting one is illegal — repair is the only action.
    roundSeedResponse: roundSeedBlocked,
    ...overrides,
  })
  await page.goto('/')
  await selectMilestone(page, 'Sprint 1')
  await page.getByRole('switch', { name: 'Include closed issues' }).click()
  await expect(page.getByTestId('issue-card-113')).toBeVisible()
}

/** Opens the round modal for #113 through its repair affordance. */
async function openRepairModal(page: Page, overrides: Partial<RouteOverrides> = {}) {
  await openBrokenRoundBoard(page, overrides)
  await page.getByTestId('repair-round-action-113').click()
  await expect(page.getByRole('heading', { name: 'Start New QC Round' })).toBeVisible()
}

/** Collects the bodies POSTed to /api/issues/:n/rounds/repair. */
function captureRepairRequests(page: Page): RepairRoundRequest[] {
  const bodies: RepairRoundRequest[] = []
  page.on('request', (request) => {
    if (request.method() === 'POST' && /\/api\/issues\/\d+\/rounds\/repair$/.test(request.url())) {
      bodies.push(request.postDataJSON() as RepairRoundRequest)
    }
  })
  return bodies
}

// ---------------------------------------------------------------------------
// The affordance appears only when a follow-up step is incomplete
// ---------------------------------------------------------------------------

test('an open round with incomplete follow-up steps offers a repair affordance', async ({ page }) => {
  await openBrokenRoundBoard(page)

  const action = page.getByTestId('repair-round-action-113')
  await expect(action).toBeVisible()
  await expect(action).toHaveText('Repair Round 2')
  // The start-round affordance is not offered: a round is already open.
  await expect(page.getByTestId('start-round-action-113')).toHaveCount(0)
})

test('a healthy open round offers no repair affordance', async ({ page }) => {
  await setupRoutes(page, {
    milestoneIssues: { 1: [multiRoundIssue] },
    issueStatuses: { results: [multiRoundStatus], errors: [] },
  })
  await page.goto('/')
  await selectMilestone(page, 'Sprint 1')

  await expect(page.getByTestId('issue-card-112')).toBeVisible()
  await expect(page.getByTestId('repair-round-action-112')).toHaveCount(0)
})

test('an un-notified but otherwise complete round offers no repair affordance', async ({ page }) => {
  await setupRoutes(page, {
    milestoneIssues: { 1: [multiRoundIssue] },
    issueStatuses: { results: [quietRoundStatus], errors: [] },
  })
  await page.goto('/')
  await selectMilestone(page, 'Sprint 1')

  // `notification_missing` is a fact, not a defect: nobody was told the round
  // exists because the author chose that, so there is nothing to repair.
  await expect(page.getByTestId('issue-card-112')).toBeVisible()
  await expect(page.getByTestId('repair-round-action-112')).toHaveCount(0)
})

test('a legacy single-round issue offers no repair affordance', async ({ page }) => {
  await setupRoutes(page, {
    milestoneIssues: { 1: [legacyRoundIssue] },
    issueStatuses: { results: [legacyRoundStatus], errors: [] },
  })
  await page.goto('/')
  await selectMilestone(page, 'Sprint 1')

  await expect(page.getByTestId('issue-card-110')).toBeVisible()
  await expect(page.getByTestId('repair-round-action-110')).toHaveCount(0)
})

// ---------------------------------------------------------------------------
// Repairing from the round modal
// ---------------------------------------------------------------------------

test('repair succeeds and reports each step', async ({ page }) => {
  const bodies = captureRepairRequests(page)
  await openRepairModal(page)

  // The modal explains what is incomplete instead of only saying "cannot start".
  const available = page.getByTestId('round-repair-available')
  await expect(available).toContainText('Round 2 is incomplete')
  await expect(available).toContainText('the issue was left closed')
  await expect(available).toContainText('never notifies anyone')

  await page.getByTestId('repair-round-submit').click()

  await expect(page.getByTestId('repair-success')).toContainText('Round 2 is now fully recorded')
  await expect(page.getByTestId('repair-still-failing')).toHaveCount(0)
  await expect(page.getByTestId('repair-error')).toHaveCount(0)
  await expect(page.getByTestId('repair-step-reopened')).toContainText('Done')
  await expect(page.getByTestId('repair-step-body_marker')).toContainText('Done')
  await expect(page.getByTestId('repair-step-notification')).toContainText('Skipped')

  // A repair never asks for a notification on its own.
  expect(bodies).toHaveLength(1)
  expect(bodies[0].notification).toBe('none')
})

test('a repair that reports a still-failing step is not presented as an error', async ({ page }) => {
  await openRepairModal(page, { repairRoundResponse: repairRoundStillFailing })

  await page.getByTestId('repair-round-submit').click()

  // Reported, per step, in the same success-shaped panel — never a red failure.
  const stillFailing = page.getByTestId('repair-still-failing')
  await expect(stillFailing).toBeVisible()
  await expect(stillFailing).toContainText('can be repeated once the cause is fixed')
  await expect(page.getByTestId('repair-error')).toHaveCount(0)
  await expect(page.getByTestId('nothing-to-repair')).toHaveCount(0)
  await expect(page.getByTestId('repair-step-reopened')).toContainText('Failed')
  await expect(page.getByTestId('repair-step-reopened')).toContainText('403 Forbidden')
  await expect(page.getByTestId('repair-step-body_marker')).toContainText('Done')
})

test('a repair that found nothing to do reads as a success', async ({ page }) => {
  await openRepairModal(page, { repairRoundResponse: repairRoundNothingDone })

  await page.getByTestId('repair-round-submit').click()

  await expect(page.getByTestId('repair-success')).toContainText('Nothing needed repairing')
  await expect(page.getByTestId('repair-error')).toHaveCount(0)
  // Skipped on the repair path means "already correct", not "deliberately not run".
  await expect(page.getByTestId('repair-step-reopened')).toContainText('Already correct')
})

test('a 409 renders the nothing-to-repair precondition, not a generic failure', async ({ page }) => {
  await openRepairModal(page, { repairRoundResponse: nothingToRepairError })

  await page.getByTestId('repair-round-submit').click()

  const precondition = page.getByTestId('nothing-to-repair')
  await expect(precondition).toBeVisible()
  await expect(precondition).toContainText('Nothing to repair')
  await expect(precondition).toContainText('no round is open on this issue')
  await expect(page.getByTestId('repair-error')).toHaveCount(0)
  await expect(page.getByTestId('repair-result')).toHaveCount(0)
})

test('a repair transport failure renders as an error', async ({ page }) => {
  await openRepairModal(page, { repairRoundResponse: null })

  await page.getByTestId('repair-round-submit').click()

  await expect(page.getByTestId('repair-error')).toContainText('Internal server error')
  await expect(page.getByTestId('nothing-to-repair')).toHaveCount(0)
})

// ---------------------------------------------------------------------------
// Retrying straight after a start whose follow-up steps failed
// ---------------------------------------------------------------------------

/** #111 is approved, so a round may be started; its start then partly fails. */
async function startRoundWithFailedStep(page: Page, overrides: Partial<RouteOverrides> = {}) {
  const { approvedRoundIssue, approvedRoundStatus } = await import('../fixtures/index')
  const changedIssue = { ...approvedRoundIssue, state: 'open' as const, closed_at: null }
  await setupRoutes(page, {
    milestoneIssues: { 1: [changedIssue] },
    issueStatuses: {
      results: [
        {
          ...approvedRoundStatus,
          issue: changedIssue,
          qc_status: { ...approvedRoundStatus.qc_status, status: 'changes_after_approval' as const },
        },
      ],
      errors: [],
    },
    startRoundResponse: startRoundNeedsRepair,
    ...overrides,
  })
  await page.goto('/')
  await selectMilestone(page, 'Sprint 1')
  await page.getByTestId('start-round-action-111').click()
  await page.getByTestId('start-round-submit').click()
  await expect(page.getByTestId('start-round-result')).toBeVisible()
}

test('the needs-repair panel offers a retry instead of claiming a re-run is safe', async ({ page }) => {
  await startRoundWithFailedStep(page)

  const needsRepair = page.getByTestId('needs-repair')
  // The corrected copy: re-running the round action is explicitly *not* the remedy.
  await expect(needsRepair).toContainText('Starting the round again would not retry them')
  await expect(needsRepair).toContainText('extend it')
  await expect(needsRepair).toContainText('Retry follow-up steps')
  await expect(page.getByTestId('retry-follow-up-steps')).toBeVisible()
})

test('Retry follow-up steps repairs the round without re-posting it', async ({ page }) => {
  const repairBodies = captureRepairRequests(page)
  const startBodies: unknown[] = []
  page.on('request', (request) => {
    if (request.method() === 'POST' && /\/api\/issues\/\d+\/rounds$/.test(request.url())) {
      startBodies.push(request.postDataJSON())
    }
  })

  await startRoundWithFailedStep(page, { repairRoundResponse: repairRoundSuccess })
  expect(startBodies).toHaveLength(1)

  await page.getByTestId('retry-follow-up-steps').click()

  await expect(page.getByTestId('repair-success')).toContainText('fully recorded')
  await expect(page.getByTestId('start-round-success')).toBeVisible()
  // Exactly one repair, and no second round comment.
  expect(repairBodies).toHaveLength(1)
  expect(startBodies).toHaveLength(1)
  // The failed step here was the reopen, not the notification, so none is asked for.
  expect(repairBodies[0].notification).toBe('none')
})

test('a failed notification step is retried with the mode the start asked for', async ({ page }) => {
  const repairBodies = captureRepairRequests(page)

  await startRoundWithFailedStep(page, {
    startRoundResponse: {
      ...startRoundNeedsRepair,
      reopened: { status: 'done' },
      notification: { status: 'failed', error: 'Could not post the notification' },
    },
  })

  await page.getByTestId('retry-follow-up-steps').click()
  await expect(page.getByTestId('repair-result')).toBeVisible()

  // The author did ask to notify and it did not land, so the retry asks again.
  expect(repairBodies[0].notification).toBe('full')
})

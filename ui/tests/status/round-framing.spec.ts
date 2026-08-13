// P4: the user-facing language of the two approval-lifecycle actions.
//
// "Start new round" is an append — the previous approval stays true, downstream QCs
// that relied on it still stand — while "Retract approval" is an amend that says a
// past approval was wrong and so may invalidate what depended on it. These tests
// pin that the two surfaces read differently, and that neither uses "re-open"
// language, which collides with GitHub's own issue-reopen and made routine work
// look like an alarm.
//
// Scope is deliberately narrow: only the two surfaces this phase reworded — the
// retract-approval tab and the start-new-round modal — so GitHub's own terminology
// elsewhere in the app cannot false-positive here.

import { test, expect, type Page } from 'playwright/test'
import { setupRoutes } from '../helpers/routes'
import {
  approvedModalIssue,
  approvedModalStatus,
  approvedChildBlocked,
  approvedRoundIssue,
  approvedRoundStatus,
  openMilestone,
  startRoundWithImpactedIssues,
} from '../fixtures/index'
import type { Issue, IssueStatusResponse } from '../../src/api/issues'

/** Any spelling of GitHub's "reopen", which must not appear on these surfaces. */
const REOPEN_LANGUAGE = /re-?open/i

async function selectSprint1(page: Page) {
  await page.getByPlaceholder('Search milestones…').click()
  await page.getByRole('option', { name: /Sprint 1/ }).click()
}

/** Status tab → #42's detail modal, which defaults to the retract-approval tab. */
async function openRetractTab(page: Page) {
  await setupRoutes(page, {
    milestones: [openMilestone],
    milestoneIssues: { 1: [approvedModalIssue] },
    issueStatuses: { results: [approvedModalStatus], errors: [] },
    blockedResponse: [approvedChildBlocked],
  })
  await page.goto('/')
  await selectSprint1(page)
  await page.getByTestId(`issue-card-${approvedModalIssue.number}`).click()
  const panel = page.getByRole('tabpanel', { name: 'Retract approval' })
  await expect(panel).toBeVisible()
  return panel
}

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

/** Status tab → #111's "Start new round" affordance → the start-round modal. */
async function openStartRoundModal(page: Page) {
  await setupRoutes(page, {
    milestoneIssues: { 1: [changedIssue] },
    issueStatuses: { results: [changedAfterApprovalStatus], errors: [] },
    startRoundResponse: startRoundWithImpactedIssues,
  })
  await page.goto('/')
  await selectSprint1(page)
  await page.getByTestId('start-round-action-111').click()
  await expect(page.getByRole('heading', { name: 'Start New QC Round' })).toBeVisible()
}

// ---------------------------------------------------------------------------
// "re-open" must not appear on either surface
// ---------------------------------------------------------------------------

test('retract-approval surface uses no re-open language', async ({ page }) => {
  const panel = await openRetractTab(page)
  expect(await panel.innerText()).not.toMatch(REOPEN_LANGUAGE)
  await expect(page.getByRole('tab', { name: 'Retract approval', exact: true })).toBeVisible()
})

test('start-new-round surface uses no re-open language, including its step list', async ({ page }) => {
  await openStartRoundModal(page)
  const modal = page.getByRole('dialog')
  expect(await modal.innerText()).not.toMatch(REOPEN_LANGUAGE)

  // The result panel names the follow-up steps, one of which sets the issue open.
  await page.getByTestId('start-round-submit').click()
  await expect(page.getByTestId('start-round-result')).toBeVisible()
  await expect(page.getByTestId('step-reopened')).toContainText('Set the issue back to open')
  expect(await modal.innerText()).not.toMatch(REOPEN_LANGUAGE)
})

// ---------------------------------------------------------------------------
// The two framings are distinguishable to a user who sees them a week apart
// ---------------------------------------------------------------------------

test('retraction reads as invalidation; a new round reads as a notice', async ({ page }) => {
  const panel = await openRetractTab(page)
  const retractText = await panel.getByTestId('retract-guidance').innerText()
  // Amend: the approval was wrong, and what relied on it may not survive.
  expect(retractText).toMatch(/wrong/i)
  expect(retractText).toMatch(/no longer be valid/i)
  expect(retractText).toMatch(/redone/i)
  // …and it points at the other action rather than leaving the user to guess.
  expect(retractText).toMatch(/Start new round/i)
  // Never the new-round promise: retraction does not leave approvals standing.
  expect(retractText).not.toMatch(/still stands/i)

  await openStartRoundModal(page)
  const roundText = await page.getByTestId('new-round-guidance').innerText()
  // Append: routine, blameless, and nothing downstream is invalidated.
  expect(roundText).toMatch(/append/i)
  expect(roundText).toMatch(/remains valid/i)
  expect(roundText).toMatch(/nothing was wrong/i)
  expect(roundText).toMatch(/Retract approval/i)
  expect(roundText).not.toMatch(/redone/i)

  // The two impact readouts are worded apart too: a notice, not an invalidation.
  await page.getByTestId('start-round-submit').click()
  const impact = await page.getByTestId('impact-list').innerText()
  expect(impact).toMatch(/Notice only/i)
  expect(impact).toMatch(/previous approval still stands/i)
  expect(impact).not.toMatch(/invalid|redone/i)
})

// The user-facing language of the two approval-lifecycle actions.
//
// "Start new round" is an append — the previous approval stays true, downstream QCs
// that relied on it still stand — while "Unapprove" is an amend that says a past
// approval was wrong and so may invalidate what depended on it. These tests pin
// that the two surfaces read differently, and that neither uses "re-open" language,
// which collides with GitHub's own issue-reopen and made routine work look like an
// alarm.
//
// The orientation notes on both surfaces are deliberately one line each: the heavy
// lifting is done by having two separate actions and by the impact readouts, which
// are where the real difference has to be legible. So the assertions below are
// light on the notes and firm on the impact lists.
//
// Scope is deliberately narrow: only the two surfaces this phase reworded — the
// unapprove tab and the start-new-round modal — so GitHub's own terminology
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

/** Status tab → #42's detail modal, which defaults to the unapprove tab. */
async function openUnapproveTab(page: Page) {
  await setupRoutes(page, {
    milestones: [openMilestone],
    milestoneIssues: { 1: [approvedModalIssue] },
    issueStatuses: { results: [approvedModalStatus], errors: [] },
    blockedResponse: [approvedChildBlocked],
  })
  await page.goto('/')
  await selectSprint1(page)
  await page.getByTestId(`issue-card-${approvedModalIssue.number}`).click()
  const panel = page.getByRole('tabpanel', { name: 'Unapprove' })
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

test('unapprove surface uses no re-open language', async ({ page }) => {
  const panel = await openUnapproveTab(page)
  expect(await panel.innerText()).not.toMatch(REOPEN_LANGUAGE)
  await expect(page.getByRole('tab', { name: 'Unapprove', exact: true })).toBeVisible()
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
// The two framings stay distinguishable
// ---------------------------------------------------------------------------

test('each surface names the other action without borrowing its promise', async ({ page }) => {
  const panel = await openUnapproveTab(page)
  const note = await panel.getByTestId('unapprove-note').innerText()
  // Points at the other action rather than leaving the user to guess…
  expect(note).toMatch(/start a new round/i)
  // …but never borrows the new-round promise: unapproving does not leave
  // approvals standing, which is the whole reason the two actions are separate.
  expect(note).not.toMatch(/stays valid|still stands/i)

  await openStartRoundModal(page)
  const roundNote = await page.getByTestId('new-round-guidance').innerText()
  // The one fact a user cannot derive from the form itself.
  expect(roundNote).toMatch(/stays valid/i)
  // And never the invalidation vocabulary, or routine work reads as an alarm.
  expect(roundNote).not.toMatch(/redone|no longer be valid|wrong/i)
})

test('the impact readouts are worded apart: a notice, not an invalidation', async ({ page }) => {
  await openStartRoundModal(page)
  await page.getByTestId('start-round-submit').click()
  const impact = await page.getByTestId('impact-list').innerText()
  expect(impact).toMatch(/Notice only/i)
  expect(impact).toMatch(/previous approval still stands/i)
  expect(impact).not.toMatch(/invalid|redone/i)
})

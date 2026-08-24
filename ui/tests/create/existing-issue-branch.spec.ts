import { test, expect } from 'playwright/test'
import type { Page } from 'playwright/test'
import { setupRoutes } from '../helpers/routes'
import { libIssue, openMilestone } from '../fixtures'
import type { Issue } from '../../src/api/issues'

// ---------------------------------------------------------------------------
// D52 — the branch label says which round it belongs to when rounds exist
//
// The create tab's existing-issue list is a rounds-less context: it never fetches
// `rounds[]`, so `issue.branch` is all it has. Per D50 that value is only the
// *current* branch when the body carries no `## QC Rounds` marker.
// ---------------------------------------------------------------------------

async function openMilestoneIssues(page: Page, issues: Issue[]) {
  await setupRoutes(page, {
    milestones: [openMilestone],
    milestoneIssues: { 1: issues },
    issueStatuses: { results: [], errors: [] },
  })
  await page.goto('/')
  await page.getByRole('button', { name: 'Create' }).click()
  await page.getByPlaceholder('Select a milestone').click()
  await page.getByRole('option', { name: /Sprint 1/ }).click()
}

test('D52: no marker ⇒ the branch is current and labelled plainly', async ({ page }) => {
  await openMilestoneIssues(page, [{ ...libIssue, branch: 'main', has_qc_rounds_marker: false }])

  const branch = page.getByTestId('existing-issue-branch')
  await expect(branch).toHaveText('Branch: main')
})

test('D52: marker present ⇒ the branch is labelled as round 1\'s, not current', async ({ page }) => {
  await openMilestoneIssues(page, [{ ...libIssue, branch: 'main', has_qc_rounds_marker: true }])

  // Still rendered (D50 reversed its removal) — but no longer presented as current.
  const branch = page.getByTestId('existing-issue-branch')
  await expect(branch).toHaveText('Branch (round 1): main')
})

// P3 / S1-S3, S5, S7: the round rail, the membership-scoped commit picker,
// draft-gap collapse, the `next_notification_from` default and the comparison
// receipt — all inside IssueDetailModal.

import { test, expect, type Page } from 'playwright/test'
import { setupRoutes } from '../helpers/routes'
import {
  openMilestone,
  legacyRoundIssue,
  legacyRoundStatus,
  multiRoundIssue,
  multiRoundStatus,
  ROUND1_OPENED,
  ROUND1_CLOSED,
  DRAFT_GAP_COMMIT,
  ROUND2_OPENED,
} from '../fixtures/index'
import type { Issue, IssueStatusResponse } from '../../src/api/issues'

const SHORT = (hash: string) => hash.slice(0, 7)

async function openModal(page: Page, issue: Issue, status: IssueStatusResponse) {
  await setupRoutes(page, {
    milestones: [openMilestone],
    milestoneIssues: { 1: [issue] },
    issueStatuses: { results: [status], errors: [] },
  })
  await page.goto('/')
  await page.getByPlaceholder('Search milestones…').click()
  await page.getByRole('option', { name: /Sprint 1/ }).click()
  await page.getByTestId(`issue-card-${issue.number}`).click()
  await expect(page.getByRole('tablist')).toBeVisible()
}

/** Opens the modal on the Notify tab and returns that panel. */
async function openNotify(page: Page, issue: Issue, status: IssueStatusResponse) {
  await openModal(page, issue, status)
  await page.getByRole('tab', { name: 'Notify', exact: true }).click()
  const panel = page.getByRole('tabpanel', { name: 'Notify' })
  await expect(panel).toBeVisible()
  return panel
}

// ── Local fixture variants ───────────────────────────────────────────────────

/**
 * Round 2 open with no notification of its own yet, so the API's
 * `next_notification_from` is the previous round's approval and the default
 * already spans the whole round.
 */
const roundTwoNoNotification: IssueStatusResponse = {
  ...multiRoundStatus,
  commits: multiRoundStatus.commits.map((c) =>
    c.hash === ROUND2_OPENED ? { ...c, statuses: [] } : c,
  ),
  rounds: [multiRoundStatus.rounds[0], { ...multiRoundStatus.rounds[1], event_count: 0 }],
  next_notification_from: ROUND1_CLOSED,
}

/** A comment-sourced round whose comment URL is absent (cache-loaded comment). */
const roundTwoNoCommentUrl: IssueStatusResponse = {
  ...multiRoundStatus,
  rounds: [
    multiRoundStatus.rounds[0],
    {
      ...multiRoundStatus.rounds[1],
      checklist_source: { kind: 'comment', comment_id: null, comment_url: null },
    },
  ],
}

// ---------------------------------------------------------------------------
// S1: round rail
// ---------------------------------------------------------------------------

test('S1: a single Initial QC round renders as one quiet line, with no extra chrome', async ({ page }) => {
  const panel = await openNotify(page, legacyRoundIssue, legacyRoundStatus)
  const rail = panel.getByTestId('round-rail')

  await expect(rail.getByTestId('round-line-1')).toBeVisible()
  await expect(rail).toContainText('Initial QC')
  // No accordion sections, no scope note, no draft-gap row: nothing that a
  // pre-rounds legacy issue did not already have.
  await expect(panel.getByTestId('round-section-1')).toHaveCount(0)
  await expect(panel.getByTestId('picker-scope')).toHaveCount(0)
  await expect(panel.getByTestId('draft-gap-row')).toHaveCount(0)
  await expect(panel.getByTestId('round-rail-start')).toHaveCount(0)
})

test('S1: both rounds appear in the rail, newest expanded', async ({ page }) => {
  const panel = await openNotify(page, multiRoundIssue, multiRoundStatus)
  const rail = panel.getByTestId('round-rail')

  await expect(rail.getByTestId('round-section-1')).toBeVisible()
  await expect(rail.getByTestId('round-section-2')).toBeVisible()
  await expect(rail).toContainText('Initial QC')
  await expect(rail).toContainText('Round 2')

  // Newest expanded by default, oldest collapsed.
  await expect(rail.getByTestId('round-toggle-2')).toHaveAttribute('aria-expanded', 'true')
  await expect(rail.getByTestId('round-toggle-1')).toHaveAttribute('aria-expanded', 'false')
  await expect(rail.getByTestId('round-detail-2')).toBeVisible()
  await expect(rail.getByTestId('round-detail-1')).not.toBeVisible()

  // Round 2 states its anchor, what it compares against, and its event count.
  await expect(rail.getByTestId('round-detail-2')).toContainText(SHORT(ROUND2_OPENED))
  await expect(rail.getByTestId('round-detail-2')).toContainText(SHORT(ROUND1_CLOSED))
  await expect(rail.getByTestId('round-counts-2')).toContainText('1 event')
})

test('S1: expanding the older round reveals its approval receipt', async ({ page }) => {
  const panel = await openNotify(page, multiRoundIssue, multiRoundStatus)
  const rail = panel.getByTestId('round-rail')

  await rail.getByTestId('round-toggle-1').click()
  const detail = rail.getByTestId('round-detail-1')
  await expect(detail).toBeVisible()
  await expect(detail.getByTestId('round-closed-1')).toContainText(SHORT(ROUND1_CLOSED))
  await expect(detail.getByTestId('round-closed-1')).toContainText('reviewer1')
})

test('S1: a comment-sourced round with no comment_url renders the checklist as plain text', async ({ page }) => {
  const panel = await openNotify(page, multiRoundIssue, roundTwoNoCommentUrl)
  const checklist = panel.getByTestId('round-rail').getByTestId('round-checklist-2')

  await expect(checklist).toContainText('Code Review')
  await expect(checklist.getByRole('link')).toHaveCount(0)
})

test('S1: a comment-sourced round with a comment_url links to it', async ({ page }) => {
  const panel = await openNotify(page, multiRoundIssue, multiRoundStatus)
  const link = panel.getByTestId('round-rail').getByTestId('round-checklist-2').getByRole('link')

  await expect(link).toHaveAttribute('href', /issuecomment-5002/)
})

// ---------------------------------------------------------------------------
// S2: membership-scoped picker
// ---------------------------------------------------------------------------

test('S2: with Round 2 open, only its membership is offered by default', async ({ page }) => {
  const panel = await openNotify(page, multiRoundIssue, multiRoundStatus)
  const track = panel.getByTestId('notify-picker')

  await expect(panel.getByTestId('picker-scope')).toContainText('scoped to Round 2')
  await expect(track.getByText(SHORT(ROUND2_OPENED))).toBeVisible()
  // Initial QC's commits and the draft-gap commit are all out of scope.
  await expect(track.getByText(SHORT(DRAFT_GAP_COMMIT))).toHaveCount(0)
  await expect(track.getByText(SHORT(ROUND1_OPENED))).toHaveCount(0)
})

test('S2: Show all commits widens the track to the full history', async ({ page }) => {
  const panel = await openNotify(page, multiRoundIssue, multiRoundStatus)
  const track = panel.getByTestId('notify-picker')

  await panel.getByLabel('Show all commits').check()

  await expect(track.getByText(SHORT(ROUND1_OPENED))).toBeVisible()
  await expect(track.getByText(SHORT(ROUND1_CLOSED))).toBeVisible()
  await expect(panel.getByTestId('picker-scope')).toHaveCount(0)
})

test('S2: the Review and Approve pickers are scoped to the open round too', async ({ page }) => {
  await openModal(page, multiRoundIssue, multiRoundStatus)

  for (const [tab, testId] of [['Review', 'review-picker'], ['Approve', 'approve-picker']] as const) {
    await page.getByRole('tab', { name: tab, exact: true }).click()
    const panel = page.getByRole('tabpanel', { name: tab })
    await expect(panel.getByTestId('picker-scope')).toContainText('scoped to Round 2')
    await expect(panel.getByTestId(testId).getByText(SHORT(DRAFT_GAP_COMMIT))).toHaveCount(0)
  }
})

test('S2: a legacy single-round picker is not scoped and still offers every commit', async ({ page }) => {
  const panel = await openNotify(page, legacyRoundIssue, legacyRoundStatus)

  await expect(panel.getByTestId('picker-scope')).toHaveCount(0)
  await expect(panel.getByTestId('notify-picker').getByText(SHORT(ROUND1_OPENED))).toBeVisible()
  // From and To both land on the only commit, exactly as before rounds existed.
  await expect(panel.locator('text=From:').locator('..').getByText(SHORT(ROUND1_OPENED))).toBeVisible()
  await expect(panel.locator('text=To:').locator('..').getByText(SHORT(ROUND1_OPENED))).toBeVisible()
})

// ---------------------------------------------------------------------------
// S3: draft-gap collapse
// ---------------------------------------------------------------------------

test('S3: the draft-gap run collapses to a marker in full-history mode and expands on click', async ({ page }) => {
  const panel = await openNotify(page, multiRoundIssue, multiRoundStatus)
  const track = panel.getByTestId('notify-picker')

  // No marker while scoped — the gap is outside the window entirely.
  await expect(panel.getByTestId('draft-gap-row')).toHaveCount(0)

  await panel.getByLabel('Show all commits').check()

  const marker = panel.getByTestId('draft-gap-2')
  await expect(marker).toBeVisible()
  await expect(marker).toContainText('1 commit')
  // Collapsed: the gap commit is not on the track yet.
  await expect(track.getByText(SHORT(DRAFT_GAP_COMMIT))).toHaveCount(0)

  await marker.click()
  await expect(track.getByText(SHORT(DRAFT_GAP_COMMIT))).toBeVisible()

  // And it collapses back.
  await marker.click()
  await expect(track.getByText(SHORT(DRAFT_GAP_COMMIT))).toHaveCount(0)
})

test('S3: a single-round issue has no draft-gap markers even in full-history mode', async ({ page }) => {
  const panel = await openNotify(page, legacyRoundIssue, legacyRoundStatus)
  await panel.getByLabel('Show all commits').check()
  await expect(panel.getByTestId('draft-gap-row')).toHaveCount(0)
})

// ---------------------------------------------------------------------------
// S5: the `next_notification_from` default
// ---------------------------------------------------------------------------

test('S5: the notify from-commit comes from next_notification_from', async ({ page }) => {
  const panel = await openNotify(page, multiRoundIssue, roundTwoNoNotification)

  // next_notification_from is Initial QC's approval, not Round 2's anchor.
  await expect(panel.locator('text=From:').locator('..').getByText(SHORT(ROUND1_CLOSED))).toBeVisible()
  await expect(panel.locator('text=To:').locator('..').getByText(SHORT(ROUND2_OPENED))).toBeVisible()
})

test('S5: the whole-round default is labelled "Since <previous round> approval"', async ({ page }) => {
  const panel = await openNotify(page, multiRoundIssue, roundTwoNoNotification)

  const label = panel.getByTestId('since-previous-approval')
  await expect(label).toContainText('Since Initial QC approval')
  await expect(label).toContainText('Round 2')
})

test('S5: no whole-round label when the default does not span the round', async ({ page }) => {
  // multiRoundStatus's next_notification_from is Round 2's own anchor.
  const panel = await openNotify(page, multiRoundIssue, multiRoundStatus)
  await expect(panel.getByTestId('since-previous-approval')).toHaveCount(0)
})

test('S5: an empty diff is called out and offers a one-click whole-round preset', async ({ page }) => {
  const panel = await openNotify(page, multiRoundIssue, multiRoundStatus)

  const alert = panel.getByTestId('empty-diff-alert')
  await expect(alert).toBeVisible()
  await expect(alert).toContainText('Nothing to compare')
  await expect(alert).toContainText(SHORT(ROUND2_OPENED))
  await expect(alert).toContainText('retraction')

  const preset = panel.getByTestId('present-whole-round')
  await expect(preset).toContainText('Initial QC')
  await expect(preset).toContainText(SHORT(ROUND1_CLOSED))

  await preset.click()

  await expect(panel.getByTestId('empty-diff-alert')).toHaveCount(0)
  await expect(panel.locator('text=From:').locator('..').getByText(SHORT(ROUND1_CLOSED))).toBeVisible()
  await expect(panel.locator('text=To:').locator('..').getByText(SHORT(ROUND2_OPENED))).toBeVisible()
})

test('S5: a legacy single-commit issue with from === to shows no empty-diff alert', async ({ page }) => {
  // from === to is an ordinary state here, and there is no earlier approval to
  // offer, so surfacing it would be pure noise.
  const panel = await openNotify(page, legacyRoundIssue, legacyRoundStatus)
  await expect(panel.getByTestId('empty-diff-alert')).toHaveCount(0)
})

// ---------------------------------------------------------------------------
// S7: comparison receipt
// ---------------------------------------------------------------------------

test('S7: the receipt shows the selected range and the number of commits spanned', async ({ page }) => {
  const panel = await openNotify(page, multiRoundIssue, roundTwoNoNotification)

  const receipt = panel.getByTestId('comparison-receipt')
  await expect(receipt).toContainText(SHORT(ROUND1_CLOSED))
  await expect(receipt).toContainText(SHORT(ROUND2_OPENED))
  // b2b2b2b → d4d4d4d spans the draft-gap commit and Round 2's anchor.
  await expect(receipt).toContainText('2 commits')
})

test('S7: the receipt reports an empty range as 0 commits', async ({ page }) => {
  const panel = await openNotify(page, multiRoundIssue, multiRoundStatus)
  const receipt = panel.getByTestId('comparison-receipt')

  await expect(receipt).toContainText(`${SHORT(ROUND2_OPENED)}`)
  await expect(receipt).toContainText('0 commits')
})

test('S7: the single-handle Review picker gets a receipt against the round anchor', async ({ page }) => {
  await openModal(page, multiRoundIssue, multiRoundStatus)
  await page.getByRole('tab', { name: 'Review', exact: true }).click()
  const panel = page.getByRole('tabpanel', { name: 'Review' })

  await expect(panel.getByTestId('comparison-receipt')).toContainText(SHORT(ROUND2_OPENED))
})

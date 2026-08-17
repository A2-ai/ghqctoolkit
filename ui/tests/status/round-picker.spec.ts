// P3 / S1-S3, S5, S7 and U1/U2: the round rail, the segment-scoped commit picker,
// gap collapse, the `next_notification_from` default and the comparison receipt —
// all inside IssueDetailModal.

import { test, expect, type Page } from 'playwright/test'
import { setupRoutes } from '../helpers/routes'
import {
  openMilestone,
  legacyRoundIssue,
  legacyRoundStatus,
  multiRoundIssue,
  multiRoundStatus,
  approvedRoundIssue,
  approvedRoundStatus,
  ROUND1_OPENED,
  ROUND1_CLOSED,
  DRAFT_GAP_COMMIT,
  ROUND2_OPENED,
  commit,
  crossBranchIssue,
  crossBranchSegments,
  crossBranchStatus,
  multiRoundSegments,
  segmentFields,
  unplaceableIssue,
  unplaceableStatus,
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
  ...segmentFields(
    multiRoundSegments({
      round2: {
        events: [],
        commits: [commit(ROUND2_OPENED, { message: 'round 2 changes' })],
      },
    }),
    ROUND1_CLOSED,
  ),
}

/** A comment-sourced round whose comment URL is absent (cache-loaded comment). */
const roundTwoNoCommentUrl: IssueStatusResponse = {
  ...multiRoundStatus,
  ...segmentFields(
    multiRoundSegments({
      round2: { checklist_source: { kind: 'comment', comment_id: null, comment_url: null } },
    }),
  ),
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
  // No start-round action: this issue's only round is still open, and a new round
  // builds on an approval. It is offered once the round closes — see start-round.spec.
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

/**
 * U2: the rail draws a branch line where a segment's branch *differs* from the
 * previous segment's. Every round declares a branch unconditionally now (D5), so
 * printing it on all of them would repeat one name down the whole rail — the fact
 * worth stating is the move.
 */
test('U2: the rail marks the branch a round moved to', async ({ page }) => {
  const panel = await openNotify(page, crossBranchIssue, crossBranchStatus)
  const rail = panel.getByTestId('round-rail')

  // Round 2 sits at position 2, and the gap before it at position 1, both on the
  // new branch — so the move is announced once, at the gap where it happened.
  await expect(rail.getByTestId('segment-branch-1')).toContainText('feature/reanalysis')
  await expect(rail.getByTestId('segment-branch-2')).toHaveCount(0)
})

test('U2: a rail whose every segment shares one branch shows no branch line', async ({ page }) => {
  const panel = await openNotify(page, multiRoundIssue, multiRoundStatus)
  const rail = panel.getByTestId('round-rail')

  await expect(rail.getByTestId('round-section-2')).toBeVisible()
  // Positions 1 and 2 only: position 0 has no previous segment, so `BranchLine`
  // returns null there for every input and asserting its absence proves nothing.
  for (const pos of [1, 2]) {
    await expect(rail.getByTestId(`segment-branch-${pos}`)).toHaveCount(0)
  }
})

/** Q10: gaps are unnamed — a count and the two rounds they sit between. */
test('U2: the gap between two rounds is described by its size, not a name', async ({ page }) => {
  const panel = await openNotify(page, multiRoundIssue, multiRoundStatus)
  const gap = panel.getByTestId('round-rail').getByTestId('gap-line-1')

  await expect(gap).toContainText('1 commit between Initial QC and Round 2')
  // No index, no name, no id.
  await expect(gap).not.toContainText('Gap')
})

/**
 * D6: the empty trailing gap of a fully-approved issue is not an event.
 *
 * The fixture must actually *have* that gap, or the assertion passes for the wrong
 * reason — a single-round open issue has no segment at position 1 at all.
 */
const approvedOpenIssue: Issue = { ...approvedRoundIssue, state: 'open', closed_at: null }

test('U2: an empty gap renders nothing', async ({ page }) => {
  // The premise: position 1 is a real, placed, empty gap.
  const gap = approvedRoundStatus.segments[1]
  expect(gap.kind).toBe('gap')
  expect(gap.commits).toHaveLength(0)
  expect(gap.placement.kind).toBe('placed')

  const panel = await openNotify(page, approvedOpenIssue, {
    ...approvedRoundStatus,
    issue: approvedOpenIssue,
  })
  await expect(panel.getByTestId('round-rail')).toBeVisible()
  // The round it follows is rendered, so the rail is not simply absent.
  await expect(panel.getByTestId('round-line-1')).toBeVisible()
  await expect(panel.getByTestId('gap-line-1')).toHaveCount(0)
})

/** D4/U2: an unresolvable segment is greyed with its reason, never an error. */
test('U2: an unplaceable segment states why it could not be placed', async ({ page }) => {
  const panel = await openNotify(page, unplaceableIssue, unplaceableStatus)
  const rail = panel.getByTestId('round-rail')

  await expect(rail.getByTestId('segment-unplaceable-2')).toContainText(
    'its branch is unavailable locally',
  )
  await expect(rail.getByTestId('segment-unplaceable-1')).toContainText(
    'the round bounding it could not be placed',
  )
  // Placed segments say nothing of the sort.
  await expect(rail.getByTestId('segment-unplaceable-0')).toHaveCount(0)
})

/**
 * S4/D4: the *picker* must not be quieter than the rail about the same failure.
 *
 * The open round owns no commits, so scoping to it offers only whatever the defaults
 * forced visible — for #115 a single Initial QC commit, which would sit under the label
 * "scoped to Round 2". That is another segment's history presented as this round's, so
 * the scope is dropped, the full history shown, and the reason named.
 */
test('U1: an unplaceable open round is named in the picker, which falls back to full history', async ({ page }) => {
  // The premise: the active segment is the open round, and it could not be placed.
  const active = unplaceableStatus.segments[unplaceableStatus.segments.length - 1]
  expect(active.kind).toBe('round')
  expect(active.placement).toMatchObject({ kind: 'unplaceable', reason: 'branch_unavailable' })

  const panel = await openNotify(page, unplaceableIssue, unplaceableStatus)
  const track = panel.getByTestId('notify-picker')

  const note = panel.getByTestId('picker-scope-unplaceable')
  await expect(note).toContainText('Round 2')
  await expect(note).toContainText('its branch is unavailable locally')
  // Not claiming a scope it cannot honour.
  await expect(panel.getByTestId('picker-scope')).toHaveCount(0)

  // Full history: both of Initial QC's commits are offered, not just the forced default.
  await expect(track.getByText(SHORT(ROUND1_OPENED))).toBeVisible()
  await expect(track.getByText(SHORT(ROUND1_CLOSED))).toBeVisible()
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
  await expect(alert).toContainText('unapproving')

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

// ---------------------------------------------------------------------------
// U1: a non-linear Gap detaches the previous-approval handle
// ---------------------------------------------------------------------------

/**
 * #114 with the notify default reaching back past the divergent gap, so the selection
 * spans it.
 */
const crossBranchFromApproval: IssueStatusResponse = {
  ...crossBranchStatus,
  next_notification_from: ROUND1_CLOSED,
}

/**
 * The negative for the pair below, built as a *single-variable mutation* of the
 * positive: same response, same segments, same track — only the gap's `continuity`
 * flips to `linear`. Written this way rather than as a separate fixture object so the
 * two cannot drift apart into a comparison of two different things, which is the way
 * a positive/negative pair silently stops testing what it claims to.
 */
const crossBranchLinearGap: IssueStatusResponse = {
  ...crossBranchFromApproval,
  segments: crossBranchSegments.map((segment) =>
    segment.kind === 'gap' ? { ...segment, continuity: { kind: 'linear' as const } } : segment,
  ),
}

test('U1: a diverged gap breaks the track and detaches the previous-approval handle', async ({ page }) => {
  const panel = await openNotify(page, crossBranchIssue, crossBranchFromApproval)

  // From reaches back across the gap; To is Round 2's own commit.
  await expect(panel.locator('text=From:').locator('..').getByText(SHORT(ROUND1_CLOSED))).toBeVisible()
  await expect(panel.locator('text=To:').locator('..').getByText(SHORT(ROUND2_OPENED))).toBeVisible()

  // The break is drawn at the divergent gap's position (1), and says which kind it is.
  const brk = panel.getByTestId('picker-break-1')
  await expect(brk).toBeVisible()
  await expect(brk).toHaveAttribute('data-continuity', 'diverged')

  // The handle reads as detached rather than as one end of a continuous span.
  await expect(panel.getByTestId('detached-previous-approval')).toBeVisible()
  await expect(panel.locator('text=From:').locator('..')).toHaveAttribute('data-detached', 'true')

  // And the receipt refuses to report a commit count across histories that do not join.
  const receipt = panel.getByTestId('comparison-receipt')
  await expect(receipt).toHaveAttribute('data-detached', 'true')
  await expect(receipt).toContainText('histories not connected')
})

test('U1: the same track over a linear gap is not broken and not detached', async ({ page }) => {
  // The premise: exactly one field differs from the positive above.
  expect(crossBranchLinearGap.segments[1]).toMatchObject({ continuity: { kind: 'linear' } })
  expect(crossBranchFromApproval.segments[1]).toMatchObject({ continuity: { kind: 'diverged' } })

  const panel = await openNotify(page, crossBranchIssue, crossBranchLinearGap)

  // Same two ends as the diverged case above.
  await expect(panel.locator('text=From:').locator('..').getByText(SHORT(ROUND1_CLOSED))).toBeVisible()
  await expect(panel.locator('text=To:').locator('..').getByText(SHORT(ROUND2_OPENED))).toBeVisible()

  await expect(panel.getByTestId('picker-break-1')).toHaveCount(0)
  await expect(panel.getByTestId('detached-previous-approval')).toHaveCount(0)
  const receipt = panel.getByTestId('comparison-receipt')
  await expect(receipt).not.toHaveAttribute('data-detached', 'true')
  await expect(receipt).toContainText('commits')
})

/** U2: the rail states the divergence too, where it happened. */
test('U2: the rail reports a diverged gap and the commit the two ends share', async ({ page }) => {
  const panel = await openNotify(page, crossBranchIssue, crossBranchStatus)
  const note = panel.getByTestId('round-rail').getByTestId('gap-continuity-1')

  await expect(note).toContainText('History diverges here')
  await expect(note).toContainText(SHORT(ROUND1_OPENED))
})

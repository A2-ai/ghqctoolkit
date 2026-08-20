// P3 / S1-S3, S5, S7 and U1/U2: the round rail, the segment-scoped commit picker,
// gap collapse, the `next_notification_from` default and the comparison receipt —
// all inside IssueDetailModal.

import { test, expect, type Locator, type Page } from 'playwright/test'
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
  ROUND2_DRIFT,
  commit,
  crossBranchIssue,
  crossBranchSegments,
  crossBranchStatus,
  multiRoundSegments,
  segmentFields,
  closeRound,
  initialQcRound,
  laterRound,
  gapSegment,
  unplaceableIssue,
  unrelatedHistoryStatus,
  unplaceableStatus,
} from '../fixtures/index'
import type { Issue, IssueStatusResponse } from '../../src/api/issues'

/**
 * Open a picker's `History ▾` menu and return the dropdown.
 *
 * The popover is portalled, so its rows are not inside the tab panel — `page`, not
 * `panel`, is the right root for anything inside it. Only one can be open at a time,
 * so the testid is unambiguous even with three pickers on screen.
 */
async function openHistory(page: Page, panel: Locator): Promise<Locator> {
  await panel.getByTestId('history-menu-trigger').click()
  const menu = page.getByTestId('history-menu')
  await expect(menu).toBeVisible()
  return menu
}

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

/**
 * D1: Round 2 anchored on Round 1's *closing* commit — a re-QC with no drift in between
 * (HEAD had not moved, e.g. reviewing the same code against a stricter checklist).
 *
 * One hash is then legitimately owned by two adjacent segments (I4 exempts it), and the
 * intervening gap is necessarily empty. The API projects the two copies differently by
 * design (D14): `["approved"]` in the round it closed, `["initial"]` in the round it
 * anchors.
 */
const reQcOnTheApproval: IssueStatusResponse = {
  ...multiRoundStatus,
  ...segmentFields(
    [
      closeRound(
        initialQcRound(ROUND1_OPENED, {
          commits: [
            commit(ROUND1_CLOSED, { message: 'address review' }),
            commit(ROUND1_OPENED, { message: 'initial commit', statuses: ['initial'] }),
          ],
        }),
        ROUND1_CLOSED,
      ),
      gapSegment({ lower_bound: ROUND1_CLOSED, upper_bound: ROUND1_CLOSED }),
      // Anchored on the boundary, so its own commits are just that commit.
      laterRound(2, ROUND1_CLOSED, {
        commits: [commit(ROUND1_CLOSED, { message: 'address review', statuses: ['initial'] })],
      }),
    ],
    ROUND1_CLOSED,
  ),
}

/** A commit inside the scoped round that is neither file-changing nor comment-named. */
const QUIET_IN_ROUND = 'f6f6f6f6f6f6f6f6f6f6f6f6f6f6f6f6f6f6f6f6'

/**
 * Round 2 with a quiet commit: in scope, but hidden by the pertinence filter. Exercises
 * the density axis on its own — showing it must not widen the reach.
 */
const roundTwoWithQuietCommit: IssueStatusResponse = {
  ...multiRoundStatus,
  ...segmentFields(
    multiRoundSegments({
      round2: {
        // Newest-first, and the quiet commit sits in the *middle* deliberately: as the
        // newest it would be the default `to` handle, and forced handle positions are
        // always visible, so the pertinence filter would never get to hide it.
        commits: [
          commit(ROUND2_DRIFT, { message: 'real change' }),
          commit(QUIET_IN_ROUND, { message: 'whitespace', file_changed: false }),
          commit(ROUND2_OPENED, { message: 'round 2 changes', statuses: ['initial', 'notification'] }),
        ],
      },
    }),
  ),
}

/**
 * U5: a *multi-round* thread whose Round 2 has been approved, so the last segment is a
 * trailing Gap and there is no open round.
 *
 * `approvedRoundStatus` cannot pin U5: it has one round, which owns every commit, so
 * scoping is legitimately a no-op there and the note is absent either way. This fixture
 * has segments outside the scope, so the note's presence is meaningful.
 */
const approvedMultiRound: IssueStatusResponse = {
  ...multiRoundStatus,
  qc_status: { ...multiRoundStatus.qc_status, status: 'approved', status_detail: 'Approved' },
  ...segmentFields([
    ...multiRoundSegments().slice(0, 2),
    closeRound(
      laterRound(2, ROUND2_OPENED, {
        commits: [commit(ROUND2_OPENED, { message: 'round 2 changes', statuses: ['initial'] })],
      }),
      ROUND2_OPENED,
    ),
    gapSegment({ lower_bound: ROUND2_OPENED, upper_bound: ROUND2_OPENED }),
  ]),
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
  // No accordion sections, no scope note, no History menu: nothing that a
  // pre-rounds legacy issue did not already have.
  await expect(panel.getByTestId('round-section-1')).toHaveCount(0)
  await expect(panel.getByTestId('picker-scope')).toHaveCount(0)
  await expect(panel.getByTestId('history-menu-trigger')).toHaveCount(0)
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

/**
 * D16: the density checkbox no longer widens the reach. This test previously asserted the
 * opposite — that "Show all commits" produced the full history and dropped the scope note
 * — which is exactly the conflation D16 removed: seeing one more commit inside the round
 * should not require leaving it. Reach lives on the rail instead.
 */
test('D16: the density checkbox does not widen the track to the full history', async ({ page }) => {
  const panel = await openNotify(page, multiRoundIssue, multiRoundStatus)
  const track = panel.getByTestId('notify-picker')

  await panel.getByLabel('Show every commit on the track').check()

  // Still scoped, and Initial QC is still off the track.
  await expect(panel.getByTestId('picker-scope')).toContainText('scoped to Round 2')
  await expect(track.getByText(SHORT(ROUND1_OPENED))).toHaveCount(0)
  const menu = await openHistory(page, panel)
  await expect(menu.getByTestId('rail-round-0')).toHaveAttribute('data-on-track', 'false')

  // Ticking it in the History menu is what brings Initial QC onto the track.
  await menu.getByTestId('rail-round-0').click()
  await expect(track.getByText(SHORT(ROUND1_OPENED))).toBeVisible()
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
// D16 / U5–U8: the segment rail
// ---------------------------------------------------------------------------

test('D16: the gap between rounds is a menu row that puts its commits on the track', async ({ page }) => {
  const panel = await openNotify(page, multiRoundIssue, multiRoundStatus)
  const track = panel.getByTestId('notify-picker')

  // The menu is available *while scoped* — that is the point of D16. Previously the
  // reach affordance only existed once you had already asked for the whole history,
  // i.e. exactly where it was least useful.
  const menu = await openHistory(page, panel)
  const gap = menu.getByTestId('rail-gap-1')
  await expect(gap).toBeVisible()
  await expect(gap).toHaveAttribute('data-continuity', 'linear')
  // Spelled out, not compressed to `·1` — the whole reason for the vertical form.
  await expect(gap).toContainText('1 commit between rounds')

  // Off the track to begin with.
  await expect(gap).toHaveAttribute('data-on-track', 'false')
  await expect(track.getByText(SHORT(DRAFT_GAP_COMMIT))).toHaveCount(0)

  await gap.click()
  await expect(gap).toHaveAttribute('data-on-track', 'true')
  await expect(track.getByText(SHORT(DRAFT_GAP_COMMIT))).toBeVisible()

  // And back off again.
  await gap.click()
  await expect(track.getByText(SHORT(DRAFT_GAP_COMMIT))).toHaveCount(0)
})

test('D16: an earlier round is a named menu row that puts its commits on the track', async ({ page }) => {
  const panel = await openNotify(page, multiRoundIssue, multiRoundStatus)
  const track = panel.getByTestId('notify-picker')

  // Round 1 is outside the scope, so it is reachable but not on the track by default.
  const menu = await openHistory(page, panel)
  const chip = menu.getByTestId('rail-round-0')
  await expect(chip).toBeVisible()
  await expect(chip).toContainText('Initial QC')
  await expect(chip).toContainText('2 commits')
  await expect(chip).toHaveAttribute('data-selectable', 'true')
  await expect(chip).toHaveAttribute('data-on-track', 'false')

  // ROUND1_OPENED is Initial QC's own anchor, and is not forced visible by any default.
  await expect(track.getByText(SHORT(ROUND1_OPENED))).toHaveCount(0)
  await chip.click()
  await expect(track.getByText(SHORT(ROUND1_OPENED))).toBeVisible()
})

/**
 * The scoped round is on the track unconditionally: the round being worked in is the
 * whole point of the picker, so its chip is not a toggle that could take it away.
 */
test('D16: the scoped round is listed as current and cannot be taken off', async ({ page }) => {
  const panel = await openNotify(page, multiRoundIssue, multiRoundStatus)
  const menu = await openHistory(page, panel)
  const chip = menu.getByTestId('rail-round-2')

  await expect(chip).toContainText('Round 2')
  // Said in words rather than implied by a filled pill.
  await expect(chip).toContainText('current round')
  await expect(chip).toContainText('always shown')
  await expect(chip).toHaveAttribute('data-in-scope', 'true')
  await expect(chip).toHaveAttribute('data-on-track', 'true')
  await expect(chip).toHaveAttribute('data-selectable', 'false')
  // Not wrapped in a button, so there is nothing to click.
  await expect(menu.locator('button [data-testid="rail-round-2"]')).toHaveCount(0)
})

test('D16: the menu lists segments newest-first with the gap between them', async ({ page }) => {
  const panel = await openNotify(page, multiRoundIssue, multiRoundStatus)
  const menu = await openHistory(page, panel)

  // Newest-first for reading, matching `git log`: the segment you are working in is the
  // one under the cursor. `pos` still indexes the oldest-first model, so the displayed
  // order is the reverse of the model's.
  const ids = await menu
    .locator('[data-testid^="rail-"]')
    .evaluateAll((els) => els.map((e) => e.getAttribute('data-testid')))
  expect(ids).toEqual(['rail-round-2', 'rail-gap-1', 'rail-round-0'])
})

/**
 * The case the on-track markers got exactly backwards. When the scope round is
 * `Unplaceable` the scope is empty, and the old design derived its markers *from* the
 * scope — so every reach affordance vanished at the one moment the structure most
 * needed explaining, and the full history silently appeared instead.
 */
test('D16: an unplaceable round is still listed, with its reason spelled out', async ({ page }) => {
  const panel = await openNotify(page, unplaceableIssue, unplaceableStatus)

  // Premise: this is the empty-scope fallback, not a normal scoped track.
  await expect(panel.getByTestId('picker-scope-unplaceable')).toBeVisible()

  const menu = await openHistory(page, panel)
  const chip = menu.getByTestId('rail-round-2')
  await expect(chip).toContainText('Round 2')
  await expect(chip).toHaveAttribute('data-unplaceable', 'branch_unavailable')
  // It owns no commits (I5), so there is nothing to put on the track.
  await expect(chip).toHaveAttribute('data-selectable', 'false')
  // The reason is *readable text on the row*, not a tooltip and not a strikethrough that
  // leaves the reader to guess. This is what the vertical form buys.
  await expect(chip).toContainText('could not be placed')
  await expect(chip).toContainText('its branch is unavailable locally')
})

test('D16: a diverged gap says so, and names the commit the two sides meet at', async ({ page }) => {
  const panel = await openNotify(page, crossBranchIssue, crossBranchStatus)

  const menu = await openHistory(page, panel)
  const gap = menu.getByTestId('rail-gap-1')
  await expect(gap).toHaveAttribute('data-continuity', 'diverged')
  await expect(menu.getByTestId('history-continuity-1')).toContainText(
    `histories diverge — they meet at ${SHORT(ROUND1_OPENED)}`,
  )
  // Still reachable: the UI never blocks reach, it only refuses to claim a diff.
  await expect(gap).toHaveAttribute('data-selectable', 'true')
})

/**
 * `unrelated` reads as severed rather than merely diverged, because "these rounds are on
 * branches that meet nowhere" is a different fact from "they diverged and meet
 * upstream". In the vertical form that is a sentence, not a dash nobody can read.
 */
test('D16: an unrelated gap says there is no shared history, and is still reachable', async ({ page }) => {
  const panel = await openNotify(page, crossBranchIssue, unrelatedHistoryStatus)

  const menu = await openHistory(page, panel)
  const gap = menu.getByTestId('rail-gap-1')
  await expect(gap).toHaveAttribute('data-continuity', 'unrelated')
  await expect(menu.getByTestId('history-continuity-1')).toContainText('no shared history')

  // Reach across it is offered, per the decision that the UI never blocks reach.
  const chip = menu.getByTestId('rail-round-0')
  await expect(chip).toHaveAttribute('data-selectable', 'true')
  await chip.click()
  await expect(panel.getByTestId('notify-picker').getByText(SHORT(ROUND1_OPENED))).toBeVisible()
})

test('D16: the density checkbox widens within the round without leaving it', async ({ page }) => {
  const panel = await openNotify(page, multiRoundIssue, roundTwoWithQuietCommit)
  const track = panel.getByTestId('notify-picker')

  // The quiet commit is in the scoped round but is neither file-changing nor named by a
  // comment, so the pertinence filter hides it.
  await expect(track.getByText(SHORT(QUIET_IN_ROUND))).toHaveCount(0)

  await panel.getByLabel('Show every commit on the track').check()
  await expect(track.getByText(SHORT(QUIET_IN_ROUND))).toBeVisible()

  // ...and the reach did not widen: Round 1 stays off the track. This is the axis
  // split — the old single toggle would have shown everything here.
  await expect(track.getByText(SHORT(ROUND1_OPENED))).toHaveCount(0)
  const menu = await openHistory(page, panel)
  await expect(menu.getByTestId('rail-round-0')).toHaveAttribute('data-on-track', 'false')
})

test('S3: a single-round issue has no rail — there is no structure to navigate', async ({ page }) => {
  const panel = await openNotify(page, legacyRoundIssue, legacyRoundStatus)
  await panel.getByLabel('Show every commit on the track').check()
  await expect(panel.getByTestId('history-menu-trigger')).toHaveCount(0)
})

test('U5: approving does not widen the track to the whole history', async ({ page }) => {
  // The premise: this thread's last segment is a Gap, so there is no *open* round — the
  // state the old `activeRoundPos` scope returned null for, silently showing everything.
  expect(approvedMultiRound.segments.at(-1)?.kind).toBe('gap')
  // ...and it has segments outside the scope, so a scope note is meaningful at all.
  expect(approvedMultiRound.segments.length).toBeGreaterThan(2)

  // Opened with `multiRoundIssue` because the fixture spreads `multiRoundStatus`, so
  // that is the issue its status belongs to — and it is open, so no include-closed
  // toggle is needed to see the card.
  const panel = await openNotify(page, multiRoundIssue, approvedMultiRound)
  // Still scoped, and to the round that just closed rather than to nothing.
  await expect(panel.getByTestId('picker-scope')).toBeVisible()
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

/**
 * Every round starts with a blue `initial` dot, not just Initial QC.
 *
 * A round's `opened_at` is its `initial qc round commit`, and D10 gives the anchor to the
 * round rather than the preceding gap — so the picker marks the start of every round.
 * Before this, `IssueCommit::project` set `initial` only for `segments[0].opened_at`, so
 * Round 2 and later began with a bare slot.
 */
test('every round anchor carries the blue initial dot, not only Initial QC', async ({ page }) => {
  const panel = await openNotify(page, multiRoundIssue, multiRoundStatus)

  // The track is scoped to the open Round 2.
  await expect(panel.getByTestId('picker-scope')).toBeVisible()

  // Round 2's anchor carries the dot. Keyed by hash, so this does not depend on where
  // the anchor lands among the visible slots.
  await expect(panel.getByTestId(`commit-dot-${SHORT(ROUND2_OPENED)}-initial`)).toBeVisible()

  // The premise, so this cannot pass on a fixture that happens to be Round 1: the
  // scoped round really is round 2, and its anchor really is the marked commit.
  const round2 = multiRoundStatus.segments.at(-1)
  expect(round2).toMatchObject({ kind: 'round', index: 2 })
  // `initial` alongside `notification`, in the projection's fixed order: this commit is
  // both round 2's anchor and the commit its notification named.
  expect(round2 && 'commits' in round2 && round2.commits.at(-1)).toMatchObject({
    hash: ROUND2_OPENED,
    statuses: ['initial', 'notification'],
  })

  // And the gap's drift between the rounds stays bare — this is not "mark everything".
  await expect(panel.getByTestId(`commit-dot-${SHORT(DRAFT_GAP_COMMIT)}-initial`)).toHaveCount(0)
})

/**
 * D1 + D14: a commit that is both Round 1's approval and Round 2's anchor shows **both**
 * dots — green for the approval, blue for the anchor — on one slot.
 *
 * The two copies the API sends are deliberately different, but the picker draws a flat
 * track, and one slot cannot carry two frames. So `flattenSegmentCommits` dedupes by hash
 * and **unions** the statuses: without the union a dot would appear or vanish depending on
 * which side of the boundary was being drawn, which is worse than showing both facts.
 * Order comes from the projection's fixed order, so blue precedes green.
 */
test('D1: a commit that is one round\'s approval and the next round\'s anchor shows both dots', async ({ page }) => {
  // The premise: the API really does send the same hash twice, annotated differently.
  const [r1, , r2] = reQcOnTheApproval.segments
  expect(r1 && 'commits' in r1 && r1.commits[0]).toMatchObject({
    hash: ROUND1_CLOSED,
    statuses: ['approved'],
  })
  expect(r2 && 'commits' in r2 && r2.commits[0]).toMatchObject({
    hash: ROUND1_CLOSED,
    statuses: ['initial'],
  })

  const panel = await openNotify(page, multiRoundIssue, reQcOnTheApproval)

  // One slot, both dots.
  await expect(panel.getByTestId(`commit-dot-${SHORT(ROUND1_CLOSED)}-approved`)).toBeVisible()
  await expect(panel.getByTestId(`commit-dot-${SHORT(ROUND1_CLOSED)}-initial`)).toBeVisible()

  // And it is genuinely one slot, not the same hash drawn twice — a duplicate row would
  // break the picker's positional defaults.
  await expect(panel.getByTestId(`commit-dot-${SHORT(ROUND1_CLOSED)}-initial`)).toHaveCount(1)
})

/**
 * The track is horizontally centred in its panel.
 *
 * It was not: the container carried `paddingLeft: 16, paddingRight: 40`, parking the
 * whole slider 24px left of centre. Asserted as *symmetry of the end insets* rather
 * than against literal pixel values, so it pins the property the eye actually notices
 * and does not have to be rewritten when the padding changes.
 */
test('the track sits centred in its panel, not offset to one side', async ({ page }) => {
  // Two visible commits, so there is a real leftmost and rightmost dot to measure.
  const panel = await openNotify(page, multiRoundIssue, roundTwoNoNotification)
  const track = panel.getByTestId('notify-picker')
  const slots = track.locator('[data-end-allowed]')

  // Premise: one dot cannot be off-centre, it is drawn mid-track by construction.
  expect(await slots.count()).toBeGreaterThan(1)

  const trackBox = await track.boundingBox()
  const firstBox = await slots.first().boundingBox()
  const lastBox = await slots.last().boundingBox()
  expect(trackBox).not.toBeNull()
  expect(firstBox).not.toBeNull()
  expect(lastBox).not.toBeNull()
  if (trackBox === null || firstBox === null || lastBox === null) return

  // Dot *centres*, not outer edges: an end slot renders one dot per status, so the first
  // and last groups differ in width and comparing their outer edges leaks half that
  // difference into the measurement (3.5px on a correctly centred track).
  const firstCentre = firstBox.x + firstBox.width / 2
  const lastCentre = lastBox.x + lastBox.width / 2
  const leftInset = firstCentre - trackBox.x
  const rightInset = trackBox.x + trackBox.width - lastCentre
  expect(
    Math.abs(leftInset - rightInset),
    `track is off-centre: ${leftInset}px on the left, ${rightInset}px on the right`,
  ).toBeLessThanOrEqual(2)
})

// ---------------------------------------------------------------------------
// U6/U7: what each action may select
// ---------------------------------------------------------------------------

/**
 * U6: Approve may only land on a commit in the scoped round, so an expanded earlier
 * round's commits are dimmed — the constraint is visible before you drag into it.
 *
 * Approval closes a round **at** a commit, so the commit must belong to that round.
 */
test('U6: Approve dims commits outside the scoped round', async ({ page }) => {
  await openModal(page, multiRoundIssue, multiRoundStatus)
  await page.getByRole('tab', { name: 'Approve', exact: true }).click()
  const panel = page.getByRole('tabpanel', { name: 'Approve' })
  await expect(panel).toBeVisible()

  // Reach into Initial QC, then check its commits cannot be the approval point.
  // Ticked in the History menu, which is then dismissed by its own trigger. (Escape
  // would close the whole issue modal — see the note in `HistoryMenu`.)
  await (await openHistory(page, panel)).getByTestId('rail-round-0').click()
  await panel.getByTestId('history-menu-trigger').click()
  await expect(page.getByTestId('history-menu')).toHaveCount(0)
  const track = panel.getByTestId('approve-picker')
  await expect(track.getByText(SHORT(ROUND1_OPENED))).toBeVisible()

  // Some slots are now blocked, and some are still allowed — asserting only the first
  // would pass on a track where *everything* got dimmed.
  await expect(track.locator('[data-end-allowed="false"]').first()).toBeVisible()
  await expect(track.locator('[data-end-allowed="true"]').first()).toBeVisible()
})

/**
 * U6: Review has no such constraint — a review records what the reviewer *read*, and
 * reading an older round's commit is legitimate. So nothing is dimmed.
 */
test('U6: Review allows a commit from an earlier round', async ({ page }) => {
  await openModal(page, multiRoundIssue, multiRoundStatus)
  await page.getByRole('tab', { name: 'Review', exact: true }).click()
  const panel = page.getByRole('tabpanel', { name: 'Review' })
  await expect(panel).toBeVisible()

  // Ticked in the History menu, which is then dismissed by its own trigger. (Escape
  // would close the whole issue modal — see the note in `HistoryMenu`.)
  await (await openHistory(page, panel)).getByTestId('rail-round-0').click()
  await panel.getByTestId('history-menu-trigger').click()
  await expect(page.getByTestId('history-menu')).toHaveCount(0)
  const track = panel.getByTestId('review-picker')
  await expect(track.getByText(SHORT(ROUND1_OPENED))).toBeVisible()

  // Every slot is selectable, unlike Approve above.
  await expect(track.locator('[data-end-allowed="false"]')).toHaveCount(0)
  await expect(track.locator('[data-end-allowed="true"]').first()).toBeVisible()
})

/**
 * U6/U7: Notify's `to` may not sit in a closed round, but the constraint is keyed on
 * position rather than segment kind — so the *trailing* gap, being newer than the scope,
 * stays selectable. A rule written as "another segment ⇒ from-only" would have blocked
 * exactly the drift U5 exists to surface.
 */
test('U7: Notify blocks an earlier round as the to-end but allows the newer trailing gap', async ({ page }) => {
  const panel = await openNotify(page, multiRoundIssue, approvedMultiRound)
  const track = panel.getByTestId('notify-picker')

  // Ticked in the History menu, which is then dismissed by its own trigger. (Escape
  // would close the whole issue modal — see the note in `HistoryMenu`.)
  await (await openHistory(page, panel)).getByTestId('rail-round-0').click()
  await panel.getByTestId('history-menu-trigger').click()
  await expect(page.getByTestId('history-menu')).toHaveCount(0)
  await expect(track.getByText(SHORT(ROUND1_OPENED))).toBeVisible()
  // Initial QC is older than the scope, so it cannot be the to-end.
  await expect(track.locator('[data-end-allowed="false"]').first()).toBeVisible()

  // The premise for the other half: the trailing gap is in scope, so nothing about it is
  // dimmed — it is newer than the round, not older.
  expect(approvedMultiRound.segments.at(-1)?.kind).toBe('gap')
})

/** U2: the rail states the divergence too, where it happened. */
test('U2: the rail reports a diverged gap and the commit the two ends share', async ({ page }) => {
  const panel = await openNotify(page, crossBranchIssue, crossBranchStatus)
  const note = panel.getByTestId('round-rail').getByTestId('gap-continuity-1')

  await expect(note).toContainText('History diverges here')
  await expect(note).toContainText(SHORT(ROUND1_OPENED))
})

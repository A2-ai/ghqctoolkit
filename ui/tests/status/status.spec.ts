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
  multiRoundIssue,
  multiRoundStatus,
  approvedRoundIssue,
  approvedRoundStatus,
  ROUND1_CLOSED,
  ROUND1_OPENED,
  ROUND2_OPENED,
  DRAFT_GAP_COMMIT,
  commit,
  crossBranchIssue,
  crossBranchStatus,
  gapSegment,
  segmentFields,
  closeRound,
  initialQcRound,
  vanishedApprovalIssue,
  vanishedApprovalStatus,
  changeRequestedDriftStatus,
  changesToCommentDriftStatus,
  driftedRoundIssue,
  ROUND2_DRIFT,
} from '../fixtures/index'
import type { IssueStatusResponse } from '../../src/api/issues'

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

  // Each lane by its own container, not by "an element containing the lane heading":
  // the latter matches <html> and <body>, which contain every lane heading and every
  // card, so it finds the link wherever the card actually is.
  await expect(
    page.getByTestId('lane-ready-for-review').getByRole('link', { name: /src\/awaiting\.rs/ }),
  ).toBeVisible()
  await expect(
    page.getByTestId('lane-findings-to-address').getByRole('link', { name: /src\/change\.rs/ }),
  ).toBeVisible()
  await expect(
    page.getByTestId('lane-changes-to-notify').getByRole('link', { name: /src\/inprogress\.rs/ }),
  ).toBeVisible()
  await expect(
    page.getByTestId('lane-approved').getByRole('link', { name: /src\/approved\.rs/ }),
  ).toBeVisible()
})

test('clicking the issue title link opens github without opening the issue modal', async ({ page }) => {
  await setupRoutes(page, {
    milestoneIssues: {
      1: [awaitingReviewIssue],
    },
    issueStatuses: {
      results: [awaitingReviewStatus],
      errors: [],
    },
  })

  await page.goto('/')
  await selectMilestone(page, 'Sprint 1')

  const titleLink = page.getByRole('link', { name: /src\/awaiting\.rs/ })
  const popupPromise = page.waitForEvent('popup')

  await titleLink.click()

  const popup = await popupPromise
  await expect(popup).toHaveURL(awaitingReviewIssue.html_url)
  await expect(page.getByRole('dialog')).not.toBeVisible()
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

// ---------------------------------------------------------------------------
// "File has changed since approval" must not fire while a round is open
// ---------------------------------------------------------------------------

/** The card tint SwimLanes applies for an unreviewed change after approval. */
const POST_APPROVAL_ORANGE = 'rgb(255, 237, 213)'

test('an open round suppresses the changed-since-approval warning on the card', async ({ page }) => {
  // #112: Initial QC approved at ROUND1_CLOSED, then two newer file-changing
  // commits, with Round 2 open over them. Those changes are what Round 2 is
  // reviewing — the normal approve/edit/start-a-round flow — so the card must not
  // colour them as unreviewed drift against the *previous* round's approval.
  await setupRoutes(page, {
    milestoneIssues: { 1: [multiRoundIssue] },
    issueStatuses: { results: [multiRoundStatus], errors: [] },
  })
  await page.goto('/')
  await selectMilestone(page, 'Sprint 1')

  const card = page.getByTestId(`issue-card-${multiRoundIssue.number}`)
  await expect(card).toBeVisible()
  // The card's own background is the visible symptom, so assert on it directly
  // rather than on a row that this status would not render anyway.
  await expect(card).not.toHaveCSS('background-color', POST_APPROVAL_ORANGE)

  // The row that is shown reports the open round's own commit, not the earlier
  // round's approval — the approval-priority tier must not win here.
  await expect(card).toContainText('Latest')
  await expect(card).toContainText(ROUND2_OPENED.slice(0, 7))
  await expect(card).not.toContainText(ROUND1_CLOSED.slice(0, 7))

  await card.hover()
  await expect(page.getByText('File has changed since approval')).toHaveCount(0)
})

/**
 * Approved, then the file moved again, with every round closed. Kept `open` so the
 * closed-issues filter plays no part in whether the card is on screen.
 */
const driftedIssue = { ...approvedRoundIssue, state: 'open' as const, closed_at: null }

const driftedAfterApproval: IssueStatusResponse = {
  ...approvedRoundStatus,
  issue: driftedIssue,
  qc_status: {
    ...approvedRoundStatus.qc_status,
    status: 'changes_after_approval',
    status_detail: 'Approved; subsequent file changes',
    // S1/S5: the trailing gap is non-empty, and its newest commit is `latest_commit`.
    latest_commit: DRAFT_GAP_COMMIT,
    // ... and its newest *file-changing* commit is what the warning reads.
    changed_commit: DRAFT_GAP_COMMIT,
  },
  ...segmentFields([
    closeRound(
      initialQcRound(ROUND1_OPENED, {
        commits: [
          commit(ROUND1_CLOSED, { message: 'address review' }),
          commit(ROUND1_OPENED, { message: 'initial commit', statuses: ['initial'] }),
        ],
      }),
      ROUND1_CLOSED,
    ),
    gapSegment({
      commits: [commit(DRAFT_GAP_COMMIT, { message: 'edit after approval' })],
      lower_bound: ROUND1_CLOSED,
      upper_bound: DRAFT_GAP_COMMIT,
    }),
  ]),
}

test('with no round open, a change after approval still warns', async ({ page }) => {
  // The counterpart: suppressing the warning must depend on a round being open,
  // not on rounds existing at all, or the warning would never fire again.
  await setupRoutes(page, {
    milestoneIssues: { 1: [driftedIssue] },
    issueStatuses: { results: [driftedAfterApproval], errors: [] },
  })
  await page.goto('/')
  await selectMilestone(page, 'Sprint 1')

  const card = page.getByTestId(`issue-card-${driftedIssue.number}`)
  await expect(card).toHaveCSS('background-color', POST_APPROVAL_ORANGE)
  await expect(card).toContainText('Changed')
  await expect(card).toContainText(DRAFT_GAP_COMMIT.slice(0, 7))

  await card.hover()
  await expect(page.getByText('File has changed since approval')).toBeVisible()
})

/**
 * A trailing gap whose **newest** commit never touched the file.
 *
 * `changed_commit` and `latest_commit` are different commits here, which is the whole
 * reason the field exists: S1 selects the newest *file-changing* commit of the gap, so
 * the "Changed" row must name that one. Rendering `latest_commit` instead would name a
 * commit that never touched the file — and that is what the card used to do.
 */
const UNTOUCHED_DRIFT = 'aaaa0000000000000000000000000000000000ff'
const TOUCHED_DRIFT = 'bbbb0000000000000000000000000000000000ff'

const driftedWhoseNewestChangedNothing: IssueStatusResponse = {
  ...approvedRoundStatus,
  issue: driftedIssue,
  qc_status: {
    ...approvedRoundStatus.qc_status,
    status: 'changes_after_approval',
    status_detail: 'Approved; subsequent file changes',
    // The gap's newest commit — it did not touch the file.
    latest_commit: UNTOUCHED_DRIFT,
    // The gap's newest *file-changing* commit, which is older.
    changed_commit: TOUCHED_DRIFT,
  },
  ...segmentFields([
    closeRound(initialQcRound(ROUND1_OPENED), ROUND1_CLOSED),
    gapSegment({
      commits: [
        commit(UNTOUCHED_DRIFT, { message: 'unrelated churn', file_changed: false }),
        commit(TOUCHED_DRIFT, { message: 'edit after approval' }),
      ],
      lower_bound: ROUND1_CLOSED,
      upper_bound: UNTOUCHED_DRIFT,
    }),
  ]),
}

test('the Changed row names the file-changing commit, not the gap\'s newest', async ({ page }) => {
  await setupRoutes(page, {
    milestoneIssues: { 1: [driftedIssue] },
    issueStatuses: { results: [driftedWhoseNewestChangedNothing], errors: [] },
  })
  await page.goto('/')
  await selectMilestone(page, 'Sprint 1')

  const card = page.getByTestId(`issue-card-${driftedIssue.number}`)
  await expect(card).toContainText('Changed')
  await expect(card).toContainText(TOUCHED_DRIFT.slice(0, 7))
  await expect(card).not.toContainText(UNTOUCHED_DRIFT.slice(0, 7))
})

// ---------------------------------------------------------------------------
// U3 / A2: the card grays on `active_branch`, never on the issue body's branch
// ---------------------------------------------------------------------------

/** The card body's opacity when grayed. */
const GRAYED_OPACITY = '0.45'

/**
 * The exact bug the segment model deletes.
 *
 * #114's Round 2 was opened on `feature/reanalysis`; the issue body still says `main`,
 * and `issue.branch` — still on the response — still reports `main`. The user is checked
 * out on `feature/reanalysis`, which is where the status was computed, so the card must
 * read normally. Reading `issue.branch` here would gray the very issue being QC'd.
 */
test('U3: a card whose active branch is the checkout is not grayed, even when the issue body disagrees', async ({ page }) => {
  // Pins the premise: the wrong field is still sitting on the response.
  expect(crossBranchStatus.issue.branch).toBe('main')
  expect(crossBranchStatus.active_branch).toBe('feature/reanalysis')

  await setupRoutes(page, {
    repo: { ...defaultRepoInfo, branch: 'feature/reanalysis' },
    milestoneIssues: { 1: [crossBranchIssue] },
    issueStatuses: { results: [crossBranchStatus], errors: [] },
  })
  await page.goto('/')
  await selectMilestone(page, 'Sprint 1')

  const body = page.getByTestId(`issue-card-body-${crossBranchIssue.number}`)
  await expect(body).toHaveCSS('opacity', '1')
  await expect(body).toContainText('feature/reanalysis')
  await expect(body).not.toContainText('different branch')
})

/** The converse, so the graying is not simply switched off. */
test('U3: a card whose active branch is not the checkout is grayed', async ({ page }) => {
  await setupRoutes(page, {
    repo: { ...defaultRepoInfo, branch: 'main' },
    milestoneIssues: { 1: [crossBranchIssue] },
    issueStatuses: { results: [crossBranchStatus], errors: [] },
  })
  await page.goto('/')
  await selectMilestone(page, 'Sprint 1')

  const body = page.getByTestId(`issue-card-body-${crossBranchIssue.number}`)
  await expect(body).toHaveCSS('opacity', GRAYED_OPACITY)
  await expect(body).toContainText('different branch')
})

/**
 * D15 clause 2: the active segment could not be placed, so the record cannot be read at
 * face value even though S1 still says `Approved`. Both facts hold at once — that is the
 * intended combination, not a contradiction — and the reason is named, because its remedy
 * (restore the missing history) is not the remedy for a checkout mismatch.
 *
 * A trailing gap's continuity is always `linear`, so a vanished approval degrades through
 * `unplaceable` (clause 2) rather than through `unrelated` (clause 3, unreachable).
 *
 * The status is `unknown`, **not** `approved`. An earlier version of this test asserted
 * "it still reads as approved"; D15's second addendum showed that state is not
 * producible, because S4 short-circuits on an unplaceable gap before S1 can return
 * `Approved`. What is pinned here is that the card grays and *names why* — and that it
 * does not claim a workflow state it cannot support.
 */
test('D15: an unplaceable active segment grays the card, names why, and reports no status', async ({ page }) => {
  await setupRoutes(page, {
    repo: { ...defaultRepoInfo, branch: 'main' },
    milestoneIssues: { 1: [vanishedApprovalIssue] },
    issueStatuses: { results: [vanishedApprovalStatus], errors: [] },
  })
  await page.goto('/')
  await selectMilestone(page, 'Sprint 1')
  // Closed issue: the toggle is what puts it on screen at all.
  await page.getByRole('switch', { name: 'Include closed issues' }).click()

  const body = page.getByTestId(`issue-card-body-${vanishedApprovalIssue.number}`)
  await expect(body).toBeVisible()

  // Grayed, and not because of the branch — the checkout matches.
  await expect(body).toContainText('main')
  await expect(body).not.toContainText('different branch')
  await expect(body).toHaveCSS('opacity', GRAYED_OPACITY)

  // The reason is named, distinguishably from a branch mismatch.
  await expect(page.getByTestId(`gray-reason-${vanishedApprovalIssue.number}`)).toContainText(
    'the round bounding it could not be placed',
  )

  // It does NOT sit in the Approved lane — `unknown` asserts nothing, so claiming the
  // approved state would be exactly the lie D15's second addendum removed. Scoped to the
  // lane's own container: scoping to "an element containing the Approved heading" would
  // match <body> and pass from any lane at all.
  await expect(
    page.getByTestId('lane-approved').getByRole('link', { name: /src\/vanished-approval\.rs/ }),
  ).toHaveCount(0)
  // The premise for that negative — the card really is on screen, in another lane — so
  // the assertion above cannot pass merely because nothing rendered.
  await expect(
    page.getByRole('link', { name: /src\/vanished-approval\.rs/ }),
  ).toBeVisible()
})

// ---------------------------------------------------------------------------
// D12: the Reviewed and Last Posted rows read event-named commits, not the tip
// ---------------------------------------------------------------------------

/**
 * `latest_commit` used to mean "a commit a comment named"; M8 redefined it as the
 * newest commit of the active segment. Both fixtures below have a drift commit as that
 * newest commit, so a card still reading `latest_commit` here would label unreviewed
 * drift as reviewed — which is the failure D12 exists to prevent.
 */
test('D12: the Reviewed row names the reviewed commit, not the branch tip', async ({ page }) => {
  // The premise: the two facts genuinely differ on this response.
  expect(changeRequestedDriftStatus.qc_status.latest_commit).toBe(ROUND2_DRIFT)
  expect(changeRequestedDriftStatus.qc_status.last_reviewed_commit).toBe(ROUND2_OPENED)
  // And the response is one the backend could emit: `change_requested` requires the
  // newest *file-changing* commit to be covered by the review, so the drift — newer
  // than the review, and the reason `latest_commit` differs — must not touch the file.
  // Otherwise `Round::status()` would read `changes_to_comment` and this fixture would
  // pin a state that never occurs.
  const round2 = changeRequestedDriftStatus.segments[2]
  expect(round2.commits[0]).toMatchObject({ hash: ROUND2_DRIFT, file_changed: false })
  expect(round2.commits[1].file_changed).toBe(true)

  await setupRoutes(page, {
    milestoneIssues: { 1: [driftedRoundIssue] },
    issueStatuses: { results: [changeRequestedDriftStatus], errors: [] },
  })
  await page.goto('/')
  await selectMilestone(page, 'Sprint 1')

  const body = page.getByTestId(`issue-card-body-${driftedRoundIssue.number}`)
  await expect(body).toContainText('Reviewed')
  await expect(body).toContainText(ROUND2_OPENED.slice(0, 7))
  await expect(body).not.toContainText(ROUND2_DRIFT.slice(0, 7))
})

test('D12: the Last Posted row names the notified commit, not the branch tip', async ({ page }) => {
  expect(changesToCommentDriftStatus.qc_status.latest_commit).toBe(ROUND2_DRIFT)
  expect(changesToCommentDriftStatus.qc_status.last_notified_commit).toBe(ROUND2_OPENED)
  // Producible, again: `last_reviewed_commit: null` means nothing was reviewed, so the
  // round carries no review event for the API's projection to derive one from — and
  // `changes_to_comment` needs the drift to be an uncovered *file* change.
  const round2 = changesToCommentDriftStatus.segments[2]
  expect(round2.kind === 'round' && round2.events.map((e) => e.kind)).toEqual(['notification'])
  expect(round2.commits[0]).toMatchObject({ hash: ROUND2_DRIFT, file_changed: true })

  await setupRoutes(page, {
    milestoneIssues: { 1: [driftedRoundIssue] },
    issueStatuses: { results: [changesToCommentDriftStatus], errors: [] },
  })
  await page.goto('/')
  await selectMilestone(page, 'Sprint 1')

  const body = page.getByTestId(`issue-card-body-${driftedRoundIssue.number}`)
  await expect(body).toContainText('Last Posted')
  await expect(body).toContainText(ROUND2_OPENED.slice(0, 7))
  await expect(body).not.toContainText(ROUND2_DRIFT.slice(0, 7))
})

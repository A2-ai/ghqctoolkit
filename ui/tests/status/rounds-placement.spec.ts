import { test, expect } from 'playwright/test'
import type { Page } from 'playwright/test'
import { setupRoutes } from '../helpers/routes'
import {
  approvalNotOnBranchError,
  branchInheritedStatus,
  defaultRepoInfo,
  holeRoundsIssue,
  holeRoundsStatus,
  latestRoundUnplaceableError,
  openMilestone,
  twoRoundIssue,
  unplaceableRoundIssue,
  unplaceableRoundStatus,
  R1_APPROVAL,
  R1_UNPLACEABLE_APPROVAL,
  R2_APPROVAL,
  R3_START,
  UNFETCHED_BRANCH,
} from '../fixtures/index'
import type { Issue, IssueStatusResponse } from '../../src/api/issues'

// §18 (D53–D56): the fold no longer degrades silently. A round it cannot place is
// represented as unplaceable and the UI says which branch to fetch; a dropped
// malformed round leaves a hole in `rounds[]` rather than renumbering the rest.

async function goToStatus(page: Page) {
  await page.goto('/')
  await page.getByPlaceholder('Search milestones…').click()
  await page.getByRole('option', { name: /Sprint 1/ }).click()
}

async function setupStatus(page: Page, issues: Issue[], statuses: IssueStatusResponse[]) {
  await setupRoutes(page, {
    repo: defaultRepoInfo,
    milestones: [openMilestone],
    milestoneIssues: { 1: issues },
    issueStatuses: { results: statuses, errors: [] },
  })
  await goToStatus(page)
}

/** Opens the detail modal on the Notify tab and returns its panel. */
async function openNotifyPanel(page: Page, issueNumber: number) {
  await page.getByTestId(`issue-card-${issueNumber}`).click()
  await expect(page.getByRole('tablist')).toBeVisible()
  await page.getByRole('tab', { name: 'Notify' }).click()
  // Only the active tab panel is in the accessibility tree.
  return page.getByRole('dialog').getByRole('tabpanel')
}

// ---------------------------------------------------------------------------
// D53/D55 — an unplaceable round renders a fetch-branch state, never a hash
// ---------------------------------------------------------------------------

test('D53/D55: an unplaceable round renders "fetch <branch>" and no commit hash', async ({ page }) => {
  await setupStatus(page, [unplaceableRoundIssue], [unplaceableRoundStatus])

  const panel = await openNotifyPanel(page, unplaceableRoundIssue.number)

  // D71: the default is the latest round, which is placed — no fetch state on the panel.
  await expect(panel.getByTestId('history-select')).toBeVisible()
  await expect(panel.getByTestId('unplaceable-round-notice')).toHaveCount(0)

  // D53.1/D77: the unplaceable round was not dropped — it is a row in the History
  // dropdown, carrying its remedy.
  await panel.getByTestId('history-select-trigger').click()
  const row = page.getByTestId('history-row-round:1')
  await expect(row.getByTestId('fetch-branch-badge')).toContainText(UNFETCHED_BRANCH)

  // D54/D55: never a hash — not the round's own unplaceable approval commit, and not
  // any substitute smuggled into the row (the pre-§18 fold re-bounded such a round at
  // the branch tip).
  await expect(row.getByText(R1_UNPLACEABLE_APPROVAL.slice(0, 7))).toHaveCount(0)
  expect(await row.textContent()).not.toMatch(/[0-9a-f]{7}/)

  // Selecting it names the branch to fetch on the panel too.
  await page.getByTestId('history-check-round:1').click()
  await expect(panel.getByTestId('unplaceable-round-notice')).toContainText(UNFETCHED_BRANCH)
})

test('D53.3/D77: selecting an unplaceable round adds nothing, and says so', async ({ page }) => {
  await setupStatus(page, [unplaceableRoundIssue], [unplaceableRoundStatus])

  const panel = await openNotifyPanel(page, unplaceableRoundIssue.number)
  const countBefore = await panel.getByTestId('history-commit-count').textContent()

  await panel.getByTestId('history-select-trigger').click()
  await page.getByTestId('history-check-round:1').click()

  // `commits` is empty (D53.3), so the selection contributes nothing — and the panel
  // says why rather than adding zero commits in silence.
  await expect(panel.getByTestId('history-commit-count')).toHaveText(countBefore ?? '')
  await expect(panel.getByTestId('unplaceable-round-notice')).toBeVisible()
})

// ---------------------------------------------------------------------------
// D55 — an issue whose *latest* round is unresolvable arrives in errors[]
// ---------------------------------------------------------------------------

test('D55: a latest-round branch_not_local error uses the existing fetch affordance', async ({ page }) => {
  await setupRoutes(page, {
    milestones: [openMilestone],
    milestoneIssues: { 1: [unplaceableRoundIssue] },
    issueStatuses: latestRoundUnplaceableError,
    issueStatusesCode: 206,
  })
  await goToStatus(page)

  // S5: no new `QCStatus` variant — the issue is simply not in `results[]`, so no
  // card is rendered for it.
  await expect(page.getByRole('link', { name: /src\/unplaceable\.rs/ })).toHaveCount(0)

  // The branch is named, and the copy-pasteable fix is the same one the pre-existing
  // `branch_not_local` affordance builds.
  const errorIcon = page.getByTestId('status-error-count')
  await expect(errorIcon).toBeVisible()
  await errorIcon.click()
  await expect(page.getByText(`git branch --track '${UNFETCHED_BRANCH}' origin/${UNFETCHED_BRANCH}`)).toBeVisible()
})

test('D60: a rewritten-approval error shows its message and offers no fetch command', async ({ page }) => {
  await setupRoutes(page, {
    milestones: [openMilestone],
    milestoneIssues: { 1: [unplaceableRoundIssue] },
    issueStatuses: approvalNotOnBranchError,
    issueStatusesCode: 206,
  })
  await goToStatus(page)

  const errorIcon = page.getByTestId('status-error-count')
  await expect(errorIcon).toBeVisible()
  await errorIcon.click()

  // The backend's message explains what happened…
  await expect(page.getByText(/no longer reachable on branch/)).toBeVisible()
  // …and no fetch/track command is offered: the branch is already local, so the
  // command would do nothing and read as a broken tool (D60). Recovering a rewritten
  // approval is a judgement call, so there is no single safe command to suggest.
  await expect(page.getByText(/git branch --track/)).toHaveCount(0)
  await expect(page.getByText('Click to show fix')).toHaveCount(0)
})

// ---------------------------------------------------------------------------
// D56 — an inherited branch is surfaced where the round is viewed
// ---------------------------------------------------------------------------

test('D56: the round switcher surfaces an inherited branch', async ({ page }) => {
  await setupStatus(page, [twoRoundIssue], [branchInheritedStatus])

  const panel = await openNotifyPanel(page, twoRoundIssue.number)
  await panel.getByTestId('history-select-trigger').click()

  // Round 2 declared no `git branch:` — it inherited round 1's — and the claim sits on
  // round 2's own row.
  await expect(page.getByTestId('history-row-round:2').getByTestId('branch-inherited-badge')).toBeVisible()

  // Round 1 declared its own, so nothing is claimed about it.
  await expect(page.getByTestId('history-row-round:1').getByTestId('branch-inherited-badge')).toHaveCount(0)
})

test('D56: the new-round modal surfaces the prior round\'s inherited branch', async ({ page }) => {
  await setupStatus(page, [twoRoundIssue], [branchInheritedStatus])

  await page.getByTestId(`new-round-${twoRoundIssue.number}`).click()
  const dialog = page.getByRole('dialog', { name: /Start QC Round/ })
  await expect(dialog).toBeVisible()

  // Branch is load-bearing for two commit walks (D7/D9), so the modal that starts the
  // next round says the branch it is comparing against was inherited.
  await expect(dialog.getByTestId('new-round-inherited-branch')).toContainText('inherited')
  await expect(dialog.getByTestId('branch-inherited-badge')).toBeVisible()
})

// ---------------------------------------------------------------------------
// D53.2 — `rounds` can have a hole; nothing derives a number from a position
// ---------------------------------------------------------------------------

test('D53.2: a hole in the round indices is rendered as declared, never renumbered', async ({ page }) => {
  await setupStatus(page, [holeRoundsIssue], [holeRoundsStatus])

  const panel = await openNotifyPanel(page, holeRoundsIssue.number)
  await panel.getByTestId('history-select-trigger').click()

  // `rounds` is [1, 3] — round 2's declaration was malformed and was dropped without
  // renumbering. M2 carries declared indices, so the rows are round 1, the gap *before
  // round 3*, and round 3. A positional index would name that gap 2 — a round that does
  // not exist.
  await expect(page.getByTestId('history-row-round:3')).toBeVisible()
  await expect(page.getByTestId('history-row-gap:3')).toBeVisible()
  await expect(page.getByTestId('history-row-round:2')).toHaveCount(0)
  await expect(page.getByTestId('history-row-gap:2')).toHaveCount(0)

  // D71: the default is the latest round — round 3, whose commits are what the slider
  // shows.
  await expect(panel.getByText(R3_START.slice(0, 7))).toBeVisible()
  await expect(panel.getByText(R1_APPROVAL.slice(0, 7))).toHaveCount(0)

  // Round 1 is still reachable at its declared index, and adds to the view.
  await page.getByTestId('history-check-round:1').click()
  await expect(panel.getByText(R1_APPROVAL.slice(0, 7)).first()).toBeVisible()
  await expect(panel.getByText(R3_START.slice(0, 7))).toBeVisible()
})

test('D53.2: the next round is the latest index + 1, not rounds.length + 1', async ({ page }) => {
  await setupStatus(page, [holeRoundsIssue], [holeRoundsStatus])

  await page.getByTestId(`new-round-${holeRoundsIssue.number}`).click()
  // Two rounds exist but the latest is round 3, so the next one is 4. Deriving the
  // number from `rounds.length` would name it 3 — a round that already exists.
  const dialog = page.getByRole('dialog', { name: 'Start QC Round 4 — src/round-hole.rs' })
  await expect(dialog).toBeVisible()
  await expect(dialog.getByTestId('new-round-submit')).toHaveText('Start Round 4')

  // The base-round Select offers the declared indices, hole included.
  await dialog.getByRole('tab', { name: 'Checklist' }).click()
  await dialog.getByTestId('new-round-base-round').click()
  await expect(page.getByRole('option', { name: /Round 3/ })).toBeVisible()
  await expect(page.getByRole('option', { name: /Round 2/ })).toHaveCount(0)
})

// The approved hash of a *placed* round is still shown — §18 withholds the hash only
// where it cannot be placed, and the U8 deep-link must not regress.
test('D53: a placed round still shows its approval hash', async ({ page }) => {
  await setupStatus(page, [unplaceableRoundIssue], [unplaceableRoundStatus])

  const panel = await openNotifyPanel(page, unplaceableRoundIssue.number)
  await expect(panel.getByText(R2_APPROVAL.slice(0, 7)).first()).toBeVisible()
})

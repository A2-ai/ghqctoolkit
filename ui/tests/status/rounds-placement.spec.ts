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
  const switcher = panel.getByTestId('round-switcher')

  // The default is the latest round, which is placed — no fetch state there.
  await expect(switcher).toBeVisible()
  await expect(panel.getByTestId('fetch-branch-badge')).toHaveCount(0)
  await expect(panel.getByTestId('unplaceable-round-notice')).toHaveCount(0)

  // D53.1: the unplaceable round was not dropped — it is still selectable at its
  // declared index.
  await switcher.getByText('1', { exact: true }).click()

  await expect(panel.getByTestId('fetch-branch-badge')).toContainText(UNFETCHED_BRANCH)
  await expect(panel.getByTestId('unplaceable-round-notice')).toContainText(UNFETCHED_BRANCH)

  // D54/D55: never a hash. Not the round's own unplaceable approval commit…
  await expect(panel.getByText(R1_UNPLACEABLE_APPROVAL.slice(0, 7))).toHaveCount(0)
  // …and not any substitute smuggled into the switcher either (the pre-§18 fold
  // re-bounded such a round at the branch tip).
  expect(await switcher.textContent()).not.toMatch(/[0-9a-f]{7}/)
})

test('D53.3: an unplaceable round owns no commits, so the slider is absent, not blank', async ({ page }) => {
  await setupStatus(page, [unplaceableRoundIssue], [unplaceableRoundStatus])

  const panel = await openNotifyPanel(page, unplaceableRoundIssue.number)
  await panel.getByTestId('round-switcher').getByText('1', { exact: true }).click()

  // `commits` is empty (D53.3), so there is no commit UI at all — and the panel says
  // why instead of simply going quiet.
  await expect(panel.getByText('Select Commits to Compare')).toHaveCount(0)
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
  // Round 2 declared no `git branch:` — it inherited round 1's.
  await expect(panel.getByTestId('branch-inherited-badge')).toBeVisible()

  // Round 1 declared its own, so nothing is claimed about it.
  await panel.getByTestId('round-switcher').getByText('1', { exact: true }).click()
  await expect(panel.getByTestId('branch-inherited-badge')).toHaveCount(0)
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
  const switcher = panel.getByTestId('round-switcher')

  // `rounds` is [1, 3] — round 2's declaration was malformed and was dropped without
  // renumbering. Position 1 holds round *3*: a `i + 1` label would say "2".
  await expect(switcher.getByText('3', { exact: true })).toBeVisible()
  await expect(switcher.getByText('2', { exact: true })).toHaveCount(0)

  // The status card reads `.index`, and the default selection is the latest round —
  // round 3, whose own commits are the ones the slider shows.
  await expect(panel.getByText('Round:', { exact: false }).first()).toBeVisible()
  await expect(panel.getByText('3', { exact: true }).first()).toBeVisible()
  await expect(panel.getByText(R3_START.slice(0, 7))).toBeVisible()
  await expect(panel.getByText(R1_APPROVAL.slice(0, 7))).toHaveCount(0)

  // Round 1 is still reachable at its declared index.
  await switcher.getByText('1', { exact: true }).click()
  await expect(panel.getByText(R1_APPROVAL.slice(0, 7)).first()).toBeVisible()
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

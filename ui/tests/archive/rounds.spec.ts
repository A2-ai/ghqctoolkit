import { test, expect } from 'playwright/test'
import type { Locator, Page } from 'playwright/test'
import { setupRoutes } from '../helpers/routes'
import {
  R1_APPROVAL,
  R2_APPROVAL,
  twoRoundIssue,
  twoRoundStatus,
} from '../fixtures/index'
import type { Milestone } from '../../src/api/milestones'

// The archive tab reads QC files out of milestones, so the fixture issue needs a
// milestone that exists in the milestone list.
const sprint1: Milestone = {
  number: 1,
  title: 'Sprint 1',
  state: 'closed',
  description: null,
  open_issues: 0,
  closed_issues: 1,
}

const archiveIssue = { ...twoRoundIssue, state: 'closed' as const, closed_at: '2024-01-02T00:00:00Z' }
const archiveStatus = { ...twoRoundStatus, issue: archiveIssue }

async function goToArchive(page: Page) {
  await page.goto('/')
  const archiveTabButton = page.getByRole('button', { name: 'Archive', exact: true })
  const moreButton = page.getByRole('button', { name: 'More', exact: true })
  await expect(archiveTabButton.or(moreButton).first()).toBeVisible({ timeout: 10_000 })
  if (await archiveTabButton.isVisible()) {
    await archiveTabButton.click()
    return
  }
  await moreButton.click()
  await page.getByRole('menuitem', { name: 'Archive', exact: true }).click()
}

async function setupArchive(page: Page) {
  await setupRoutes(page, {
    milestones: [sprint1],
    milestoneIssues: { 1: [archiveIssue] },
    issueStatuses: { results: [archiveStatus], errors: [] },
  })
  await goToArchive(page)
  await page.locator('main').getByPlaceholder('Search milestones…').click()
  await page.getByRole('option', { name: /Sprint 1/ }).click()
  await expect(page.getByText(/issues? loading/)).not.toBeVisible({ timeout: 10_000 })
}

test('U5: the round select defaults to the latest round and shows its resolved commit', async ({ page }) => {
  await setupArchive(page)

  const select = page.getByTestId(`archive-round-select-${archiveIssue.number}`)
  await expect(select).toHaveValue('Round 2')
  await expect(page.getByText(R2_APPROVAL.slice(0, 7))).toBeVisible()
})

test('U5: selecting an earlier round shows its commit and the subsequent-changes badge', async ({ page }) => {
  await setupArchive(page)

  await page.getByTestId(`archive-round-select-${archiveIssue.number}`).click()
  await page.getByRole('option', { name: 'Round 1' }).click()

  await expect(page.getByText(R1_APPROVAL.slice(0, 7))).toBeVisible()
  // Round 1 has file-changing commits after it (its `subsequent_file_changes`).
  await expect(page.getByTestId(`archive-subsequent-changes-${archiveIssue.number}`)).toBeVisible()
})

test('D28.3: generation sends all four QC fields for a QC-attached file', async ({ page }) => {
  await setupArchive(page)

  let body: { files?: Record<string, unknown>[] } | null = null
  await page.route(/\/api\/archive\/generate/, async (route, request) => {
    body = request.postDataJSON()
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ output_path: '/mock/repo/out.tar.gz' }),
    })
  })

  await page.getByRole('button', { name: 'Generate Archive' }).click()
  await expect.poll(() => body !== null).toBe(true)

  // A partial quartet is a 400 (D28.3) — milestone, approved, round and
  // subsequent_file_changes must all be present.
  expect(body!.files).toEqual([
    {
      repository_file: archiveIssue.title,
      commit: R2_APPROVAL,
      milestone: 'Sprint 1',
      approved: true,
      round: 2,
      subsequent_file_changes: false,
    },
  ])
})

test('D28.3: the selected round is the one frozen into the request', async ({ page }) => {
  await setupArchive(page)

  await page.getByTestId(`archive-round-select-${archiveIssue.number}`).click()
  await page.getByRole('option', { name: 'Round 1' }).click()

  let body: { files?: Record<string, unknown>[] } | null = null
  await page.route(/\/api\/archive\/generate/, async (route, request) => {
    body = request.postDataJSON()
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ output_path: '/mock/repo/out.tar.gz' }),
    })
  })

  await page.getByRole('button', { name: 'Generate Archive' }).click()
  await expect.poll(() => body !== null).toBe(true)

  expect(body!.files).toEqual([
    {
      repository_file: archiveIssue.title,
      commit: R1_APPROVAL,
      milestone: 'Sprint 1',
      approved: true,
      round: 1,
      // Round 1 is followed by file-changing commits — the fact the archive freezes.
      subsequent_file_changes: true,
    },
  ])
})

// ---------------------------------------------------------------------------
// Added files — the quartet on the two paths the milestone-card test never reaches
// ---------------------------------------------------------------------------

// A QC issue in a *different* milestone than the one selected in the tab, so the
// file added from it is a genuine added file rather than a milestone card.
const sprint2: Milestone = {
  number: 2,
  title: 'Sprint 2',
  state: 'closed',
  description: null,
  open_issues: 0,
  closed_issues: 1,
}

const addedQcIssue = {
  ...twoRoundIssue,
  number: 77,
  title: 'src/utils.rs',
  milestone: 'Sprint 2',
  state: 'closed' as const,
  closed_at: '2024-01-02T00:00:00Z',
}
const addedQcStatus = { ...twoRoundStatus, issue: addedQcIssue }

/** The archive tab with Sprint 1 selected and Sprint 2's QC issue reachable from the
 *  add-file modal's "Select Issue" tab. */
async function setupArchiveWithAddableQc(page: Page) {
  await setupRoutes(page, {
    milestones: [sprint1, sprint2],
    milestoneIssues: { 1: [archiveIssue], 2: [addedQcIssue] },
    issueStatuses: { results: [archiveStatus, addedQcStatus], errors: [] },
  })
  await goToArchive(page)
  await page.locator('main').getByPlaceholder('Search milestones…').click()
  await page.getByRole('option', { name: /Sprint 1/ }).click()
  await expect(page.getByText(/issues? loading/)).not.toBeVisible({ timeout: 10_000 })
}

/** Picks `src/{name}` in the add-file modal and returns with the modal on step 2. */
async function pickAddedFile(page: Page, name: string) {
  await page.getByTestId('archive-add-file-card').click()
  const modal = page.getByRole('dialog')
  await expect(modal).toBeVisible()
  await modal.getByRole('treeitem', { name: 'src' }).click()
  await modal.getByText(name, { exact: true }).click()
  await modal.getByRole('button', { name: 'Next →' }).click()
  return modal
}

/** Picks the QC issue on the modal's "Select Issue" tab. The click must land on the
 *  card body — the title is an `<Anchor>` that deliberately stops propagation. */
async function selectAddedFileIssue(modal: Locator) {
  await modal.getByRole('tab', { name: /Select Issue/ }).click()
  await modal.getByText(`Commit: ${R2_APPROVAL.slice(0, 7)}`).click()
}

async function captureGenerate(page: Page) {
  const captured: { body: { files?: Record<string, unknown>[] } | null } = { body: null }
  await page.route(/\/api\/archive\/generate/, async (route, request) => {
    captured.body = request.postDataJSON()
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ output_path: '/mock/repo/out.tar.gz' }),
    })
  })
  await page.getByRole('button', { name: 'Generate Archive' }).click()
  await expect.poll(() => captured.body !== null).toBe(true)
  return captured
}

test('D28.3: a file added via Select Issue sends all four QC fields', async ({ page }) => {
  await setupArchiveWithAddableQc(page)

  const modal = await pickAddedFile(page, 'utils.rs')
  await selectAddedFileIssue(modal)

  // The card is QC-backed, so it carries a round select of its own.
  await expect(page.getByTestId(`archive-round-select-${addedQcIssue.number}`)).toBeVisible()

  const captured = await captureGenerate(page)
  const entry = captured.body!.files!.find((f) => f.repository_file === addedQcIssue.title)
  expect(entry).toEqual({
    repository_file: addedQcIssue.title,
    commit: R2_APPROVAL,
    milestone: 'Sprint 2',
    approved: true,
    round: 2,
    subsequent_file_changes: false,
  })
})

test('D28.3: a bare added file sends none of the four QC fields', async ({ page }) => {
  await setupArchiveWithAddableQc(page)

  // `src/main.rs` has no QC issue, so it resolves to a plain commit.
  const modal = await pickAddedFile(page, 'main.rs')
  await modal.getByRole('button', { name: /Use commit/ }).click()

  const captured = await captureGenerate(page)
  const entry = captured.body!.files!.find((f) => f.repository_file === 'src/main.rs')
  // Exactly two keys: the quartet is all-or-nothing, and the pre-rewrite code sent
  // `approved: false` alone here — a guaranteed 400 (D28.3).
  expect(entry).toEqual({ repository_file: 'src/main.rs', commit: 'abc1234567890' })
})

// ---------------------------------------------------------------------------
// The round select is portaled — its options must not click the card underneath
// ---------------------------------------------------------------------------

test('U5: choosing a round on a QC-attached added file does not open the edit modal', async ({ page }) => {
  await setupArchiveWithAddableQc(page)

  const modal = await pickAddedFile(page, 'utils.rs')
  await selectAddedFileIssue(modal)
  await expect(page.getByRole('dialog')).toHaveCount(0)

  // Mantine portals the dropdown, but its options stay React-tree children of the
  // card's `onClick` — React bubbles the fiber tree, not the DOM tree.
  const select = page.getByTestId(`archive-round-select-${addedQcIssue.number}`)
  await select.click()
  await expect(page.getByRole('dialog')).toHaveCount(0)
  await page.getByRole('option', { name: 'Round 1' }).click()

  // The selection took effect…
  await expect(select).toHaveValue('Round 1')
  // …and the card's own onClick did not fire, so no "Edit: {file}" modal opened.
  await expect(page.getByRole('dialog')).toHaveCount(0)
})

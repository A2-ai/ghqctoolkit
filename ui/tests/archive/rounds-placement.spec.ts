import { test, expect } from 'playwright/test'
import type { Page } from 'playwright/test'
import { setupRoutes } from '../helpers/routes'
import {
  holeRoundsIssue,
  holeRoundsStatus,
  twoRoundIssue,
  twoRoundStatus,
  unplaceableRoundIssue,
  unplaceableRoundStatus,
  R1_UNPLACEABLE_APPROVAL,
  R2_APPROVAL,
  R3_APPROVAL,
  UNFETCHED_BRANCH,
} from '../fixtures/index'
import type { IssueStatusResponse } from '../../src/api/issues'
import type { Milestone } from '../../src/api/milestones'
import type { ArchiveGenerateResponse, SkippedFileRequest } from '../../src/api/archive'

// D55: "Archive — selecting a round whose commit does not resolve is an error naming
// the branch. It must **never** freeze a substitute commit." The substitution is the
// audit lie §18 removes: the archive would claim "this content was approved" over
// content that was not the approved content.

const sprint1: Milestone = {
  number: 1,
  title: 'Sprint 1',
  state: 'closed',
  description: null,
  open_issues: 0,
  closed_issues: 1,
}

function closed<T extends { state: 'open' | 'closed'; closed_at: string | null }>(issue: T): T {
  return { ...issue, state: 'closed' as const, closed_at: '2024-01-02T00:00:00Z' }
}

const archiveIssue = closed(unplaceableRoundIssue)
const archiveStatus: IssueStatusResponse = { ...unplaceableRoundStatus, issue: archiveIssue }

const holeIssue = closed(holeRoundsIssue)
const holeStatus: IssueStatusResponse = { ...holeRoundsStatus, issue: holeIssue }

// A second, fully placed file in the same milestone. D62 is about the *rest* of the
// archive surviving one unresolvable round, so the interesting case needs two files.
const placedIssue = closed(twoRoundIssue)
const placedStatus: IssueStatusResponse = { ...twoRoundStatus, issue: placedIssue }

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

async function setupArchive(page: Page, issue: typeof archiveIssue, status: IssueStatusResponse) {
  await setupArchiveAll(page, [issue], [status])
}

async function setupArchiveAll(page: Page, issues: typeof archiveIssue[], statuses: IssueStatusResponse[]) {
  await setupRoutes(page, {
    milestones: [sprint1],
    milestoneIssues: { 1: issues },
    issueStatuses: { results: statuses, errors: [] },
  })
  await goToArchive(page)
  await page.locator('main').getByPlaceholder('Search milestones…').click()
  await page.getByRole('option', { name: /Sprint 1/ }).click()
  await expect(page.getByText(/issues? loading/)).not.toBeVisible({ timeout: 10_000 })
}

interface GeneratePostBody {
  files?: Record<string, unknown>[]
  /** D62: the client's own skip declaration, written verbatim into the metadata.
   *  Optional on the *request* only — the response always carries it. */
  skipped?: SkippedFileRequest[]
}

/**
 * Counts POSTs to /api/archive/generate and answers them with a success body.
 *
 * D62: the real server echoes back what it wrote into the manifest, and `skipped` is
 * non-optional in the response (always `[]` when nothing was skipped), so this mock
 * echoes the declaration the same way.
 */
async function countGenerate(page: Page) {
  const captured: { count: number; bodies: GeneratePostBody[] } = { count: 0, bodies: [] }
  await page.route(/\/api\/archive\/generate/, async (route, request) => {
    captured.count += 1
    const body = request.postDataJSON() as GeneratePostBody
    captured.bodies.push(body)
    const response: ArchiveGenerateResponse = {
      output_path: '/mock/repo/out.tar.gz',
      skipped: body.skipped ?? [],
    }
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify(response),
    })
  })
  return captured
}

test('D55: an unresolved round names the branch to fetch instead of a commit', async ({ page }) => {
  await setupArchive(page, archiveIssue, archiveStatus)

  const select = page.getByTestId(`archive-round-select-${archiveIssue.number}`)
  // The default is the latest round, which is placed.
  await expect(select).toHaveValue('Round 2')
  await expect(page.getByText(R2_APPROVAL.slice(0, 7))).toBeVisible()

  await select.click()
  await page.getByRole('option', { name: 'Round 1' }).click()

  const unresolved = page.getByTestId(`archive-round-unresolved-${archiveIssue.number}`)
  await expect(unresolved).toContainText(UNFETCHED_BRANCH)
  await expect(unresolved).toContainText('round 1')
  // Never a hash: not the round's own declared approval, and not the placed round's
  // commit standing in for it.
  await expect(page.getByText(R1_UNPLACEABLE_APPROVAL.slice(0, 7))).toHaveCount(0)
  await expect(page.getByText(R2_APPROVAL.slice(0, 7))).toHaveCount(0)
})

test('D62: an unresolvable round is skipped, not blocking — declared, never substituted', async ({ page }) => {
  await setupArchiveAll(page, [archiveIssue, placedIssue], [archiveStatus, placedStatus])
  const captured = await countGenerate(page)

  await page.getByTestId(`archive-round-select-${archiveIssue.number}`).click()
  await page.getByRole('option', { name: 'Round 1' }).click()

  // 1. The omission is announced, and names the branch and the round (D53.5).
  const notice = page.getByTestId('archive-round-skips')
  await expect(notice).toContainText(UNFETCHED_BRANCH)
  await expect(notice).toContainText('round 1')
  await expect(notice).toContainText('1 file will be skipped')

  // 2. And it does not block: D61's reasoning — one stale branch must not deny the
  // user the other forty-nine files.
  const generate = page.getByRole('button', { name: 'Generate Archive' })
  await expect(generate).toBeEnabled()
  await generate.click()
  await expect.poll(() => captured.count).toBe(1)

  // 3. The skipped file is *absent from `files`* — not present with a substituted
  // commit. That substitution is the audit lie §18 removes.
  expect(captured.bodies[0].files).toEqual([
    {
      repository_file: placedIssue.title,
      commit: R2_APPROVAL,
      milestone: 'Sprint 1',
      approved: true,
      round: 2,
      subsequent_file_changes: false,
    },
  ])
  expect(captured.bodies[0].files?.map(f => f.repository_file)).not.toContain(archiveIssue.title)

  // …and declared in `skipped`, so the archive's own manifest says it is partial. Four
  // fields only: there is no commit to carry, which is the whole point.
  expect(captured.bodies[0].skipped).toEqual([
    {
      repository_file: archiveIssue.title,
      round: 1,
      branch: UNFETCHED_BRANCH,
      reason: `Fetch ${UNFETCHED_BRANCH} to archive round 1`,
    },
  ])

  // The response echoes the manifest's `skipped` list, so the confirmation says the
  // archive is partial rather than leaving that only in the file.
  await expect(page.getByTestId('archive-generate-skipped')).toContainText(archiveIssue.title)
})

test('D62: selecting a resolvable round is the remedy — the notice clears and the file returns to files', async ({ page }) => {
  await setupArchiveAll(page, [archiveIssue, placedIssue], [archiveStatus, placedStatus])
  const captured = await countGenerate(page)

  await page.getByTestId(`archive-round-select-${archiveIssue.number}`).click()
  await page.getByRole('option', { name: 'Round 1' }).click()
  await expect(page.getByTestId('archive-round-skips')).toBeVisible()

  // The notice keeps the remedy one click away (D62 "select a different round").
  await page.getByTestId(`archive-skip-choose-round-${archiveIssue.number}`).click()
  const select = page.getByTestId(`archive-round-select-${archiveIssue.number}`)
  await expect(select).toBeFocused()

  await select.click()
  await page.getByRole('option', { name: 'Round 2' }).click()
  await expect(page.getByTestId('archive-round-skips')).toHaveCount(0)

  await page.getByRole('button', { name: 'Generate Archive' }).click()
  await expect.poll(() => captured.count).toBe(1)
  // Nothing skipped ⇒ the field is omitted entirely, so a complete archive's request
  // keeps its previous shape.
  expect(captured.bodies[0].skipped).toBeUndefined()
  expect(captured.bodies[0].files).toEqual([
    {
      repository_file: placedIssue.title,
      commit: R2_APPROVAL,
      milestone: 'Sprint 1',
      approved: true,
      round: 2,
      subsequent_file_changes: false,
    },
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

test('D53.2: the archive round select and the frozen round read .index, not a position', async ({ page }) => {
  await setupArchive(page, holeIssue, holeStatus)

  // `rounds` is [1, 3]: the malformed round 2 was dropped without renumbering.
  const select = page.getByTestId(`archive-round-select-${holeIssue.number}`)
  await expect(select).toHaveValue('Round 3')
  await select.click()
  await expect(page.getByRole('option', { name: 'Round 1' })).toBeVisible()
  await expect(page.getByRole('option', { name: 'Round 2' })).toHaveCount(0)
  await page.getByRole('option', { name: 'Round 3' }).click()

  const captured = await countGenerate(page)
  await page.getByRole('button', { name: 'Generate Archive' }).click()
  await expect.poll(() => captured.count).toBe(1)
  // `round: 3` — the declared index. A position-derived number would freeze `2`, and
  // `ArchiveQC.round` would then disagree with the GitHub comment log (D53.2).
  expect(captured.bodies[0].files).toEqual([
    {
      repository_file: holeIssue.title,
      commit: R3_APPROVAL,
      milestone: 'Sprint 1',
      approved: true,
      round: 3,
      subsequent_file_changes: false,
    },
  ])
})

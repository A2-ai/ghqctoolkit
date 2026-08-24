import { readFile } from 'node:fs/promises'
import { test, expect } from 'playwright/test'
import type { Page } from 'playwright/test'
import { setupRoutes } from '../helpers/routes'
import {
  approvedIssue,
  approvedStatus,
  awaitingReviewIssue,
  awaitingReviewStatus,
  changeRequestedIssue,
  changeRequestedStatus,
  defaultRepoInfo,
  divergentDriftStatus,
  divergentGapStatus,
  nullCommentIdIssue,
  nullCommentIdStatus,
  openMilestone,
  twoRoundIssue,
  twoRoundStatus,
  DRIFT_DECOY,
  R1_APPROVAL,
  R1_START,
  R2_APPROVAL,
  R2_START,
} from '../fixtures/index'
import type { Issue, IssueStatusResponse } from '../../src/api/issues'

async function goToStatus(page: Page) {
  await page.goto('/')
  await page.getByPlaceholder('Search milestones…').click()
  await page.getByRole('option', { name: /Sprint 1/ }).click()
}

async function setupStatus(
  page: Page,
  issues: Issue[],
  statuses: IssueStatusResponse[],
  repo = defaultRepoInfo,
) {
  await setupRoutes(page, {
    repo,
    milestones: [openMilestone],
    milestoneIssues: { 1: issues },
    issueStatuses: { results: statuses, errors: [] },
  })
  await goToStatus(page)
}

// ---------------------------------------------------------------------------
// U1 — New Round only from an approved QC (D12)
// ---------------------------------------------------------------------------

test('U1: the New Round button appears only on approved and changes_after_approval', async ({ page }) => {
  const changesAfterApproval: IssueStatusResponse = {
    ...divergentDriftStatus,
    // Reuse the two-round issue's shape but not its divergent drift.
    drift: { ...divergentDriftStatus.drift, divergent: false },
  }
  await setupStatus(
    page,
    [approvedIssue, awaitingReviewIssue, changeRequestedIssue, twoRoundIssue],
    [approvedStatus, awaitingReviewStatus, changeRequestedStatus, changesAfterApproval],
  )

  await expect(page.getByTestId(`new-round-${approvedIssue.number}`)).toBeVisible()
  await expect(page.getByTestId(`new-round-${twoRoundIssue.number}`)).toBeVisible()
  await expect(page.getByTestId(`new-round-${awaitingReviewIssue.number}`)).toHaveCount(0)
  await expect(page.getByTestId(`new-round-${changeRequestedIssue.number}`)).toHaveCount(0)
})

test('U1: the New Round button is not in the notification modal', async ({ page }) => {
  await setupStatus(page, [twoRoundIssue], [twoRoundStatus])

  await page.getByTestId(`issue-card-${twoRoundIssue.number}`).click()
  await expect(page.getByRole('tablist')).toBeVisible()
  await page.getByRole('tab', { name: 'Notify' }).click()

  await expect(page.getByRole('dialog').getByRole('button', { name: 'New Round' })).toHaveCount(0)
})

// ---------------------------------------------------------------------------
// U4 — the round switcher scopes the slider to one round (§0.5's fix)
// ---------------------------------------------------------------------------

test('U4: the slider renders only the selected round\'s commits, latest by default', async ({ page }) => {
  await setupStatus(page, [twoRoundIssue], [twoRoundStatus])

  await page.getByTestId(`issue-card-${twoRoundIssue.number}`).click()
  await expect(page.getByRole('tablist')).toBeVisible()
  await page.getByRole('tab', { name: 'Notify' }).click()

  // Only the active tab panel is in the accessibility tree, so this scopes the
  // assertions to the Notify tab's own slider.
  const panel = page.getByRole('dialog').getByRole('tabpanel')
  const switcher = panel.getByTestId('round-switcher')
  await expect(switcher).toBeVisible()

  // Defaults to the latest round: round 2's commits only.
  await expect(panel.getByText(R2_START.slice(0, 7))).toBeVisible()
  await expect(panel.getByText(R1_START.slice(0, 7))).toHaveCount(0)
  await expect(panel.getByText(R1_APPROVAL.slice(0, 7))).toHaveCount(0)

  // Switch to round 1 — now round 1's commits only.
  await switcher.getByText('1', { exact: true }).click()
  await expect(panel.getByText(R1_START.slice(0, 7))).toBeVisible()
  // (R1_APPROVAL also appears in the switcher's approved badge and the From/To rows.)
  await expect(panel.getByText(R1_APPROVAL.slice(0, 7)).first()).toBeVisible()
  await expect(panel.getByText(R2_START.slice(0, 7))).toHaveCount(0)
})

// ---------------------------------------------------------------------------
// U6 — divergence badges
// ---------------------------------------------------------------------------

test('U6: a divergent preceding gap badges the round switcher', async ({ page }) => {
  await setupStatus(page, [twoRoundIssue], [divergentGapStatus])

  await page.getByTestId(`issue-card-${twoRoundIssue.number}`).click()
  await page.getByRole('tab', { name: 'Notify' }).click()

  const panel = page.getByRole('dialog').getByRole('tabpanel')
  await expect(panel.getByTestId('no-cohesive-history-badge')).toBeVisible()

  // Round 1 has no predecessor to diverge from (I13), so the badge goes away.
  await panel.getByTestId('round-switcher').getByText('1', { exact: true }).click()
  await expect(panel.getByTestId('no-cohesive-history-badge')).toHaveCount(0)
})

test('U6: a divergent drift badges the status card', async ({ page }) => {
  await setupStatus(page, [twoRoundIssue], [divergentDriftStatus])

  // On the card in the swimlane…
  await expect(page.getByTestId('drift-divergent-badge').first()).toBeVisible()

  // …and on the modal's status card.
  await page.getByTestId(`issue-card-${twoRoundIssue.number}`).click()
  await page.getByRole('tab', { name: 'Notify' }).click()
  await expect(page.getByRole('dialog').getByRole('tabpanel').getByTestId('drift-divergent-badge')).toBeVisible()
})

// ---------------------------------------------------------------------------
// U7 — the ChangesAfterApproval hash comes from drift.newest_file_change
// ---------------------------------------------------------------------------

test('U7: the changed commit is drift.newest_file_change, not a client-side rescan', async ({ page }) => {
  await setupStatus(page, [twoRoundIssue], [divergentDriftStatus])

  const card = page.getByTestId(`issue-card-${twoRoundIssue.number}`)
  await expect(card.getByText('Changed:')).toBeVisible()
  await expect(
    card.getByText(divergentDriftStatus.drift.newest_file_change!.slice(0, 7)),
  ).toBeVisible()
  // The fixture's `drift.commits` leads with a *different* file-changing commit, so a
  // client-side `commits.find(c => c.file_changed)` would render this hash instead.
  await expect(card.getByText(DRIFT_DECOY.slice(0, 7))).toHaveCount(0)
})

// ---------------------------------------------------------------------------
// U8 — the approved-commit row deep-links the approval comment
// ---------------------------------------------------------------------------

test('U8: the approved-commit row deep-links the approval comment via comment_id', async ({ page }) => {
  await setupStatus(page, [twoRoundIssue], [twoRoundStatus])

  const card = page.getByTestId(`issue-card-${twoRoundIssue.number}`)
  const link = card.getByRole('link', { name: R2_APPROVAL.slice(0, 7) })
  await expect(link).toHaveAttribute(
    'href',
    `${twoRoundIssue.html_url}#issuecomment-222`,
  )
})

test('U8/D44: an approval with no comment id renders the commit without a link', async ({ page }) => {
  await setupStatus(page, [nullCommentIdIssue], [nullCommentIdStatus])

  // The hash is still shown — only the deep-link is dropped, because a null
  // `comment_id` has no comment to point at (and `0` would be a lie).
  const card = page.getByTestId(`issue-card-${nullCommentIdIssue.number}`)
  await expect(card.getByText(R2_APPROVAL.slice(0, 7))).toBeVisible()
  await expect(card.getByRole('link', { name: R2_APPROVAL.slice(0, 7) })).toHaveCount(0)

  // Same in the detail modal's status card and the round switcher's approved badge.
  // Only the active tab panel is in the accessibility tree, so scope to it.
  await card.click()
  const panel = page.getByRole('dialog').getByRole('tabpanel')
  await expect(panel.getByText('Approved:')).toBeVisible()
  await expect(panel.getByRole('link', { name: R2_APPROVAL.slice(0, 7) })).toHaveCount(0)
})

// ---------------------------------------------------------------------------
// U2 / U3 — NewRoundModal
// ---------------------------------------------------------------------------

async function openNewRoundModal(page: Page, issueNumber: number) {
  await page.getByTestId(`new-round-${issueNumber}`).click()
  const dialog = page.getByRole('dialog', { name: /Start QC Round/ })
  await expect(dialog).toBeVisible()
  return dialog
}

test('U2/D23: branch and start commit are read-only from the checkout', async ({ page }) => {
  await setupStatus(page, [twoRoundIssue], [twoRoundStatus], {
    ...defaultRepoInfo,
    branch: 'feature-x',
    local_commit: 'fedcba9876543210fedcba9876543210fedcba98',
  })

  const dialog = await openNewRoundModal(page, twoRoundIssue.number)
  await expect(dialog.getByTestId('new-round-branch')).toHaveValue('feature-x')
  await expect(dialog.getByTestId('new-round-branch')).toHaveAttribute('readonly', '')
  await expect(dialog.getByTestId('new-round-start-commit')).toHaveValue(
    'fedcba9876543210fedcba9876543210fedcba98',
  )
  await expect(dialog.getByTestId('new-round-start-commit')).toHaveAttribute('readonly', '')
})

test('U2/D37: the base-round seed resets ticks and never re-adds the heading', async ({ page }) => {
  await setupStatus(page, [twoRoundIssue], [twoRoundStatus])

  const dialog = await openNewRoundModal(page, twoRoundIssue.number)
  await dialog.getByRole('tab', { name: 'Checklist' }).click()

  // >1 prior round ⇒ the base-round Select is shown.
  await expect(dialog.getByTestId('new-round-base-round')).toBeVisible()
  await expect(dialog.getByTestId('new-round-checklist-name')).toHaveValue('Round Two')

  const content = dialog.locator('textarea').first()
  // Seeded from the latest round with every `- [x]` reset to `- [ ]`.
  await expect(content).toHaveValue('- [ ] r2 item one\n- [ ] r2 item two')
  // D37: `checklist_content` excludes its `# ` heading — re-adding it here would
  // emit the heading twice and make the next round's parse latch onto the wrong one.
  expect(await content.inputValue()).not.toContain('#')

  // Choosing another base round reseeds name and content from that round.
  await dialog.getByTestId('new-round-base-round').click()
  await page.getByRole('option', { name: /Round 1/ }).click()
  await expect(dialog.getByTestId('new-round-checklist-name')).toHaveValue('Round One')
  await expect(content).toHaveValue('- [ ] r1 item')
  expect(await content.inputValue()).not.toContain('#')
})

test('U3: notify is disabled when the checkout is already the prior approval commit', async ({ page }) => {
  await setupStatus(page, [twoRoundIssue], [twoRoundStatus], {
    ...defaultRepoInfo,
    local_commit: R2_APPROVAL,
  })

  const dialog = await openNewRoundModal(page, twoRoundIssue.number)
  await expect(dialog.getByTestId('new-round-notify')).toBeDisabled()
  await expect(dialog.getByTestId('new-round-notify-explanation')).toBeVisible()
})

test('U3: notify is enabled when the checkout has moved past the approval', async ({ page }) => {
  await setupStatus(page, [twoRoundIssue], [twoRoundStatus], {
    ...defaultRepoInfo,
    local_commit: 'cafebabecafebabecafebabecafebabecafebabe',
  })

  const dialog = await openNewRoundModal(page, twoRoundIssue.number)
  await expect(dialog.getByTestId('new-round-notify')).toBeEnabled()
  await expect(dialog.getByTestId('new-round-notify-explanation')).toHaveCount(0)
})

test('A5: starting a round posts the checkout commit, the branch and the seeded checklist', async ({ page }) => {
  await setupStatus(page, [twoRoundIssue], [twoRoundStatus], {
    ...defaultRepoInfo,
    branch: 'feature-x',
    local_commit: 'cafebabecafebabecafebabecafebabecafebabe',
  })

  let body: Record<string, unknown> | null = null
  await page.route(/\/api\/issues\/\d+\/rounds/, async (route, request) => {
    body = request.postDataJSON()
    await route.fulfill({
      status: 201,
      contentType: 'application/json',
      body: JSON.stringify({
        round_index: 3,
        comment_url: 'https://github.com/test-owner/test-repo/issues/75#issuecomment-1',
        reopened: true,
        notification: { kind: 'not_requested' },
      }),
    })
  })

  const dialog = await openNewRoundModal(page, twoRoundIssue.number)
  await dialog.getByTestId('new-round-submit').click()
  await expect(dialog.getByTestId('new-round-result')).toBeVisible()

  expect(body).toEqual({
    start_commit: 'cafebabecafebabecafebabecafebabecafebabe',
    branch: 'feature-x',
    checklist: { name: 'Round Two', content: '- [ ] r2 item one\n- [ ] r2 item two' },
    notify: true,
    note: null,
    include_diff: true,
  })
})

test('D45: a failed re-open and a failed notification are both surfaced', async ({ page }) => {
  await setupStatus(page, [twoRoundIssue], [twoRoundStatus])

  await page.route(/\/api\/issues\/\d+\/rounds/, (route) =>
    route.fulfill({
      status: 201,
      contentType: 'application/json',
      body: JSON.stringify({
        round_index: 3,
        comment_url: 'https://github.com/test-owner/test-repo/issues/75#issuecomment-1',
        reopened: false,
        notification: { kind: 'failed', error: 'GitHub API 502' },
      }),
    }),
  )

  const dialog = await openNewRoundModal(page, twoRoundIssue.number)
  await dialog.getByTestId('new-round-submit').click()
  await expect(dialog.getByTestId('new-round-result')).toBeVisible()

  // Both steps are non-fatal, and neither may be silent (D45).
  await expect(dialog.getByTestId('new-round-not-reopened')).toBeVisible()
  await expect(dialog.getByTestId('new-round-notification-failed')).toContainText('GitHub API 502')
})

test('D45: a notification that was never requested surfaces nothing', async ({ page }) => {
  await setupStatus(page, [twoRoundIssue], [twoRoundStatus])

  await page.route(/\/api\/issues\/\d+\/rounds/, (route) =>
    route.fulfill({
      status: 201,
      contentType: 'application/json',
      body: JSON.stringify({
        round_index: 3,
        comment_url: 'https://github.com/test-owner/test-repo/issues/75#issuecomment-1',
        reopened: true,
        notification: { kind: 'not_requested' },
      }),
    }),
  )

  const dialog = await openNewRoundModal(page, twoRoundIssue.number)
  await dialog.getByTestId('new-round-submit').click()
  await expect(dialog.getByTestId('new-round-result')).toBeVisible()

  await expect(dialog.getByTestId('new-round-not-reopened')).toHaveCount(0)
  await expect(dialog.getByTestId('new-round-notification-failed')).toHaveCount(0)
})

// ---------------------------------------------------------------------------
// D47 — the preview comes from POST /api/preview/round, not from the client
// ---------------------------------------------------------------------------

// The round comment body has exactly one implementation, and it is the server's
// (D47). A client-side `roundCommentMarkdown()` is how the preview drifted from what
// gets posted — it could not emit the `[file contents at initial qc commit]` line at
// all — so its absence from the source is the thing worth pinning.
test('D47: the client no longer re-implements the round comment body', async () => {
  const src = await readFile(
    new URL('../../src/components/NewRoundModal.tsx', import.meta.url),
    'utf8',
  )
  expect(src).not.toContain('roundCommentMarkdown')
  expect(src).toContain('useRoundPreview')
})

test('D47: the preview tab renders the HTML from POST /api/preview/round', async ({ page }) => {
  await setupStatus(page, [twoRoundIssue], [twoRoundStatus])

  const requests: unknown[] = []
  page.on('request', (request) => {
    if (request.url().includes('/api/preview/round') && request.method() === 'POST') {
      requests.push(request.postDataJSON())
    }
  })

  const dialog = await openNewRoundModal(page, twoRoundIssue.number)
  await dialog.getByRole('tab', { name: 'Preview' }).click()

  // The body is the server's, not a client re-implementation: the
  // `[file contents at initial qc commit]` link is a line the UI cannot produce.
  const frame = dialog.frameLocator('iframe[title="Round Comment Preview"]')
  await expect(frame.getByRole('heading', { name: 'QC Round 3' })).toBeVisible()
  await expect(frame.getByRole('link', { name: 'file contents at initial qc commit' })).toBeVisible()

  // D47: no `round_index` is sent — the server derives it, so the preview cannot
  // claim a round number the creation would not use.
  expect(requests).toHaveLength(1)
  expect(requests[0]).toEqual({
    issue_number: twoRoundIssue.number,
    start_commit: defaultRepoInfo.local_commit,
    branch: defaultRepoInfo.branch,
    // Round 2's checklist, ticks reset (D37) — the seed the Checklist tab shows.
    checklist: { name: 'Round Two', content: '- [ ] r2 item one\n- [ ] r2 item two' },
  })
})

test('D47: a failed round preview surfaces the error instead of a stale body', async ({ page }) => {
  await setupStatus(page, [twoRoundIssue], [twoRoundStatus])
  await page.route(/\/api\/preview\/round/, (route) =>
    route.fulfill({
      status: 500,
      contentType: 'application/json',
      body: JSON.stringify({ error: 'render failed' }),
    }),
  )

  const dialog = await openNewRoundModal(page, twoRoundIssue.number)
  await dialog.getByRole('tab', { name: 'Preview' }).click()

  await expect(dialog.getByTestId('new-round-preview-error')).toContainText('render failed')
  await expect(dialog.locator('iframe[title="Round Comment Preview"]')).toHaveCount(0)
})

test('D47: the preview is not one request per keystroke', async ({ page }) => {
  await setupStatus(page, [twoRoundIssue], [twoRoundStatus])

  const names: string[] = []
  page.on('request', (request) => {
    if (request.url().includes('/api/preview/round') && request.method() === 'POST') {
      names.push(request.postDataJSON().checklist.name)
    }
  })

  const dialog = await openNewRoundModal(page, twoRoundIssue.number)
  await dialog.getByRole('tab', { name: 'Preview' }).click()
  await expect(dialog.locator('iframe[title="Round Comment Preview"]')).toBeVisible()

  await dialog.getByRole('tab', { name: 'Checklist' }).click()
  await dialog.getByTestId('new-round-checklist-name').fill('')
  await dialog.getByTestId('new-round-checklist-name').pressSequentially('Round Three', { delay: 20 })
  await dialog.getByRole('tab', { name: 'Preview' }).click()
  await expect(dialog.locator('iframe[title="Round Comment Preview"]')).toBeVisible()

  // The settled value is what the server was asked about, and eleven keystrokes did
  // not become eleven requests — the tab gate plus the debounce keep it to a handful.
  await expect.poll(() => names[names.length - 1]).toBe('Round Three')
  expect(names.length).toBeLessThanOrEqual(3)
})

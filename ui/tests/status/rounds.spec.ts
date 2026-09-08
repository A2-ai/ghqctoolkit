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
  approvableTwoRoundStatus,
  holeRoundsIssue,
  holeRoundsStatus,
  quietGapStatus,
  QUIET_GAP_COMMIT,
  divergentEmptyGapStatus,
  divergentGapStatus,
  driftingStatus,
  otherBranchStatus,
  breakNoFileChangeStatus,
  GAP_COMMIT,
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

async function openNotifyPanel(page: Page, issueNumber: number) {
  await page.getByTestId(`issue-card-${issueNumber}`).click()
  await expect(page.getByRole('tablist')).toBeVisible()
  await page.getByRole('tab', { name: 'Notify' }).click()
  // Only the active tab panel is in the accessibility tree.
  return page.getByRole('dialog').getByRole('tabpanel')
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

/**
 * D71: §22 widens the slider but must not reopen §0.5 — the opening view is still one
 * round, and everything else is opt-in.
 */
test('D71: the slider opens on the latest round alone', async ({ page }) => {
  await setupStatus(page, [twoRoundIssue], [twoRoundStatus])

  const panel = await openNotifyPanel(page, twoRoundIssue.number)
  await expect(panel.getByTestId('history-select')).toBeVisible()

  // Round 2's commits only — the pre-§22 view, unchanged.
  await expect(panel.getByText(R2_START.slice(0, 7))).toBeVisible()
  await expect(panel.getByText(R1_START.slice(0, 7))).toHaveCount(0)
  await expect(panel.getByText(R1_APPROVAL.slice(0, 7))).toHaveCount(0)
})

/**
 * D70: the whole point — selecting an earlier segment *adds* its commits rather than
 * replacing them, so a range can span rounds. Before §22 this was unreachable.
 */
test('D70: adding an earlier round puts both rounds on one slider', async ({ page }) => {
  await setupStatus(page, [twoRoundIssue], [twoRoundStatus])

  const panel = await openNotifyPanel(page, twoRoundIssue.number)
  await panel.getByTestId('history-select-trigger').click()
  await page.getByTestId('history-check-round:1').click()

  // Both rounds now — additive, not a switch.
  await expect(panel.getByText(R1_START.slice(0, 7))).toBeVisible()
  await expect(panel.getByText(R2_START.slice(0, 7))).toBeVisible()
})

/**
 * D80: the tail block cannot be deselected — `current_commit` has nowhere else legal to
 * come from (D79), so an empty-tail selection must not be constructible.
 */
test('D80: the latest round cannot be deselected on the Notify tab', async ({ page }) => {
  await setupStatus(page, [twoRoundIssue], [twoRoundStatus])

  const panel = await openNotifyPanel(page, twoRoundIssue.number)
  await panel.getByTestId('history-select-trigger').click()
  await expect(page.getByTestId('history-check-round:2')).toBeDisabled()
  // An earlier round is freely selectable — only the tail is pinned.
  await expect(page.getByTestId('history-check-round:1')).toBeEnabled()
})

// ---------------------------------------------------------------------------
// U6 — divergence badges
// ---------------------------------------------------------------------------

test('U6/D74: a divergent gap is badged and broken in the History dropdown', async ({ page }) => {
  await setupStatus(page, [twoRoundIssue], [divergentGapStatus])

  const panel = await openNotifyPanel(page, twoRoundIssue.number)
  await panel.getByTestId('history-select-trigger').click()

  // The badge sits on the row whose history does not continue the previous row's, and
  // the break marker is drawn above it (D74).
  await expect(page.getByTestId('history-row-gap:2').getByTestId('no-cohesive-history-badge')).toBeVisible()
  await expect(page.getByTestId('history-break-gap:2')).toBeVisible()

  // I13: round 1 has no predecessor to diverge from, so its row carries neither.
  await expect(page.getByTestId('history-row-round:1').getByTestId('no-cohesive-history-badge')).toHaveCount(0)
  await expect(page.getByTestId('history-break-round:1')).toHaveCount(0)
})

test('U6: a divergent drift badges the status card', async ({ page }) => {
  await setupStatus(page, [twoRoundIssue], [divergentDriftStatus])

  // On the card in the swimlane…
  await expect(page.getByTestId('drift-divergent-badge').first()).toBeVisible()

  // …and on the modal's status card. Clicked near the corner rather than at the centre:
  // this card offers "New Round" (D12 permits it from `changes_after_approval`), and a
  // centre click can land on that button, which stops propagation and opens the round
  // modal instead of the detail one.
  await page.getByTestId(`issue-card-${twoRoundIssue.number}`).click({ position: { x: 8, y: 8 } })
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
  await expect(dialog.getByTestId('new-round-branch')).toHaveText('feature-x')
  await expect(dialog.getByTestId('new-round-start-commit')).toHaveText(
    'fedcba9876543210fedcba9876543210fedcba98',
  )
  // D23: rendered as facts, not as inputs. `readOnly` text boxes read as editable and
  // imply the commit picker the round flow deliberately does not have — so there is no
  // editable control here at all.
  await expect(dialog.getByTestId('new-round-branch').locator('input')).toHaveCount(0)
  await expect(dialog.getByTestId('new-round-start-commit').locator('input')).toHaveCount(0)
  await expect(dialog.getByRole('tabpanel').locator('input[type="text"]')).toHaveCount(0)
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
  await dialog.getByRole('tab', { name: 'Notify' }).click()
  await expect(dialog.getByTestId('new-round-notify')).toBeDisabled()
  await expect(dialog.getByTestId('new-round-notify-explanation')).toBeVisible()
})

test('U3: notify is enabled when the checkout has moved past the approval', async ({ page }) => {
  await setupStatus(page, [twoRoundIssue], [twoRoundStatus], {
    ...defaultRepoInfo,
    local_commit: 'cafebabecafebabecafebabecafebabecafebabe',
  })

  const dialog = await openNewRoundModal(page, twoRoundIssue.number)
  await dialog.getByRole('tab', { name: 'Notify' }).click()
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

test('D47: the Preview button renders the HTML from POST /api/preview/round', async ({ page }) => {
  await setupStatus(page, [twoRoundIssue], [twoRoundStatus])

  const requests: unknown[] = []
  page.on('request', (request) => {
    // Anchored: `/api/preview/round` is a prefix of `/api/preview/round-diff`, and the
    // Round tab fires the latter on open.
    if (/\/api\/preview\/round$/.test(request.url()) && request.method() === 'POST') {
      requests.push(request.postDataJSON())
    }
  })

  const dialog = await openNewRoundModal(page, twoRoundIssue.number)
  await dialog.getByTestId('new-round-preview-button').click()

  // The body is the server's, not a client re-implementation: the
  // `[file contents at initial qc commit]` link is a line the UI cannot produce.
  // The preview is its own modal, portaled to the body — not inside the round dialog.
  const frame = page.frameLocator('iframe[title="Round Comment Preview"]')
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
  await page.route(/\/api\/preview\/round$/, (route) =>
    route.fulfill({
      status: 500,
      contentType: 'application/json',
      body: JSON.stringify({ error: 'render failed' }),
    }),
  )

  const dialog = await openNewRoundModal(page, twoRoundIssue.number)
  await dialog.getByTestId('new-round-preview-button').click()

  await expect(page.getByTestId('new-round-preview-error')).toContainText('render failed')
  await expect(page.locator('iframe[title="Round Comment Preview"]')).toHaveCount(0)
})

test('D47: the preview is not one request per keystroke', async ({ page }) => {
  await setupStatus(page, [twoRoundIssue], [twoRoundStatus])

  const names: string[] = []
  page.on('request', (request) => {
    if (/\/api\/preview\/round$/.test(request.url()) && request.method() === 'POST') {
      names.push(request.postDataJSON().checklist.name)
    }
  })

  const dialog = await openNewRoundModal(page, twoRoundIssue.number)
  await dialog.getByTestId('new-round-preview-button').click()
  await expect(page.locator('iframe[title="Round Comment Preview"]')).toBeVisible()
  await page.keyboard.press('Escape')

  await dialog.getByRole('tab', { name: 'Checklist' }).click()
  await dialog.getByTestId('new-round-checklist-name').fill('')
  await dialog.getByTestId('new-round-checklist-name').pressSequentially('Round Three', { delay: 20 })
  await dialog.getByTestId('new-round-preview-button').click()
  await expect(page.locator('iframe[title="Round Comment Preview"]')).toBeVisible()

  // The settled value is what the server was asked about, and eleven keystrokes did
  // not become eleven requests — the closed-modal gate plus the debounce keep it to a
  // handful.
  await expect.poll(() => names[names.length - 1]).toBe('Round Three')
  expect(names.length).toBeLessThanOrEqual(3)
})

// ---------------------------------------------------------------------------
// The checklist seed's provenance is stated even when it is not selectable
// ---------------------------------------------------------------------------

test('the single-round case still says which round the checklist came from', async ({ page }) => {
  await setupStatus(page, [approvedIssue], [approvedStatus])

  const dialog = await openNewRoundModal(page, approvedIssue.number)
  await dialog.getByRole('tab', { name: 'Checklist' }).click()

  // One prior round means nothing to choose between, but the user is still looking at
  // a checklist that came from somewhere — round 1, whose checklist lives in the issue
  // body rather than in a `# QC Round 1` comment (D2).
  await expect(dialog.getByTestId('new-round-base-round')).toHaveCount(0)
  await expect(dialog.getByTestId('new-round-base-round-static')).toContainText(
    'Round 1 (the initial QC)',
  )
  await expect(dialog.getByTestId('new-round-base-round-static')).toContainText('Code Review')
})

// ---------------------------------------------------------------------------
// D5 — the notification preview is the pair `create_round` posts
// ---------------------------------------------------------------------------

test('D5: the notification preview asks for the commit pair the creation will post', async ({ page }) => {
  await setupStatus(page, [twoRoundIssue], [twoRoundStatus])

  const requests: unknown[] = []
  page.on('request', (request) => {
    if (/\/api\/preview\/\d+\/comment/.test(request.url()) && request.method() === 'POST') {
      requests.push(request.postDataJSON())
    }
  })

  const dialog = await openNewRoundModal(page, twoRoundIssue.number)
  await dialog.getByRole('tab', { name: 'Notify' }).click()
  await dialog.getByTestId('new-round-notification-preview-button').click()

  await expect(page.locator('iframe[title="Round Notification Preview"]')).toBeVisible()

  // The round notification is a plain `QCComment` (D5), so the preview goes through
  // the same server endpoint the notify modal uses — and the pair is fixed by the
  // round, not chosen here: current = the checkout, previous = the prior approval.
  expect(requests).toHaveLength(1)
  expect(requests[0]).toEqual({
    current_commit: defaultRepoInfo.local_commit,
    previous_commit: R2_APPROVAL,
    note: null,
    include_diff: true,
  })
})

test('D5: there is no Notification preview tab when no notification will be posted', async ({ page }) => {
  await setupStatus(page, [twoRoundIssue], [twoRoundStatus], {
    ...defaultRepoInfo,
    // U3: the checkout is the prior approval, so notify is forced off.
    local_commit: R2_APPROVAL,
  })

  const dialog = await openNewRoundModal(page, twoRoundIssue.number)
  await dialog.getByTestId('new-round-preview-button').click()

  const preview = page.getByRole('dialog', { name: 'Round Preview' })
  await expect(preview.getByRole('tab', { name: 'Round comment' })).toBeVisible()
  // Previewing a comment nobody is going to send misrepresents what Start Round does.
  await expect(preview.getByRole('tab', { name: 'Notification' })).toHaveCount(0)
})

// ---------------------------------------------------------------------------
// The footer matches the notify modal: Preview and the action, no Cancel
// ---------------------------------------------------------------------------

test('the footer is Preview and Start Round, and Escape is what leaves', async ({ page }) => {
  await setupStatus(page, [twoRoundIssue], [twoRoundStatus])

  const dialog = await openNewRoundModal(page, twoRoundIssue.number)
  await expect(dialog.getByRole('button', { name: 'Cancel' })).toHaveCount(0)
  await expect(dialog.getByTestId('new-round-preview-button')).toBeVisible()
  await expect(dialog.getByTestId('new-round-submit')).toBeVisible()

  // Dropping Cancel is only safe because the two documented exits still work.
  await page.keyboard.press('Escape')
  await expect(dialog).toHaveCount(0)
})

// ---------------------------------------------------------------------------
// The Round tab shows the change a new round would be opened over
// ---------------------------------------------------------------------------

test('the Round tab renders the diff since the approval, on open', async ({ page }) => {
  await setupStatus(page, [twoRoundIssue], [twoRoundStatus])

  const requests: unknown[] = []
  page.on('request', (request) => {
    if (request.url().includes('/api/preview/round-diff') && request.method() === 'POST') {
      requests.push(request.postDataJSON())
    }
  })

  const dialog = await openNewRoundModal(page, twoRoundIssue.number)

  // No tab click: the first screen is the decision — is there anything here worth a
  // round? — so the change is on it.
  const frame = page.frameLocator('iframe[title="Round Diff Preview"]')
  await expect(frame.getByText('+rewritten line')).toBeVisible()

  // D5: only the new end is sent. The old end is the prior round's approval, derived
  // server-side exactly as `create_round` derives the notification's previous commit.
  expect(requests).toHaveLength(1)
  expect(requests[0]).toEqual({
    issue_number: twoRoundIssue.number,
    start_commit: defaultRepoInfo.local_commit,
  })
})

test('U3: nothing is fetched when the checkout is the approved commit', async ({ page }) => {
  await setupStatus(page, [twoRoundIssue], [twoRoundStatus], {
    ...defaultRepoInfo,
    local_commit: R2_APPROVAL,
  })

  let asked = 0
  page.on('request', (request) => {
    if (request.url().includes('/api/preview/round-diff')) asked += 1
  })

  const dialog = await openNewRoundModal(page, twoRoundIssue.number)

  // U3 already proves there is no difference, so asking the server would spend a
  // round-trip to be told so.
  await expect(dialog.getByTestId('new-round-diff-no-changes')).toBeVisible()
  await expect(dialog.getByTestId('new-round-diff')).toHaveCount(0)
  expect(asked).toBe(0)
})

test('a failed diff says so instead of reading as "nothing changed"', async ({ page }) => {
  await setupRoutes(page, {
    repo: defaultRepoInfo,
    milestones: [openMilestone],
    milestoneIssues: { 1: [twoRoundIssue] },
    issueStatuses: { results: [twoRoundStatus], errors: [] },
    roundDiffPreviewHtml: null,
  })
  await goToStatus(page)

  const dialog = await openNewRoundModal(page, twoRoundIssue.number)

  // An unreadable commit is not an unchanged file — collapsing the two would hide a
  // fetch problem behind a reassuring empty diff.
  await expect(dialog.getByTestId('new-round-diff-error')).toContainText('Failed to render the round diff')
  await expect(page.locator('iframe[title="Round Diff Preview"]')).toHaveCount(0)
  await expect(dialog.getByTestId('new-round-diff-no-changes')).toHaveCount(0)
})

test('the Round tab counts the commits since the approval', async ({ page }) => {
  // `driftingStatus`'s drift holds three commits of which exactly one touched the file,
  // so the two numbers differ: a build that reported `drift.commits.length` for both,
  // or a constant, cannot pass. The counts come off `drift.commits` — which commit is
  // which stays the server's call (U7).
  await setupStatus(page, [twoRoundIssue], [driftingStatus])

  const dialog = await openNewRoundModal(page, twoRoundIssue.number)

  // Asserted as the whole sentence, not a substring: `toContainText('0 commit')` also
  // matches "10 commits", so a substring check is no check at all here.
  await expect(dialog.getByTestId('new-round-drift-summary')).toHaveText(
    '3 commits since the approval, 1 of which touched this file.',
  )
})

// ---------------------------------------------------------------------------
// §22 — cross-round selection on the slider
// ---------------------------------------------------------------------------

/** D73: a gap is a selectable segment, not decoration — its commits are real commits. */
test('D73: selecting a gap puts its commits on the slider', async ({ page }) => {
  await setupStatus(page, [twoRoundIssue], [twoRoundStatus])

  const panel = await openNotifyPanel(page, twoRoundIssue.number)
  await expect(panel.getByText(GAP_COMMIT.slice(0, 7))).toHaveCount(0)

  await panel.getByTestId('history-select-trigger').click()
  await page.getByTestId('history-check-gap:2').click()
  await expect(panel.getByText(GAP_COMMIT.slice(0, 7))).toBeVisible()
})

/**
 * D79: `to` ⇒ `current_commit` must come from the tail block. Both handles are driven to
 * the far left, into round 1; the max handle clamps to the tail's first commit instead of
 * following.
 */
test('D79: the current commit clamps into the tail block, however far left the handles go', async ({ page }) => {
  // An *approved* QC gates Post behind the "Notify anyway" acknowledgement, so this uses
  // the open-latest-round fixture to keep the assertion about the clamp alone.
  await setupStatus(page, [twoRoundIssue], [approvableTwoRoundStatus])

  let body: Record<string, unknown> | null = null
  await page.route(/\/api\/issues\/\d+\/comment/, async (route, request) => {
    body = request.postDataJSON()
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ comment_url: 'https://github.com/o/r/issues/75#issuecomment-1' }),
    })
  })

  const panel = await openNotifyPanel(page, twoRoundIssue.number)
  await panel.getByTestId('history-select-trigger').click()
  await page.getByTestId('history-check-round:1').click()
  // Closing the popover: Escape here would close the detail modal itself.
  await panel.getByTestId('history-select-trigger').click()

  // Drive both thumbs to the oldest commit in the view — round 1's initial commit.
  const thumbs = panel.getByRole('slider')
  await expect(thumbs).toHaveCount(2)
  await thumbs.nth(0).focus()
  await page.keyboard.press('Home')
  await thumbs.nth(1).focus()
  await page.keyboard.press('Home')

  await panel.getByRole('button', { name: 'Post' }).click()
  await expect.poll(() => body).not.toBeNull()

  // Without the clamp `current_commit` would be a round-1 commit — a notification
  // claiming an old commit is "current".
  expect(body!.current_commit).toBe(R2_START)
  expect(body!.previous_commit).toBe(R1_START)
})

/**
 * The clamp has to be *visible*. Clamping only the derived `to` left the max thumb parked
 * wherever it was dragged while the From/To rows — and the posted comment — used a
 * different commit, so the control displayed a range it was not going to send.
 */
test('D79: the thumbs sit exactly where From and To say they do', async ({ page }) => {
  await setupStatus(page, [twoRoundIssue], [approvableTwoRoundStatus])

  const panel = await openNotifyPanel(page, twoRoundIssue.number)
  await panel.getByTestId('history-select-trigger').click()
  await page.getByTestId('history-check-round:1').click()
  // Closing the popover: Escape here would close the detail modal itself.
  await panel.getByTestId('history-select-trigger').click()

  // Both thumbs driven into round 1 — the max one has nowhere legal to go there.
  const thumbs = panel.getByRole('slider')
  await expect(thumbs).toHaveCount(2)
  await thumbs.nth(0).focus()
  await page.keyboard.press('Home')
  await thumbs.nth(1).focus()
  await page.keyboard.press('Home')

  // Only `CommitSlider` renders marks, and they are the visible commits in order, so a
  // thumb's `aria-valuenow` indexes straight into them.
  const labels = panel.locator('.mantine-Slider-markLabel')
  const positions = (await thumbs.evaluateAll((els) =>
    els.map((el) => Number(el.getAttribute('aria-valuenow'))),
  )).sort((a, b) => a - b)

  // The max thumb refused the drag and stayed on the tail's first commit; before the fix
  // it sat at position 0 while `To:` read R2_START.
  expect(positions[0]).toBe(0)
  expect(positions[1]).toBeGreaterThan(0)
  expect((await labels.nth(positions[0]).innerText()).trim()).toBe(R1_START.slice(0, 7))
  expect((await labels.nth(positions[1]).innerText()).trim()).toBe(R2_START.slice(0, 7))

  // …and the rows agree with the thumbs, which is the whole point.
  await expect(panel.getByTestId('commit-block-from')).toContainText(R1_START.slice(0, 7))
  await expect(panel.getByTestId('commit-block-to')).toContainText(R2_START.slice(0, 7))
})

/** D74: the omitted-segment break is drawn on the slider, in the same language as the
 *  dropdown's — one concept for "not adjacent in history". */
test('D74: skipping a segment draws a break on the slider track', async ({ page }) => {
  await setupStatus(page, [twoRoundIssue], [twoRoundStatus])

  const panel = await openNotifyPanel(page, twoRoundIssue.number)
  await panel.getByTestId('history-select-trigger').click()
  // Round 1 without the gap between it and round 2 — the discontinuity is the user's own.
  await page.getByTestId('history-check-round:1').click()
  // Closing the popover: Escape here would close the detail modal itself.
  await panel.getByTestId('history-select-trigger').click()

  await expect(panel.getByTestId(`slider-break-${R2_START.slice(0, 7)}`)).toBeVisible()

  // Adding the omitted gap back makes the selection contiguous, and the marker goes.
  await panel.getByTestId('history-select-trigger').click()
  await page.getByTestId('history-check-gap:2').click()
  // Closing the popover: Escape here would close the detail modal itself.
  await panel.getByTestId('history-select-trigger').click()
  await expect(panel.getByTestId(`slider-break-${R2_START.slice(0, 7)}`)).toHaveCount(0)
})

/** D74/D22: a divergent gap keeps its break even when it owns no commits (D8's overlap
 *  case) — the marker is carried to the next commit that appears, not dropped. */
test('D74: a divergent gap with no commits still breaks the track', async ({ page }) => {
  await setupStatus(page, [twoRoundIssue], [divergentEmptyGapStatus])

  const panel = await openNotifyPanel(page, twoRoundIssue.number)
  await panel.getByTestId('history-select-trigger').click()

  // D90: an empty gap is not selectable, so divergence has to be legible without ever
  // putting the gap on the slider — the dropdown's rail carries it.
  await expect(page.getByTestId('history-check-gap:2')).toBeDisabled()
  await expect(page.getByTestId('history-break-gap:2')).toBeVisible()
  await expect(page.getByTestId('history-row-gap:2').getByTestId('no-cohesive-history-badge')).toBeVisible()

  await page.getByTestId('history-check-round:1').click()
  // Closing the popover: Escape here would close the detail modal itself.
  await panel.getByTestId('history-select-trigger').click()

  // The track still breaks between the two rounds. Only the marker is asserted: the
  // *alert* is range-scoped by design, and the default handles sit inside the tail, so
  // this range does not cross the break. That half is D75's test.
  await expect(panel.getByTestId(`slider-break-${R2_START.slice(0, 7)}`)).toBeVisible()
})

/**
 * D75: across a break the order-derived walk is not a fact, so `include_diff` goes
 * conservative rather than silently dropping the diff. Every commit in the crossed range
 * has `file_changed: false`, so the walk alone would post `include_diff: false`.
 */
test('D75: a range crossing a break keeps the diff on offer', async ({ page }) => {
  await setupStatus(page, [twoRoundIssue], [breakNoFileChangeStatus])

  let body: Record<string, unknown> | null = null
  await page.route(/\/api\/issues\/\d+\/comment/, async (route, request) => {
    body = request.postDataJSON()
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ comment_url: 'https://github.com/o/r/issues/75#issuecomment-1' }),
    })
  })

  const panel = await openNotifyPanel(page, twoRoundIssue.number)
  await panel.getByTestId('history-select-trigger').click()
  await page.getByTestId('history-check-round:1').click()
  // Closing the popover: Escape here would close the detail modal itself.
  await panel.getByTestId('history-select-trigger').click()

  const thumbs = panel.getByRole('slider')
  await thumbs.nth(0).focus()
  await page.keyboard.press('Home')

  await expect(panel.getByTestId('notify-range-crosses-break')).toBeVisible()
  await panel.getByRole('button', { name: 'Post' }).click()
  await expect.poll(() => body).not.toBeNull()

  expect(body!.include_diff).toBe(true)
})

/** D83: the Approve tab has no segment control at all — an approval must land inside the
 *  round that will own it (D8), and the switcher this replaces posted ones that did not. */
test('D83: the Approve tab offers no segment selection', async ({ page }) => {
  // The Approve tab is only enabled while the latest round is open.
  await setupStatus(page, [twoRoundIssue], [approvableTwoRoundStatus])

  await page.getByTestId(`issue-card-${twoRoundIssue.number}`).click()
  // `exact` matters: 'Approve' also matches 'Unapprove'.
  await page.getByRole('tab', { name: 'Approve', exact: true }).click()
  const panel = page.getByRole('dialog').getByRole('tabpanel')

  await expect(panel.getByTestId('history-select')).toHaveCount(0)
  await expect(panel.getByTestId('approve-round-scope')).toContainText('2')
  await expect(panel.getByTestId('approve-round-scope')).toContainText('belongs to the round it closes')
})

/** D82: Review's commit is the old end of its diff, so nothing is pinned there — every
 *  segment is a legal place to rest, which is what makes cross-round review possible. */
test('D82: the Review tab pins nothing', async ({ page }) => {
  await setupStatus(page, [twoRoundIssue], [approvableTwoRoundStatus])

  await page.getByTestId(`issue-card-${twoRoundIssue.number}`).click()
  await page.getByRole('tab', { name: 'Review' }).click()
  const panel = page.getByRole('dialog').getByRole('tabpanel')

  await panel.getByTestId('history-select-trigger').click()
  await expect(page.getByTestId('history-check-round:2')).toBeEnabled()
  await expect(page.getByTestId('history-check-round:1')).toBeEnabled()
})

/**
 * The counts describe what selecting a row **adds to the slider**, so they count commits
 * the slider draws — file-changing or status-bearing — not raw commits in the span. A gap
 * whose only commit touched nothing reads as `0`, because selecting it adds nothing
 * visible; counting it as `1` advertised a commit that never appeared.
 */
test('the History counts describe what the slider will draw', async ({ page }) => {
  await setupStatus(page, [twoRoundIssue], [quietGapStatus])

  const panel = await openNotifyPanel(page, twoRoundIssue.number)
  await panel.getByTestId('history-select-trigger').click()

  // Round 1's initial commit *is* its approval — one commit, and it changed the file.
  await expect(page.getByTestId('history-row-round:1')).toContainText('1 commit (1 file changing)')
  await expect(page.getByTestId('history-row-round:2')).toContainText('1 commit (1 file changing)')
  // The gap owns a commit, and that commit changed nothing. Both facts are stated, so
  // "nothing here" and "nothing *interesting* here" cannot be confused.
  await expect(page.getByTestId('history-row-gap:2')).toContainText('1 commit (0 file changing)')

  // D90: it owns a commit, so it stays selectable.
  await expect(page.getByTestId('history-check-gap:2')).toBeEnabled()

  await page.getByTestId('history-check-round:1').click()
  await page.getByTestId('history-check-gap:2').click()
  await expect(panel.getByTestId('history-commit-count')).toHaveText('3 commits (2 file changing)')

  // The gap's commit is hidden, not discarded: "Show all commits" reaches it.
  await panel.getByTestId('history-select-trigger').click()
  await expect(panel.getByText(QUIET_GAP_COMMIT.slice(0, 7))).toHaveCount(0)
  await panel.getByLabel('Show all commits').check()
  await expect(panel.getByText(QUIET_GAP_COMMIT.slice(0, 7))).toBeVisible()
})

/** D90: a gap that owns nothing at all cannot be ticked — there is nothing to add. */
test('D90: an empty gap is not selectable', async ({ page }) => {
  await setupStatus(page, [twoRoundIssue], [divergentEmptyGapStatus])

  const panel = await openNotifyPanel(page, twoRoundIssue.number)
  await panel.getByTestId('history-select-trigger').click()

  await expect(page.getByTestId('history-row-gap:2')).toContainText('0 commits')
  await expect(page.getByTestId('history-check-gap:2')).toBeDisabled()
  // A round showing nothing is a different case — unplaceable, where empty means "fetch
  // the branch" — so the rule keys on segment kind, not on the count alone (D90.2).
  await expect(page.getByTestId('history-check-round:1')).toBeEnabled()
})

/** D91.2: the disabling rule keys on the **raw commit count**, never on divergence. The
 *  two travel together in every other fixture, so without this a build reading
 *  `segment.divergent` instead would pass the suite — and would lock the user out of
 *  exactly the commits they most need to inspect once history has diverged. */
test('D91.2: a divergent gap that owns commits stays selectable', async ({ page }) => {
  await setupStatus(page, [twoRoundIssue], [divergentGapStatus])

  const panel = await openNotifyPanel(page, twoRoundIssue.number)
  await panel.getByTestId('history-select-trigger').click()

  // Divergent, and it owns a commit: the badge is shown and the checkbox still works.
  await expect(page.getByTestId('history-row-gap:2').getByTestId('no-cohesive-history-badge')).toBeVisible()
  await expect(page.getByTestId('history-row-gap:2')).toContainText('1 commit')
  await expect(page.getByTestId('history-check-gap:2')).toBeEnabled()

  await page.getByTestId('history-check-gap:2').click()
  await expect(page.getByTestId('history-check-gap:2')).toBeChecked()
})

/** D96: branch scopes a round's commit walk (D7/D9), so an older round on a different
 *  branch is a fact about where its commits came from — the dropdown names the branch
 *  rather than leaving the row to read as if it were on the current one. */
test('D96: a round on another branch names that branch in the History dropdown', async ({ page }) => {
  await setupStatus(page, [twoRoundIssue], [otherBranchStatus])

  const panel = await openNotifyPanel(page, twoRoundIssue.number)
  await panel.getByTestId('history-select-trigger').click()

  await expect(page.getByTestId('history-row-round:1').getByTestId('other-branch-badge'))
    .toHaveText('on feature/round-one')
  // The round the QC is on now is the reference, so it is never badged against itself.
  await expect(page.getByTestId('history-row-round:2').getByTestId('other-branch-badge'))
    .toHaveCount(0)
})

/** The badge is about *difference*, not about branches in general: when every round
 *  shares one branch the dropdown stays quiet. Without this, a badge rendered
 *  unconditionally would still pass the test above. */
test('D96: rounds sharing a branch carry no branch badge', async ({ page }) => {
  await setupStatus(page, [twoRoundIssue], [twoRoundStatus])

  const panel = await openNotifyPanel(page, twoRoundIssue.number)
  await panel.getByTestId('history-select-trigger').click()

  // Anchor the absence to a dropdown that demonstrably rendered: a bare `toHaveCount(0)`
  // also passes when nothing opened at all.
  await expect(page.getByTestId('history-row-round:1')).toBeVisible()
  await expect(page.getByTestId('history-row-round:2')).toBeVisible()
  await expect(page.getByTestId('other-branch-badge')).toHaveCount(0)
})

/** Newest-first: the round in progress is the top row and round 1 is the bottom one. */
test('the History dropdown reads newest-first', async ({ page }) => {
  await setupStatus(page, [twoRoundIssue], [quietGapStatus])

  const panel = await openNotifyPanel(page, twoRoundIssue.number)
  await panel.getByTestId('history-select-trigger').click()

  const rows = page.locator('[data-testid^="history-row-"]')
  await expect(rows).toHaveCount(3)
  // Display order is reversed; the server's `history` order is untouched.
  await expect(rows.nth(0)).toHaveAttribute('data-testid', 'history-row-round:2')
  await expect(rows.nth(1)).toHaveAttribute('data-testid', 'history-row-gap:2')
  await expect(rows.nth(2)).toHaveAttribute('data-testid', 'history-row-round:1')
})

/**
 * The slider's wrapper used to pad 16px left and 40px right — room for the last mark's
 * label but not the first's — which inset the track unevenly and read as an off-centre
 * slider. Both end labels are centred on their marks and overhang by about half a
 * hash-width, so the padding has to be symmetric. Measured rather than asserted against
 * the literal, so the geometry is what is pinned.
 */
test('the commit slider track is centred in its container', async ({ page }) => {
  await setupStatus(page, [twoRoundIssue], [quietGapStatus])

  const panel = await openNotifyPanel(page, twoRoundIssue.number)
  const track = panel.locator('.mantine-Slider-track').first()
  await expect(track).toBeVisible()
  const scroller = panel.locator('.mantine-ScrollArea-viewport').first()

  const trackBox = (await track.boundingBox())!
  const scrollerBox = (await scroller.boundingBox())!
  const leftInset = trackBox.x - scrollerBox.x
  const rightInset = scrollerBox.x + scrollerBox.width - (trackBox.x + trackBox.width)

  expect(Math.abs(leftInset - rightInset)).toBeLessThanOrEqual(1)
})

// ---------------------------------------------------------------------------
// Which round the QC is on, on both status cards
// ---------------------------------------------------------------------------

test('both status cards carry a round pill', async ({ page }) => {
  await setupStatus(page, [twoRoundIssue], [twoRoundStatus])

  // The swimlane card, so rounds are scannable without opening anything.
  const card = page.getByTestId(`issue-card-${twoRoundIssue.number}`)
  await expect(card.getByTestId('round-pill')).toHaveText('Round 2')

  // And the detail modal's card, from the same component so the two cannot drift.
  await card.click()
  await page.getByRole('tab', { name: 'Notify' }).click()
  // Every tab panel stays mounted and each renders the card, so scope to the active one.
  await expect(
    page.getByRole('dialog').getByRole('tabpanel').getByTestId('round-pill'),
  ).toHaveText('Round 2')
})

/** D53.2: the pill shows the **declared** index. `rounds` is [1, 3] here, so a pill
 *  derived from `rounds.length` would say "Round 2" — a round that does not exist. */
test('D53.2: the round pill shows the declared index, not a count', async ({ page }) => {
  await setupStatus(page, [holeRoundsIssue], [holeRoundsStatus])

  await expect(
    page.getByTestId(`issue-card-${holeRoundsIssue.number}`).getByTestId('round-pill'),
  ).toHaveText('Round 3')
})

/** A single-round QC says so rather than staying silent: an absent pill would leave the
 *  reader unable to tell a first round from a UI that forgot to mention rounds. */
test('a single-round QC still shows Round 1', async ({ page }) => {
  await setupStatus(page, [approvedIssue], [approvedStatus])

  await expect(
    page.getByTestId(`issue-card-${approvedIssue.number}`).getByTestId('round-pill'),
  ).toHaveText('Round 1')
})

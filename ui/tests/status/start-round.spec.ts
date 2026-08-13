// Start-new-round modal (S4) and the status-surface affordance (S6).
//
// Everything is driven through the Status tab: the affordance lives on the issue
// card, which is also the only place the modal is opened from in this slice.

import { test, expect, type Page } from 'playwright/test'
import { setupRoutes } from '../helpers/routes'
import {
  approvedRoundIssue,
  approvedRoundStatus,
  legacyRoundIssue,
  legacyRoundStatus,
  multiRoundIssue,
  multiRoundStatus,
  roundSeedCanStart,
  roundSeedBlocked,
  roundSeedAnchorUnresolved,
  roundSeedMultipleChecklists,
  ROUND2_CHECKLIST,
  startRoundSuccess,
  startRoundNeedsRepair,
  startRoundWithImpactedIssues,
  roundStillOpenError,
  roundFields,
  closeRound,
  initialQcRound,
  laterRound,
  ROUND1_OPENED,
  ROUND1_CLOSED,
  ROUND2_OPENED,
} from '../fixtures/index'
import type { Issue, IssueStatusResponse } from '../../src/api/issues'
import type { RouteOverrides } from '../helpers/routes'
import type { StartRoundRequest } from '../../src/api/rounds'

// ---------------------------------------------------------------------------
// Fixtures local to this spec: #111 approved, then the file changed again.
// ---------------------------------------------------------------------------

const changedIssue: Issue = { ...approvedRoundIssue, state: 'open', closed_at: null }

const changedAfterApprovalStatus: IssueStatusResponse = {
  ...approvedRoundStatus,
  issue: changedIssue,
  qc_status: {
    ...approvedRoundStatus.qc_status,
    status: 'changes_after_approval',
    status_detail: 'Approved; subsequent file changes',
  },
}

/** #112's rounds, with Round 2 closed too, so a third round is legal. */
const multiRoundApprovedStatus: IssueStatusResponse = {
  ...multiRoundStatus,
  qc_status: {
    ...multiRoundStatus.qc_status,
    status: 'approved',
    status_detail: 'Approved',
  },
  ...roundFields([
    closeRound(initialQcRound(ROUND1_OPENED), ROUND1_CLOSED),
    closeRound(laterRound(2, ROUND2_OPENED, ROUND1_CLOSED, { event_count: 1 }), ROUND2_OPENED),
  ]),
}

async function selectMilestone(page: Page, milestoneTitle: string) {
  await page.getByPlaceholder('Search milestones…').click()
  await page.getByRole('option', { name: new RegExp(milestoneTitle) }).click()
}

/** Loads the Status tab with #111 in `changes_after_approval` and opens the modal. */
async function openStartRoundModal(page: Page, overrides: Partial<RouteOverrides> = {}) {
  await setupRoutes(page, {
    milestoneIssues: { 1: [changedIssue] },
    issueStatuses: { results: [changedAfterApprovalStatus], errors: [] },
    ...overrides,
  })
  await page.goto('/')
  await selectMilestone(page, 'Sprint 1')
  await page.getByTestId('start-round-action-111').click()
  await expect(page.getByRole('heading', { name: 'Start New QC Round' })).toBeVisible()
}

/**
 * Opens the Checklist tab. The modal now opens on Changes, so any test touching the
 * editor, the name or the source picker has to get there first — and an assertion
 * that something is *absent* would otherwise pass vacuously.
 */
async function openChecklistTab(page: Page) {
  await page.getByRole('tab', { name: 'Checklist' }).click()
  await expect(page.getByTestId('checklist-panel')).toBeVisible()
}

/** Opens the Changes tab, which holds the commit range, the diff and the note. */
async function openChangesTab(page: Page) {
  await page.getByRole('tab', { name: 'Changes' }).click()
  await expect(page.getByTestId('changes-panel')).toBeVisible()
}

/** Opens the Notification tab, where the mode control now lives. */
async function openNotificationTab(page: Page) {
  await page.getByRole('tab', { name: 'Notification' }).click()
  await expect(page.getByTestId('notification-panel')).toBeVisible()
}

/** Collects the bodies POSTed to /api/issues/:n/rounds. */
function captureStartRoundRequests(page: Page): StartRoundRequest[] {
  const bodies: StartRoundRequest[] = []
  page.on('request', (request) => {
    if (request.method() === 'POST' && /\/api\/issues\/\d+\/rounds$/.test(request.url())) {
      bodies.push(request.postDataJSON() as StartRoundRequest)
    }
  })
  return bodies
}

interface CommentPreviewBody {
  current_commit: string
  previous_commit: string | null
  note: string | null
  include_diff: boolean
}

/** Bodies POSTed to the notify tab's comment-preview endpoint, which rounds reuse. */
function captureCommentPreviewRequests(page: Page): CommentPreviewBody[] {
  const bodies: CommentPreviewBody[] = []
  page.on('request', (request) => {
    if (request.method() === 'POST' && /\/api\/preview\/\d+\/comment$/.test(request.url())) {
      bodies.push(request.postDataJSON() as CommentPreviewBody)
    }
  })
  return bodies
}

// ---------------------------------------------------------------------------
// S6: the status-surface affordance
// ---------------------------------------------------------------------------

test('S6: changes_after_approval shows a Start new round affordance that opens the modal', async ({ page }) => {
  await setupRoutes(page, {
    milestoneIssues: { 1: [changedIssue] },
    issueStatuses: { results: [changedAfterApprovalStatus], errors: [] },
  })

  await page.goto('/')
  await selectMilestone(page, 'Sprint 1')

  const action = page.getByTestId('start-round-action-111')
  await expect(action).toBeVisible()
  await expect(action).toHaveText('Start new round')

  await action.click()
  await expect(page.getByRole('heading', { name: 'Start New QC Round' })).toBeVisible()
  // Seeded from the round seed endpoint, for the issue that was clicked.
  await expect(page.getByTestId('next-round-name')).toHaveText('Round 2')
  await expect(page.getByTestId('start-round-submit')).toBeVisible()
})

test('S6: a single Initial QC round shows no round badge and no start-round action', async ({ page }) => {
  await setupRoutes(page, {
    milestoneIssues: { 1: [legacyRoundIssue] },
    issueStatuses: { results: [legacyRoundStatus], errors: [] },
  })

  await page.goto('/')
  await selectMilestone(page, 'Sprint 1')

  await expect(page.getByTestId('issue-card-110')).toBeVisible()
  await expect(page.getByTestId('round-badge-110')).toHaveCount(0)
  await expect(page.getByTestId('start-round-action-110')).toHaveCount(0)
})

test('S6: a multi-round issue shows a Round N badge', async ({ page }) => {
  await setupRoutes(page, {
    milestoneIssues: { 1: [multiRoundIssue] },
    issueStatuses: { results: [multiRoundStatus], errors: [] },
  })

  await page.goto('/')
  await selectMilestone(page, 'Sprint 1')

  await expect(page.getByTestId('round-badge-112')).toHaveText('Round 2')
})

// A cleanly approved issue, sharing everything with `changedAfterApprovalStatus`
// except the status itself, so the assertions below isolate the gate. Kept `open`
// so the closed-issues filter plays no part.
const cleanlyApprovedStatus: IssueStatusResponse = {
  ...approvedRoundStatus,
  issue: changedIssue,
  qc_status: {
    ...approvedRoundStatus.qc_status,
    status: 'approved',
    status_detail: 'Approved',
  },
}

test('S6: the new-round affordance is offered for a cleanly approved issue too', async ({ page }) => {
  // It used to be gated on `changes_after_approval`, so it appeared and disappeared
  // with commits unrelated to the QC. Any approved round can start the next one.
  await setupRoutes(page, {
    milestoneIssues: { 1: [changedIssue] },
    issueStatuses: { results: [cleanlyApprovedStatus], errors: [] },
  })

  await page.goto('/')
  await selectMilestone(page, 'Sprint 1')

  await expect(page.getByTestId('start-round-action-111')).toBeVisible()
})

test('S6: the affordance reads identically whether or not the file changed after approval', async ({ page }) => {
  await setupRoutes(page, {
    milestoneIssues: { 1: [changedIssue] },
    issueStatuses: { results: [cleanlyApprovedStatus], errors: [] },
  })
  await page.goto('/')
  await selectMilestone(page, 'Sprint 1')
  const approvedLabel = await page.getByTestId('start-round-action-111').innerText()

  await setupRoutes(page, {
    milestoneIssues: { 1: [changedIssue] },
    issueStatuses: { results: [changedAfterApprovalStatus], errors: [] },
  })
  await page.goto('/')
  await selectMilestone(page, 'Sprint 1')
  const changedLabel = await page.getByTestId('start-round-action-111').innerText()

  expect(approvedLabel).toBe(changedLabel)
})

// ---------------------------------------------------------------------------
// S4: blocked seeds — the action is not offered
// ---------------------------------------------------------------------------

test('blocked: last round still open — reason shown, no submit offered', async ({ page }) => {
  await openStartRoundModal(page, { roundSeedResponse: roundSeedBlocked })

  const blocked = page.getByTestId('round-blocked')
  await expect(blocked).toBeVisible()
  await expect(blocked).toContainText('Initial QC is still open')
  await expect(page.getByTestId('start-round-submit')).toHaveCount(0)
})

test('blocked: anchor could not be resolved — reason shown, no submit offered', async ({ page }) => {
  await openStartRoundModal(page, { roundSeedResponse: roundSeedAnchorUnresolved })

  const blocked = page.getByTestId('round-blocked')
  await expect(blocked).toBeVisible()
  await expect(blocked).toContainText('Could not resolve HEAD of branch')
  await expect(page.getByTestId('start-round-submit')).toHaveCount(0)
  // A null anchor renders as unavailable rather than crashing.
  await openChangesTab(page)
  await expect(page.getByTestId('round-anchor')).toContainText('not available')
})

// ---------------------------------------------------------------------------
// S4: notification mode reaches the request body
// ---------------------------------------------------------------------------

test('notification mode defaults to full and reaches the request body', async ({ page }) => {
  const bodies = captureStartRoundRequests(page)
  await openStartRoundModal(page)

  await page.getByTestId('start-round-submit').click()
  await expect(page.getByTestId('start-round-result')).toBeVisible()

  expect(bodies).toHaveLength(1)
  expect(bodies[0].notification).toBe('full')
  expect(bodies[0].checklist_content).toBe(roundSeedCanStart.checklist_content)
  expect(bodies[0].checklist_name).toBe('Code Review')
})

test('notification mode metadata_only reaches the request body', async ({ page }) => {
  const bodies = captureStartRoundRequests(page)
  await openStartRoundModal(page)

  await openNotificationTab(page)
  await page.getByTestId('notification-mode-metadata_only').click()
  await page.getByTestId('start-round-submit').click()
  await expect(page.getByTestId('start-round-result')).toBeVisible()

  expect(bodies[0].notification).toBe('metadata_only')
})

test('notification mode none warns that the reviewer is not notified and reaches the request body', async ({ page }) => {
  const bodies = captureStartRoundRequests(page)
  await openStartRoundModal(page)

  await openNotificationTab(page)
  await expect(page.getByTestId('notification-none-warning')).toHaveCount(0)
  await page.getByTestId('notification-mode-none').click()
  await expect(page.getByTestId('notification-none-warning')).toContainText('Nobody is told the round exists')

  await page.getByTestId('start-round-submit').click()
  await expect(page.getByTestId('start-round-result')).toBeVisible()

  expect(bodies[0].notification).toBe('none')
})

// ---------------------------------------------------------------------------
// S4: the checklist editor
// ---------------------------------------------------------------------------

test('checklist editor is seeded, freely editable, and the edit reaches the request body', async ({ page }) => {
  const bodies = captureStartRoundRequests(page)
  await openStartRoundModal(page)
  await openChecklistTab(page)

  const editor = page.locator('.mantine-Modal-body textarea')
  await expect(editor).toHaveValue(roundSeedCanStart.checklist_content!)

  await editor.fill('- [ ] Rewritten item\n- [ ] Second item')
  await page.getByTestId('start-round-submit').click()
  await expect(page.getByTestId('start-round-result')).toBeVisible()

  expect(bodies[0].checklist_content).toBe('- [ ] Rewritten item\n- [ ] Second item')
})

// ---------------------------------------------------------------------------
// Choosing which round's checklist to base the new one on
// ---------------------------------------------------------------------------

test('with a single recoverable checklist there is no picker to show', async ({ page }) => {
  // Starting round 2, so Initial QC is the only possible base — a one-item
  // dropdown would be noise.
  await openStartRoundModal(page)
  await openChecklistTab(page)
  // Meaningful only from inside the panel: an unrendered panel has no picker either.
  await expect(page.getByTestId('checklist-panel')).toBeVisible()
  await expect(page.getByTestId('checklist-source-round')).toHaveCount(0)
})

test('the picker offers every round and defaults to the most recent', async ({ page }) => {
  await openStartRoundModal(page, { roundSeedResponse: roundSeedMultipleChecklists })
  await openChecklistTab(page)

  const picker = page.getByTestId('checklist-source-round')
  await expect(picker).toBeVisible()
  // Pre-selected: the newest round, which is what the form used to use unconditionally.
  await expect(picker).toHaveValue('Round 2')
  await expect(page.locator('.mantine-Modal-body textarea')).toHaveValue(ROUND2_CHECKLIST)

  await picker.click()
  await expect(page.getByRole('option', { name: 'Initial QC', exact: true })).toBeVisible()
  await expect(page.getByRole('option', { name: 'Round 2', exact: true })).toBeVisible()
})

test('picking an earlier round re-seeds the editor and the name, and that reaches the request body', async ({ page }) => {
  const bodies = captureStartRoundRequests(page)
  await openStartRoundModal(page, { roundSeedResponse: roundSeedMultipleChecklists })
  await openChecklistTab(page)

  const initialQc = roundSeedMultipleChecklists.checklist_options[0]
  await page.getByTestId('checklist-source-round').click()
  await page.getByRole('option', { name: 'Initial QC', exact: true }).click()

  // The whole point: round 3 can go back to the original checklist rather than
  // inheriting round 2's trimmed one.
  await expect(page.locator('.mantine-Modal-body textarea')).toHaveValue(initialQc.content)
  await expect(page.getByLabel('Name', { exact: true })).toHaveValue('Code Review')

  await page.getByTestId('start-round-submit').click()
  await expect(page.getByTestId('start-round-result')).toBeVisible()
  expect(bodies[0].checklist_content).toBe(initialQc.content)
  expect(bodies[0].checklist_name).toBe('Code Review')
})

test('null checklist_content starts an empty editor with a note, and blocks submission', async ({ page }) => {
  await openStartRoundModal(page, {
    roundSeedResponse: {
      ...roundSeedCanStart,
      checklist_content: null,
      checklist_name: null,
      checklist_options: [],
      default_round: null,
    },
  })

  // The empty-state alert sits outside the tabs, so it is visible on any of them.
  await expect(page.getByTestId('no-prior-checklist')).toContainText('No prior checklist was found')
  await openChecklistTab(page)
  await expect(page.locator('.mantine-Modal-body textarea')).toHaveValue('')
  // Nothing to review against yet, so the action stays unavailable until filled.
  await expect(page.getByTestId('start-round-submit')).toBeDisabled()
})

// ---------------------------------------------------------------------------
// S4: result handling
// ---------------------------------------------------------------------------

test('needs_repair renders as a success with the failed step visible and a repair hint', async ({ page }) => {
  await openStartRoundModal(page, { startRoundResponse: startRoundNeedsRepair })

  await page.getByTestId('start-round-submit').click()

  // Success framing, never an error.
  const success = page.getByTestId('start-round-success')
  await expect(success).toContainText('Round 2 started')
  await expect(page.getByTestId('start-round-error')).toHaveCount(0)
  await expect(page.getByTestId('round-still-open')).toHaveCount(0)
  await expect(page.getByTestId('round-comment-link')).toHaveAttribute(
    'href',
    startRoundNeedsRepair.round_comment_url,
  )

  // The repair path is spelled out as safe to re-run.
  await expect(page.getByTestId('needs-repair')).toContainText('safe')

  // Failed vs skipped vs done are distinguishable per step.
  await expect(page.getByTestId('step-reopened')).toContainText('Failed')
  await expect(page.getByTestId('step-reopened')).toContainText('403 Forbidden')
  await expect(page.getByTestId('step-notification')).toContainText('Skipped')
  await expect(page.getByTestId('step-body_marker')).toContainText('Done')
})

test('a 409 renders the round-already-open precondition, not a generic failure', async ({ page }) => {
  await openStartRoundModal(page, { startRoundResponse: roundStillOpenError })

  await page.getByTestId('start-round-submit').click()

  const precondition = page.getByTestId('round-still-open')
  await expect(precondition).toBeVisible()
  await expect(precondition).toContainText('Initial QC is still open')
  await expect(page.getByTestId('start-round-error')).toHaveCount(0)
  await expect(page.getByTestId('start-round-result')).toHaveCount(0)
})

// ---------------------------------------------------------------------------
// S4: impact preview
// ---------------------------------------------------------------------------

test('impacted issues render as information only', async ({ page }) => {
  await openStartRoundModal(page, { startRoundResponse: startRoundWithImpactedIssues })

  await page.getByTestId('start-round-submit').click()

  const list = page.getByTestId('impact-list')
  await expect(list).toBeVisible()
  await expect(list).toContainText('Notice only — the previous approval still stands')
  await expect(page.getByTestId('impact-issue-91')).toContainText('src/file_a.rs')
  await expect(page.getByTestId('impact-empty')).toHaveCount(0)
  await expect(page.getByTestId('impact-unavailable')).toHaveCount(0)
})

test('no downstream issues renders an empty-list message', async ({ page }) => {
  await openStartRoundModal(page, { startRoundResponse: startRoundSuccess })

  await page.getByTestId('start-round-submit').click()

  await expect(page.getByTestId('impact-empty')).toContainText('No downstream QCs appear to depend on this file')
  await expect(page.getByTestId('impact-unavailable')).toHaveCount(0)
})

test('api_available false renders distinctly from an empty downstream list', async ({ page }) => {
  await openStartRoundModal(page, {
    startRoundResponse: {
      ...startRoundSuccess,
      impacted_issues: { api_available: false, issues: [] },
    },
  })

  await page.getByTestId('start-round-submit').click()

  await expect(page.getByTestId('impact-unavailable')).toContainText('could not be checked')
  await expect(page.getByTestId('impact-empty')).toHaveCount(0)
})

// ---------------------------------------------------------------------------
// The round rail's action (in the issue detail modal) opens the same modal
// ---------------------------------------------------------------------------

test('the round rail offers Start a new round, which opens the start-round modal', async ({ page }) => {
  await setupRoutes(page, {
    milestoneIssues: { 1: [multiRoundIssue] },
    issueStatuses: { results: [multiRoundApprovedStatus], errors: [] },
  })

  await page.goto('/')
  await selectMilestone(page, 'Sprint 1')
  await page.getByTestId('issue-card-112').click()

  // An approved issue's modal opens on Unapprove, which has no rail; the rail lives
  // on the comment-posting tabs.
  await page.getByRole('tab', { name: 'Notify', exact: true }).click()
  const railAction = page.getByRole('tabpanel', { name: 'Notify' }).getByTestId('round-rail-start')
  await expect(railAction).toBeVisible()
  await railAction.click()

  // One dialog at a time: the detail modal gives way to the round modal, which is
  // the single owner of this state — no second copy of it anywhere.
  await expect(page.getByRole('heading', { name: 'Start New QC Round' })).toBeVisible()
  await expect(page.getByTestId('round-rail')).toHaveCount(0)
  await expect(page.getByTestId('next-round-name')).toHaveText('Round 2')
})

test('a single approved Initial QC round shows the rail without collapse chrome', async ({ page }) => {
  await setupRoutes(page, {
    milestoneIssues: { 1: [changedIssue] },
    issueStatuses: { results: [changedAfterApprovalStatus], errors: [] },
  })

  await page.goto('/')
  await selectMilestone(page, 'Sprint 1')
  await page.getByTestId('issue-card-111').click()
  await page.getByRole('tab', { name: 'Notify', exact: true }).click()

  // The quiet case: one line, and the action is offered without extra sections.
  const panel = page.getByRole('tabpanel', { name: 'Notify' })
  await expect(panel.getByTestId('round-line-1')).toBeVisible()
  await expect(panel.getByTestId('round-section-1')).toHaveCount(0)
  await expect(panel.getByTestId('round-rail-start')).toBeVisible()
})

test('the rail offers no new round while the current one is open', async ({ page }) => {
  // A new round builds on an approval, and the backend refuses one over an open
  // round — a second round comment there extends the round instead of opening one.
  // So the affordance must not be reachable from an issue under review.
  await setupRoutes(page, {
    milestoneIssues: { 1: [multiRoundIssue] },
    issueStatuses: { results: [multiRoundStatus], errors: [] },
  })

  await page.goto('/')
  await selectMilestone(page, 'Sprint 1')
  await page.getByTestId('issue-card-112').click()

  const panel = page.getByRole('tabpanel')
  await expect(panel.getByTestId('round-rail')).toBeVisible()
  await expect(panel.getByTestId('round-section-2')).toBeVisible()
  await expect(panel.getByTestId('round-rail-start')).toHaveCount(0)
})

// ---------------------------------------------------------------------------
// The Changes tab: the commit range, its diff, and the note
// ---------------------------------------------------------------------------

test('the Changes tab shows both ends of the round and the diff between them', async ({ page }) => {
  await openStartRoundModal(page, { roundSeedResponse: roundSeedMultipleChecklists })
  await openChangesTab(page)

  const anchor = page.getByTestId('round-anchor')
  await expect(anchor).toContainText('Opens at (HEAD)')
  await expect(anchor).toContainText('Compares against')

  const diff = page.getByTestId('round-diff')
  await expect(diff).toBeVisible()
  await expect(diff).toContainText('+added line')
  await expect(diff).toContainText('-removed line')
  // The fence is chrome, not content.
  await expect(diff).not.toContainText('```')
})

test('the diff is requested once for the seed file across the round range', async ({ page }) => {
  const urls: string[] = []
  page.on('request', (request) => {
    if (request.url().includes('/api/commits/diff')) urls.push(request.url())
  })

  // Changes is the tab the modal opens on, so the diff is fetched straight away.
  await openStartRoundModal(page, { roundSeedResponse: roundSeedMultipleChecklists })
  await expect(page.getByTestId('round-diff')).toBeVisible()

  expect(urls).toHaveLength(1)
  const url = new URL(urls[0])
  expect(url.searchParams.get('file')).toBe(roundSeedMultipleChecklists.file)
  expect(url.searchParams.get('from')).toBe(roundSeedMultipleChecklists.previous_approval)
  expect(url.searchParams.get('to')).toBe(roundSeedMultipleChecklists.anchor)

  // Panels unmount when inactive, so leaving and returning remounts the component.
  // The query cache must absorb that rather than re-diffing on every tab click.
  await openChecklistTab(page)
  await openChangesTab(page)
  await expect(page.getByTestId('round-diff')).toBeVisible()
  expect(urls).toHaveLength(1)
})

test('a null diff reads as no changes rather than as a failure', async ({ page }) => {
  // A round opened at the very commit it compares against is a normal state, not an error.
  await openStartRoundModal(page, {
    roundSeedResponse: roundSeedMultipleChecklists,
    commitDiffResponse: { diff: null },
  })
  await openChangesTab(page)

  await expect(page.getByTestId('round-diff-empty')).toContainText('No changes')
  await expect(page.getByTestId('round-diff')).toHaveCount(0)
  await expect(page.getByTestId('round-diff-error')).toHaveCount(0)
  // Still startable: an empty diff blocks nothing.
  await expect(page.getByTestId('start-round-submit')).toBeEnabled()
})

test('the note lives on the Changes tab and reaches the request body', async ({ page }) => {
  const bodies = captureStartRoundRequests(page)
  await openStartRoundModal(page, { roundSeedResponse: roundSeedMultipleChecklists })
  await openChangesTab(page)

  await page.getByLabel('Note (optional)').fill('New data arrived from the client')
  await page.getByTestId('start-round-submit').click()
  await expect(page.getByTestId('start-round-result')).toBeVisible()

  expect(bodies[0].note).toBe('New data arrived from the client')
})

/**
 * The round's note and the reviewer's message answer different questions, and one
 * shared `note` used to serve both. Filling only one must leave the other null.
 */
test('the round note and the reviewer message are sent as separate fields', async ({ page }) => {
  const bodies = captureStartRoundRequests(page)
  await openStartRoundModal(page)

  await openChangesTab(page)
  await page.getByLabel('Note (optional)').fill('Rerun after the covariate fix')
  await openNotificationTab(page)
  await page.getByTestId('notification-note').fill('Focus on the ETA block, the rest is unchanged')

  await page.getByTestId('start-round-submit').click()
  await expect(page.getByTestId('start-round-result')).toBeVisible()

  expect(bodies[0].note).toBe('Rerun after the covariate fix')
  expect(bodies[0].notification_note).toBe('Focus on the ETA block, the rest is unchanged')
})

test('a round note alone leaves the reviewer message null', async ({ page }) => {
  const bodies = captureStartRoundRequests(page)
  await openStartRoundModal(page)

  await openChangesTab(page)
  await page.getByLabel('Note (optional)').fill('Rerun after the covariate fix')
  await page.getByTestId('start-round-submit').click()
  await expect(page.getByTestId('start-round-result')).toBeVisible()

  expect(bodies[0].note).toBe('Rerun after the covariate fix')
  // No fallback: an unwritten message is not the round's reason repeated.
  expect(bodies[0].notification_note).toBeNull()
})

/**
 * A message with no comment to ride on would be silently discarded, so the field is
 * not offered at all once nothing is being posted.
 */
test('the reviewer message is withdrawn when no notification is posted', async ({ page }) => {
  await openStartRoundModal(page)
  await openNotificationTab(page)

  await expect(page.getByTestId('notification-note')).toBeVisible()
  await page.getByTestId('notification-mode-none').click()
  await expect(page.getByTestId('notification-note')).toHaveCount(0)

  // ...and offered again on any mode that does post one.
  await page.getByTestId('notification-mode-metadata_only').click()
  await expect(page.getByTestId('notification-note')).toBeVisible()
})

/**
 * A blocked seed offers no submit, so the mode is moot — but the whole card is the
 * hit target, and a card that still responds reads as a form that will act.
 */
test('a blocked round cannot change its notification mode', async ({ page }) => {
  await openStartRoundModal(page, { roundSeedResponse: roundSeedBlocked })
  await openNotificationTab(page)

  await page.getByTestId('notification-mode-none').click({ force: true })
  await expect(page.getByTestId('notification-none-warning')).toHaveCount(0)
  // Still on the default, and the message field never appeared to be filled in.
  await expect(page.getByTestId('notification-note')).toBeDisabled()
})

/**
 * The preview reuses the notify tab's endpoint because the round's notification *is*
 * a QCComment. What it must carry is the round's own range plus whatever the form
 * currently says — not the round note, and not a fixed diff setting.
 */
test('the notification preview renders the comment for the round range and message', async ({ page }) => {
  const previews = captureCommentPreviewRequests(page)
  await openStartRoundModal(page)

  await openChangesTab(page)
  await page.getByLabel('Note (optional)').fill('Round reason, not for the reviewer')
  await openNotificationTab(page)
  await page.getByTestId('notification-note').fill('Check the ETA block')
  await page.getByTestId('notification-preview').click()

  await expect(page.getByTestId('notification-preview-frame')).toBeVisible()

  expect(previews).toHaveLength(1)
  expect(previews[0].current_commit).toBe(roundSeedCanStart.anchor)
  expect(previews[0].previous_commit).toBe(roundSeedCanStart.previous_approval)
  expect(previews[0].note).toBe('Check the ETA block')
  expect(previews[0].include_diff).toBe(true)
})

test('previewing in metadata-only mode asks for no diff', async ({ page }) => {
  const previews = captureCommentPreviewRequests(page)
  await openStartRoundModal(page)
  await openNotificationTab(page)

  await page.getByTestId('notification-mode-metadata_only').click()
  await page.getByTestId('notification-preview').click()
  await expect(page.getByTestId('notification-preview-frame')).toBeVisible()

  expect(previews[0].include_diff).toBe(false)
})

/** Nothing is posted in silent mode, so there is nothing to preview. */
test('the preview is withdrawn when no notification is posted', async ({ page }) => {
  await openStartRoundModal(page)
  await openNotificationTab(page)

  await expect(page.getByTestId('notification-preview')).toBeVisible()
  await page.getByTestId('notification-mode-none').click()
  await expect(page.getByTestId('notification-preview')).toHaveCount(0)
})

/** Each mode's rationale is readable without selecting it — the point of the cards. */
test('every notification mode shows its description at once', async ({ page }) => {
  await openStartRoundModal(page)
  await openNotificationTab(page)

  await expect(page.getByTestId('notification-mode-full')).toContainText('inline diff')
  await expect(page.getByTestId('notification-mode-metadata_only')).toContainText('without the inline diff')
  await expect(page.getByTestId('notification-mode-none')).toContainText('silently')
})

// ---------------------------------------------------------------------------
// Header
// ---------------------------------------------------------------------------

test('the issue line links to the issue on GitHub', async ({ page }) => {
  await openStartRoundModal(page)

  const link = page.getByTestId('round-issue-link')
  await expect(link).toContainText(`#${changedIssue.number}`)
  await expect(link).toContainText(changedIssue.title)
  await expect(link).toHaveAttribute('href', changedIssue.html_url)
  await expect(link).toHaveAttribute('target', '_blank')
})

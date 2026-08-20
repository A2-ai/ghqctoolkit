// Archive under round semantics: what the card says, what the picker offers, and what
// travels on the wire (U1–U3, U5, U6, U8, A1/A2).
//
// Every fixture here leaves `qc_status.last_approved_commit` and `qc_status.latest_commit`
// **null** on purpose. Those two fields were the whole of the deleted `archiveCommitOf`, so
// any commit these cards show, and any commit a preview requests, can only have come from
// that round's `archive_preview`. If the old expression ever comes back, these specs report
// a dash.
//
// Each scenario states the `commit` and `approval` its previews carry, because those are S1's
// and I2's answers and belong to the fixture author. `superseding_causes` is not stated on
// its own authority anywhere: it is a fact about the segments *after* a round, so
// `segmentFields` finalizes it from the whole thread (`coherentPreviews`), and the arrays
// written out below agree with that pass rather than replacing it.
//
// The last spec in this file is the exception, and deliberately so — it hands the UI a
// preview that **contradicts** its own round to prove the UI renders the wire instead of
// re-deriving it.

import { test, expect } from 'playwright/test'
import { setupRoutes } from '../helpers/routes'
import {
  closeRound,
  commit,
  gapSegment,
  initialQcRound,
  laterRound,
  notificationEvent,
  ROUND1_CLOSED,
  ROUND1_OPENED,
  ROUND2_DRIFT,
  ROUND2_OPENED,
  segmentFields,
} from '../fixtures/index'
import type { Issue, IssueStatusResponse, QCStatus } from '../../src/api/issues'
import type { ArchivePreview, RoundSegment, Segment } from '../../src/api/rounds'
import type { Milestone } from '../../src/api/milestones'
import type { FileTreeResponse } from '../../src/api/files'
import type { ArchiveGenerateRequest, ArchiveIssueFileRequest } from '../../src/api/archive'

// ── Fixtures ──────────────────────────────────────────────────────────────────

const milestoneA: Milestone = {
  number: 10,
  title: 'Milestone A',
  state: 'closed',
  description: null,
  open_issues: 0,
  closed_issues: 4,
}

function makeIssue(number: number, title: string, state: Issue['state'] = 'closed'): Issue {
  return {
    number,
    title,
    state,
    html_url: `https://github.com/test-owner/test-repo/issues/${number}`,
    assignees: [],
    labels: ['ghqc', 'main'],
    milestone: 'Milestone A',
    created_at: '2024-01-01T00:00:00Z',
    updated_at: '2024-01-06T00:00:00Z',
    closed_at: state === 'closed' ? '2024-01-05T00:00:00Z' : null,
    created_by: 'test-user',
    branch: 'main',
    checklist_name: 'Code Review',
    relevant_files: [],
    file_history: [],
  }
}

function statusOf(
  issue: Issue,
  segments: Segment[],
  status: QCStatus['status'] = 'approved',
  qcOverrides: Partial<QCStatus> = {},
): IssueStatusResponse {
  return {
    issue,
    qc_status: {
      status,
      status_detail: '',
      standing_approval: null,
      // Deliberately null — see the file header.
      last_approved_commit: null,
      initial_commit: null,
      latest_commit: null,
      changed_commit: null,
      last_reviewed_commit: null,
      last_notified_commit: null,
      ...qcOverrides,
    },
    dirty: false,
    checklist_summary: { completed: 5, total: 5, percentage: 1.0 },
    // Required on the wire and always emitted, so a fixture that omits it is the wrong
    // shape even though nothing in this tab reads it (§31.3).
    blocking_qc_status: EMPTY_BLOCKING_QC,
    ...segmentFields(segments),
  }
}

/** No blocking QC: the shape the wire always emits, with nothing in it. */
const EMPTY_BLOCKING_QC: IssueStatusResponse['blocking_qc_status'] = {
  total: 0,
  approved_count: 0,
  summary: '',
  approved: [],
  not_approved: [],
  errors: [],
}

/** The approval a closed round's preview carries — the fixture default's `by`/`at`. */
function approvalOf(round: number, commit: string): ArchivePreview['approval'] {
  return { round, commit, by: 'reviewer1', at: '2024-01-05T00:00:00Z' }
}

/** Re-state a round's preview: what the archive would take, and why it may not be current. */
function withPreview(round: RoundSegment, preview: ArchivePreview | null): RoundSegment {
  return { ...round, archive_preview: preview }
}

/** One round, closed, trailing empty gap: the steady approved state. */
function approvedSegments(): Segment[] {
  return [closeRound(initialQcRound(ROUND1_OPENED), ROUND1_CLOSED), gapSegment()]
}

/**
 * Approved in Initial QC, then re-opened as Round 2 on a *later* anchor. The archive's
 * default is Round 2 (D9), so it takes unapproved bytes (S2) — the one intended behaviour
 * change, and the reason U3 labels it instead of gating it.
 */
function reopenedSegments(): Segment[] {
  return [
    // Round 1's own preview: still its approval, and no longer provably current, because the
    // thread's latest round is open (clause 2).
    withPreview(closeRound(initialQcRound(ROUND1_OPENED), ROUND1_CLOSED), {
      commit: ROUND1_CLOSED,
      approval: approvalOf(1, ROUND1_CLOSED),
      superseding_causes: ['round_open'],
    }),
    gapSegment({ lower_bound: ROUND1_CLOSED, upper_bound: ROUND2_OPENED }),
    laterRound(2, ROUND2_OPENED, {
      // Drift nobody acted on is newer than the notification, and is *not* what the
      // archive takes (S6): `latest_actioned_commit` names the notified commit.
      commits: [
        commit(ROUND2_DRIFT),
        commit(ROUND2_OPENED, { statuses: ['initial', 'notification'] }),
      ],
      events: [notificationEvent(ROUND2_OPENED)],
      latest_actioned_commit: ROUND2_OPENED,
    }),
  ]
}

/**
 * I2 on the wire: Round 2's anchor **is** Initial QC's approval (D1 — HEAD had not moved),
 * so selecting the open Round 2 archives approved bytes under a different frame.
 */
function twoFramesSegments(): Segment[] {
  return [
    withPreview(closeRound(initialQcRound(ROUND1_OPENED), ROUND1_CLOSED), {
      commit: ROUND1_CLOSED,
      approval: approvalOf(1, ROUND1_CLOSED),
      superseding_causes: ['round_open'],
    }),
    gapSegment({ lower_bound: ROUND1_CLOSED, upper_bound: ROUND1_CLOSED }),
    laterRound(2, ROUND1_CLOSED, {
      commits: [commit(ROUND1_CLOSED, { statuses: ['initial', 'approved'] })],
      latest_actioned_commit: ROUND1_CLOSED,
      // The projected I2 answer: previewing the **open** round 2 yields approved bytes whose
      // `approval.round` is **1**. Nothing about round 2 is asserted approved.
      archive_preview: {
        commit: ROUND1_CLOSED,
        approval: approvalOf(1, ROUND1_CLOSED),
        superseding_causes: ['round_open'],
      },
    }),
  ]
}

/**
 * Initial QC closed and fully placed; Round 2 open and unplaceable. The default selection
 * is Round 2, which cannot be archived (§18.1/§20.2) — and retargeting to Initial QC is a
 * well-formed request the old active-segment gate refused (§20.5).
 */
function unplaceableLatestSegments(): Segment[] {
  return [
    // Two causes, in clause order: the latest round is open (2) and what follows this round
    // could not be located (4). Round 1 is still archivable — the gate is the selected round.
    withPreview(closeRound(initialQcRound(ROUND1_OPENED), ROUND1_CLOSED), {
      commit: ROUND1_CLOSED,
      approval: approvalOf(1, ROUND1_CLOSED),
      superseding_causes: ['round_open', 'undeterminable'],
    }),
    gapSegment({ placement: { kind: 'unplaceable', reason: 'neighbour_unplaceable' } }),
    laterRound(2, ROUND2_OPENED, {
      branch: 'feature/gone',
      commits: [],
      latest_actioned_commit: null,
      placement: { kind: 'unplaceable', reason: 'branch_unavailable' },
    }),
  ]
}

const singleIssue = makeIssue(100, 'src/single.R')
const reopenedIssue = makeIssue(101, 'src/reopened.R', 'open')
const twoFramesIssue = makeIssue(102, 'src/two-frames.R', 'open')
const brokenIssue = makeIssue(103, 'src/broken.R', 'open')

const singleStatus = statusOf(singleIssue, approvedSegments())
const reopenedStatus = statusOf(reopenedIssue, reopenedSegments(), 'awaiting_review')
const twoFramesStatus = statusOf(twoFramesIssue, twoFramesSegments(), 'awaiting_review')
const brokenStatus = statusOf(brokenIssue, unplaceableLatestSegments(), 'unknown')

const archiveRootTree: FileTreeResponse = {
  path: '',
  entries: [
    { name: 'src', kind: 'directory' },
    { name: 'README.md', kind: 'file' },
  ],
}
const archiveSrcTree: FileTreeResponse = {
  path: 'src',
  entries: [{ name: 'helpers.R', kind: 'file' }],
}

// ── Helpers ───────────────────────────────────────────────────────────────────

function captureArchiveRequests(page: import('playwright/test').Page): ArchiveGenerateRequest[] {
  const bodies: ArchiveGenerateRequest[] = []
  page.on('request', (request) => {
    if (request.method() === 'POST' && /\/api\/archive\/generate/.test(request.url())) {
      bodies.push(request.postDataJSON() as ArchiveGenerateRequest)
    }
  })
  return bodies
}

async function goToArchive(page: import('playwright/test').Page) {
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

function main(page: import('playwright/test').Page) {
  return page.locator('main')
}

async function selectMilestoneA(page: import('playwright/test').Page) {
  await main(page).getByPlaceholder('Search milestones…').click()
  await page.getByRole('option', { name: /Milestone A/ }).click()
  await expect(page.getByText(/issues? loading/)).not.toBeVisible({ timeout: 10_000 })
}

async function openArchiveWith(
  page: import('playwright/test').Page,
  statuses: IssueStatusResponse[],
  extra: Parameters<typeof setupRoutes>[1] = {},
) {
  await setupRoutes(page, {
    milestones: [milestoneA],
    milestoneIssues: { 10: statuses.map((s) => s.issue) },
    issueStatuses: { results: statuses, errors: [] },
    fileTree: { '': archiveRootTree, src: archiveSrcTree },
    ...extra,
  })
  await goToArchive(page)
  await selectMilestoneA(page)
}

function generateButton(page: import('playwright/test').Page) {
  return main(page).getByRole('button', { name: 'Generate Archive' })
}

/** Mode-1 entries only, for asserting the round a selection sent. */
function issueEntries(body: ArchiveGenerateRequest): ArchiveIssueFileRequest[] {
  return body.files.filter((f): f is ArchiveIssueFileRequest => f.mode === 'issue')
}

// ── U2: the picker appears only where there is something to choose ────────────

test('a single-round file gets no round picker, and reads as its round’s approval', async ({ page }) => {
  await openArchiveWith(page, [singleStatus])

  await expect(page.getByTestId('archive-provenance-100')).toHaveText(
    'Initial QC · approved by @reviewer1 on 2024-01-05 · b2b2b2b',
  )
  // `superseding_causes: []` is a *positive* claim of currency, not an absence: the card
  // says nothing about supersession and the summary counts the file as current. A renderer
  // that treated the empty array as unknown would fail here.
  await expect(page.getByTestId('archive-superseded-100')).toHaveCount(0)
  await expect(main(page).getByTestId('archive-summary')).toHaveText(
    '1 file · 1 approved & current',
  )
  await expect(page.getByTestId('archive-round-picker-100')).toHaveCount(0)
})

// ── U1/U3/D9/S2: the default is the latest round, labelled and ungated ────────

test('a reopened file defaults to its open round’s unapproved bytes, with no gate', async ({ page }) => {
  const bodies = captureArchiveRequests(page)
  await openArchiveWith(page, [reopenedStatus])

  // S1 row 3 with S6: the notified commit, not the newer drift commit nobody acted on.
  await expect(page.getByTestId('archive-provenance-101')).toHaveText(
    'Round 2 · unapproved · d4d4d4d',
  )
  await expect(page.getByTestId('archive-superseded-101')).toContainText('Round 2 is open')
  await expect(main(page).getByTestId('archive-unapproved-note')).toContainText(
    '1 file will be archived at unapproved bytes',
  )

  // D9 forbids a gate: no confirmation, no blocking modal, and the button is live.
  await expect(generateButton(page)).toBeEnabled()
  await generateButton(page).click()
  await expect(page.getByText(/Archive written to/)).toBeVisible()
  await expect(page.getByRole('dialog')).toHaveCount(0)

  expect(bodies).toHaveLength(1)
  expect(bodies[0].files).toEqual([{ mode: 'issue', issue_number: 101, round: null }])
})

test('retargeting a file to an earlier round sends that round and marks the override', async ({ page }) => {
  const bodies = captureArchiveRequests(page)
  await openArchiveWith(page, [reopenedStatus])

  const picker = page.getByTestId('archive-round-picker-101')
  await expect(picker).toContainText('Round 2 · latest')
  await picker.click()

  // The options describe what each round would archive, so the choice can be made.
  await expect(page.getByTestId('archive-round-option-101-1')).toContainText(
    'Initial QC · closed · approved by @reviewer1 on 2024-01-05 · b2b2b2b',
  )
  await expect(page.getByTestId('archive-round-option-101-2')).toContainText(
    'Round 2 · open · would archive d4d4d4d',
  )
  await page.getByTestId('archive-round-option-101-1').click()

  await expect(page.getByTestId('archive-provenance-101')).toHaveText(
    'Initial QC · approved by @reviewer1 on 2024-01-05 · b2b2b2b',
  )
  await expect(page.getByTestId('archive-round-picker-101')).toContainText('Initial QC · override')
  await expect(page.getByTestId('archive-round-picker-101')).toHaveAttribute('data-override', 'true')
  // Still not the newest QC state, and it says which round overtook it.
  await expect(page.getByTestId('archive-superseded-101')).toContainText('Round 2 is open')

  await generateButton(page).click()
  await expect(page.getByText(/Archive written to/)).toBeVisible()
  expect(issueEntries(bodies[0])).toEqual([{ mode: 'issue', issue_number: 101, round: 1 }])
})

// ── I2: two round frames, never collapsed into one ───────────────────────────

test('an open round anchored at the previous approval names both frames', async ({ page }) => {
  await openArchiveWith(page, [twoFramesStatus])

  const provenance = page.getByTestId('archive-provenance-102')
  await expect(provenance).toHaveText(
    "Round 2 · bytes are Initial QC's approval by @reviewer1 on 2024-01-05 · b2b2b2b",
  )
  // The forbidden rendering: Round 2 is open and is never asserted approved.
  await expect(provenance).not.toContainText('Round 2 · approved')
  await expect(page.getByTestId('archive-superseded-102')).toContainText('Round 2 is open')
})

// ── U8/§11.1/§18.1: an unplaceable *selected* round blocks, with its reason ───

test('an unplaceable selected round blocks generation, names the reason, and acknowledging leaves the file out', async ({ page }) => {
  const bodies = captureArchiveRequests(page)
  await openArchiveWith(page, [singleStatus, brokenStatus])

  const callout = main(page).getByTestId('archive-unplaceable-callout')
  await expect(callout).toContainText('1 file cannot be archived')
  await expect(callout).toContainText('src/broken.R (#103, Round 2): its branch is unavailable locally')
  await expect(page.getByTestId('archive-blocked-note-103')).toContainText(
    'Round 2 cannot be archived — its branch is unavailable locally',
  )
  // It blocks — the old note said only how many files were dropped and generated anyway.
  await expect(generateButton(page)).toBeDisabled()

  await callout.getByTestId('archive-unplaceable-acknowledge').click()
  await expect(generateButton(page)).toBeEnabled()
  await generateButton(page).click()
  await expect(page.getByText(/Archive written to/)).toBeVisible()

  // Acknowledging proceeds *without* the file. It is never included.
  expect(bodies).toHaveLength(1)
  expect(bodies[0].files).toEqual([{ mode: 'issue', issue_number: 100, round: null }])
})

test('retargeting away from an unplaceable round makes the file archivable again', async ({ page }) => {
  const bodies = captureArchiveRequests(page)
  await openArchiveWith(page, [brokenStatus])

  await expect(main(page).getByTestId('archive-unplaceable-callout')).toBeVisible()
  await page.getByTestId('archive-round-picker-103').click()
  await page.getByTestId('archive-round-option-103-1').click()

  // The gate is the selected round, not the active segment: Initial QC is placed and
  // closed, so the file is archivable even though Round 2 is not.
  await expect(main(page).getByTestId('archive-unplaceable-callout')).toHaveCount(0)
  await expect(page.getByTestId('archive-provenance-103')).toHaveText(
    'Initial QC · approved by @reviewer1 on 2024-01-05 · b2b2b2b',
  )
  // Both causes of the projected array, in clause order, each in its own words — and the
  // undeterminable wording covers both halves of the clause, since an `Unrelated` gap is
  // placed and readable and simply shares no history.
  await expect(page.getByTestId('archive-superseded-103')).toHaveText(
    'Not the newest QC state: Round 2 is open; what happened after Initial QC could not be ' +
      'located, or spans histories with no common ancestor.',
  )
  await generateButton(page).click()
  await expect(page.getByText(/Archive written to/)).toBeVisible()
  expect(bodies[0].files).toEqual([{ mode: 'issue', issue_number: 103, round: 1 }])
})

// ── A1/A2/D4: mode 2 is unchanged, and carries no QC claim ───────────────────

test('a manually added file travels as mode file with the commit the user picked', async ({ page }) => {
  const bodies = captureArchiveRequests(page)
  await openArchiveWith(page, [singleStatus])

  await page.getByTestId('archive-add-file-card').click()
  const modal = page.getByRole('dialog')
  await modal.getByRole('treeitem', { name: 'src' }).click()
  await modal.getByRole('treeitem', { name: 'helpers.R' }).click()
  await modal.getByRole('button', { name: 'Next →' }).click()
  await modal.getByRole('button', { name: /Use commit/ }).click()

  await generateButton(page).click()
  await expect(page.getByText(/Archive written to/)).toBeVisible()

  expect(bodies).toHaveLength(1)
  // No `approved`, no `milestone`, no `round`: the field that made every added file a 400
  // is gone with the shape that carried it.
  expect(bodies[0].files).toEqual([
    { mode: 'issue', issue_number: 100, round: null },
    { mode: 'file', repository_file: 'src/helpers.R', commit: 'abc1234567890' },
  ])
})

// ── U5: the pre-generate summary ─────────────────────────────────────────────

test('the summary counts approved & current, superseded and unapproved before generating', async ({ page }) => {
  // Approved with a file-changing commit after it: S3 clause 3 — the bytes are still the
  // approval (R2), and the summary says they are not the newest QC state.
  const driftedIssue = makeIssue(104, 'src/drifted.R')
  const driftedStatus = statusOf(
    driftedIssue,
    [
      withPreview(closeRound(initialQcRound(ROUND1_OPENED), ROUND1_CLOSED), {
        // Still the approval — R2 keeps the bytes and merely labels them — and no longer
        // provably current, because the gap trailing it changed the file (clause 3).
        commit: ROUND1_CLOSED,
        approval: approvalOf(1, ROUND1_CLOSED),
        superseding_causes: ['changed_since'],
      }),
      gapSegment({
        lower_bound: ROUND1_CLOSED,
        commits: [commit(ROUND2_DRIFT, { file_changed: true })],
      }),
    ],
    'changes_after_approval',
    { changed_commit: ROUND2_DRIFT },
  )

  await openArchiveWith(page, [singleStatus, driftedStatus, reopenedStatus])

  await expect(main(page).getByTestId('archive-summary')).toHaveText(
    '3 files · 1 approved & current · 1 approved but superseded · 1 unapproved (Round 2 open)',
  )
  await expect(page.getByTestId('archive-superseded-104')).toContainText(
    'the file changed after this approval',
  )
})

// ── U4: round-aware bulk filters ─────────────────────────────────────────────

test('the under-review filter narrows the archive and says how many files it hid', async ({ page }) => {
  const bodies = captureArchiveRequests(page)
  await openArchiveWith(page, [singleStatus, reopenedStatus])

  await page.getByTestId('archive-filter-under_review').click()

  await expect(page.getByTestId('archive-provenance-101')).toBeVisible()
  await expect(page.getByTestId('archive-provenance-100')).toHaveCount(0)
  await expect(main(page).getByTestId('archive-filter-note')).toContainText('1 file hidden')

  await generateButton(page).click()
  await expect(page.getByText(/Archive written to/)).toBeVisible()
  expect(bodies[0].files).toEqual([{ mode: 'issue', issue_number: 101, round: null }])
})

test('the never-approved filter finds the file no round ever closed on', async ({ page }) => {
  const freshIssue = makeIssue(105, 'src/fresh.R', 'open')
  const freshStatus = statusOf(
    freshIssue,
    [initialQcRound(ROUND1_OPENED, { latest_actioned_commit: ROUND1_OPENED })],
    'awaiting_review',
  )

  await openArchiveWith(page, [singleStatus, freshStatus])
  // S4: a never-approved thread is out until its milestone says otherwise.
  await expect(page.getByTestId('archive-provenance-105')).toHaveCount(0)
  await page.getByRole('switch', { name: 'Include non-approved' }).click()
  await expect(page.getByTestId('archive-provenance-105')).toBeVisible()

  await page.getByTestId('archive-filter-never_approved').click()
  await expect(page.getByTestId('archive-provenance-105')).toBeVisible()
  await expect(page.getByTestId('archive-provenance-100')).toHaveCount(0)
})

// ── §31.1: the wire wins, even when the wire is obviously wrong ───────────────

/** A sha that appears nowhere in the thread below: only the wire names it. */
const WIRE_ONLY_COMMIT = 'f9f9f9f9f9f9f9f9f9f9f9f9f9f9f9f9f9f9f9f9'

const wireWinsIssue = makeIssue(106, 'src/wire-wins.R')

/**
 * **This fixture is INTENTIONALLY INCOHERENT. Do not "fix" it into consistency — doing so
 * silently deletes the only guard against the regression §26.6 exists to prevent.**
 *
 * Every other fixture models what a faithful projection would return, so a reintroduced —
 * and *correct* — TypeScript clone of S1's row rule and I2's tie-break would produce
 * byte-identical output everywhere and be caught by nothing. That is the duplicate-rule
 * defect §25.1 escalated, and review by grep is otherwise its only defence.
 *
 * So this thread's single round is **closed at `ROUND1_CLOSED` by `reviewer1` on 2024-01-05**
 * with an **empty, linear trailing gap**, while its `archive_preview` claims:
 *
 * | Field | The wire says | A local re-derivation would say |
 * |---|---|---|
 * | `commit` | `WIRE_ONLY_COMMIT` (`f9f9f9f…`) | `ROUND1_CLOSED` (`b2b2b2b…`) — S1 row 2 |
 * | `approval.round` | `7` — a round this thread does not have | `1` — §13.1, the round that closed |
 * | `approval.by` / `.at` | `wire-only` / 2026-01-02 | `reviewer1` / 2024-01-05 |
 * | `superseding_causes` | `['changed_since']` | `[]` — nothing follows but an empty linear gap |
 *
 * Every assertion below therefore *fails* if the UI computes any of it, and passes only if
 * the UI renders what it was sent. It is built by hand rather than through `segmentFields`
 * because that helper finalizes cause lists from the thread, which would repair the very
 * disagreement being tested.
 */
const wireWinsStatus: IssueStatusResponse = {
  ...statusOf(wireWinsIssue, approvedSegments()),
  segments: [
    {
      ...closeRound(initialQcRound(ROUND1_OPENED), ROUND1_CLOSED),
      archive_preview: {
        commit: WIRE_ONLY_COMMIT,
        approval: {
          round: 7,
          commit: WIRE_ONLY_COMMIT,
          by: 'wire-only',
          at: '2026-01-02T09:00:00Z',
        },
        superseding_causes: ['changed_since'],
      },
    },
    gapSegment({ lower_bound: ROUND1_CLOSED, upper_bound: ROUND1_CLOSED }),
  ],
}

test('the card renders the projected preview even when it contradicts the round it sits on', async ({ page }) => {
  const bodies = captureArchiveRequests(page)
  let previewedCommit: string | null = null

  await openArchiveWith(page, [wireWinsStatus])

  // Registered AFTER openArchiveWith on purpose: Playwright runs the most recently added
  // matching handler first, and openArchiveWith's setupRoutes installs an `/api/**`
  // catch-all. Registering this first lets that catch-all shadow it, so the handler never
  // fires and `previewedCommit` stays null. `flatten.spec.ts`'s preview test orders it the
  // same way for the same reason.
  await page.route(/\/api\/files\/content/, async (route, request) => {
    previewedCommit = new URL(request.url()).searchParams.get('commit')
    await route.fulfill({ status: 200, contentType: 'text/plain', body: 'bytes' })
  })

  // The provenance line is the wire's, down to the round number, the reviewer and the date.
  const provenance = page.getByTestId('archive-provenance-106')
  await expect(provenance).toHaveText(
    "Initial QC · bytes are Round 7's approval by @wire-only on 2026-01-02 · f9f9f9f",
  )
  // And none of it is the answer a re-derivation from this round's own fields would give.
  await expect(provenance).not.toContainText('b2b2b2b')
  await expect(provenance).not.toContainText('reviewer1')
  await expect(provenance).not.toContainText('2024-01-05')

  // A correct clause evaluation over this thread yields no causes at all; the wire says one,
  // so the card says one — and the summary counts the file as superseded, not current.
  await expect(page.getByTestId('archive-superseded-106')).toHaveText(
    'Not the newest QC state: the file changed after this approval.',
  )
  await expect(main(page).getByTestId('archive-summary')).toHaveText(
    '1 file · 1 approved but superseded',
  )

  // End to end: the commit the UI acts on is the previewed one, not the round's own.
  await main(page).getByRole('button', { name: 'Preview' }).click()
  await expect(page.getByRole('dialog')).toBeVisible()
  expect(previewedCommit).toBe(WIRE_ONLY_COMMIT)

  // Dismiss the preview before touching the page behind it: Mantine's modal overlay sits in a
  // portal over `main` and swallows the click on Generate Archive otherwise.
  await page.keyboard.press('Escape')
  await expect(page.getByRole('dialog')).toBeHidden()

  // None of it travels: the request still carries the issue and the round, and nothing else.
  await main(page).getByRole('button', { name: 'Generate Archive' }).click()
  await expect(page.getByText(/Archive written to/)).toBeVisible()
  expect(bodies[0].files).toEqual([{ mode: 'issue', issue_number: 106, round: null }])
})

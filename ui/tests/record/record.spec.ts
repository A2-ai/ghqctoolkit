import { test, expect } from 'playwright/test'
import { setupRoutes } from '../helpers/routes'
import { closedMilestone, openMilestone, defaultRepoInfo, rootFileTree, srcFileTree } from '../fixtures/index'
import type { Issue, IssueStatusResponse, BatchIssueStatusResponse, QCStatus } from '../../src/api/issues'

// ── Test-local fixtures ───────────────────────────────────────────────────────

const issue90: Issue = {
  number: 90,
  title: 'src/analysis.R',
  state: 'closed',
  html_url: 'https://github.com/test-owner/test-repo/issues/90',
  assignees: [],
  labels: ['ghqc', 'main'],
  milestone: 'Sprint 0',
  created_at: '2024-01-01T00:00:00Z',
  updated_at: '2024-01-10T00:00:00Z',
  closed_at: '2024-01-10T00:00:00Z',
  created_by: 'test-user',
  branch: 'main',
  checklist_name: 'Code Review',
  relevant_files: [],
}

const issue91: Issue = {
  ...issue90,
  number: 91,
  title: 'src/helper.R',
  html_url: 'https://github.com/test-owner/test-repo/issues/91',
}

function makeStatus(issue: Issue, status: QCStatus['status']): IssueStatusResponse {
  return {
    issue,
    qc_status: {
      status,
      status_detail: '',
      approved_commit: status === 'approved' ? 'aaa1111' : null,
      initial_commit: 'bbb2222',
      latest_commit: 'ccc3333',
    },
    dirty: false,
    branch: 'main',
    commits: [{ hash: 'ccc3333', message: 'initial', statuses: ['initial'], file_changed: true }],
    checklist_summary: { completed: 5, total: 5, percentage: 1.0 },
    blocking_qc_status: { total: 0, approved_count: 0, summary: '0/0', approved: [], not_approved: [], errors: [] },
  }
}

const allApprovedBatch: BatchIssueStatusResponse = {
  results: [makeStatus(issue90, 'approved'), makeStatus(issue91, 'approved')],
  errors: [],
}

const unapprovedBatch: BatchIssueStatusResponse = {
  results: [makeStatus(issue90, 'approved'), makeStatus(issue91, 'awaiting_review')],
  errors: [],
}

const allErrorBatch: BatchIssueStatusResponse = {
  results: [],
  errors: [
    { issue_number: 90, kind: 'fetch_failed', error: 'Not found' },
    { issue_number: 91, kind: 'fetch_failed', error: 'Not found' },
  ],
}

// File tree including a PDF for the "Add File" tests
const rootFileTreeWithPdf = {
  path: '',
  entries: [
    { name: 'src', kind: 'directory' },
    { name: 'context.pdf', kind: 'file' },
  ],
}

// ── Shared setup ─────────────────────────────────────────────────────────────

/** Default setup for Record tab tests: Sprint 0 (closed) with two approved issues */
async function setup(page: import('playwright/test').Page, overrides: Parameters<typeof setupRoutes>[1] = {}) {
  await setupRoutes(page, {
    milestones: [closedMilestone],
    milestoneIssues: { 2: [issue90, issue91] },
    issueStatuses: allApprovedBatch,
    fileTree: { '': rootFileTreeWithPdf, src: srcFileTree },
    ...overrides,
  })
}

async function goToRecord(page: import('playwright/test').Page) {
  await page.goto('/')
  await page.getByRole('button', { name: 'Record' }).click()
}

async function selectMilestone(page: import('playwright/test').Page, title: string) {
  await page.getByPlaceholder('Search milestones…').click()
  await page.getByRole('option', { name: new RegExp(title) }).click()
}

/** Wait for the milestone card's issue-loading text to disappear */
async function waitForCardLoaded(page: import('playwright/test').Page) {
  await expect(page.getByText(/issues? loading/)).not.toBeVisible({ timeout: 10_000 })
}

// ── Tests ────────────────────────────────────────────────────────────────────

test('record tab renders core elements', async ({ page }) => {
  await setup(page)
  await goToRecord(page)

  await expect(page.getByText('Milestones')).toBeVisible()
  await expect(page.getByPlaceholder('Search milestones…')).toBeVisible()
  await expect(page.getByText('Record Structure')).toBeVisible()
  await expect(page.getByRole('button', { name: 'Add File' })).toBeVisible()
  await expect(page.getByLabel('Output Path')).toBeVisible()
  await expect(page.getByRole('button', { name: 'Generate' })).toBeVisible()
})

test('only closed milestones shown by default', async ({ page }) => {
  await setup(page, { milestones: [closedMilestone, openMilestone] })
  await goToRecord(page)

  await page.getByPlaceholder('Search milestones…').click()
  await expect(page.getByRole('option', { name: /Sprint 0/ })).toBeVisible()
  await expect(page.getByRole('option', { name: /Sprint 1/ })).not.toBeVisible()
})

test('open milestone visible after toggling show open milestones', async ({ page }) => {
  await setup(page, { milestones: [closedMilestone, openMilestone] })
  await goToRecord(page)

  await page.getByLabel('Show open milestones').click()
  await page.getByPlaceholder('Search milestones…').click()
  await expect(page.getByRole('option', { name: /Sprint 1/ })).toBeVisible()
})

test('selecting all-approved milestone shows green card and fires preview', async ({ page }) => {
  await setup(page)
  await goToRecord(page)
  await selectMilestone(page, 'Sprint 0')
  await waitForCardLoaded(page)

  // No error or warning indicators on a fully-approved milestone
  await expect(page.getByTestId('status-error-count')).not.toBeVisible()
  await expect(page.getByTestId('unapproved-warning')).not.toBeVisible()

  // Preview iframe appears once the request fires
  await expect(page.locator('iframe[src*="preview.pdf"]')).toBeVisible({ timeout: 10_000 })
})

test('unapproved issues show yellow warning with count', async ({ page }) => {
  await setup(page, { issueStatuses: unapprovedBatch })
  await goToRecord(page)
  await selectMilestone(page, 'Sprint 0')
  await waitForCardLoaded(page)

  // Warning indicator with count "1"
  const warning = page.getByTestId('unapproved-warning')
  await expect(warning).toBeVisible()
  await expect(warning).toContainText('1')

  // Milestone is still included — preview fires
  await expect(page.locator('iframe[src*="preview.pdf"]')).toBeVisible({ timeout: 10_000 })
})

test('status fetch errors show red card and no preview fires', async ({ page }) => {
  await setup(page, { issueStatuses: allErrorBatch })
  await goToRecord(page)
  await selectMilestone(page, 'Sprint 0')
  await waitForCardLoaded(page)

  // Error indicator with count "2"
  const errorCount = page.getByTestId('status-error-count')
  await expect(errorCount).toBeVisible()
  await expect(errorCount).toContainText('2')

  // Preview iframe must NOT appear (milestone excluded)
  await expect(page.locator('iframe[src*="preview.pdf"]')).not.toBeVisible()

  // Error message shown instead
  await expect(page.getByText(/All selected milestones failed/)).toBeVisible()
})

test('open milestone shows unlock icon in card', async ({ page }) => {
  await setup(page, {
    milestones: [openMilestone],
    milestoneIssues: { 1: [{ ...issue90, milestone: 'Sprint 1' }] },
    issueStatuses: { results: [makeStatus({ ...issue90, milestone: 'Sprint 1' }, 'approved')], errors: [] },
  })
  await goToRecord(page)

  await page.getByLabel('Show open milestones').click()
  await selectMilestone(page, 'Sprint 1')
  await waitForCardLoaded(page)

  await expect(page.getByTestId('open-milestone-indicator')).toBeVisible()
})

test('no spinner in preview area while statuses are loading', async ({ page }) => {
  // When all milestones error, no preview request fires → no loading spinner
  await setup(page, { issueStatuses: allErrorBatch })
  await goToRecord(page)
  await selectMilestone(page, 'Sprint 0')
  await waitForCardLoaded(page)

  // The preview overlay/spinner must not be present
  await expect(page.getByText('Generating preview…')).not.toBeVisible()
})

test('output path auto-populates from repo and milestone name', async ({ page }) => {
  await setup(page)
  await goToRecord(page)
  await selectMilestone(page, 'Sprint 0')
  await waitForCardLoaded(page)

  // repo=test-repo, milestone title=Sprint 0 → spaces become hyphens
  await expect(page.getByLabel('Output Path')).toHaveValue('test-repo-Sprint-0.pdf')
})

test('output path appends -tables when tables only is toggled', async ({ page }) => {
  await setup(page)
  await goToRecord(page)
  await selectMilestone(page, 'Sprint 0')
  await waitForCardLoaded(page)

  await page.getByLabel('Tables only').click()

  await expect(page.getByLabel('Output Path')).toHaveValue('test-repo-Sprint-0-tables.pdf')
})

test('output path excludes errored milestones', async ({ page }) => {
  // Sprint 0 errors, Sprint 1 (open) is OK
  const sprint1Issue: Issue = { ...issue90, number: 92, milestone: 'Sprint 1' }
  await setupRoutes(page, {
    milestones: [closedMilestone, openMilestone],
    milestoneIssues: {
      2: [issue90, issue91],   // Sprint 0 — will error
      1: [sprint1Issue],       // Sprint 1 — all approved
    },
    issueStatuses: {
      results: [makeStatus(sprint1Issue, 'approved')],
      errors: [
        { issue_number: 90, kind: 'fetch_failed', error: 'Not found' },
        { issue_number: 91, kind: 'fetch_failed', error: 'Not found' },
      ],
    },
  })
  await goToRecord(page)

  // Select Sprint 0 (errors), then Sprint 1 (ok — needs open toggle)
  await selectMilestone(page, 'Sprint 0')
  await page.getByLabel('Show open milestones').click()
  await selectMilestone(page, 'Sprint 1')
  await waitForCardLoaded(page)

  // Sprint 0 is errored → excluded; output path contains only Sprint 1
  const outputPath = page.getByLabel('Output Path')
  await expect(outputPath).not.toHaveValue(/Sprint-0/)
  await expect(outputPath).toHaveValue(/Sprint-1/)
})

test('adding a context file via browse appears in the list', async ({ page }) => {
  await setup(page)
  await goToRecord(page)

  await page.getByRole('button', { name: 'Add File' }).click()
  await expect(page.getByRole('dialog')).toBeVisible()

  // Browse Server tab is active by default; only PDFs are shown
  await expect(page.getByText('context.pdf')).toBeVisible()
  await expect(page.getByText('main.rs')).not.toBeVisible()

  // Select the PDF
  await page.getByText('context.pdf').click()
  await page.getByRole('button', { name: 'Add Selected File' }).click()

  // Modal closes and file appears in context list
  await expect(page.getByRole('dialog')).not.toBeVisible()
  await expect(page.getByText('context.pdf')).toBeVisible()
})

test('removing a context file removes it from the list', async ({ page }) => {
  await setup(page)
  await goToRecord(page)

  // Add a file first
  await page.getByRole('button', { name: 'Add File' }).click()
  await page.getByText('context.pdf').click()
  await page.getByRole('button', { name: 'Add Selected File' }).click()

  // Remove it
  await page.getByRole('button', { name: 'Remove context.pdf' }).click()

  // File no longer in the context list
  await expect(page.getByRole('button', { name: 'Remove context.pdf' })).not.toBeVisible()
})

test('generate button fires and shows success', async ({ page }) => {
  await setup(page)
  await goToRecord(page)
  await selectMilestone(page, 'Sprint 0')
  await waitForCardLoaded(page)

  // Output path auto-filled; Generate button enabled
  await expect(page.getByRole('button', { name: 'Generate' })).toBeEnabled({ timeout: 10_000 })
  await page.getByRole('button', { name: 'Generate' }).click()

  await expect(page.getByText(/PDF written to/)).toBeVisible({ timeout: 10_000 })
})

test('generate button shows error on failure', async ({ page }) => {
  await setup(page, { recordGenerateSuccess: false })
  await goToRecord(page)
  await selectMilestone(page, 'Sprint 0')
  await waitForCardLoaded(page)

  await expect(page.getByRole('button', { name: 'Generate' })).toBeEnabled({ timeout: 10_000 })
  await page.getByRole('button', { name: 'Generate' }).click()

  await expect(page.getByText(/Generate failed/)).toBeVisible({ timeout: 10_000 })
})

test('generate button disabled without output path', async ({ page }) => {
  await setup(page)
  await goToRecord(page)
  await selectMilestone(page, 'Sprint 0')
  await waitForCardLoaded(page)

  // Clear the auto-filled output path
  await page.getByLabel('Output Path').fill('')

  await expect(page.getByRole('button', { name: 'Generate' })).toBeDisabled()
})

test('preview does not re-fire when only errored milestone changes', async ({ page }) => {
  // Track how many times /api/record/preview is called
  let previewCallCount = 0
  await setup(page)
  await page.route(/\/api\/record\/preview$/, (route) => {
    previewCallCount++
    route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ key: `key-${previewCallCount}` }) })
  })
  await goToRecord(page)

  // Select Sprint 0 → all approved → preview fires once
  await selectMilestone(page, 'Sprint 0')
  await waitForCardLoaded(page)
  await expect(page.locator('iframe[src*="preview.pdf"]')).toBeVisible({ timeout: 10_000 })
  expect(previewCallCount).toBe(1)

  // Now add a second milestone that errors — the included set doesn't change → no re-fire
  await setupRoutes(page, {
    milestones: [closedMilestone, openMilestone],
    milestoneIssues: { 2: [issue90, issue91], 1: [] },
    issueStatuses: allApprovedBatch,
  })
  // Route the status for the new milestone to error
  await page.route(/\/api\/milestones\/1\/issues/, (route) => {
    route.fulfill({ status: 500, contentType: 'application/json', body: JSON.stringify({ error: 'Failed' }) })
  })
  await page.getByLabel('Show open milestones').click()
  await selectMilestone(page, 'Sprint 1')
  await waitForCardLoaded(page)

  // Sprint 1 errored and is excluded — preview count must remain 1
  expect(previewCallCount).toBe(1)
})

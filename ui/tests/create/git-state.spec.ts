import { test, expect, type Page } from 'playwright/test'
import { setupRoutes } from '../helpers/routes'
import { defaultRepoInfo, openMilestone, mainRsIssue, libIssue } from '../fixtures/index'

// ---------------------------------------------------------------------------
// Helper: queue a single file into the Create tab
// ---------------------------------------------------------------------------
async function queueFile(page: Page, filename: string, skipExpand = false): Promise<void> {
  await page.getByText('Create New QC').click()
  const modal = page.getByRole('dialog', { name: 'Create QC Issue' })
  await expect(modal).toBeVisible()

  if (!skipExpand) {
    await modal.getByRole('treeitem', { name: 'src' }).click()
  }
  await modal.getByRole('treeitem', { name: filename }).click()

  // Select the Code Review checklist — this triggers onSelect + onChange → canQueue=true
  await modal.getByRole('tab', { name: 'Select a Checklist' }).click()
  await modal.getByRole('button', { name: 'Code Review' }).click()

  await modal.getByRole('button', { name: 'Queue' }).click()
  await expect(modal).not.toBeVisible()
}

// ---------------------------------------------------------------------------
// Test 1: Clean state — Create button enabled, no warning tooltip
// ---------------------------------------------------------------------------
test('clean git state — Create button enabled with no warning tooltip', async ({ page }) => {
  await setupRoutes(page, {
    repo: { ...defaultRepoInfo, git_status: 'clean', dirty_files: [] },
    issueStatuses: { results: [], errors: [] },
  })
  await page.goto('/')
  await page.getByRole('button', { name: 'Create' }).click()
  await page.getByRole('button', { name: 'New' }).click()

  await queueFile(page, 'main.rs')

  await page.getByPlaceholder('e.g. Sprint 4').fill('Test Milestone')

  const createButton = page.getByRole('button', { name: /Create \d+ QC/ })
  await expect(createButton).toBeEnabled()

  // Hover the Create button — no warning tooltip should appear
  await createButton.hover()
  await expect(page.getByText('Recommended to be in a clean git state before creating issues')).not.toBeVisible()
  await expect(page.getByText('Push to synchronize with remote before creating issues')).not.toBeVisible()
  await expect(page.getByText('Pull to synchronize with remote before creating issues')).not.toBeVisible()
  await expect(page.getByText('Resolve divergence before creating issues')).not.toBeVisible()

  // Hover the queued card — no dirty tooltip should appear
  await page.getByText('src/main.rs').first().hover()
  await expect(page.getByText('This file has uncommitted local changes')).not.toBeVisible()
})

// ---------------------------------------------------------------------------
// Test 2: Dirty file queued — button yellow + card yellow tooltip
// ---------------------------------------------------------------------------
test('dirty file queued — Create button and card show yellow warning', async ({ page }) => {
  await setupRoutes(page, {
    repo: { ...defaultRepoInfo, git_status: 'clean', dirty_files: ['src/main.rs'] },
    issueStatuses: { results: [], errors: [] },
  })
  await page.goto('/')
  await page.getByRole('button', { name: 'Create' }).click()
  await page.getByRole('button', { name: 'New' }).click()

  await queueFile(page, 'main.rs')

  await page.getByPlaceholder('e.g. Sprint 4').fill('Test Milestone')

  // Create button is enabled (dirty is a warning, not a block)
  await expect(page.getByRole('button', { name: /Create \d+ QC/ })).toBeEnabled()

  // Hover Create button — yellow dirty warning tooltip
  await page.getByRole('button', { name: /Create \d+ QC/ }).hover()
  await expect(page.getByText('Recommended to be in a clean git state before creating issues')).toBeVisible()

  // Hover the queued card — dirty card tooltip
  await page.getByText('src/main.rs').first().hover()
  await expect(page.getByText('This file has uncommitted local changes')).toBeVisible()
})

// ---------------------------------------------------------------------------
// Test 3: Ahead — Create button enabled with orange tooltip
// ---------------------------------------------------------------------------
test('ahead git state — Create button enabled with push warning tooltip', async ({ page }) => {
  await setupRoutes(page, {
    repo: { ...defaultRepoInfo, git_status: 'ahead', dirty_files: [] },
    issueStatuses: { results: [], errors: [] },
  })
  await page.goto('/')
  await page.getByRole('button', { name: 'Create' }).click()
  await page.getByRole('button', { name: 'New' }).click()

  await queueFile(page, 'main.rs')

  await page.getByPlaceholder('e.g. Sprint 4').fill('Test Milestone')

  await expect(page.getByRole('button', { name: /Create \d+ QC/ })).toBeEnabled()

  await page.getByRole('button', { name: /Create \d+ QC/ }).hover()
  await expect(page.getByText('Push to synchronize with remote before creating issues')).toBeVisible()
})

// ---------------------------------------------------------------------------
// Test 4: Behind — Create button enabled with orange tooltip
// ---------------------------------------------------------------------------
test('behind git state — Create button enabled with pull warning tooltip', async ({ page }) => {
  await setupRoutes(page, {
    repo: { ...defaultRepoInfo, git_status: 'behind', dirty_files: [] },
    issueStatuses: { results: [], errors: [] },
  })
  await page.goto('/')
  await page.getByRole('button', { name: 'Create' }).click()
  await page.getByRole('button', { name: 'New' }).click()

  await queueFile(page, 'main.rs')

  await page.getByPlaceholder('e.g. Sprint 4').fill('Test Milestone')

  await expect(page.getByRole('button', { name: /Create \d+ QC/ })).toBeEnabled()

  await page.getByRole('button', { name: /Create \d+ QC/ }).hover()
  await expect(page.getByText('Pull to synchronize with remote before creating issues')).toBeVisible()
})

// ---------------------------------------------------------------------------
// Test 5: Diverged — Create button enabled with red tooltip
// ---------------------------------------------------------------------------
test('diverged git state — Create button enabled with diverge error tooltip', async ({ page }) => {
  await setupRoutes(page, {
    repo: { ...defaultRepoInfo, git_status: 'diverged', dirty_files: [] },
    issueStatuses: { results: [], errors: [] },
  })
  await page.goto('/')
  await page.getByRole('button', { name: 'Create' }).click()
  await page.getByRole('button', { name: 'New' }).click()

  await queueFile(page, 'main.rs')

  await page.getByPlaceholder('e.g. Sprint 4').fill('Test Milestone')

  await expect(page.getByRole('button', { name: /Create \d+ QC/ })).toBeEnabled()

  await page.getByRole('button', { name: /Create \d+ QC/ }).hover()
  await expect(page.getByText('Resolve divergence before creating issues')).toBeVisible()
})

// ---------------------------------------------------------------------------
// Test 6: Conflict — red cards, Create button disabled
// ---------------------------------------------------------------------------
test('conflict — queued files matching milestone issues turn cards red and disable Create', async ({ page }) => {
  await setupRoutes(page, {
    repo: { ...defaultRepoInfo, git_status: 'clean', dirty_files: [] },
    milestones: [openMilestone],
    milestoneIssues: {
      1: [mainRsIssue, libIssue],
    },
    issueStatuses: { results: [], errors: [] },
  })
  await page.goto('/')
  await page.getByRole('button', { name: 'Create' }).click()

  // Phase A: queue both files in 'new' mode — milestoneNumber=null means no claims from milestone
  await page.getByRole('button', { name: 'New' }).click()
  await queueFile(page, 'main.rs')              // expand src
  await queueFile(page, 'lib.rs', true)         // src already expanded (keepMounted)

  // Phase B: switch to 'select' mode and select Sprint 1 (which has both files as existing issues)
  await page.getByRole('button', { name: 'Select' }).click()
  await page.getByPlaceholder('Select a milestone').click()
  await page.getByRole('option', { name: /Sprint 1/ }).click()

  // Create button is disabled due to conflicts
  await expect(page.getByRole('button', { name: /Create \d+ QC/ })).toBeDisabled()

  // Hover src/main.rs card — conflict tooltip
  await page.getByText('src/main.rs').first().hover()
  await expect(page.getByText('"src/main.rs" already has an issue in milestone "Sprint 1"')).toBeVisible()

  // Hover src/lib.rs card — conflict tooltip
  await page.getByText('src/lib.rs').first().hover()
  await expect(page.getByText('"src/lib.rs" already has an issue in milestone "Sprint 1"')).toBeVisible()
})

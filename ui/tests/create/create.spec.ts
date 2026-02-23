import { test, expect } from 'playwright/test'
import { setupRoutes } from '../helpers/routes'

// ---------------------------------------------------------------------------
// Test 1: AddFileCard disabled when no milestone selected in 'select' mode
// ---------------------------------------------------------------------------
test('AddFileCard disabled when no milestone selected in select mode', async ({ page }) => {
  await setupRoutes(page)
  await page.goto('/')
  await page.getByRole('button', { name: 'Create' }).click()

  // In 'select' mode with no milestone chosen, AddFileCard shows disabled message
  await expect(page.getByText('Select a milestone first')).toBeVisible()
  await expect(page.getByText('Create New QC')).toBeVisible()

  // Switching to 'New' mode enables AddFileCard immediately (no milestone required)
  await page.getByRole('button', { name: 'New' }).click()
  await expect(page.getByText('Select a milestone first')).not.toBeVisible()
})

// ---------------------------------------------------------------------------
// Test 2: Full create workflow
// ---------------------------------------------------------------------------
test('full create workflow', async ({ page }) => {
  await setupRoutes(page, {
    // issueStatuses not needed for the create tab
    issueStatuses: { results: [], errors: [] },
  })
  await page.goto('/')
  await page.getByRole('button', { name: 'Create' }).click()

  // ── Phase A: First QC — src/main.rs with custom checklist + rel files + reviewer ──

  // Switch to 'New' milestone mode so AddFileCard is enabled
  await page.getByRole('button', { name: 'New' }).click()
  await page.getByText('Create New QC').click()

  const modal = page.getByRole('dialog', { name: 'Create QC Issue' })
  await expect(modal).toBeVisible()

  // -- Select file: expand src → click main.rs
  await modal.getByRole('treeitem', { name: 'src' }).click()
  await modal.getByRole('treeitem', { name: 'main.rs' }).click()

  // -- Checklist: click "+ New", rename to "My Checklist", save
  await modal.getByRole('tab', { name: 'Select a Checklist' }).click()
  await modal.getByRole('button', { name: '+ New' }).click()
  await modal.getByLabel('Name').clear()
  await modal.getByLabel('Name').fill('My Checklist')
  await modal.locator('textarea').clear()
  await modal.locator('textarea').fill('- [ ] Step 1')
  await modal.getByRole('button', { name: 'Save' }).click()
  await expect(modal.getByRole('button', { name: 'My Checklist' })).toBeVisible()

  // -- Relevant files tab
  await modal.getByRole('tab', { name: 'Select Relevant Files' }).click()

  // Rel file 1: src/lib.rs — has existing issue #10, select it
  await modal.getByText('Add Relevant File').click()
  const picker1 = page.getByRole('dialog', { name: 'Add Relevant File' })
  await picker1.getByRole('treeitem', { name: 'src' }).click()
  await picker1.getByRole('treeitem', { name: /lib\.rs/ }).click()
  await picker1.getByLabel('Issue').click()
  await page.getByRole('option', { name: /#10/ }).click()
  await picker1.getByRole('button', { name: 'Add' }).click()
  await expect(picker1).not.toBeVisible()

  // Rel file 2: src/utils.rs — no issues (hasIssues=false), requires description
  await modal.getByText('Add Relevant File').click()
  const picker2 = page.getByRole('dialog', { name: 'Add Relevant File' })
  await picker2.getByRole('treeitem', { name: 'src' }).click()
  await picker2.getByRole('treeitem', { name: 'utils.rs' }).click()
  // Issue select is disabled, Add button is disabled until description filled
  await expect(picker2.getByLabel('Issue')).toBeDisabled()
  await expect(picker2.getByRole('button', { name: 'Add' })).toBeDisabled()
  await picker2.getByLabel('Description').fill('No upstream issue')
  await expect(picker2.getByRole('button', { name: 'Add' })).toBeEnabled()
  await picker2.getByRole('button', { name: 'Add' }).click()
  await expect(picker2).not.toBeVisible()

  // -- Reviewer
  await modal.getByRole('tab', { name: 'Select Reviewer(s)' }).click()
  await modal.getByPlaceholder('Search by login or name').fill('reviewer1')
  await page.getByRole('option', { name: /reviewer1/ }).click()

  // -- Queue the first item
  await expect(modal.getByRole('button', { name: 'Queue' })).toBeEnabled()
  await modal.getByRole('button', { name: 'Queue' }).click()
  await expect(modal).not.toBeVisible()
  await expect(page.getByText('src/main.rs').first()).toBeVisible()

  // ── Phase B: Second QC — src/lib.rs, reuse saved checklist, ref queued main.rs ──

  await page.getByText('Create New QC').click()
  await expect(modal).toBeVisible()

  // src is already expanded from Phase A (modal keepMounted, tree state persists)
  await modal.getByRole('treeitem', { name: /lib\.rs/ }).click()

  // Checklist: the saved "My Checklist" tab persists across modal open/close
  await modal.getByRole('tab', { name: 'Select a Checklist' }).click()
  await expect(modal.getByRole('button', { name: 'My Checklist' })).toBeVisible()
  await modal.getByRole('button', { name: 'My Checklist' }).click()

  // Relevant file: src/main.rs — it's queued, select the "Queued" option
  await modal.getByRole('tab', { name: 'Select Relevant Files' }).click()
  await modal.getByText('Add Relevant File').click()
  const picker3 = page.getByRole('dialog', { name: 'Add Relevant File' })
  await picker3.getByRole('treeitem', { name: 'src' }).click()
  await picker3.getByRole('treeitem', { name: /main\.rs/ }).click()
  await picker3.getByLabel('Issue').click()
  await page.getByRole('option', { name: 'Queued' }).click()
  await picker3.getByRole('button', { name: 'Add' }).click()
  await expect(picker3).not.toBeVisible()

  // Queue the second item
  await expect(modal.getByRole('button', { name: 'Queue' })).toBeEnabled()
  await modal.getByRole('button', { name: 'Queue' }).click()
  await expect(modal).not.toBeVisible()
  await expect(page.getByText('src/lib.rs').first()).toBeVisible()

  // ── Phase C: Batch relevant files — add src/external.rs (#11) to both QCs ──

  await page.getByRole('button', { name: 'Batch Relevant Files' }).click()
  const batchModal = page.getByRole('dialog', { name: 'Batch Apply Relevant Files' })
  await expect(batchModal).toBeVisible()

  // Select src/external.rs and its issue #11
  await batchModal.getByRole('treeitem', { name: 'src' }).click()
  await batchModal.getByRole('treeitem', { name: /external\.rs/ }).click()
  await batchModal.getByLabel('Issue').click()
  await page.getByRole('option', { name: /#11/ }).click()

  // Select all queued items and apply
  await batchModal.getByRole('checkbox', { name: /Select All/ }).click()
  await expect(batchModal.getByRole('button', { name: 'Apply' })).toBeEnabled()
  await batchModal.getByRole('button', { name: 'Apply' }).click()
  await expect(batchModal).not.toBeVisible()

  // Both QueuedIssueCards now list src/external.rs in their relevant files
  await expect(page.getByText('src/external.rs')).toHaveCount(2)

  // ── Phase D: Enter milestone name and create issues ──

  // Create button is disabled until a milestone name is entered
  await expect(page.getByRole('button', { name: /Create \d+ QC/ })).toBeDisabled()

  await page.getByPlaceholder('e.g. Sprint 4').fill('My Milestone')
  await expect(page.getByRole('button', { name: /Create \d+ QC/ })).toBeEnabled()

  await page.getByRole('button', { name: /Create \d+ QC/ }).click()

  // CreateResultModal shows links to the newly created issues
  const resultModal = page.getByRole('dialog', { name: /QC Issues Created/ })
  await expect(resultModal).toBeVisible()
  await expect(resultModal.getByRole('link', { name: 'src/main.rs' })).toBeVisible()
  await expect(resultModal.getByRole('link', { name: 'src/lib.rs' })).toBeVisible()
})

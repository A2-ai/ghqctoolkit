import { test, expect } from 'playwright/test'
import type { Page } from 'playwright/test'
import { setupRoutes } from '../helpers/routes'

/**
 * Tests for the checklist save semantics implemented in ChecklistTab.tsx:
 * - Tab edits auto-persist within a modal session (switching tabs preserves drafts)
 * - "Save" persists across modal opens via uiSession.savedCustomTabs
 * - Queue is NOT an implicit Save
 * - Editing a queued item shows a pinned "queued" tab with the original snapshot,
 *   and the same-named current/edited tab remains selectable below it.
 */

async function openCreateModal(page: Page) {
  await page.goto('/')
  await page.getByRole('button', { name: 'Create' }).click()
  await page.getByRole('button', { name: 'New' }).click()
  await page.getByText('Create New QC').click()
  const modal = page.getByRole('dialog', { name: 'Create QC Issue' })
  await expect(modal).toBeVisible()
  return modal
}

async function selectMainRsAndChecklistTab(modal: ReturnType<Page['getByRole']>) {
  await modal.getByRole('treeitem', { name: 'src' }).click()
  await modal.getByRole('treeitem', { name: 'main.rs' }).click()
  await modal.getByRole('tab', { name: 'Select a Checklist' }).click()
}

test('tab edits auto-persist when switching between checklist tabs within a modal', async ({ page }) => {
  await setupRoutes(page, { issueStatuses: { results: [], errors: [] } })

  const modal = await openCreateModal(page)
  await selectMainRsAndChecklistTab(modal)

  // Edit "Code Review" without saving
  await modal.getByRole('button', { name: 'Code Review' }).click()
  await modal.locator('textarea').fill('- [ ] Edited code-review (unsaved)')

  // Make a custom tab and put unsaved content on it
  await modal.getByRole('button', { name: '+ New' }).click()
  await modal.getByLabel('Name').fill('Scratch')
  await modal.locator('textarea').fill('- [ ] Scratch draft (unsaved)')

  // Switch back to Code Review — its unsaved edit must still be there
  await modal.getByRole('button', { name: 'Code Review' }).click()
  await expect(modal.locator('textarea')).toHaveValue('- [ ] Edited code-review (unsaved)')

  // And switching back to Scratch — its unsaved edit must still be there
  await modal.getByRole('button', { name: 'Scratch' }).click()
  await expect(modal.locator('textarea')).toHaveValue('- [ ] Scratch draft (unsaved)')
})

test('Save persists across modal opens; unsaved edits do not', async ({ page }) => {
  await setupRoutes(page, { issueStatuses: { results: [], errors: [] } })

  const modal = await openCreateModal(page)
  await selectMainRsAndChecklistTab(modal)

  // Edit Code Review and Save it
  await modal.getByRole('button', { name: 'Code Review' }).click()
  await modal.locator('textarea').fill('- [ ] Saved revision')
  await modal.getByRole('button', { name: 'Save' }).click()

  // Edit again WITHOUT saving, then close modal
  await modal.locator('textarea').fill('- [ ] Unsaved revision')
  await page.keyboard.press('Escape')
  await expect(modal).not.toBeVisible()

  // Re-open the modal — the saved revision is what loads, not the unsaved one
  await page.getByText('Create New QC').click()
  await expect(modal).toBeVisible()
  await modal.getByRole('treeitem', { name: 'main.rs' }).click()
  await modal.getByRole('tab', { name: 'Select a Checklist' }).click()
  await modal.getByRole('button', { name: 'Code Review' }).click()
  await expect(modal.locator('textarea')).toHaveValue('- [ ] Saved revision')
})

test('queueing is not an implicit save: next modal open shows last saved content', async ({ page }) => {
  await setupRoutes(page, { issueStatuses: { results: [], errors: [] } })

  const modal = await openCreateModal(page)
  await selectMainRsAndChecklistTab(modal)

  // Pick Code Review, edit content, do NOT save, then queue
  await modal.getByRole('button', { name: 'Code Review' }).click()
  const original = await modal.locator('textarea').inputValue()
  await modal.locator('textarea').fill('- [ ] Queued without save')
  await modal.getByRole('button', { name: 'Queue' }).click()
  await expect(modal).not.toBeVisible()

  // Open a fresh modal — Code Review should be back to its original content
  await page.getByText('Create New QC').click()
  await expect(modal).toBeVisible()
  await modal.getByRole('treeitem', { name: 'lib.rs' }).click()
  await modal.getByRole('tab', { name: 'Select a Checklist' }).click()
  await modal.getByRole('button', { name: 'Code Review' }).click()
  await expect(modal.locator('textarea')).toHaveValue(original)
})

test('editing a queued item shows pinned queued snapshot alongside current checklist', async ({ page }) => {
  await setupRoutes(page, { issueStatuses: { results: [], errors: [] } })

  const modal = await openCreateModal(page)
  await selectMainRsAndChecklistTab(modal)

  // Queue file_a (main.rs) with the original Code Review checklist
  await modal.getByRole('button', { name: 'Code Review' }).click()
  const originalCodeReview = await modal.locator('textarea').inputValue()
  await modal.getByRole('button', { name: 'Queue' }).click()
  await expect(modal).not.toBeVisible()

  // Queue file_b (lib.rs) with an edited+saved Code Review
  await page.getByText('Create New QC').click()
  await expect(modal).toBeVisible()
  await modal.getByRole('treeitem', { name: 'lib.rs' }).click()
  await modal.getByRole('tab', { name: 'Select a Checklist' }).click()
  await modal.getByRole('button', { name: 'Code Review' }).click()
  await modal.locator('textarea').fill('- [ ] Edited and saved Code Review')
  await modal.getByRole('button', { name: 'Save' }).click()
  await modal.getByRole('button', { name: 'Queue' }).click()
  await expect(modal).not.toBeVisible()

  // Re-open file_a's queued card (click the card to edit)
  await page.getByText('src/main.rs').first().click()
  await expect(modal).toBeVisible()
  await modal.getByRole('tab', { name: 'Select a Checklist' }).click()

  // The pinned "queued" snapshot tab is active and shows the ORIGINAL content,
  // not the post-Save edited content.
  const queuedTab = modal.getByRole('button', { name: 'Code Review queued' })
  const currentTab = modal.getByRole('button', { name: 'Code Review', exact: true })
  await expect(queuedTab).toBeVisible()
  await expect(currentTab).toBeVisible()
  await expect(modal.locator('textarea')).toHaveValue(originalCodeReview)

  // Switching to the same-named current checklist tab shows the latest saved/edited content.
  await currentTab.click()
  await expect(modal.locator('textarea')).toHaveValue('- [ ] Edited and saved Code Review')

  // Switching back to the queued snapshot tab restores the original content.
  await queuedTab.click()
  await expect(modal.locator('textarea')).toHaveValue(originalCodeReview)
})

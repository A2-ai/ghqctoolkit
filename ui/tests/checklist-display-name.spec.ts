/**
 * Tests that checklist_display_name is used throughout the UI, and that
 * both "checklist" and "checklists" as the configured value produce the
 * same displayed text (idempotent singularization/pluralization).
 */
import { test, expect } from 'playwright/test'
import { setupRoutes } from './helpers/routes'

// Run each scenario twice: once with the singular form, once with the plural form.
// Both must produce identical visible text.
const variants = ['checklist', 'checklists'] as const

// ---------------------------------------------------------------------------
// Configuration tab — section title is plural, option labels are singular
// ---------------------------------------------------------------------------
for (const displayName of variants) {
  test(`config tab: section title and labels with display_name="${displayName}"`, async ({ page }) => {
    await setupRoutes(page, { checklistDisplayName: displayName })
    await page.goto('/')
    await page.getByRole('button', { name: 'Configuration' }).click()

    // Section title should be the plural, capitalized: "Checklists"
    await expect(page.getByText('Checklists', { exact: true })).toBeVisible()

    // Option labels should be singular, capitalized: "Checklist directory", "Checklist note" (if shown)
    await expect(page.getByText('Checklist directory')).toBeVisible()
  })
}

// ---------------------------------------------------------------------------
// Create tab — modal tab name is singular
// ---------------------------------------------------------------------------
for (const displayName of variants) {
  test(`create modal: checklist tab label with display_name="${displayName}"`, async ({ page }) => {
    await setupRoutes(page, {
      checklistDisplayName: displayName,
      issueStatuses: { results: [], errors: [] },
    })
    await page.goto('/')
    await page.getByRole('button', { name: 'Create' }).click()
    await page.getByRole('button', { name: 'New' }).click()
    await page.getByText('Create New QC').click()

    const modal = page.getByRole('dialog', { name: 'Create QC Issue' })
    await expect(modal).toBeVisible()

    // Tab should read "Select a Checklist"
    await expect(modal.getByRole('tab', { name: 'Select a Checklist' })).toBeVisible()
  })
}

// ---------------------------------------------------------------------------
// Create modal checklist tab — placeholder text uses singular
// ---------------------------------------------------------------------------
for (const displayName of variants) {
  test(`checklist tab: placeholder text with display_name="${displayName}"`, async ({ page }) => {
    await setupRoutes(page, {
      checklistDisplayName: displayName,
      issueStatuses: { results: [], errors: [] },
    })
    await page.goto('/')
    await page.getByRole('button', { name: 'Create' }).click()
    await page.getByRole('button', { name: 'New' }).click()
    await page.getByText('Create New QC').click()

    const modal = page.getByRole('dialog', { name: 'Create QC Issue' })
    await modal.getByRole('tab', { name: 'Select a Checklist' }).click()

    // Placeholder: "Select a checklist from the list, or click + New to create one."
    await expect(modal.getByText('Select a checklist from the list, or click + New to create one.')).toBeVisible()
  })
}

import { test, expect } from 'playwright/test'
import { readFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import type { Page } from 'playwright/test'
import { setupRoutes } from '../helpers/routes'

const FIXTURE_DIR = resolve(dirname(fileURLToPath(import.meta.url)), '../fixtures/files')
const DOCX_BYTES = readFileSync(resolve(FIXTURE_DIR, 'multi-page.docx'))
const XLSX_BYTES = readFileSync(resolve(FIXTURE_DIR, 'multi-sheet.xlsx'))

const DOCX_MIME = 'application/vnd.openxmlformats-officedocument.wordprocessingml.document'
const XLSX_MIME = 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet'

const fileTree = {
  '': { path: '', entries: [{ name: 'src', kind: 'directory' as const }] },
  src: {
    path: 'src',
    entries: [
      { name: 'multi-page.docx', kind: 'file' as const },
      { name: 'multi-sheet.xlsx', kind: 'file' as const },
    ],
  },
}

async function serveBinaryFixtures(page: Page): Promise<void> {
  // Registered after setupRoutes so it takes precedence over the default raw handler.
  await page.route(/\/api\/files\/raw/, (route, request) => {
    const url = new URL(request.url())
    const path = url.searchParams.get('path') ?? ''
    if (path.endsWith('multi-page.docx')) {
      route.fulfill({ status: 200, contentType: DOCX_MIME, body: DOCX_BYTES })
    } else if (path.endsWith('multi-sheet.xlsx')) {
      route.fulfill({ status: 200, contentType: XLSX_MIME, body: XLSX_BYTES })
    } else {
      route.fulfill({ status: 404, body: 'not found' })
    }
  })
}

async function openCreateModalAndPreview(page: Page, fileName: string): Promise<void> {
  await page.goto('/')
  await page.getByRole('button', { name: 'Create' }).click()
  await page.getByRole('button', { name: 'New' }).click()
  await page.getByText('Create New QC').click()
  const createModal = page.getByRole('dialog', { name: 'Create QC Issue' })
  await expect(createModal).toBeVisible()
  await createModal.getByRole('treeitem', { name: 'src' }).click()
  await createModal.getByRole('treeitem', { name: fileName }).click()
  await createModal.getByRole('button', { name: 'View File' }).click()
}

// ---------------------------------------------------------------------------
// xlsx — multi-sheet rendering
// ---------------------------------------------------------------------------
test('xlsx preview renders all sheets as tabs and switches content', async ({ page }) => {
  await setupRoutes(page, { fileTree, issueStatuses: { results: [], errors: [] } })
  await serveBinaryFixtures(page)
  await openCreateModalAndPreview(page, 'multi-sheet.xlsx')

  const modal = page.getByRole('dialog', { name: 'multi-sheet.xlsx' })

  // Both sheet tabs render.
  const alphaTab = modal.getByRole('tab', { name: 'alpha' })
  const betaTab = modal.getByRole('tab', { name: 'beta' })
  await expect(alphaTab).toBeVisible()
  await expect(betaTab).toBeVisible()

  // First sheet (alpha) is active by default and shows its A1 value.
  await expect(alphaTab).toHaveAttribute('data-active', /.*/)
  const sheetContent = modal.locator('.xlsx-sheet')
  await expect(sheetContent).toContainText('alpha')

  // Switching to beta swaps the rendered table content.
  await betaTab.click()
  await expect(modal.locator('.xlsx-sheet')).toContainText('beta')
  await expect(betaTab).toHaveAttribute('data-active', /.*/)
})

// ---------------------------------------------------------------------------
// docx — multi-page rendering and zoom controls
// ---------------------------------------------------------------------------
test('docx preview renders multiple pages including landscape last page', async ({ page }) => {
  await setupRoutes(page, { fileTree, issueStatuses: { results: [], errors: [] } })
  await serveBinaryFixtures(page)
  await openCreateModalAndPreview(page, 'multi-page.docx')

  const modal = page.getByRole('dialog', { name: 'multi-page.docx' })
  const pages = modal.locator('.docx-preview-host section.docx')

  // Wait for the loader to finish and pages to render.
  await expect(pages).toHaveCount(3, { timeout: 15_000 })

  // Last page is landscape — its rendered width should exceed its height.
  const lastPageBox = await pages.last().boundingBox()
  expect(lastPageBox).not.toBeNull()
  expect(lastPageBox!.width).toBeGreaterThan(lastPageBox!.height)

  // Earlier pages should be portrait (taller than wide).
  const firstPageBox = await pages.first().boundingBox()
  expect(firstPageBox).not.toBeNull()
  expect(firstPageBox!.height).toBeGreaterThan(firstPageBox!.width)
})

test('docx preview zoom controls update the displayed percentage', async ({ page }) => {
  await setupRoutes(page, { fileTree, issueStatuses: { results: [], errors: [] } })
  await serveBinaryFixtures(page)
  await openCreateModalAndPreview(page, 'multi-page.docx')

  const modal = page.getByRole('dialog', { name: 'multi-page.docx' })
  await expect(modal.locator('.docx-preview-host section.docx').first()).toBeVisible({ timeout: 15_000 })

  const percentLabel = modal.locator('text=/^\\d+%$/').first()
  const initial = await percentLabel.textContent()
  expect(initial).toMatch(/^\d+%$/)
  const initialPct = Number(initial!.replace('%', ''))

  // Reset to 100%
  await modal.getByRole('button', { name: 'Reset to 100%' }).click()
  await expect(percentLabel).toHaveText('100%')

  // Zoom in: +10%
  await modal.getByRole('button', { name: 'Zoom in' }).click()
  await expect(percentLabel).toHaveText('110%')

  // Zoom out twice: 90%
  await modal.getByRole('button', { name: 'Zoom out' }).click()
  await modal.getByRole('button', { name: 'Zoom out' }).click()
  await expect(percentLabel).toHaveText('90%')

  // Fit to width returns to the auto-computed scale.
  await modal.getByRole('button', { name: 'Fit to width' }).click()
  await expect(percentLabel).toHaveText(`${initialPct}%`)
})

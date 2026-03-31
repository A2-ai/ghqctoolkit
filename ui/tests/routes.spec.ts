import { test, expect } from 'playwright/test'
import { setupRoutes } from './helpers/routes'

test('root redirects to /status', async ({ page }) => {
  await setupRoutes(page)
  await page.goto('/')
  await expect(page).toHaveURL(/\/status$/)
  await expect(page).not.toHaveURL(/\/@fs\//)
})

test('direct route loads record screen', async ({ page }) => {
  await setupRoutes(page)
  await page.goto('/record')
  await expect(page.locator('main').getByText('Milestones', { exact: true })).toBeVisible()
  await expect(page).toHaveURL(/\/record$/)
})

test('direct route loads archive screen', async ({ page }) => {
  await setupRoutes(page)
  await page.goto('/archive')
  await expect(page.locator('main').getByText('Milestones', { exact: true })).toBeVisible()
  await expect(page.locator('main').getByRole('switch', { name: 'Flatten directory structure' })).toBeVisible()
  await expect(page).toHaveURL(/\/archive$/)
})

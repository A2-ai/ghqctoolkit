import { test, expect } from 'playwright/test'
import { setupRoutes } from '../helpers/routes'
import {
  openMilestone,
  approvedModalIssue,
  approvedModalStatus,
  inProgressModalIssue,
  inProgressModalStatus,
  approvedChildBlocked,
  approvedChildIssue,
  approvedChildStatus,
  notApprovedChildBlocked,
  notApprovedChildIssue,
  grandchildBlocked,
  grandchildIssue,
} from '../fixtures/index'

// ---------------------------------------------------------------------------
// Helper: open the modal for a given issue (does NOT navigate to unapprove tab)
// ---------------------------------------------------------------------------
async function openModal(
  page: import('playwright/test').Page,
  issue: typeof approvedModalIssue,
  status: typeof approvedModalStatus,
  routeOverrides: Parameters<typeof setupRoutes>[1] = {},
) {
  await setupRoutes(page, {
    milestones: [openMilestone],
    milestoneIssues: { 1: [issue] },
    issueStatuses: { results: [status], errors: [] },
    ...routeOverrides,
  })
  await page.goto('/')
  await page.getByPlaceholder('Search milestones…').click()
  await page.getByRole('option', { name: /Sprint 1/ }).click()
  await page.getByTestId(`issue-card-${issue.number}`).click()
  await expect(page.getByRole('tablist')).toBeVisible()
}

// ---------------------------------------------------------------------------
// Tab state: disabled only when blocked unavailable AND not approved
// ---------------------------------------------------------------------------

test('unapprove tab: disabled for non-approved issue when /blocked returns 501', async ({ page }) => {
  await openModal(page, inProgressModalIssue, inProgressModalStatus, {
    blockedResponse: 501,
  })
  await expect(page.getByRole('tab', { name: 'Unapprove', exact: true })).toBeDisabled()
})

test('unapprove tab: enabled for non-approved issue when /blocked is available', async ({ page }) => {
  await openModal(page, inProgressModalIssue, inProgressModalStatus, {
    blockedResponse: [],
  })
  await expect(page.getByRole('tab', { name: 'Unapprove', exact: true })).not.toBeDisabled()
})

test('unapprove tab: enabled for approved issue even when /blocked returns 501', async ({ page }) => {
  await openModal(page, approvedModalIssue, approvedModalStatus, {
    blockedResponse: 501,
  })
  await expect(page.getByRole('tab', { name: 'Unapprove', exact: true })).not.toBeDisabled()
})

// ---------------------------------------------------------------------------
// Swim lane layout
// ---------------------------------------------------------------------------

test('approved root appears in To Unapprove lane with reason input', async ({ page }) => {
  await openModal(page, approvedModalIssue, approvedModalStatus)
  const panel = page.getByRole('tabpanel', { name: 'Unapprove' })

  await expect(panel.getByText('To Unapprove')).toBeVisible()
  await expect(panel.locator('[data-testid="to-unapprove-lane"]').getByText(approvedModalIssue.title)).toBeVisible()
  await expect(panel.getByPlaceholder('Reason (required)')).toBeVisible()
})

test('not-approved root shows Nothing to unapprove and root in Not Approved lane', async ({ page }) => {
  await openModal(page, inProgressModalIssue, inProgressModalStatus)
  await page.getByRole('tab', { name: 'Unapprove', exact: true }).click()
  const panel = page.getByRole('tabpanel', { name: 'Unapprove' })

  await expect(panel.getByText('Nothing to unapprove')).toBeVisible()
  await expect(panel.locator('[data-testid="not-approved-lane"]').getByText(inProgressModalIssue.title)).toBeVisible()
  await expect(panel.getByPlaceholder('Reason (required)')).not.toBeAttached()
})

test('approved child appears in Impacted Approvals lane', async ({ page }) => {
  await openModal(page, approvedModalIssue, approvedModalStatus, {
    blockedResponseByIssue: { [approvedModalIssue.number]: [approvedChildBlocked] },
  })
  const panel = page.getByRole('tabpanel', { name: 'Unapprove' })

  await expect(panel.getByText('Impacted Approvals')).toBeVisible()
  await expect(panel.getByText(approvedChildIssue.title)).toBeVisible()
  // Impacted card has expand button
  await expect(panel.getByRole('button', { name: 'Expand children', exact: true })).toBeVisible()
})

test('not-approved child appears in Not Approved lane', async ({ page }) => {
  await openModal(page, approvedModalIssue, approvedModalStatus, {
    blockedResponseByIssue: { [approvedModalIssue.number]: [notApprovedChildBlocked] },
  })
  const panel = page.getByRole('tabpanel', { name: 'Unapprove' })

  await expect(panel.getByText('Not Approved')).toBeVisible()
  await expect(panel.getByText(notApprovedChildIssue.title)).toBeVisible()
  // Not Approved cards have no expand/collapse buttons
  await expect(panel.getByRole('button', { name: 'Expand children' })).not.toBeAttached()
})

// ---------------------------------------------------------------------------
// Swim lane interactions
// ---------------------------------------------------------------------------

test('Unapprove button disabled until reason filled in To Unapprove card', async ({ page }) => {
  await openModal(page, approvedModalIssue, approvedModalStatus)
  const panel = page.getByRole('tabpanel', { name: 'Unapprove' })

  await expect(panel.getByRole('button', { name: 'Unapprove' })).toBeDisabled()
  await panel.getByPlaceholder('Reason (required)').fill('Regression found')
  await expect(panel.getByRole('button', { name: 'Unapprove' })).not.toBeDisabled()
})

test('post unapprove shows result modal with issue link', async ({ page }) => {
  await openModal(page, approvedModalIssue, approvedModalStatus)
  const panel = page.getByRole('tabpanel', { name: 'Unapprove' })

  await panel.getByPlaceholder('Reason (required)').fill('Regression found')
  await panel.getByRole('button', { name: 'Unapprove' }).click()

  await expect(page.getByRole('heading', { name: 'Unapproved' })).toBeVisible()
  await expect(page.getByLabel('Unapproved').getByRole('link', { name: approvedModalIssue.title })).toBeVisible()
})

test('expand children button loads grandchildren into lanes', async ({ page }) => {
  await openModal(page, approvedModalIssue, approvedModalStatus, {
    blockedResponseByIssue: {
      [approvedModalIssue.number]: [approvedChildBlocked],
      [approvedChildIssue.number]: [grandchildBlocked],
    },
  })
  const panel = page.getByRole('tabpanel', { name: 'Unapprove' })

  await expect(panel.getByText(approvedChildIssue.title)).toBeVisible()
  await panel.getByRole('button', { name: 'Expand children', exact: true }).click()
  await expect(panel.getByText(grandchildIssue.title)).toBeVisible()
})

// ---------------------------------------------------------------------------
// Fallback mode (/blocked returns 501)
// ---------------------------------------------------------------------------

test('fallback: shows simplified form instead of swim lanes', async ({ page }) => {
  await openModal(page, approvedModalIssue, approvedModalStatus, {
    blockedResponse: 501,
  })
  const panel = page.getByRole('tabpanel', { name: 'Unapprove' })

  await expect(panel.getByText(/Impact analysis is unavailable/)).toBeVisible()
  // Swim lane headers should not be present
  await expect(panel.getByText('Impacted Approvals')).not.toBeAttached()
  await expect(panel.getByRole('textbox', { name: 'Reason (required)' })).toBeVisible()
})

test('fallback: Unapprove disabled until reason filled', async ({ page }) => {
  await openModal(page, approvedModalIssue, approvedModalStatus, {
    blockedResponse: 501,
  })
  const panel = page.getByRole('tabpanel', { name: 'Unapprove' })

  await expect(panel.getByRole('button', { name: 'Unapprove' })).toBeDisabled()
  await panel.getByRole('textbox', { name: 'Reason (required)' }).fill('Need to revert')
  await expect(panel.getByRole('button', { name: 'Unapprove' })).not.toBeDisabled()
})

test('fallback: Preview button opens preview modal', async ({ page }) => {
  await openModal(page, approvedModalIssue, approvedModalStatus, {
    blockedResponse: 501,
  })
  const panel = page.getByRole('tabpanel', { name: 'Unapprove' })

  await panel.getByRole('button', { name: 'Preview' }).click()
  await expect(page.getByTitle('Unapprove Preview')).toBeVisible()
})

test('fallback: post unapprove shows result modal', async ({ page }) => {
  await openModal(page, approvedModalIssue, approvedModalStatus, {
    blockedResponse: 501,
  })
  const panel = page.getByRole('tabpanel', { name: 'Unapprove' })

  await panel.getByRole('textbox', { name: 'Reason (required)' }).fill('Regression found')
  await panel.getByRole('button', { name: 'Unapprove' }).click()

  await expect(page.getByRole('heading', { name: 'Unapproved' })).toBeVisible()
  await expect(page.getByLabel('Unapproved').getByRole('link', { name: approvedModalIssue.title })).toBeVisible()
})

// ---------------------------------------------------------------------------
// Cache update: unapproval with opened:true marks issue open in milestone cache
// ---------------------------------------------------------------------------

// After unapproval when the backend reports opened:true, the issue's state is
// patched to 'open' in the React Query milestone cache. This means the card
// stays visible in the swimlane even after "Include closed issues" is toggled
// off — proving the cache was updated rather than the issue relying on the
// include-closed filter.
test('after unapproval with opened:true, previously-closed issue stays visible when include-closed is toggled off', async ({ page }) => {
  // approvedChildIssue has state:'closed' — not visible by default
  await setupRoutes(page, {
    milestones: [openMilestone],
    milestoneIssues: { 1: [approvedChildIssue] },
    issueStatuses: { results: [approvedChildStatus], errors: [] },
    postUnapproveResponse: {
      unapproval_url: 'https://github.com/test-owner/test-repo/issues/80#issuecomment-55555',
      opened: true,
    },
  })
  await page.goto('/')
  await page.getByPlaceholder('Search milestones…').click()
  await page.getByRole('option', { name: /Sprint 1/ }).click()

  // Closed issue is hidden until toggle is enabled
  await expect(page.getByTestId(`issue-card-${approvedChildIssue.number}`)).not.toBeVisible()
  await page.getByRole('switch', { name: 'Include closed issues' }).click()
  await expect(page.getByTestId(`issue-card-${approvedChildIssue.number}`)).toBeVisible()

  // Open the modal (defaults to Unapprove tab since issue is approved)
  await page.getByTestId(`issue-card-${approvedChildIssue.number}`).click()
  await expect(page.getByRole('tablist')).toBeVisible()

  const panel = page.getByRole('tabpanel', { name: 'Unapprove' })
  await panel.getByPlaceholder('Reason (required)').fill('Reverting approval')
  await panel.getByRole('button', { name: 'Unapprove' }).click()

  await expect(page.getByRole('heading', { name: 'Unapproved' })).toBeVisible()
  // The outer issue detail modal uses withCloseButton=false, so the only
  // .mantine-Modal-close in the DOM is the inner result modal's Mantine button.
  await page.locator('.mantine-Modal-close').click()
  await page.getByRole('button', { name: 'Close' }).click()

  // Toggle include-closed OFF — issue should still be visible because state was set to 'open'
  await page.getByRole('switch', { name: 'Include closed issues' }).click()
  await expect(page.getByTestId(`issue-card-${approvedChildIssue.number}`)).toBeVisible()
})

import { test, expect } from 'playwright/test'
import { setupRoutes } from '../helpers/routes'
import {
  openMilestone,
  singleCommitIssue,
  singleCommitStatus,
  multiCommitIssue,
  multiCommitStatus,
  notifOnNonFileIssue,
  notifOnNonFileStatus,
  dirtyModalIssue,
  dirtyModalStatus,
  cleanModalStatus,
} from '../fixtures/index'

// ---------------------------------------------------------------------------
// Helper: navigate to Status tab with a single issue loaded, return card locator
// ---------------------------------------------------------------------------
async function setupAndOpenModal(
  page: import('playwright/test').Page,
  issue: typeof singleCommitIssue,
  status: typeof singleCommitStatus,
) {
  await setupRoutes(page, {
    milestones: [openMilestone],
    milestoneIssues: { 1: [issue] },
    issueStatuses: { results: [status], errors: [] },
    // Use the issue number in the comment URL so it's predictable
    postCommentResponse: { comment_url: `https://github.com/test-owner/test-repo/issues/${issue.number}#issuecomment-99999` },
  })
  await page.goto('/')
  await page.getByPlaceholder('Search milestones…').click()
  await page.getByRole('option', { name: /Sprint 1/ }).click()
  await page.getByTestId(`issue-card-${issue.number}`).click()
  // Wait for the tab list to be visible — confirms modal opened
  await expect(page.getByRole('tablist')).toBeVisible()
}

// ---------------------------------------------------------------------------
// 1. Modal opens when a swimlane card is clicked
// ---------------------------------------------------------------------------
test('modal opens when card is clicked', async ({ page }) => {
  await setupAndOpenModal(page, singleCommitIssue, singleCommitStatus)
  await expect(page.getByRole('tab', { name: 'Notify' })).toBeVisible()
})

// ---------------------------------------------------------------------------
// 2. Modal has 4 tabs; Notify is active by default
// ---------------------------------------------------------------------------
test('modal has 4 tabs with Notify active by default', async ({ page }) => {
  await setupAndOpenModal(page, singleCommitIssue, singleCommitStatus)

  const tablist = page.getByRole('tablist')
  await expect(tablist.getByRole('tab', { name: 'Notify', exact: true })).toBeVisible()
  await expect(tablist.getByRole('tab', { name: 'Review', exact: true })).toBeVisible()
  await expect(tablist.getByRole('tab', { name: 'Approve', exact: true })).toBeVisible()
  await expect(tablist.getByRole('tab', { name: 'Unapprove', exact: true })).toBeVisible()

  // Notify panel content is visible; other panels are not rendered yet
  await expect(page.getByText('Commit Range')).toBeVisible()
})

// ---------------------------------------------------------------------------
// 3. X button closes the modal
// ---------------------------------------------------------------------------
test('X button closes the modal', async ({ page }) => {
  await setupAndOpenModal(page, singleCommitIssue, singleCommitStatus)
  await expect(page.getByRole('tablist')).toBeVisible()

  await page.getByRole('button', { name: 'Close' }).click()
  await expect(page.getByRole('tablist')).not.toBeVisible()
})

// ---------------------------------------------------------------------------
// 4. Notify tab status card shows branch, assignees, and status pill
// ---------------------------------------------------------------------------
test('status card shows branch, assignees, and status pill', async ({ page }) => {
  await setupAndOpenModal(page, singleCommitIssue, singleCommitStatus)

  // Branch
  await expect(page.getByText(/Branch:.*feature-branch/).first()).toBeVisible()
  // Assignees
  await expect(page.getByText(/Reviewers:.*alice/).first()).toBeVisible()
  // Status pill (status = 'awaiting_review' → displayed as 'awaiting review')
  await expect(page.getByText('awaiting review', { exact: false }).first()).toBeVisible()
})

// ---------------------------------------------------------------------------
// 5. Review / Approve / Unapprove tabs show "Coming soon"
// ---------------------------------------------------------------------------
test('non-Notify tabs show Coming soon', async ({ page }) => {
  await setupAndOpenModal(page, singleCommitIssue, singleCommitStatus)

  for (const tab of ['Review', 'Approve', 'Unapprove']) {
    await page.getByRole('tab', { name: tab, exact: true }).click()
    // Scope to this tab's panel to avoid strict-mode violation across all rendered panels
    await expect(page.getByLabel(tab, { exact: true }).getByText('Coming soon')).toBeVisible()
  }
})

// ---------------------------------------------------------------------------
// 6. Single commit: slider renders without crashing; From and To show same hash
// ---------------------------------------------------------------------------
test('single commit: slider renders and From/To show the same hash', async ({ page }) => {
  await setupAndOpenModal(page, singleCommitIssue, singleCommitStatus)

  // aaaaaaa is the first 7 chars of the single commit hash
  const hashLocator = page.getByText('aaaaaaa')
  // Should appear at least twice: once in slider mark, once each in From/To blocks
  await expect(hashLocator.first()).toBeVisible()

  // From and To should both reference the same commit
  const fromText = page.locator('text=From:').locator('..')
  const toText = page.locator('text=To:').locator('..')
  await expect(fromText.getByText('aaaaaaa')).toBeVisible()
  await expect(toText.getByText('aaaaaaa')).toBeVisible()
})

// ---------------------------------------------------------------------------
// 7. Multi-commit: default FROM is the last notification commit (bbbbbbb),
//    default TO is the last file-changed commit after FROM (ddddddd)
// ---------------------------------------------------------------------------
test('multi-commit: default From/To are set correctly', async ({ page }) => {
  await setupAndOpenModal(page, multiCommitIssue, multiCommitStatus)

  // From: bbbbbbb (notification commit), To: ddddddd (latest file-changed after FROM)
  await expect(page.locator('text=From:').locator('..').getByText('bbbbbbb')).toBeVisible()
  await expect(page.locator('text=To:').locator('..').getByText('ddddddd')).toBeVisible()
})

// ---------------------------------------------------------------------------
// 8. Show all commits toggle reveals the hidden commit (ccccccc)
// ---------------------------------------------------------------------------
test('show all commits toggle reveals hidden commits', async ({ page }) => {
  await setupAndOpenModal(page, multiCommitIssue, multiCommitStatus)

  // ccccccc is hidden initially (no file change, no statuses, not the exception index)
  await expect(page.getByText('ccccccc')).not.toBeVisible()

  await page.getByRole('checkbox', { name: 'Show all commits' }).click()

  await expect(page.getByText('ccccccc')).toBeVisible()
})

// ---------------------------------------------------------------------------
// 9. Include diff checkbox is shown when file is changed in the From→To range
// ---------------------------------------------------------------------------
test('include diff shown when file changed in range', async ({ page }) => {
  // multi-commit: FROM=bbbbbbb TO=ddddddd; ddddddd is file_changed=true → include diff visible
  await setupAndOpenModal(page, multiCommitIssue, multiCommitStatus)
  await expect(page.getByRole('checkbox', { name: 'Include diff' })).toBeVisible()
})

// ---------------------------------------------------------------------------
// 10. Include diff NOT shown when no file change between From and To
//     (notification on non-file-changing commit: FROM == TO, no change in range)
// ---------------------------------------------------------------------------
test('include diff hidden when no file change in range', async ({ page }) => {
  await setupAndOpenModal(page, notifOnNonFileIssue, notifOnNonFileStatus)
  // FROM == TO == bbbbbbb; no file change in that empty range
  // Checkbox is always rendered but disabled when no file change in range
  await expect(page.getByRole('checkbox', { name: 'Include diff' })).toBeDisabled()
})

// ---------------------------------------------------------------------------
// 11. Notification on non-file-changing commit: both From and To default to
//     that commit (bbbbbbb), which is the last notification
// ---------------------------------------------------------------------------
test('notification on non-file-changing commit: From and To both default to that commit', async ({ page }) => {
  await setupAndOpenModal(page, notifOnNonFileIssue, notifOnNonFileStatus)

  await expect(page.locator('text=From:').locator('..').getByText('bbbbbbb')).toBeVisible()
  await expect(page.locator('text=To:').locator('..').getByText('bbbbbbb')).toBeVisible()
})

// ---------------------------------------------------------------------------
// 12. Preview button opens the preview modal
// ---------------------------------------------------------------------------
test('preview button opens preview modal', async ({ page }) => {
  await setupAndOpenModal(page, multiCommitIssue, multiCommitStatus)

  await page.getByRole('button', { name: 'Preview' }).click()
  await expect(page.getByTitle('Comment Preview')).toBeVisible()
})

// ---------------------------------------------------------------------------
// 13. Post comment: success modal appears with a GitHub link
// ---------------------------------------------------------------------------
test('post comment shows success modal with GitHub link', async ({ page }) => {
  await setupAndOpenModal(page, multiCommitIssue, multiCommitStatus)

  await page.getByRole('button', { name: 'Post' }).click()

  await expect(page.getByRole('heading', { name: 'Comment Posted' })).toBeVisible()
  await expect(page.getByRole('link', { name: 'View on GitHub' })).toBeVisible()
})

// ---------------------------------------------------------------------------
// 14. Dirty indicator shown in modal status card when issue is dirty
// ---------------------------------------------------------------------------
test('dirty asterisk shown in modal when issue is dirty', async ({ page }) => {
  await setupAndOpenModal(page, dirtyModalIssue, dirtyModalStatus)

  const notifyPanel = page.getByRole('tabpanel', { name: 'Notify' })
  await notifyPanel.getByTestId('dirty-indicator').hover()
  await expect(page.getByText('This file has uncommitted local changes')).toBeVisible()
})

// ---------------------------------------------------------------------------
// 15. Dirty indicator not present in modal when issue is clean
// ---------------------------------------------------------------------------
test('dirty asterisk not shown in modal when issue is clean', async ({ page }) => {
  await setupAndOpenModal(page, dirtyModalIssue, cleanModalStatus)

  const notifyPanel = page.getByRole('tabpanel', { name: 'Notify' })
  await expect(notifyPanel.getByTestId('dirty-indicator')).not.toBeAttached()
})

// ---------------------------------------------------------------------------
// 16. Post comment failure: error modal shown with error message
// ---------------------------------------------------------------------------
test('post comment failure shows error modal', async ({ page }) => {
  await setupRoutes(page, {
    milestones: [openMilestone],
    milestoneIssues: { 1: [multiCommitIssue] },
    issueStatuses: { results: [multiCommitStatus], errors: [] },
    postCommentResponse: null,  // triggers 500
  })
  await page.goto('/')
  await page.getByPlaceholder('Search milestones…').click()
  await page.getByRole('option', { name: /Sprint 1/ }).click()
  await page.getByTestId(`issue-card-${multiCommitIssue.number}`).click()
  await expect(page.getByRole('tablist')).toBeVisible()

  await page.getByRole('button', { name: 'Post' }).click()

  await expect(page.getByText('Post Failed')).toBeVisible()
})

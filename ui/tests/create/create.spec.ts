import { test, expect } from 'playwright/test'
import { setupRoutes } from '../helpers/routes'
import { createdMilestone, openMilestone } from '../fixtures'

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

test('collaborators default from file authors and are sent in create request', async ({ page }) => {
  const requests: Array<{ collaborators?: string[] }> = []

  await setupRoutes(page, {
    issueStatuses: { results: [], errors: [] },
    fileCollaborators: {
      'src/main.rs': ['Jane Doe <jane@example.com>'],
    },
  })

  await page.route(/\/api\/milestones\/(\d+)\/issues/, async (route, request) => {
    if (request.method() !== 'POST') {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify([]),
      })
      return
    }

    requests.push(...((await request.postDataJSON()) as Array<{ collaborators?: string[] }>))
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify([{
        issue_url: 'https://github.com/test-owner/test-repo/issues/301',
        issue_number: 301,
        blocking_created: [],
        blocking_errors: [],
      }]),
    })
  })

  await page.goto('/create')
  await page.getByRole('button', { name: 'New' }).click()
  await page.getByText('Create New QC').click()

  const modal = page.getByRole('dialog', { name: 'Create QC Issue' })
  await modal.getByRole('treeitem', { name: 'src' }).click()
  await modal.getByRole('treeitem', { name: 'main.rs' }).click()
  await modal.getByRole('tab', { name: 'Select a Checklist' }).click()
  await modal.getByRole('button', { name: 'Code Review' }).click()
  await modal.getByRole('tab', { name: 'Select Collaborators' }).click()
  await expect(modal.getByText('Author: test-user')).toBeVisible()
  await expect(modal.getByRole('button', { name: 'Remove Jane Doe <jane@example.com>' })).toBeVisible()
  await modal.getByLabel('Add collaborator').fill('John Smith <john@example.com>')
  await modal.getByRole('button', { name: 'Add' }).click()
  await modal.getByRole('button', { name: 'Queue' }).click()

  await page.getByPlaceholder('e.g. Sprint 4').fill('Milestone X')
  await page.getByRole('button', { name: 'Create 1 QC Issue' }).click()

  await expect(page.getByRole('dialog', { name: '1 QC Issue Created' })).toBeVisible()
  expect(requests).toHaveLength(1)
  expect(requests[0]?.collaborators).toEqual([
    'Jane Doe <jane@example.com>',
    'John Smith <john@example.com>',
  ])
})

test('pdf files open in embedded preview during issue creation', async ({ page }) => {
  await setupRoutes(page, {
    issueStatuses: { results: [], errors: [] },
    fileTree: {
      '': {
        path: '',
        entries: [
          { name: 'docs', kind: 'directory' },
        ],
      },
      docs: {
        path: 'docs',
        entries: [
          { name: 'report.pdf', kind: 'file' },
        ],
      },
    },
  })

  await page.goto('/create')
  await page.getByRole('button', { name: 'New' }).click()
  await page.getByText('Create New QC').click()

  const modal = page.getByRole('dialog', { name: 'Create QC Issue' })
  await modal.getByRole('treeitem', { name: 'docs' }).click()
  await modal.getByRole('treeitem', { name: 'report.pdf' }).click()
  await modal.getByRole('button', { name: 'View File' }).click()

  const preview = page.getByRole('dialog', { name: 'report.pdf' })
  await expect(preview).toBeVisible()
  const iframe = preview.getByTitle('PDF Preview')
  await expect(iframe).toBeVisible()
  await expect(iframe).toHaveAttribute('src', /\/api\/files\/raw\?path=docs%2Freport\.pdf/)
})

test('unsupported files show a file-type message during issue creation', async ({ page }) => {
  await setupRoutes(page, {
    issueStatuses: { results: [], errors: [] },
    fileTree: {
      '': {
        path: '',
        entries: [
          { name: 'docs', kind: 'directory' },
        ],
      },
      docs: {
        path: 'docs',
        entries: [
          { name: 'plan.docx', kind: 'file' },
        ],
      },
    },
  })

  await page.goto('/create')
  await page.getByRole('button', { name: 'New' }).click()
  await page.getByText('Create New QC').click()

  const modal = page.getByRole('dialog', { name: 'Create QC Issue' })
  await modal.getByRole('treeitem', { name: 'docs' }).click()
  await modal.getByRole('treeitem', { name: 'plan.docx' }).click()
  await modal.getByRole('button', { name: 'View File' }).click()

  const preview = page.getByRole('dialog', { name: 'plan.docx' })
  await expect(preview).toBeVisible()
  await expect(preview.getByText('Preview is not available for .docx files.')).toBeVisible()
})

test('missing files show a not found message during issue creation', async ({ page }) => {
  await setupRoutes(page, {
    issueStatuses: { results: [], errors: [] },
  })

  await page.route(/\/api\/files\/content/, async (route, request) => {
    const url = new URL(request.url())
    if (url.searchParams.get('path') !== 'src/main.rs') {
      await route.continue()
      return
    }
    await route.fulfill({
      status: 404,
      contentType: 'application/json',
      body: JSON.stringify({ error: 'File not found: src/main.rs' }),
    })
  })

  await page.goto('/create')
  await page.getByRole('button', { name: 'New' }).click()
  await page.getByText('Create New QC').click()

  const modal = page.getByRole('dialog', { name: 'Create QC Issue' })
  await modal.getByRole('treeitem', { name: 'src' }).click()
  await modal.getByRole('treeitem', { name: 'main.rs' }).click()
  await modal.getByRole('button', { name: 'View File' }).click()

  const preview = page.getByRole('dialog', { name: 'main.rs' })
  await expect(preview).toBeVisible()
  await expect(preview.getByText('Error: File not found: src/main.rs')).toBeVisible()
})

test('collaborators are hidden and sent empty when disabled in configuration', async ({ page }) => {
  const requests: Array<{ collaborators?: string[] }> = []

  await setupRoutes(page, {
    issueStatuses: { results: [], errors: [] },
    includeCollaborators: false,
    fileCollaborators: {
      'src/main.rs': ['Jane Doe <jane@example.com>'],
    },
  })

  await page.route(/\/api\/milestones\/(\d+)\/issues/, async (route, request) => {
    if (request.method() !== 'POST') {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify([]),
      })
      return
    }

    requests.push(...((await request.postDataJSON()) as Array<{ collaborators?: string[] }>))
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify([{
        issue_url: 'https://github.com/test-owner/test-repo/issues/302',
        issue_number: 302,
        blocking_created: [],
        blocking_errors: [],
      }]),
    })
  })

  await page.goto('/create')
  await page.getByRole('button', { name: 'New' }).click()
  await page.getByText('Create New QC').click()

  const modal = page.getByRole('dialog', { name: 'Create QC Issue' })
  await modal.getByRole('treeitem', { name: 'src' }).click()
  await modal.getByRole('treeitem', { name: 'main.rs' }).click()
  await modal.getByRole('tab', { name: 'Select a Checklist' }).click()
  await modal.getByRole('button', { name: 'Code Review' }).click()

  await expect(modal.getByRole('tab', { name: 'Select Collaborators' })).toHaveCount(0)
  await expect(modal.getByText(/^Collaborators:/)).toHaveCount(0)

  await modal.getByRole('button', { name: 'Queue' }).click()
  await page.getByPlaceholder('e.g. Sprint 4').fill('Milestone Y')
  await page.getByRole('button', { name: 'Create 1 QC Issue' }).click()

  await expect(page.getByRole('dialog', { name: '1 QC Issue Created' })).toBeVisible()
  expect(requests).toHaveLength(1)
  expect(requests[0]?.collaborators).toEqual([])
})

test('new milestone name stays non-duplicate until success modal closes', async ({ page }) => {
  await setupRoutes(page, {
    issueStatuses: { results: [], errors: [] },
    milestones: [openMilestone],
    milestonesAfterCreate: [openMilestone, createdMilestone],
    createIssuesDelayMs: 750,
    createIssues: [{
      issue_url: 'https://github.com/test-owner/test-repo/issues/201',
      issue_number: 201,
      blocking_created: [],
      blocking_errors: [],
    }],
  })
  await page.goto('/')
  await page.getByRole('button', { name: 'Create' }).click()
  await page.getByRole('button', { name: 'New' }).click()

  const milestoneName = page.getByLabel('Name').first()
  await milestoneName.fill(createdMilestone.title)

  await page.getByText('Create New QC').click()
  const modal = page.getByRole('dialog', { name: 'Create QC Issue' })
  await expect(modal).toBeVisible()
  await modal.getByRole('treeitem', { name: 'src' }).click()
  await modal.getByRole('treeitem', { name: 'main.rs' }).click()
  await modal.getByRole('tab', { name: 'Select a Checklist' }).click()
  await modal.getByRole('button', { name: 'Code Review' }).click()
  await modal.getByRole('button', { name: 'Queue' }).click()
  await expect(modal).not.toBeVisible()

  const refreshedMilestones = page.waitForResponse((response) =>
    response.url().endsWith('/api/milestones') && response.request().method() === 'GET',
  )
  await page.getByRole('button', { name: 'Create 1 QC Issue' }).click()
  await refreshedMilestones

  await expect(page.getByText('Name already exists')).not.toBeVisible()

  const resultModal = page.getByRole('dialog', { name: '1 QC Issue Created' })
  await expect(resultModal).toBeVisible()
  await expect(page.getByText('Name already exists')).not.toBeVisible()

  await resultModal.getByRole('button', { name: 'Done' }).click()
  await expect(resultModal).not.toBeVisible()
  await expect(page.getByPlaceholder('e.g. Sprint 4')).not.toBeVisible()
  await expect(page.getByPlaceholder('Select a milestone')).toBeVisible()
  await expect(page.getByPlaceholder('Select a milestone')).toHaveValue(createdMilestone.title)
  await expect(page.getByRole('button', { name: /Create \d+ QC Issue/ })).toBeDisabled()
})

test('create tab preserves queued items and saved checklists across tab switches until refresh', async ({ page }) => {
  await setupRoutes(page, {
    issueStatuses: { results: [], errors: [] },
  })
  await page.goto('/')
  await page.getByRole('button', { name: 'Create' }).click()
  await page.getByRole('button', { name: 'New' }).click()
  await page.getByText('Create New QC').click()

  const modal = page.getByRole('dialog', { name: 'Create QC Issue' })
  await expect(modal).toBeVisible()

  await modal.getByRole('treeitem', { name: 'src' }).click()
  await modal.getByRole('treeitem', { name: 'main.rs' }).click()

  await modal.getByRole('tab', { name: 'Select a Checklist' }).click()
  await modal.getByRole('button', { name: '+ New' }).click()
  await modal.getByLabel('Name').fill('Persistent Checklist')
  await modal.locator('textarea').fill('- [ ] Persists across tabs')
  await modal.getByRole('button', { name: 'Save' }).click()
  await modal.getByRole('button', { name: 'Queue' }).click()

  await expect(modal).not.toBeVisible()
  await expect(page.getByText('src/main.rs').first()).toBeVisible()

  await page.getByRole('button', { name: 'Configuration' }).click()
  await expect(page.getByText('Create New QC')).not.toBeVisible()

  await page.getByRole('button', { name: 'Create' }).click()
  await expect(page.getByText('src/main.rs').first()).toBeVisible()

  await page.getByText('Create New QC').click()
  await expect(modal).toBeVisible()
  await modal.getByRole('tab', { name: 'Select a Checklist' }).click()
  await expect(modal.getByRole('button', { name: 'Persistent Checklist' })).toBeVisible()
  await modal.getByRole('button', { name: 'Persistent Checklist' }).click()
  await expect(modal.getByLabel('Name')).toHaveValue('Persistent Checklist')
  await expect(modal.locator('textarea')).toHaveValue('- [ ] Persists across tabs')
  await page.keyboard.press('Escape')
  await expect(modal).not.toBeVisible()

  await page.reload()
  await page.getByRole('button', { name: 'Create', exact: true }).click()
  await expect(page.getByText('src/main.rs').first()).not.toBeVisible()

  await page.getByRole('button', { name: 'New' }).click()
  await page.getByText('Create New QC').click()
  await expect(modal).toBeVisible()
  await modal.getByRole('tab', { name: 'Select a Checklist' }).click()
  await expect(modal.getByRole('button', { name: 'Persistent Checklist' })).not.toBeVisible()
})

test('create issue modal closes on route change', async ({ page }) => {
  await setupRoutes(page, {
    issueStatuses: { results: [], errors: [] },
  })
  await page.goto('/create')

  await page.getByRole('button', { name: 'New' }).click()
  await page.getByText('Create New QC').click()
  await expect(page.getByRole('dialog', { name: 'Create QC Issue' })).toBeVisible()

  await page.getByRole('button', { name: 'Configuration' }).click({ force: true })
  await page.getByRole('button', { name: 'Create', exact: true }).click({ force: true })

  await expect(page.getByRole('dialog', { name: 'Create QC Issue' })).not.toBeVisible()
  await expect(page.getByText('Create New QC')).toBeVisible()
})

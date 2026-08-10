import { test, expect } from 'playwright/test'
import { setupRoutes } from '../helpers/routes'
import type { Checklist } from '../../src/api/checklists'
import {
  makeConfigGitRepo,
  configRepoClean,
  configRepoBehind,
  configRepoAhead,
  configRepoDiverged,
} from '../fixtures/index'

// ---------------------------------------------------------------------------
// Fixture data
// ---------------------------------------------------------------------------

const defaultOptions = {
  prepended_checklist_note: null,
  checklist_display_name: 'Code Review',
  include_collaborators: true,
  logo_path: 'logo.png',
  logo_found: false,
  checklist_directory: 'checklists/',
  record_path: 'records/',
  ui_repo_refresh_rate_seconds: 15,
}

const twoChecklists: Checklist[] = [
  { name: 'Code Review', content: '- [ ] Review logic\n- [ ] Check tests' },
  { name: 'Custom', content: '' },
]

const notConfigured = {
  directory: '/mock/config',
  exists: false,
  git_repository: null,
  options: defaultOptions,
  checklists: [],
  config_repo_env: null,
}

const configured = {
  directory: '/mock/config',
  exists: true,
  git_repository: configRepoClean,
  options: defaultOptions,
  checklists: twoChecklists,
  config_repo_env: null,
}

const localDirectoryConfig = {
  directory: '/mock/config',
  exists: true,
  git_repository: null,
  options: defaultOptions,
  checklists: twoChecklists,
  config_repo_env: null,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/** Register a custom /api/configuration mock that overrides setupRoutes' default. */
async function mockConfiguration(
  page: import('playwright/test').Page,
  getResponse: object,
  postResponse?: object,
) {
  await page.route('/api/configuration', (route, request) => {
    if (request.method() === 'POST') {
      route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify(postResponse ?? getResponse),
      })
    } else {
      route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify(getResponse),
      })
    }
  })
}

async function goToConfiguration(page: import('playwright/test').Page) {
  await page.goto('/')
  await page.getByRole('button', { name: 'Configuration' }).click()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// 1. Warning tooltip when repo is not configured
// ---------------------------------------------------------------------------
test('configuration tab shows warning tooltip when repo not configured', async ({ page }) => {
  await setupRoutes(page)
  await mockConfiguration(page, notConfigured)

  await page.goto('/')

  const configTab = page.getByRole('button', { name: 'Configuration' })
  await expect(configTab).toBeVisible()

  // The tab should render the warning badge (IconInfoCircle makes it yellow)
  await configTab.hover()
  await expect(page.getByText('Configuration repository is not set up')).toBeVisible()
})

// ---------------------------------------------------------------------------
// 2. No warning tooltip when already configured
// ---------------------------------------------------------------------------
test('configuration tab has no warning when already configured', async ({ page }) => {
  await setupRoutes(page)
  await mockConfiguration(page, configured)

  await page.goto('/')

  // The Configuration tab should exist but not have the warning badge
  await expect(page.getByRole('button', { name: 'Configuration' })).toBeVisible()
  await expect(page.getByText('Configuration repository is not set up')).not.toBeVisible()
})

// ---------------------------------------------------------------------------
// 3. Not configured — setup form visible, Set Up disabled without a URL
// ---------------------------------------------------------------------------
test('setup form: Set Up button disabled when URL input is empty', async ({ page }) => {
  await setupRoutes(page)
  await mockConfiguration(page, notConfigured)

  await goToConfiguration(page)

  await expect(page.getByLabel('Git URL')).toBeVisible()
  await expect(page.getByRole('button', { name: 'Set Up' })).toBeDisabled()
})

// ---------------------------------------------------------------------------
// 4. Existing config directory without git metadata renders read-only status
// ---------------------------------------------------------------------------
test('existing configuration directory disables setup and reports non-git state', async ({ page }) => {
  await setupRoutes(page)
  await mockConfiguration(page, localDirectoryConfig)

  await goToConfiguration(page)

  await expect(page.getByText('Not a git repository')).toBeVisible()
  await expect(page.getByText('/mock/config')).toBeVisible()
  await expect(page.getByLabel('Git URL')).not.toBeVisible()
  await expect(page.getByRole('button', { name: 'Set Up' })).not.toBeVisible()
})

// ---------------------------------------------------------------------------
// 5. Not configured — typing a URL enables Set Up; clicking Set Up transitions
//    to configured state
// ---------------------------------------------------------------------------
test('typing a URL enables Set Up and clicking it renders configured state', async ({ page }) => {
  await setupRoutes(page)
  await mockConfiguration(page, notConfigured, configured)

  await goToConfiguration(page)

  const input = page.getByLabel('Git URL')
  const setupBtn = page.getByRole('button', { name: 'Set Up' })

  await expect(setupBtn).toBeDisabled()
  await input.fill('https://github.com/myorg/config-repo')
  await expect(setupBtn).toBeEnabled()

  await setupBtn.click()

  // After successful POST, the configured state is rendered
  await expect(page.getByText('myorg / config-repo')).toBeVisible()
  await expect(page.getByText('clean')).toBeVisible()
  // Setup form is gone
  await expect(input).not.toBeVisible()
})

// ---------------------------------------------------------------------------
// 6. GHQC_CONFIG_REPO set — URL pre-filled, input disabled, hint shown,
//    Set Up enabled without typing
// ---------------------------------------------------------------------------
test('GHQC_CONFIG_REPO pre-fills and disables the URL input', async ({ page }) => {
  const envUrl = 'https://github.com/myorg/config-repo'
  const withEnv = { ...notConfigured, config_repo_env: envUrl }

  await setupRoutes(page)
  await mockConfiguration(page, withEnv, configured)

  await goToConfiguration(page)

  const input = page.getByLabel('Git URL')
  await expect(input).toHaveValue(envUrl)
  await expect(input).toBeDisabled()
  await expect(page.getByText('Set by GHQC_CONFIG_REPO')).toBeVisible()

  // Set Up should be enabled because the env URL is present
  await expect(page.getByRole('button', { name: 'Set Up' })).toBeEnabled()
})

// ---------------------------------------------------------------------------
// 7. GHQC_CONFIG_REPO set — clicking Set Up transitions to configured state
// ---------------------------------------------------------------------------
test('Set Up with GHQC_CONFIG_REPO transitions to configured state', async ({ page }) => {
  const envUrl = 'https://github.com/myorg/config-repo'
  const withEnv = { ...notConfigured, config_repo_env: envUrl }

  await setupRoutes(page)
  await mockConfiguration(page, withEnv, configured)

  await goToConfiguration(page)

  await page.getByRole('button', { name: 'Set Up' }).click()

  await expect(page.getByText('myorg / config-repo')).toBeVisible()
  await expect(page.getByText('Set by GHQC_CONFIG_REPO')).not.toBeVisible()
})

// ---------------------------------------------------------------------------
// 8. Setup POST failure — error message shown below the form
// ---------------------------------------------------------------------------
test('setup POST failure shows error message', async ({ page }) => {
  await setupRoutes(page)

  // GET returns not-configured; POST returns 500
  await page.route('/api/configuration', (route, request) => {
    if (request.method() === 'POST') {
      route.fulfill({ status: 500, contentType: 'application/json', body: '{}' })
    } else {
      route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify(notConfigured),
      })
    }
  })

  await goToConfiguration(page)

  await page.getByLabel('Git URL').fill('https://github.com/myorg/config-repo')
  await page.getByRole('button', { name: 'Set Up' }).click()

  await expect(page.getByText('Setup failed: 500')).toBeVisible()
  // Form remains visible after error
  await expect(page.getByLabel('Git URL')).toBeVisible()
})

// ---------------------------------------------------------------------------
// 9. Already configured — shows owner/repo, status badge, no setup form
// ---------------------------------------------------------------------------
test('already configured renders repo info and status badge', async ({ page }) => {
  await setupRoutes(page)
  await mockConfiguration(page, configured)

  await goToConfiguration(page)

  await expect(page.getByText('myorg / config-repo')).toBeVisible()
  await expect(page.getByText('clean')).toBeVisible()
  await expect(page.getByLabel('Git URL')).not.toBeVisible()
})

// ---------------------------------------------------------------------------
// 10. Already configured — dirty_files listed when present
// ---------------------------------------------------------------------------
test('dirty files shown when config repo has uncommitted changes', async ({ page }) => {
  const dirty = {
    ...configured,
    git_repository: makeConfigGitRepo({
      status: 'ahead',
      status_detail: 'Repository is ahead by 1 commit',
      ahead_commits: ['ddd4444'],
      dirty_files: ['checklists/review.yaml'],
    }),
  }

  await setupRoutes(page)
  await mockConfiguration(page, dirty)

  await goToConfiguration(page)

  await expect(page.getByText('ahead', { exact: true })).toBeVisible()
  await expect(page.getByText(/Dirty:.*checklists\/review\.yaml/)).toBeVisible()
})

// ---------------------------------------------------------------------------
// 11. Checklists section: Custom is filtered out; selecting a checklist
//     shows its content
// ---------------------------------------------------------------------------
test('checklists section filters Custom and shows content on selection', async ({ page }) => {
  await setupRoutes(page)
  await mockConfiguration(page, configured)

  await goToConfiguration(page)

  // "Custom" should not appear as a selectable checklist button
  expect(await page.getByRole('button', { name: 'Custom' }).count()).toBe(0)

  // "Code Review" should be visible and its content shown in the textarea
  await expect(page.getByRole('button', { name: 'Code Review', exact: true })).toBeVisible()
  await expect(page.getByRole('textbox').filter({ hasText: '- [ ] Review logic' })).toBeVisible()
})

// ---------------------------------------------------------------------------
// 12. Options section renders configuration values
// ---------------------------------------------------------------------------
test('options section renders display name, collaborator setting, paths, refresh rate, and logo status', async ({ page }) => {
  await setupRoutes(page)
  await mockConfiguration(page, configured)

  await goToConfiguration(page)

  await expect(page.getByText('Display name')).toBeVisible()
  // Value from options.checklist_display_name
  await expect(page.getByText('Code Review').first()).toBeVisible()
  await expect(page.getByText('Include collaborators')).toBeVisible()
  await expect(page.getByText('Yes')).toBeVisible()
  await expect(page.getByText('checklists/')).toBeVisible()
  await expect(page.getByText('records/')).toBeVisible()
  await expect(page.getByText('UI repo refresh rate')).toBeVisible()
  await expect(page.getByText('15s')).toBeVisible()
  // logo_found=false → ✗
  await expect(page.getByText('✗')).toBeVisible()
})

// ---------------------------------------------------------------------------
// 13. Tab warning when the configuration repository is behind its remote
// ---------------------------------------------------------------------------
test('configuration tab warns when the config repo is behind its remote', async ({ page }) => {
  await setupRoutes(page, { configGitRepository: configRepoBehind })

  await page.goto('/')

  const configTab = page.getByRole('button', { name: 'Configuration' })
  await configTab.hover()
  await expect(
    page.getByText('Configuration repository is 3 commits behind myorg/config-repo'),
  ).toBeVisible()
})

// ---------------------------------------------------------------------------
// 14. Tab warning when the configuration repository has diverged
// ---------------------------------------------------------------------------
test('configuration tab warns when the config repo has diverged', async ({ page }) => {
  await setupRoutes(page, { configGitRepository: configRepoDiverged })

  await page.goto('/')

  await page.getByRole('button', { name: 'Configuration' }).hover()
  await expect(
    page.getByText('Configuration repository has diverged from its remote (2 ahead, 3 behind)'),
  ).toBeVisible()
})

// ---------------------------------------------------------------------------
// 15. No tab warning for clean or ahead — intentional local work is not nagged
// ---------------------------------------------------------------------------
for (const [label, gitRepo] of [
  ['clean', configRepoClean],
  ['ahead', configRepoAhead],
] as const) {
  test(`configuration tab has no warning when the config repo is ${label}`, async ({ page }) => {
    await setupRoutes(page, { configGitRepository: gitRepo })

    await page.goto('/')

    const configTab = page.getByRole('button', { name: 'Configuration' })
    await expect(configTab).toBeVisible()
    await configTab.hover()
    await expect(page.getByText(/Configuration repository (is \d+ commit|has diverged)/)).not.toBeVisible()
  })
}

// ---------------------------------------------------------------------------
// 16. Always-visible strip shows status without expanding any section
// ---------------------------------------------------------------------------
test('git status strip is visible without expanding a section', async ({ page }) => {
  await setupRoutes(page, { configGitRepository: configRepoBehind })

  await goToConfiguration(page)

  await expect(page.getByText('myorg / config-repo')).toBeVisible()
  await expect(page.getByText('behind', { exact: true })).toBeVisible()
  await expect(page.getByText('Repository is behind by 3 commits')).toBeVisible()
  await expect(page.getByText('/mock/config')).toBeVisible()
})

// ---------------------------------------------------------------------------
// 17. Update button is enabled only when the repo is behind
// ---------------------------------------------------------------------------
test('Update button is enabled only when the config repo is behind', async ({ page }) => {
  await setupRoutes(page, { configGitRepository: configRepoBehind })
  await goToConfiguration(page)
  await expect(page.getByRole('button', { name: 'Update' })).toBeEnabled()
})

for (const [label, gitRepo] of [
  ['clean', configRepoClean],
  ['ahead', configRepoAhead],
  ['diverged', configRepoDiverged],
  ['dirty', makeConfigGitRepo({ status: 'behind', behind_commits: ['aaa1111'], dirty_files: ['checklists/review.yaml'] })],
] as const) {
  test(`Update button is disabled when the config repo is ${label}`, async ({ page }) => {
    await setupRoutes(page, { configGitRepository: gitRepo })
    await goToConfiguration(page)
    await expect(page.getByRole('button', { name: 'Update' })).toBeDisabled()
  })
}

// ---------------------------------------------------------------------------
// 18. Disabled Update button explains itself via a tooltip on its wrapper
// ---------------------------------------------------------------------------
test('disabled Update button explains why it cannot run', async ({ page }) => {
  await setupRoutes(page, { configGitRepository: configRepoDiverged })

  await goToConfiguration(page)

  await page.getByRole('button', { name: 'Update' }).hover()
  await expect(
    page.getByText('The configuration repository has diverged from its remote. Resolve it manually with git.'),
  ).toBeVisible()
})

// ---------------------------------------------------------------------------
// 19. A successful update refreshes the displayed status
// ---------------------------------------------------------------------------
test('successful update refreshes the displayed status', async ({ page }) => {
  await setupRoutes(page, {
    configGitRepository: configRepoBehind,
    configUpdateResult: configRepoClean,
  })

  await goToConfiguration(page)

  await expect(page.getByText('Repository is behind by 3 commits')).toBeVisible()
  await page.getByRole('button', { name: 'Update' }).click()

  await expect(page.getByText('Configuration repository updated')).toBeVisible()
  await expect(page.getByText('clean', { exact: true })).toBeVisible()
  await expect(page.getByText('Repository is behind by 3 commits')).not.toBeVisible()
  await expect(page.getByRole('button', { name: 'Update' })).toBeDisabled()

  // The tab warning clears once the repo is up to date
  await page.getByRole('button', { name: 'Configuration' }).hover()
  await expect(page.getByText(/Configuration repository is \d+ commit/)).not.toBeVisible()
})

// ---------------------------------------------------------------------------
// 20. A 409 refusal renders the backend's message verbatim
// ---------------------------------------------------------------------------
test('update refusal renders the 409 error message verbatim', async ({ page }) => {
  const reason = 'Cannot update: the configuration repository has uncommitted changes in checklists/review.yaml'

  await setupRoutes(page, {
    configGitRepository: configRepoBehind,
    configUpdateResult: reason,
  })

  await goToConfiguration(page)

  await page.getByRole('button', { name: 'Update' }).click()

  await expect(page.getByText(reason)).toBeVisible()
  // Status is unchanged after a refusal
  await expect(page.getByText('Repository is behind by 3 commits')).toBeVisible()
})

// ---------------------------------------------------------------------------
// 21. Refresh control re-reads the configuration status
// ---------------------------------------------------------------------------
test('refresh control re-fetches the configuration status', async ({ page }) => {
  await setupRoutes(page, { configGitRepository: configRepoBehind })

  await goToConfiguration(page)

  let refetched = false
  page.on('request', (req) => {
    if (req.url().includes('/api/configuration') && req.method() === 'GET') refetched = true
  })

  await page.getByLabel('Re-check repository status').click()
  await expect.poll(() => refetched).toBe(true)
})

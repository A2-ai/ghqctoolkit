import type { Page } from 'playwright/test'
import {
  defaultRepoInfo,
  openMilestone,
  awaitingReviewStatus,
  libIssue,
  externalIssue,
  defaultAssignees,
  defaultChecklists,
  rootFileTree,
  srcFileTree,
  createdMilestone,
  createIssueResponses,
  configRepoClean,
} from '../fixtures/index'
import type { ConfigGitRepository } from '../../src/api/configuration'
import type { BatchIssueStatusResponse } from '../../src/api/issues'
import type { Milestone } from '../../src/api/milestones'
import type { Issue } from '../../src/api/issues'
import type { RepoInfo } from '../../src/api/repo'
import type { Assignee } from '../../src/api/assignees'
import type { Checklist } from '../../src/api/checklists'
import type { FileTreeResponse } from '../../src/api/files'
import type { CreateIssueResponse } from '../../src/api/create'
import type { ArchiveGenerateResponse } from '../../src/api/archive'
import type { BlockedIssueStatus, CommentResponse, CreateRoundResponse, ReviewResponse, UnapprovalResponse } from '../../src/api/issues'

export interface RouteOverrides {
  repo: RepoInfo
  milestones: Milestone[]
  /** Milestones returned by GET /api/milestones after a successful create */
  milestonesAfterCreate: Milestone[] | null
  /** Map from milestone number to issue list */
  milestoneIssues: Record<number, Issue[]>
  /** Full batch response returned for /api/issues/status */
  issueStatuses: BatchIssueStatusResponse
  /** HTTP status code for /api/issues/status (default 200) */
  issueStatusesCode: number
  /** checklist_display_name returned by GET /api/configuration (default: 'checklists') */
  checklistDisplayName: string
  /** Whether collaborators should be exposed in create flows */
  includeCollaborators: boolean
  /** Effective repo refresh interval returned by GET /api/configuration (default: 15) */
  uiRepoRefreshRateSeconds: number
  /** Checklists returned by GET /api/configuration */
  checklists: Checklist[]
  /** git_repository returned by GET /api/configuration (default: null → local directory only) */
  configGitRepository: ConfigGitRepository | null
  /**
   * Result of POST /api/configuration/update. A ConfigGitRepository is returned
   * as the new git_repository with status 200; a string is returned as a 409
   * `{ error }` body.
   */
  configUpdateResult: ConfigGitRepository | string
  /** options.allow_config_update returned by GET /api/configuration (default: true) */
  allowConfigUpdate: boolean
  /** Assignees returned by /api/assignees */
  assignees: Assignee[]
  /** File tree responses keyed by path ('' for root, 'src' for src/, etc.) */
  fileTree: Record<string, FileTreeResponse>
  /** Default collaborator lists keyed by file path */
  fileCollaborators: Record<string, string[]>
  /** Milestone returned by POST /api/milestones */
  createMilestone: Milestone
  /** Issue responses returned by POST /api/milestones/:n/issues */
  createIssues: CreateIssueResponse[]
  /** Artificial delay for POST /api/milestones/:n/issues in milliseconds */
  createIssuesDelayMs: number
  /** Response for POST /api/issues/:n/comment; null → 500 error */
  postCommentResponse: CommentResponse | null
  /** Response for POST /api/issues/:n/review; null → 500 error */
  postReviewResponse: ReviewResponse | null
  /** HTML response for POST /api/preview/previous-qc-diff */
  previousQcDiffPreviewHtml: string
  /** HTML response for POST /api/preview/round (D47); null → 500 error */
  roundPreviewHtml: string | null
  /** Response for POST /api/issues/:n/approve; null → 500 error */
  postApproveResponse: { approval_url: string; skipped_unapproved: number[]; skipped_errors: unknown[]; closed: boolean } | null
  /** Response for POST /api/issues/:n/unapprove; null → 500 error */
  postUnapproveResponse: UnapprovalResponse | null
  /** Default response for GET /api/issues/:n/blocked; 501 → not implemented; null → 500 */
  blockedResponse: BlockedIssueStatus[] | 501 | null
  /** Per-issue overrides for GET /api/issues/:n/blocked (takes precedence over blockedResponse) */
  blockedResponseByIssue: Record<number, BlockedIssueStatus[] | 501 | null>
  /** Response for POST /api/record/preview; null → 500 */
  recordPreviewResponse: { key: string } | null
  /** Whether POST /api/record/generate succeeds (false → 500) */
  recordGenerateSuccess: boolean
  /** Response for POST /api/record/upload; null → 400 */
  recordUploadResponse: { temp_path: string } | null
  /** Response for POST /api/archive/generate; null → 500.
   *  Typed against the real response so D62's non-optional `skipped` cannot drift. */
  archiveGenerateResponse: ArchiveGenerateResponse | null
  /** Response for POST /api/issues/:n/rounds (A5); null → 500 error */
  createRoundResponse: CreateRoundResponse | null
  /** Response for GET /api/commits */
  commitsResponse: { commits: { hash: string; message: string; file_changed: boolean }[]; total: number; page: number; page_size: number }
}

const defaultOverrides: RouteOverrides = {
  repo: defaultRepoInfo,
  milestones: [openMilestone],
  milestonesAfterCreate: null,
  milestoneIssues: {
    1: [libIssue, externalIssue],
  },
  issueStatuses: {
    results: [awaitingReviewStatus],
    errors: [],
  },
  issueStatusesCode: 200,
  checklistDisplayName: 'checklists',
  includeCollaborators: true,
  uiRepoRefreshRateSeconds: 15,
  checklists: defaultChecklists,
  configGitRepository: null,
  configUpdateResult: configRepoClean,
  allowConfigUpdate: true,
  assignees: defaultAssignees,
  fileTree: { '': rootFileTree, src: srcFileTree },
  fileCollaborators: {
    'src/lib.rs': ['Jane Doe <jane@example.com>'],
    'src/external.rs': ['Jane Doe <jane@example.com>'],
  },
  createMilestone: createdMilestone,
  createIssues: createIssueResponses,
  createIssuesDelayMs: 0,
  postCommentResponse: { comment_url: 'https://github.com/test-owner/test-repo/issues/71#issuecomment-99999' },
  postReviewResponse: {
    comment_url: 'https://github.com/test-owner/test-repo/issues/70#issuecomment-88888',
    stash: { status: 'stashed', message: 'Stashed local changes for src/single.rs' },
  },
  previousQcDiffPreviewHtml: '<p>Previous QC diff preview</p>',
  // D47: the round comment body is rendered server-side, so the mock stands in for
  // `markdown_to_html(QCRound::generate_body(..))` — including the
  // `[file contents at initial qc commit]` line the client cannot produce.
  roundPreviewHtml:
    '<h1>QC Round 3</h1><p><a href="https://github.com/test-owner/test-repo/blob/ccc3333/src/two-rounds.rs">file contents at initial qc commit</a></p>',
  postApproveResponse: { approval_url: 'https://github.com/test-owner/test-repo/issues/70#issuecomment-77777', skipped_unapproved: [], skipped_errors: [], closed: true },
  postUnapproveResponse: { unapproval_url: 'https://github.com/test-owner/test-repo/issues/74#issuecomment-66666', opened: true },
  blockedResponse: [],
  blockedResponseByIssue: {},
  recordPreviewResponse: { key: 'preview-test-key' },
  recordGenerateSuccess: true,
  recordUploadResponse: { temp_path: '/tmp/ghqc-uploads/test123.pdf' },
  archiveGenerateResponse: { output_path: '/mock/repo/test-archive.tar.gz', skipped: [] },
  commitsResponse: { commits: [{ hash: 'abc1234567890', message: 'Initial commit', file_changed: true }], total: 1, page: 0, page_size: 10 },
  createRoundResponse: {
    round_index: 2,
    comment_url: 'https://github.com/test-owner/test-repo/issues/74#issuecomment-55555',
    reopened: true,
    notification: { kind: 'posted', url: 'https://github.com/test-owner/test-repo/issues/74#issuecomment-55556' },
  },
}

export async function setupRoutes(page: Page, overrides: Partial<RouteOverrides> = {}): Promise<void> {
  const cfg: RouteOverrides = { ...defaultOverrides, ...overrides }
  let milestoneCreated = false

  await page.route('/api/repo', (route) => {
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify(cfg.repo),
    })
  })

  await page.route('/api/milestones', (route, request) => {
    if (request.method() === 'POST') {
      milestoneCreated = true
      route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify(cfg.createMilestone),
      })
    } else {
      const milestones = milestoneCreated && cfg.milestonesAfterCreate !== null
        ? cfg.milestonesAfterCreate
        : cfg.milestones
      route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify(milestones),
      })
    }
  })

  // Handle /api/milestones/:n/issues — GET returns issue list, POST returns create responses
  await page.route(/\/api\/milestones\/(\d+)\/issues/, async (route, request) => {
    if (request.method() === 'POST') {
      if (cfg.createIssuesDelayMs > 0) {
        await new Promise((resolve) => setTimeout(resolve, cfg.createIssuesDelayMs))
      }
      route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify(cfg.createIssues),
      })
    } else {
      const url = request.url()
      const match = url.match(/\/api\/milestones\/(\d+)\/issues/)
      const milestoneNum = match ? Number(match[1]) : -1
      const issues = cfg.milestoneIssues[milestoneNum] ?? []
      route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify(issues),
      })
    }
  })

  await page.route(/\/api\/issues\/status/, (route) => {
    route.fulfill({
      status: cfg.issueStatusesCode,
      contentType: 'application/json',
      body: JSON.stringify(cfg.issueStatuses),
    })
  })

  // Mutated by a successful POST /api/configuration/update so that the
  // subsequent GET reflects the updated repository.
  let currentConfigGitRepository = cfg.configGitRepository

  const configStatusBody = () => ({
      directory: '/mock/config',
      exists: true,
      git_repository: currentConfigGitRepository,
      options: {
        prepended_checklist_note: null,
        checklist_display_name: cfg.checklistDisplayName,
        include_collaborators: cfg.includeCollaborators,
        logo_path: 'logo.png',
        logo_found: false,
        checklist_directory: 'checklists/',
        record_path: 'records/',
        ui_repo_refresh_rate_seconds: cfg.uiRepoRefreshRateSeconds,
        allow_config_update: cfg.allowConfigUpdate,
      },
      checklists: cfg.checklists,
      config_repo_env: null,
  })

  await page.route('/api/configuration', (route) => {
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify(configStatusBody()),
    })
  })

  await page.route('/api/configuration/update', (route, request) => {
    if (request.method() !== 'POST') { void route.continue(); return }
    if (typeof cfg.configUpdateResult === 'string') {
      route.fulfill({
        status: 409,
        contentType: 'application/json',
        body: JSON.stringify({ error: cfg.configUpdateResult }),
      })
      return
    }
    currentConfigGitRepository = cfg.configUpdateResult
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify(configStatusBody()),
    })
  })

  await page.route('/api/assignees', (route) => {
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify(cfg.assignees),
    })
  })

  await page.route(/\/api\/files\/tree/, (route, request) => {
    const url = new URL(request.url())
    const path = url.searchParams.get('path') ?? ''
    const treeResponse = cfg.fileTree[path] ?? { path, entries: [] }
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify(treeResponse),
    })
  })

  await page.route(/\/api\/files\/content/, async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'text/plain',
      body: '// mock file content',
    })
  })

  await page.route(/\/api\/files\/raw/, (route, request) => {
    const url = new URL(request.url())
    const path = url.searchParams.get('path') ?? ''
    const lowerPath = path.toLowerCase()
    const contentType = lowerPath.endsWith('.pdf')
      ? 'application/pdf'
      : lowerPath.endsWith('.doc')
        ? 'application/msword'
        : lowerPath.endsWith('.docx')
          ? 'application/vnd.openxmlformats-officedocument.wordprocessingml.document'
          : 'application/octet-stream'
    route.fulfill({
      status: 200,
      contentType,
      body: lowerPath.endsWith('.pdf') ? '%PDF-1.7\n% mock pdf\n' : 'mock binary content',
    })
  })

  await page.route(/\/api\/files\/collaborators/, (route, request) => {
    const url = new URL(request.url())
    const path = url.searchParams.get('path') ?? ''
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        path,
        author: cfg.repo.current_user ?? null,
        collaborators: cfg.fileCollaborators[path] ?? [],
      }),
    })
  })

  await page.route(/\/api\/preview\/issue/, (route) => {
    route.fulfill({
      status: 200,
      contentType: 'text/html',
      body: '<p>Preview</p>',
    })
  })

  await page.route(/\/api\/preview\/round/, (route) => {
    if (cfg.roundPreviewHtml === null) {
      route.fulfill({
        status: 500,
        contentType: 'application/json',
        body: JSON.stringify({ error: 'Failed to render the round comment' }),
      })
      return
    }
    route.fulfill({
      status: 200,
      contentType: 'text/html',
      body: cfg.roundPreviewHtml,
    })
  })

  await page.route(/\/api\/preview\/previous-qc-diff/, (route) => {
    route.fulfill({
      status: 200,
      contentType: 'text/html',
      body: cfg.previousQcDiffPreviewHtml,
    })
  })

  await page.route(/\/api\/preview\/\d+\/comment/, (route) => {
    route.fulfill({
      status: 200,
      contentType: 'text/html',
      body: '<p>Comment preview</p>',
    })
  })

  await page.route(/\/api\/preview\/\d+\/review/, (route) => {
    route.fulfill({
      status: 200,
      contentType: 'text/html',
      body: '<p>Review preview</p>',
    })
  })

  await page.route(/\/api\/preview\/\d+\/approve/, (route) => {
    route.fulfill({
      status: 200,
      contentType: 'text/html',
      body: '<p>Approve preview</p>',
    })
  })

  await page.route(/\/api\/preview\/\d+\/unapprove/, (route) => {
    route.fulfill({
      status: 200,
      contentType: 'text/html',
      body: '<p>Unapprove preview</p>',
    })
  })

  await page.route(/\/api\/issues\/\d+\/blocked/, (route, request) => {
    if (request.method() !== 'GET') { void route.continue(); return }
    const match = request.url().match(/\/api\/issues\/(\d+)\/blocked/)
    const issueNum = match ? Number(match[1]) : -1
    const response = issueNum in cfg.blockedResponseByIssue
      ? cfg.blockedResponseByIssue[issueNum]
      : cfg.blockedResponse
    if (response === 501) {
      route.fulfill({ status: 501, contentType: 'application/json', body: JSON.stringify({ error: 'Not implemented' }) })
    } else if (response === null) {
      route.fulfill({ status: 500, contentType: 'application/json', body: JSON.stringify({ error: 'Internal error' }) })
    } else {
      route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(response) })
    }
  })

  await page.route(/\/api\/issues\/\d+\/comment/, (route, request) => {
    if (request.method() === 'POST') {
      if (cfg.postCommentResponse) {
        route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify(cfg.postCommentResponse),
        })
      } else {
        route.fulfill({
          status: 500,
          contentType: 'application/json',
          body: JSON.stringify({ error: 'Internal server error' }),
        })
      }
    } else {
      route.continue()
    }
  })

  await page.route(/\/api\/issues\/\d+\/approve/, (route, request) => {
    if (request.method() === 'POST') {
      if (cfg.postApproveResponse) {
        route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify(cfg.postApproveResponse),
        })
      } else {
        route.fulfill({
          status: 500,
          contentType: 'application/json',
          body: JSON.stringify({ error: 'Internal server error' }),
        })
      }
    } else {
      route.continue()
    }
  })

  await page.route(/\/api\/issues\/\d+\/unapprove/, (route, request) => {
    if (request.method() === 'POST') {
      if (cfg.postUnapproveResponse) {
        route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify(cfg.postUnapproveResponse),
        })
      } else {
        route.fulfill({
          status: 500,
          contentType: 'application/json',
          body: JSON.stringify({ error: 'Internal server error' }),
        })
      }
    } else {
      route.continue()
    }
  })

  // Record PDF endpoints — preview.pdf must be registered before preview to avoid substring match
  await page.route(/\/api\/record\/preview\.pdf/, (route) => {
    route.fulfill({ status: 200, contentType: 'application/pdf', body: '' })
  })

  await page.route(/\/api\/record\/preview$/, (route) => {
    if (cfg.recordPreviewResponse) {
      route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(cfg.recordPreviewResponse) })
    } else {
      route.fulfill({ status: 500, contentType: 'application/json', body: JSON.stringify({ error: 'Preview failed' }) })
    }
  })

  await page.route(/\/api\/record\/generate/, (route) => {
    if (cfg.recordGenerateSuccess) {
      route.fulfill({ status: 200 })
    } else {
      route.fulfill({ status: 500, contentType: 'application/json', body: JSON.stringify({ error: 'Generate failed' }) })
    }
  })

  await page.route(/\/api\/record\/upload/, (route) => {
    if (cfg.recordUploadResponse) {
      route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(cfg.recordUploadResponse) })
    } else {
      route.fulfill({ status: 400, contentType: 'application/json', body: JSON.stringify({ error: 'Upload failed' }) })
    }
  })

  await page.route(/\/api\/commits/, (route) => {
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify(cfg.commitsResponse),
    })
  })

  await page.route(/\/api\/archive\/generate/, (route) => {
    if (cfg.archiveGenerateResponse) {
      route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify(cfg.archiveGenerateResponse),
      })
    } else {
      route.fulfill({
        status: 500,
        contentType: 'application/json',
        body: JSON.stringify({ error: 'Archive generation failed' }),
      })
    }
  })

  // A5: POST /api/issues/:n/rounds
  await page.route(/\/api\/issues\/\d+\/rounds/, (route, request) => {
    if (request.method() !== 'POST') { void route.continue(); return }
    if (cfg.createRoundResponse) {
      route.fulfill({
        status: 201,
        contentType: 'application/json',
        body: JSON.stringify(cfg.createRoundResponse),
      })
    } else {
      route.fulfill({
        status: 500,
        contentType: 'application/json',
        body: JSON.stringify({ error: 'Internal server error' }),
      })
    }
  })

  await page.route(/\/api\/issues\/\d+\/review/, (route, request) => {
    if (request.method() === 'POST') {
      if (cfg.postReviewResponse) {
        route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify(cfg.postReviewResponse),
        })
      } else {
        route.fulfill({
          status: 500,
          contentType: 'application/json',
          body: JSON.stringify({ error: 'Internal server error' }),
        })
      }
    } else {
      route.continue()
    }
  })
}

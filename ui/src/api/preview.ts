import { useQuery } from '@tanstack/react-query'
import type { CreateIssueRequest } from './create'
import type { ApproveRequest, CreateCommentRequest, ReviewRequest, UnapproveRequest } from './issues'
import { API_BASE } from '../config'

export type FilePreviewKind = 'text' | 'doc' | 'unsupported'

const DOC_EXTENSIONS = new Set(['pdf', 'doc', 'docx', 'xls', 'xlsx', 'csv', 'png', 'jpg', 'jpeg', 'gif', 'bmp', 'webp'])

export function getFilePreviewKind(path: string): FilePreviewKind {
  const ext = path.split('.').pop()?.toLowerCase()
  if (ext && DOC_EXTENSIONS.has(ext)) return 'doc'
  return 'text'
}

export function buildFileRawUrl(path: string, commit?: string | null): string {
  const params = new URLSearchParams({ path })
  if (commit) params.set('commit', commit)
  return `${API_BASE}/files/raw?${params.toString()}`
}

export function getFileExtensionLabel(path: string): string {
  const ext = path.split('.').pop()?.trim().toLowerCase()
  return ext ? `.${ext}` : 'this file type'
}

export async function ensureFileExists(path: string, commit?: string | null): Promise<void> {
  const res = await fetch(buildFileRawUrl(path, commit))
  if (!res.ok) {
    const data = await res.json().catch(() => null)
    throw new Error(data?.error ?? `Failed to fetch file: ${res.status}`)
  }
}

export interface FileContentRequest {
  path: string
  commit?: string | null
}

export interface PreviousQCDiffPreviewRequest {
  current_file: string
  previous_file: string
  previous_issue_number: number
  current_commit: string
}

export async function fetchFileContent({ path, commit }: FileContentRequest): Promise<string> {
  const params = new URLSearchParams({ path })
  if (commit) params.set('commit', commit)
  const res = await fetch(`${API_BASE}/files/content?${params.toString()}`)
  if (!res.ok) {
    const data = await res.json().catch(() => null)
    throw new Error(data?.error ?? `Failed to fetch file: ${res.status}`)
  }
  return res.text()
}

export async function fetchCommentPreview(issueNumber: number, request: CreateCommentRequest): Promise<string> {
  const res = await fetch(`${API_BASE}/preview/${issueNumber}/comment`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(request),
  })
  if (!res.ok) {
    const data = await res.json().catch(() => null)
    throw new Error(data?.error ?? `Failed to fetch preview: ${res.status}`)
  }
  return res.text()
}

export async function fetchReviewPreview(issueNumber: number, request: ReviewRequest): Promise<string> {
  const res = await fetch(`${API_BASE}/preview/${issueNumber}/review`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(request),
  })
  if (!res.ok) {
    const data = await res.json().catch(() => null)
    throw new Error(data?.error ?? `Failed to fetch review preview: ${res.status}`)
  }
  return res.text()
}

export async function fetchApprovePreview(issueNumber: number, request: ApproveRequest): Promise<string> {
  const res = await fetch(`${API_BASE}/preview/${issueNumber}/approve`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(request),
  })
  if (!res.ok) {
    const data = await res.json().catch(() => null)
    throw new Error(data?.error ?? `Failed to fetch approve preview: ${res.status}`)
  }
  return res.text()
}

export async function fetchUnapprovePreview(issueNumber: number, request: UnapproveRequest): Promise<string> {
  const res = await fetch(`${API_BASE}/preview/${issueNumber}/unapprove`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(request),
  })
  if (!res.ok) {
    const data = await res.json().catch(() => null)
    throw new Error(data?.error ?? `Failed to fetch unapprove preview: ${res.status}`)
  }
  return res.text()
}

export async function fetchIssuePreview(request: CreateIssueRequest): Promise<string> {
  const res = await fetch(`${API_BASE}/preview/issue`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(request),
  })
  if (!res.ok) {
    const data = await res.json().catch(() => null)
    throw new Error(data?.error ?? `Failed to fetch preview: ${res.status}`)
  }
  return res.text()
}

/**
 * Body for `POST /api/preview/round` (D47). There is deliberately **no
 * `round_index`**: the server derives it the same way `POST /rounds` does, so a
 * preview cannot title itself with a round number the creation would not use.
 */
export interface RoundPreviewRequest {
  issue_number: number
  start_commit: string
  branch: string
  /** `content` excludes the `# {name}` heading line (D37). */
  checklist: { name: string; content: string }
}

/**
 * D47: renders the `# QC Round N` comment through the server's real
 * `QCRound::generate_body` — the same code path `POST /rounds` posts. The round
 * comment body has exactly one implementation, so the preview cannot drift from
 * what gets posted, and only the server can emit the
 * `[file contents at initial qc commit](url)` line (the URL comes from the git
 * provider's blob-URL builder, which the UI cannot compute without guessing the
 * host).
 */
export async function previewRound(request: RoundPreviewRequest): Promise<string> {
  const res = await fetch(`${API_BASE}/preview/round`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(request),
  })
  if (!res.ok) {
    const data = await res.json().catch(() => null)
    throw new Error(data?.error ?? `Failed to fetch round preview: ${res.status}`)
  }
  return res.text()
}

/**
 * `request === null` disables the fetch — callers pass null while the preview is
 * not being looked at, or before the checkout's branch/commit are known. Pass
 * debounced checklist fields: the checklist is an editable textarea and the query
 * key is the request, so an undebounced value would fire a request per keystroke.
 */
export function useRoundPreview(request: RoundPreviewRequest | null) {
  return useQuery({
    queryKey: ['preview', 'round', request],
    queryFn: () => previewRound(request!),
    enabled: request !== null,
    // The other preview paths report a failure the moment it happens; the global
    // default of three retries would leave the user watching a spinner for seconds
    // before the error appears, and any edit re-fires the request anyway.
    retry: false,
  })
}

export async function fetchPreviousQCDiffPreview(request: PreviousQCDiffPreviewRequest): Promise<string> {
  const res = await fetch(`${API_BASE}/preview/previous-qc-diff`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(request),
  })
  if (!res.ok) {
    const data = await res.json().catch(() => null)
    throw new Error(data?.error ?? `Failed to fetch previous QC diff preview: ${res.status}`)
  }
  return res.text()
}

/**
 * The round notification is a plain `QCComment` (D5) — the very type
 * `POST /issues/:n/comment` posts — so `POST /preview/:n/comment` renders it through
 * the identical server-side body builder. There is deliberately no round-specific
 * notification preview endpoint, because there is no round-specific notification:
 * `create_round` builds a `QCComment` with `current_commit` = the round's start and
 * `previous_commit` = the prior round's approval, and this preview is asked for
 * exactly that pair.
 *
 * `request === null` disables the fetch. Pass a debounced note for the same reason
 * `useRoundPreview` wants a debounced checklist — the note is a live textarea.
 */
export function useCommentPreview(issueNumber: number, request: CreateCommentRequest | null) {
  return useQuery({
    queryKey: ['preview', 'comment', issueNumber, request],
    queryFn: () => fetchCommentPreview(issueNumber, request!),
    enabled: request !== null,
    // Same reasoning as useRoundPreview: report the failure at once rather than
    // spinning through three retries, since any edit re-fires the request anyway.
    retry: false,
  })
}

/**
 * Body for `POST /api/preview/round-diff`. Carries only the *new* end of the
 * comparison: the old end is the prior round's approval, derived server-side exactly as
 * `create_round` derives the notification's `previous commit` (D5). A client that could
 * pass both ends could show a diff for a transition that is not the one about to happen.
 */
export interface RoundDiffPreviewRequest {
  issue_number: number
  start_commit: string
}

export async function previewRoundDiff(request: RoundDiffPreviewRequest): Promise<string> {
  const res = await fetch(`${API_BASE}/preview/round-diff`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(request),
  })
  if (!res.ok) {
    const data = await res.json().catch(() => null)
    throw new Error(data?.error ?? `Failed to fetch round diff: ${res.status}`)
  }
  return res.text()
}

/**
 * The change a new round would be opened over: this file, between the prior round's
 * approval and the checked-out commit. It is the same diff the `# QC Notification`
 * embeds, through the same `diff_utils::file_diff_between_commits`.
 *
 * `request === null` disables the fetch — callers pass null when the checkout is still
 * sitting on the approval, because there is provably nothing to diff (U3) and asking
 * would spend a round-trip to be told so.
 */
export function useRoundDiffPreview(request: RoundDiffPreviewRequest | null) {
  return useQuery({
    queryKey: ['preview', 'round-diff', request],
    queryFn: () => previewRoundDiff(request!),
    enabled: request !== null,
    retry: false,
  })
}

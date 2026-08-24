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

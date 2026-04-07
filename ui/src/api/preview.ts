import type { CreateIssueRequest } from './create'
import type { ApproveRequest, CreateCommentRequest, ReviewRequest, UnapproveRequest } from './issues'
import { API_BASE } from '../config'

export type FilePreviewKind = 'text' | 'pdf' | 'unsupported'

export function getFilePreviewKind(path: string): FilePreviewKind {
  const ext = path.split('.').pop()?.toLowerCase()
  if (ext === 'pdf') return 'pdf'
  if (ext === 'doc' || ext === 'docx' || ext === 'xls' || ext === 'xlsx') return 'unsupported'
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

import type { CreateIssueRequest } from './create'
import type { ApproveRequest, CreateCommentRequest, ReviewRequest } from './issues'

export async function fetchFileContent(path: string): Promise<string> {
  const res = await fetch(`/api/files/content?path=${encodeURIComponent(path)}`)
  if (!res.ok) {
    const data = await res.json().catch(() => null)
    throw new Error(data?.error ?? `Failed to fetch file: ${res.status}`)
  }
  return res.text()
}

export async function fetchCommentPreview(issueNumber: number, request: CreateCommentRequest): Promise<string> {
  const res = await fetch(`/api/preview/${issueNumber}/comment`, {
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
  const res = await fetch(`/api/preview/${issueNumber}/review`, {
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
  const res = await fetch(`/api/preview/${issueNumber}/approve`, {
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

export async function fetchIssuePreview(request: CreateIssueRequest): Promise<string> {
  const res = await fetch('/api/preview/issue', {
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

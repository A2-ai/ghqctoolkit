import { API_BASE } from '../config'

export interface BranchCommit {
  hash: string
  message: string
  file_changed: boolean
}

export interface PagedCommitsResponse {
  commits: BranchCommit[]
  total: number
  page: number
  page_size: number
}

export interface FetchBranchCommitsOptions {
  file?: string
  page?: number
  pageSize?: number
  /** Commit hash prefix; backend returns the page containing this commit. */
  locate?: string
}

export async function fetchBranchCommits(
  options: FetchBranchCommitsOptions = {},
): Promise<PagedCommitsResponse> {
  const { file, page = 0, pageSize, locate } = options
  const params = new URLSearchParams()
  if (file) params.set('file', file)
  params.set('page', String(page))
  if (pageSize !== undefined) params.set('page_size', String(pageSize))
  if (locate !== undefined) params.set('locate', locate)
  const res = await fetch(`${API_BASE}/commits?${params.toString()}`)
  if (!res.ok) {
    const data = await res.json().catch(() => null)
    throw new Error(data?.error ?? `Failed to fetch commits: ${res.status}`)
  }
  return res.json()
}

export interface CommitDiffResponse {
  /**
   * Markdown diff of the file between the two commits. `null` when there is no
   * difference, or the file could not be read at one of them — neither of which is
   * an error, so callers render it as "no changes" rather than as a failure.
   */
  diff: string | null
}

export async function fetchCommitDiff(
  file: string,
  from: string,
  to: string,
): Promise<CommitDiffResponse> {
  const params = new URLSearchParams({ file, from, to })
  const res = await fetch(`${API_BASE}/commits/diff?${params.toString()}`)
  if (!res.ok) {
    const data = await res.json().catch(() => null)
    throw new Error(data?.error ?? `Failed to fetch diff: ${res.status}`)
  }
  return res.json()
}

export function commitDiffQueryKey(file: string, from: string, to: string) {
  return ['commit-diff', file, from, to] as const
}

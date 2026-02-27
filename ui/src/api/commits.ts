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
  const res = await fetch(`/api/commits?${params.toString()}`)
  if (!res.ok) {
    const data = await res.json().catch(() => null)
    throw new Error(data?.error ?? `Failed to fetch commits: ${res.status}`)
  }
  return res.json()
}

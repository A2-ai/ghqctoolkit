import { useQuery } from '@tanstack/react-query'

export type GitStatus = 'clean' | 'ahead' | 'behind' | 'diverged'

export interface RepoInfo {
  owner: string
  repo: string
  branch: string
  local_commit: string
  remote_commit: string
  git_status: GitStatus
  git_status_detail: string
  dirty_files: string[]
  current_user: string | null
}

async function fetchRepoInfo(): Promise<RepoInfo> {
  const res = await fetch('/api/repo')
  if (!res.ok) {
    let message = `Failed to fetch repo info: ${res.status}`
    try {
      const data = await res.json()
      if (typeof data.error === 'string') message = data.error
    } catch {}
    throw new Error(message)
  }
  return res.json()
}

export function useRepoInfo() {
  return useQuery({
    queryKey: ['repo'],
    queryFn: fetchRepoInfo,
    refetchInterval: 45_000,
  })
}

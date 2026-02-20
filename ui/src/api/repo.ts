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
}

async function fetchRepoInfo(): Promise<RepoInfo> {
  const res = await fetch('/api/repo')
  if (!res.ok) throw new Error(`Failed to fetch repo info: ${res.status}`)
  return res.json() 
}

export function useRepoInfo() {
  return useQuery({
    queryKey: ['repo'],
    queryFn: fetchRepoInfo,
    refetchInterval: 45_000,
  })
}

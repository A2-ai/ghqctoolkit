import { useQuery, useQueryClient } from '@tanstack/react-query'
import { useEffect } from 'react'

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

const ACTIVITY_EVENTS = ['mousemove', 'mousedown', 'keydown', 'touchstart', 'scroll'] as const
const INACTIVE_MS = 5 * 60 * 1000

let lastActivityAt = Date.now()

export function useRepoInfo() {
  const queryClient = useQueryClient()

  const query = useQuery({
    queryKey: ['repo'],
    queryFn: fetchRepoInfo,
    refetchInterval: () => (Date.now() - lastActivityAt < INACTIVE_MS ? 30_000 : false),
    refetchOnWindowFocus: true,
  })

  useEffect(() => {
    function onActivity() {
      const wasInactive = Date.now() - lastActivityAt >= INACTIVE_MS
      lastActivityAt = Date.now()
      if (wasInactive) {
        queryClient.invalidateQueries({ queryKey: ['repo'] })
      }
    }

    for (const event of ACTIVITY_EVENTS) {
      window.addEventListener(event, onActivity, { passive: true })
    }
    return () => {
      for (const event of ACTIVITY_EVENTS) {
        window.removeEventListener(event, onActivity)
      }
    }
  }, [queryClient])

  return query
}

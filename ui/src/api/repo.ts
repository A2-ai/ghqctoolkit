import { useQuery, useQueryClient } from '@tanstack/react-query'
import { useEffect, useRef } from 'react'
import { useConfigurationStatus } from './configuration'
import { API_BASE } from '../config'

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
  const res = await fetch(`${API_BASE}/repo`)
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
const DEFAULT_REFRESH_RATE_MS = 15_000

let lastActivityAt = Date.now()

export function useRepoInfo() {
  const queryClient = useQueryClient()
  const configurationQuery = useConfigurationStatus()
  const refreshRateMs =
    (configurationQuery.data?.options.ui_repo_refresh_rate_seconds ?? 15) * 1000

  const query = useQuery({
    queryKey: ['repo'],
    queryFn: fetchRepoInfo,
    refetchInterval: () =>
      (Date.now() - lastActivityAt < INACTIVE_MS
        ? refreshRateMs || DEFAULT_REFRESH_RATE_MS
        : false),
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

  const prevCommitRef = useRef<string | null>(null)
  useEffect(() => {
    const commit = query.data?.local_commit ?? null
    if (commit !== null && prevCommitRef.current !== null && commit !== prevCommitRef.current) {
      queryClient.invalidateQueries({ queryKey: ['issue', 'status'] })
    }
    prevCommitRef.current = commit
  }, [query.data?.local_commit, queryClient])

  return query
}

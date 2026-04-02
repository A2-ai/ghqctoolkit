import { useQuery } from '@tanstack/react-query'
import type { Checklist } from '~/api/checklists'
import { resolveDisplayName } from '~/utils/displayName'
import { API_BASE } from '../config'

export interface ConfigGitRepository {
  owner: string
  repo: string
  status: 'clean' | 'ahead' | 'behind' | 'diverged'
  dirty_files: string[]
}

export interface ConfigurationOptions {
  prepended_checklist_note: string | null
  checklist_display_name: string
  include_collaborators: boolean
  logo_path: string
  logo_found: boolean
  checklist_directory: string
  record_path: string
  ui_repo_refresh_rate_seconds: number
}

export interface ConfigurationStatus {
  directory: string
  exists: boolean
  git_repository: ConfigGitRepository | null
  options: ConfigurationOptions
  checklists: Checklist[]
  config_repo_env: string | null
}

async function fetchConfigurationStatus(): Promise<ConfigurationStatus> {
  const res = await fetch(`${API_BASE}/configuration`)
  if (!res.ok) throw new Error(`Failed to fetch configuration status: ${res.status}`)
  return res.json()
}

export function useConfigurationStatus() {
  return useQuery({
    queryKey: ['configuration', 'status'],
    queryFn: fetchConfigurationStatus,
  })
}

/** Returns singular/plural display names derived from the configured checklist_display_name. */
export function useChecklistDisplayName(): { singular: string; plural: string } {
  const { data } = useConfigurationStatus()
  return resolveDisplayName(data?.options.checklist_display_name ?? 'checklist')
}

export async function setupConfiguration(url: string): Promise<ConfigurationStatus> {
  const res = await fetch(`${API_BASE}/configuration`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ url }),
  })
  if (!res.ok) {
    const data = await res.json().catch(() => ({}))
    throw new Error((data as { error?: string }).error ?? `Setup failed: ${res.status}`)
  }
  return res.json()
}

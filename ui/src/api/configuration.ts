import { useQuery } from '@tanstack/react-query'
import type { Checklist } from '~/api/checklists'
import { resolveDisplayName } from '~/utils/displayName'

export interface ConfigGitRepository {
  owner: string
  repo: string
  status: 'clean' | 'ahead' | 'behind' | 'diverged'
  dirty_files: string[]
}

export interface ConfigurationOptions {
  prepended_checklist_note: string | null
  checklist_display_name: string
  logo_path: string
  logo_found: boolean
  checklist_directory: string
  record_path: string
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
  const res = await fetch('/api/configuration')
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
  const res = await fetch('/api/configuration', {
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

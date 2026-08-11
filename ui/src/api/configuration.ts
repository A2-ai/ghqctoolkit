import { useQuery } from '@tanstack/react-query'
import type { Checklist } from '~/api/checklists'
import { resolveDisplayName } from '~/utils/displayName'
import { API_BASE } from '../config'

export type ConfigGitStatus = 'clean' | 'ahead' | 'behind' | 'diverged'

export interface ConfigGitRepository {
  owner: string
  repo: string
  status: ConfigGitStatus
  status_detail: string
  ahead_commits: string[]
  behind_commits: string[]
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
  /**
   * Whether this deployment permits UI-initiated updates of the configuration
   * repository. Deployments that provision the configuration repository
   * centrally set this to false; users there have no write access, so the UI
   * must not offer an update it cannot perform. Resolved server-side; a
   * missing value (older backend) is treated as true — see
   * {@link configUpdateAllowed}.
   */
  allow_config_update: boolean
}

export interface ConfigurationStatus {
  directory: string
  exists: boolean
  git_repository: ConfigGitRepository | null
  options: ConfigurationOptions
  checklists: Checklist[]
  config_repo_env: string | null
}

/**
 * Whether the UI may offer to update the configuration repository.
 * Defaults to true when the backend omits the flag so that an older backend
 * does not silently hide the Update button.
 */
export function configUpdateAllowed(status: ConfigurationStatus | undefined): boolean {
  return status?.options.allow_config_update !== false
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

/**
 * Pulls the latest commits into the configuration repository.
 * The backend refuses (409) when the worktree is dirty, has unpushed commits,
 * or has diverged — the returned message is written to be shown verbatim.
 */
export async function updateConfiguration(): Promise<ConfigurationStatus> {
  const res = await fetch(`${API_BASE}/configuration/update`, {
    method: 'POST',
  })
  if (!res.ok) {
    const data = await res.json().catch(() => ({}))
    throw new Error((data as { error?: string }).error ?? `Update failed: ${res.status}`)
  }
  return res.json()
}

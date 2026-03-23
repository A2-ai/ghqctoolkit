import { useRepoInfo } from '~/api/repo'
import { useMilestoneIssues } from '~/api/issues'
import { SwimLanes } from './SwimLanes'
import { useUiSession } from '~/state/uiSession'

export function StatusTab() {
  const { data: repoData } = useRepoInfo()
  const { status } = useUiSession()
  const dirtyFiles = new Set(repoData?.dirty_files ?? [])
  const { statuses: rawStatuses } = useMilestoneIssues(
    status.selectedMilestones,
    status.includeClosedIssues,
  )

  const statuses = rawStatuses.map((entry) =>
    !entry.dirty && dirtyFiles.has(entry.issue.title) ? { ...entry, dirty: true } : entry,
  )

  return (
    <SwimLanes
      statuses={statuses}
      currentBranch={repoData?.branch ?? ''}
      remoteCommit={repoData?.remote_commit ?? ''}
    />
  )
}

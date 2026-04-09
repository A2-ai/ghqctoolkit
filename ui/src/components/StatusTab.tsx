import { useState } from 'react'
import { Alert } from '@mantine/core'
import { useRepoInfo } from '~/api/repo'
import { useConfirmRename, useMilestoneIssues, useRenames } from '~/api/issues'
import { useMilestones } from '~/api/milestones'
import { SwimLanes } from './SwimLanes'
import { RenamePromptBanner } from './RenamePromptBanner'
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

  // Rename detection — runs in the background; empty while milestones are loading.
  const { data: milestones = [] } = useMilestones()
  const getMilestoneName = (n: number) => milestones.find((m) => m.number === n)?.title ?? `#${n}`
  const { renames } = useRenames(status.selectedMilestones)
  const confirmRename = useConfirmRename(status.selectedMilestones)
  const [dismissed, setDismissed] = useState<Set<number>>(new Set())
  const [confirming, setConfirming] = useState<Set<number>>(new Set())
  const [renameError, setRenameError] = useState<string | null>(null)

  const visibleRenames = renames.filter((r) => !dismissed.has(r.issue_number))

  function handleConfirm(issueNumber: number, newPath: string) {
    setConfirming((prev) => new Set(prev).add(issueNumber))
    setRenameError(null)
    confirmRename.mutate(
      { issueNumber, newPath },
      {
        onSettled: () => {
          setConfirming((prev) => {
            const next = new Set(prev)
            next.delete(issueNumber)
            return next
          })
        },
        onError: (error) => {
          setRenameError(error instanceof Error ? error.message : 'Failed to confirm rename')
        },
      },
    )
  }

  function handleDismiss(issueNumber: number) {
    setDismissed((prev) => new Set(prev).add(issueNumber))
  }

  return (
    <div style={{ height: '100%', minHeight: 0, display: 'flex', flexDirection: 'column' }}>
      <RenamePromptBanner
        renames={visibleRenames}
        onConfirm={handleConfirm}
        onDismiss={handleDismiss}
        confirming={confirming}
        getMilestoneName={getMilestoneName}
      />
      {renameError && (
        <Alert color="red" mb="xs" title="Rename failed" withCloseButton onClose={() => setRenameError(null)}>
          {renameError}
        </Alert>
      )}
      <div style={{ flex: 1, minHeight: 0 }}>
        <SwimLanes
          statuses={statuses}
          currentBranch={repoData?.branch ?? ''}
          remoteCommit={repoData?.remote_commit ?? ''}
        />
      </div>
    </div>
  )
}

import { AppShell, Text } from '@mantine/core'
import { useState } from 'react'
import { SwimLanes } from './SwimLanes'
import { useRepoInfo } from '~/api/repo'
import { RepoStatus } from './RepoStatus'
import { MilestoneFilter } from './MilestoneFilter'
import { useMilestoneIssues } from '~/api/issues'

export function AppLayout() {
  const { data: repoData, isError: repoIsError, error: repoError } = useRepoInfo()
  const [selectedMilestones, setSelectedMilestones] = useState<number[]>([])
  const [includeClosedIssues, setIncludeClosedIssues] = useState(false)

  const { statuses: rawStatuses, milestoneStatusByMilestone } = useMilestoneIssues(
    selectedMilestones,
    includeClosedIssues,
  )

  const dirtyFiles = new Set(repoData?.dirty_files ?? [])
  const statuses = rawStatuses.map((s) =>
    !s.dirty && dirtyFiles.has(s.issue.title) ? { ...s, dirty: true } : s
  )


  if (repoIsError) {
    const message = (repoError as Error)?.message ?? 'Failed to load repository information'
    return (
      <div style={{
        display: 'flex',
        flexDirection: 'column',
        alignItems: 'center',
        justifyContent: 'center',
        height: '100vh',
        gap: 24,
        backgroundColor: '#f8f9fa',
      }}>
        <img src="/logo.png" alt="ghqc logo" style={{ height: 80 }} />
        <div style={{
          backgroundColor: '#ffe3e3',
          border: '1px solid #ff8787',
          borderRadius: 8,
          padding: '20px 28px',
          maxWidth: 520,
          textAlign: 'center',
        }}>
          <Text fw={700} size="lg" c="#c92a2a" mb={8}>Unable to load repository</Text>
          <Text size="sm" c="#c92a2a">{message}</Text>
        </div>
      </div>
    )
  }

  return (
    <AppShell
      header={{ height: 80 }}
      navbar={{ width: 240, breakpoint: 'sm' }}
      padding="md"
    >
      <AppShell.Header style={{ backgroundColor: '#d7e7d3' }}>
        <div style={{
          display: 'grid',
          gridTemplateColumns: '1fr auto 1fr',
          alignItems: 'center',
          height: '100%',
          padding: '0 16px',
        }}>
          <img src="/logo.png" alt="ghqc logo" style={{ height: 65 }} />

          {repoData && (
            <span style={{ fontSize: 28, fontWeight: 900 }}>
              {repoData.owner}/{repoData.repo}
            </span>
          )}

          <div style={{ display: 'flex', justifyContent: 'flex-end' }}>
            {repoData && <RepoStatus data={repoData} />}
          </div>
        </div>
      </AppShell.Header>

      <AppShell.Navbar p="md">
        <MilestoneFilter
          selected={selectedMilestones}
          onSelect={setSelectedMilestones}
          includeClosedIssues={includeClosedIssues}
          onIncludeClosedIssuesChange={setIncludeClosedIssues}
          milestoneStatusByMilestone={milestoneStatusByMilestone}
        />
      </AppShell.Navbar>

      <AppShell.Main>
        <SwimLanes statuses={statuses} currentBranch={repoData?.branch ?? ''} remoteCommit={repoData?.remote_commit ?? ''} />
      </AppShell.Main>
    </AppShell>
  )
}

import { AppShell } from '@mantine/core'
import { useState } from 'react'
import { SwimLanes } from './SwimLanes'
import { useRepoInfo } from '~/api/repo'
import { RepoStatus } from './RepoStatus'
import { MilestoneFilter } from './MilestoneFilter'
import { useMilestoneIssues } from '~/api/issues'

export function AppLayout() {
  const { data: repoData } = useRepoInfo()
  const [selectedMilestones, setSelectedMilestones] = useState<number[]>([])
  const [includeClosedIssues, setIncludeClosedIssues] = useState(false)

  const { statuses, milestoneStatusByMilestone } = useMilestoneIssues(
    selectedMilestones,
    includeClosedIssues,
  )


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
        <SwimLanes statuses={statuses} currentBranch={repoData?.branch ?? ''} />
      </AppShell.Main>
    </AppShell>
  )
}

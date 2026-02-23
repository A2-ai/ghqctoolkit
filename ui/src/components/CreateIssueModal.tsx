import { useMemo, useState } from 'react'
import { Group, Modal, Tabs } from '@mantine/core'
import { FileTreeBrowser } from './FileTreeBrowser'
import { IssuePreviewCard } from './IssuePreviewCard'
import { ChecklistTab } from './ChecklistTab'
import type { ChecklistDraft } from './ChecklistTab'
import { useRepoInfo } from '~/api/repo'
import { useIssuesForMilestone } from '~/api/issues'

interface Props {
  opened: boolean
  onClose: () => void
  milestoneNumber: number | null
}

export function CreateIssueModal({ opened, onClose, milestoneNumber }: Props) {
  const [selectedFile, setSelectedFile] = useState<string | null>(null)
  const [checklistDraft, setChecklistDraft] = useState<ChecklistDraft>({ name: '', content: '' })
  const { data: repoInfo } = useRepoInfo()
  const { data: milestoneIssues = [] } = useIssuesForMilestone(milestoneNumber)

  // Issue titles ARE the file path (e.g. "scripts/file_b.R"); build a set for O(1) lookup
  const claimedFiles = useMemo<Set<string>>(
    () => new Set(milestoneIssues.map((i) => i.title)),
    [milestoneIssues],
  )

  return (
    <Modal
      opened={opened}
      onClose={onClose}
      title="Create QC Issue"
      size={900}
      centered
    >
      <Tabs defaultValue="file">
        <Tabs.List grow>
          <Tabs.Tab value="file">Select a File</Tabs.Tab>
          <Tabs.Tab value="checklist">Select a Checklist</Tabs.Tab>
          <Tabs.Tab value="relevant">Select Relevant Files</Tabs.Tab>
          <Tabs.Tab value="reviewers">Select Reviewer(s)</Tabs.Tab>
        </Tabs.List>

        <Group align="flex-start" gap="md" wrap="nowrap" pt="md">
          <div style={{ flex: '1 1 0', minWidth: 0 }}>
            <Tabs.Panel value="file">
              <FileTreeBrowser
                selectedFile={selectedFile}
                onSelect={setSelectedFile}
                claimedFiles={claimedFiles}
              />
            </Tabs.Panel>
            <Tabs.Panel value="checklist">
              <ChecklistTab onChange={setChecklistDraft} />
            </Tabs.Panel>
            <Tabs.Panel value="relevant">{null}</Tabs.Panel>
            <Tabs.Panel value="reviewers">{null}</Tabs.Panel>
          </div>

          <div style={{ flex: '0 0 260px' }}>
            <IssuePreviewCard
              file={selectedFile}
              branch={repoInfo?.branch ?? null}
              createdBy={repoInfo?.current_user ?? null}
              checklistName={checklistDraft.name || null}
            />
          </div>
        </Group>
      </Tabs>
    </Modal>
  )
}

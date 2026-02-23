import { useEffect, useMemo, useState } from 'react'
import { Button, Group, Modal, Tabs } from '@mantine/core'
import { FileTreeBrowser } from './FileTreeBrowser'
import { IssuePreviewCard } from './IssuePreviewCard'
import { ChecklistTab } from './ChecklistTab'
import type { ChecklistDraft } from './ChecklistTab'
import { useRepoInfo } from '~/api/repo'
import { useIssuesForMilestone } from '~/api/issues'

export interface QueuedItem {
  file: string
  checklistName: string
  checklistContent: string
  branch: string | null
  createdBy: string | null
}

interface Props {
  opened: boolean
  onClose: () => void
  milestoneNumber: number | null
  onQueue: (item: QueuedItem) => void
}

export function CreateIssueModal({ opened, onClose, milestoneNumber, onQueue }: Props) {
  const [selectedFile, setSelectedFile] = useState<string | null>(null)
  const [checklistDraft, setChecklistDraft] = useState<ChecklistDraft>({ name: '', content: '' })
  const [checklistSelected, setChecklistSelected] = useState(false)
  const { data: repoInfo } = useRepoInfo()
  const { data: milestoneIssues = [] } = useIssuesForMilestone(milestoneNumber)

  // Reset state each time the modal opens
  useEffect(() => {
    if (opened) {
      setSelectedFile(null)
      setChecklistSelected(false)
      setChecklistDraft({ name: '', content: '' })
    }
  }, [opened])

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
              <ChecklistTab
                onChange={setChecklistDraft}
                onSelect={() => setChecklistSelected(true)}
              />
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

        <Group justify="flex-end" pt="sm">
          <Button
            disabled={!selectedFile || (!checklistSelected && !(checklistDraft.name.trim() && checklistDraft.content.trim()))}
            onClick={() => {
              onQueue({
                file: selectedFile!,
                checklistName: checklistDraft.name,
                checklistContent: checklistDraft.content,
                branch: repoInfo?.branch ?? null,
                createdBy: repoInfo?.current_user ?? null,
              })
              onClose()
            }}
          >
            Queue
          </Button>
        </Group>
      </Tabs>
    </Modal>
  )
}

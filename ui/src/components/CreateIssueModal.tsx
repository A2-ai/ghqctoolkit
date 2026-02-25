import { useEffect, useMemo, useState } from 'react'
import { Button, Group, Modal, ScrollArea, Tabs } from '@mantine/core'
import { FileTreeBrowser } from './FileTreeBrowser'
import { IssuePreviewCard } from './IssuePreviewCard'
import { ChecklistTab } from './ChecklistTab'
import { RelevantFilesTab } from './RelevantFilesTab'
import { ReviewersTab } from './ReviewersTab'
import type { ChecklistDraft } from './ChecklistTab'
import { useRepoInfo } from '~/api/repo'
import { useIssuesForMilestone } from '~/api/issues'
import type { RelevantFileKind } from '~/api/issues'
import { toCreateIssueRequest } from '~/api/create'
import { fetchFileContent, fetchIssuePreview } from '~/api/preview'
import { wrapInGithubStyles } from '~/utils/github'

export type { RelevantFileKind }

export interface RelevantFileDraft {
  file: string
  kind: RelevantFileKind
  issueNumber: number | null
  milestoneTitle: string | null
  description: string
}

export interface QueuedItem {
  file: string
  checklistName: string
  checklistContent: string
  branch: string | null
  createdBy: string | null
  milestoneTitle: string | null
  assignees: string[]
  relevantFiles: RelevantFileDraft[]
}

interface Props {
  opened: boolean
  onClose: () => void
  milestoneNumber: number | null
  milestoneTitle: string | null
  onQueue: (item: QueuedItem) => void
  onUpdate: (index: number, item: QueuedItem) => void
  queuedItems: QueuedItem[]
  editItem?: QueuedItem | null
  editIndex?: number | null
}

export function CreateIssueModal({ opened, onClose, milestoneNumber, milestoneTitle, onQueue, onUpdate, queuedItems, editItem, editIndex }: Props) {
  const [selectedFile, setSelectedFile] = useState<string | null>(null)
  const [checklistDraft, setChecklistDraft] = useState<ChecklistDraft>({ name: '', content: '' })
  const [checklistSelected, setChecklistSelected] = useState(false)
  const [assignees, setAssignees] = useState<string[]>([])
  const [relevantFiles, setRelevantFiles] = useState<RelevantFileDraft[]>([])
  const [activeTab, setActiveTab] = useState<string | null>('file')
  const [filePreviewOpen, setFilePreviewOpen] = useState(false)
  const [filePreviewContent, setFilePreviewContent] = useState<string | null>(null)
  const [filePreviewLoading, setFilePreviewLoading] = useState(false)
  const [issuePreviewOpen, setIssuePreviewOpen] = useState(false)
  const [issuePreviewHtml, setIssuePreviewHtml] = useState<string | null>(null)
  const [issuePreviewLoading, setIssuePreviewLoading] = useState(false)
  const { data: repoInfo } = useRepoInfo()
  const { data: milestoneIssues = [] } = useIssuesForMilestone(milestoneNumber)

  // Populate state each time the modal opens (fresh create or edit)
  useEffect(() => {
    if (opened) {
      setActiveTab('file')
      if (editItem) {
        setSelectedFile(editItem.file)
        setChecklistDraft({ name: editItem.checklistName, content: editItem.checklistContent })
        setChecklistSelected(true)
        setAssignees(editItem.assignees)
        setRelevantFiles(editItem.relevantFiles)
      } else {
        setSelectedFile(null)
        setChecklistSelected(false)
        setChecklistDraft({ name: '', content: '' })
        setAssignees([])
        setRelevantFiles([])
      }
    }
  }, [opened])

  async function handleViewFile() {
    if (!selectedFile) return
    setFilePreviewLoading(true)
    try {
      const content = await fetchFileContent(selectedFile)
      setFilePreviewContent(content)
      setFilePreviewOpen(true)
    } catch (err) {
      setFilePreviewContent(`Error: ${(err as Error).message}`)
      setFilePreviewOpen(true)
    } finally {
      setFilePreviewLoading(false)
    }
  }

  async function handlePreviewIssue() {
    if (!selectedFile) return
    const item: QueuedItem = {
      file: selectedFile,
      checklistName: checklistDraft.name,
      checklistContent: checklistDraft.content,
      branch: repoInfo?.branch ?? null,
      createdBy: repoInfo?.current_user ?? null,
      milestoneTitle,
      assignees,
      relevantFiles,
    }
    // Include queued-but-no-issue-number items in the "batch" so they show as New in the preview
    const batchFiles = new Set([
      selectedFile,
      ...relevantFiles.filter(rf => rf.kind !== 'file' && rf.issueNumber === null).map(rf => rf.file),
    ])
    const request = toCreateIssueRequest(item, batchFiles)
    setIssuePreviewLoading(true)
    try {
      const html = await fetchIssuePreview(request)
      setIssuePreviewHtml(html)
      setIssuePreviewOpen(true)
    } catch (err) {
      setIssuePreviewHtml(`<pre>Error: ${(err as Error).message}</pre>`)
      setIssuePreviewOpen(true)
    } finally {
      setIssuePreviewLoading(false)
    }
  }

  const canQueue = !!selectedFile && (checklistSelected || (checklistDraft.name.trim().length > 0 && checklistDraft.content.trim().length > 0))

  // Issue titles ARE the file path (e.g. "scripts/file_b.R"); build a set for O(1) lookup
  const claimedFiles = useMemo<Set<string>>(
    () => new Set([
      ...milestoneIssues.map((i) => i.title),
      ...queuedItems.filter((_, i) => i !== editIndex).map((q) => q.file),
    ]),
    [milestoneIssues, queuedItems, editIndex],
  )

  return (
    <Modal
      opened={opened}
      onClose={onClose}
      title="Create QC Issue"
      size={900}
      centered
      keepMounted
    >
      <Tabs value={activeTab} onChange={setActiveTab} keepMounted={false}>
        <Tabs.List grow>
          <Tabs.Tab value="file">Select a File</Tabs.Tab>
          <Tabs.Tab value="checklist">Select a Checklist</Tabs.Tab>
          <Tabs.Tab value="relevant">Select Relevant Files</Tabs.Tab>
          <Tabs.Tab value="reviewers">Select Reviewer(s)</Tabs.Tab>
        </Tabs.List>

        <Group align="flex-start" gap="md" wrap="nowrap" pt="md">
          <div style={{ flex: '1 1 0', minWidth: 0 }}>
            <Tabs.Panel value="file" keepMounted>
              <FileTreeBrowser
                selectedFile={selectedFile}
                onSelect={setSelectedFile}
                claimedFiles={claimedFiles}
              />
            </Tabs.Panel>
            <Tabs.Panel value="checklist" keepMounted>
              <ChecklistTab
                onChange={setChecklistDraft}
                onSelect={() => setChecklistSelected(true)}
                initialDraft={editItem ? { name: editItem.checklistName, content: editItem.checklistContent } : null}
              />
            </Tabs.Panel>
            <Tabs.Panel value="relevant">
              <RelevantFilesTab
                relevantFiles={relevantFiles}
                onAdd={(draft) => setRelevantFiles((prev) => [...prev, draft])}
                onRemove={(i) => setRelevantFiles((prev) => prev.filter((_, idx) => idx !== i))}
                onUpdate={(i, draft) => setRelevantFiles((prev) => prev.map((rf, idx) => idx === i ? draft : rf))}
                queuedItems={queuedItems}
              />
            </Tabs.Panel>
            <Tabs.Panel value="reviewers">
              <ReviewersTab value={assignees} onChange={setAssignees} />
            </Tabs.Panel>
          </div>

          <div style={{ flex: '0 0 260px' }}>
            <IssuePreviewCard
              file={selectedFile}
              branch={repoInfo?.branch ?? null}
              createdBy={repoInfo?.current_user ?? null}
              checklistName={checklistDraft.name || null}
              assignees={assignees}
              relevantFiles={relevantFiles}
            />
          </div>
        </Group>

        <Group justify="flex-end" pt="sm">
          <Button
            variant="default"
            disabled={!selectedFile}
            loading={filePreviewLoading}
            onClick={handleViewFile}
          >
            View File
          </Button>
          <Button
            variant="default"
            disabled={!canQueue}
            loading={issuePreviewLoading}
            onClick={handlePreviewIssue}
          >
            Preview Issue
          </Button>
          <Button
            disabled={!canQueue}
            onClick={() => {
              const item: QueuedItem = {
                file: selectedFile!,
                checklistName: checklistDraft.name,
                checklistContent: checklistDraft.content,
                branch: repoInfo?.branch ?? null,
                createdBy: repoInfo?.current_user ?? null,
                milestoneTitle,
                assignees,
                relevantFiles,
              }
              if (editIndex != null) {
                onUpdate(editIndex, item)
              } else {
                onQueue(item)
              }
              onClose()
            }}
          >
            {editIndex != null ? 'Update' : 'Queue'}
          </Button>
        </Group>
      </Tabs>

      {/* File content preview */}
      <Modal
        opened={filePreviewOpen}
        onClose={() => setFilePreviewOpen(false)}
        title={selectedFile ?? 'File Preview'}
        size={800}
        centered
      >
        <ScrollArea h={500}>
          <pre style={{
            margin: 0,
            padding: '12px 16px',
            borderRadius: 6,
            background: '#e9ecef',
            color: '#212529',
            fontFamily: 'monospace',
            fontSize: 12,
            lineHeight: 1.6,
            whiteSpace: 'pre-wrap',
            wordBreak: 'break-all',
          }}>
            {filePreviewContent ?? ''}
          </pre>
        </ScrollArea>
      </Modal>

      {/* Issue body HTML preview */}
      <Modal
        opened={issuePreviewOpen}
        onClose={() => setIssuePreviewOpen(false)}
        title="Issue Preview"
        size={800}
        centered
      >
        <iframe
          srcDoc={issuePreviewHtml ? wrapInGithubStyles(issuePreviewHtml) : ''}
          style={{ width: '100%', height: 500, border: '1px solid var(--mantine-color-gray-3)' , borderRadius: 6 }}
          title="Issue Preview"
        />
      </Modal>
    </Modal>
  )
}

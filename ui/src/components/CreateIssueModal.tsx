import { useEffect, useMemo, useState } from 'react'
import { Button, Group, Modal, ScrollArea, Tabs } from '@mantine/core'
import { FileTreeBrowser } from './FileTreeBrowser'
import { IssuePreviewCard } from './IssuePreviewCard'
import { ChecklistTab } from './ChecklistTab'
import { RelevantFilesTab } from './RelevantFilesTab'
import { ReviewersTab } from './ReviewersTab'
import { CollaboratorsTab } from './CollaboratorsTab'
import type { ChecklistDraft } from './ChecklistTab'
import { useRepoInfo } from '~/api/repo'
import { useIssuesForMilestone } from '~/api/issues'
import { useChecklistDisplayName, useConfigurationStatus } from '~/api/configuration'
import { capitalize } from '~/utils/displayName'
import type { RelevantFileKind } from '~/api/issues'
import { toCreateIssueRequest } from '~/api/create'
import { fetchFileContent, fetchIssuePreview } from '~/api/preview'
import { fetchFileCollaborators } from '~/api/files'
import { wrapInGithubStyles } from '~/utils/github'
import { useUiSession } from '~/state/uiSession'

export type { RelevantFileKind }

export interface RelevantFileDraft {
  file: string
  kind: RelevantFileKind
  issueNumber: number | null
  milestoneTitle: string | null
  description: string
  /** Only meaningful for previous_qc entries — whether to post a diff comment. Defaults to true. */
  includeDiff?: boolean
}

export interface QueuedItem {
  file: string
  checklistName: string
  checklistContent: string
  branch: string | null
  createdBy: string | null
  milestoneTitle: string | null
  assignees: string[]
  collaborators: string[]
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
  const { create, setCreate } = useUiSession()
  const [filePreviewLoading, setFilePreviewLoading] = useState(false)
  const [issuePreviewLoading, setIssuePreviewLoading] = useState(false)
  const [collaboratorsLoading, setCollaboratorsLoading] = useState(false)
  const { data: repoInfo } = useRepoInfo()
  const { data: milestoneIssues = [] } = useIssuesForMilestone(milestoneNumber)
  const { data: configStatus } = useConfigurationStatus()
  const { singular } = useChecklistDisplayName()
  const singularCap = capitalize(singular)
  const modal = create.modal
  const includeCollaborators = configStatus?.options.include_collaborators ?? true

  function handleSelectFile(selectedFile: string | null) {
    setCreate((prev) => ({
      ...prev,
      modal: {
        ...prev.modal,
        selectedFile,
        collaboratorAuthor: null,
        collaborators: [],
        collaboratorsSourceFile: null,
      },
    }))
  }

  async function handleViewFile() {
    if (!modal.selectedFile) return
    setFilePreviewLoading(true)
    try {
      const content = await fetchFileContent(modal.selectedFile)
      setCreate((prev) => ({
        ...prev,
        modal: { ...prev.modal, filePreviewContent: content, filePreviewOpen: true },
      }))
    } catch (err) {
      setCreate((prev) => ({
        ...prev,
        modal: { ...prev.modal, filePreviewContent: `Error: ${(err as Error).message}`, filePreviewOpen: true },
      }))
    } finally {
      setFilePreviewLoading(false)
    }
  }

  async function handlePreviewIssue() {
    if (!modal.selectedFile) return
    const item: QueuedItem = {
      file: modal.selectedFile,
      checklistName: modal.checklistDraft.name,
      checklistContent: modal.checklistDraft.content,
      branch: repoInfo?.branch ?? null,
      createdBy: repoInfo?.current_user ?? null,
      milestoneTitle,
      assignees: modal.assignees,
      collaborators: modal.collaborators,
      relevantFiles: modal.relevantFiles,
    }
    // Include queued-but-no-issue-number items in the "batch" so they show as New in the preview
    const batchFiles = new Set([
      modal.selectedFile,
      ...modal.relevantFiles.filter(rf => rf.kind !== 'file' && rf.issueNumber === null).map(rf => rf.file),
    ])
    const request = toCreateIssueRequest(item, batchFiles, includeCollaborators)
    setIssuePreviewLoading(true)
    try {
      const html = await fetchIssuePreview(request)
      setCreate((prev) => ({
        ...prev,
        modal: { ...prev.modal, issuePreviewHtml: html, issuePreviewOpen: true },
      }))
    } catch (err) {
      setCreate((prev) => ({
        ...prev,
        modal: { ...prev.modal, issuePreviewHtml: `<pre>Error: ${(err as Error).message}</pre>`, issuePreviewOpen: true },
      }))
    } finally {
      setIssuePreviewLoading(false)
    }
  }

  const canQueue = !!modal.selectedFile && (
    modal.checklistSelected ||
    (modal.checklistDraft.name.trim().length > 0 && modal.checklistDraft.content.trim().length > 0)
  )

  // Issue titles ARE the file path (e.g. "scripts/file_b.R"); build a set for O(1) lookup
  const claimedFiles = useMemo<Set<string>>(
    () => new Set([
      ...milestoneIssues.map((i) => i.title),
      ...queuedItems.filter((_, i) => i !== editIndex).map((q) => q.file),
    ]),
    [milestoneIssues, queuedItems, editIndex],
  )

  useEffect(() => {
    if (includeCollaborators) return
    if (
      modal.activeTab !== 'collaborators' &&
      modal.collaborators.length === 0 &&
      modal.collaboratorAuthor === null &&
      modal.collaboratorsSourceFile === null
    ) {
      return
    }

    setCreate((prev) => ({
      ...prev,
      modal: {
        ...prev.modal,
        activeTab: prev.modal.activeTab === 'collaborators' ? 'reviewers' : prev.modal.activeTab,
        collaborators: [],
        collaboratorAuthor: null,
        collaboratorsSourceFile: null,
      },
    }))
  }, [
    includeCollaborators,
    modal.activeTab,
    modal.collaboratorAuthor,
    modal.collaborators,
    modal.collaboratorsSourceFile,
    setCreate,
  ])

  useEffect(() => {
    if (!includeCollaborators) {
      setCollaboratorsLoading(false)
      return
    }
    const selectedFile = modal.selectedFile
    if (!selectedFile || modal.collaboratorsSourceFile === selectedFile) return

    let cancelled = false
    setCollaboratorsLoading(true)
    void fetchFileCollaborators(selectedFile)
      .then((response) => {
        if (cancelled) return
        setCreate((prev) => ({
            ...prev,
            modal: {
              ...prev.modal,
              collaboratorAuthor: response.author,
              collaborators: response.collaborators,
              collaboratorsSourceFile: selectedFile,
            },
        }))
      })
      .catch(() => {
        if (cancelled) return
        setCreate((prev) => ({
          ...prev,
            modal: {
              ...prev.modal,
              collaboratorAuthor: null,
              collaborators: [],
              collaboratorsSourceFile: selectedFile,
            },
        }))
      })
      .finally(() => {
        if (!cancelled) setCollaboratorsLoading(false)
      })

    return () => {
      cancelled = true
    }
  }, [includeCollaborators, modal.selectedFile, modal.collaboratorsSourceFile, setCreate])

  return (
    <Modal
      opened={opened}
      onClose={onClose}
      title="Create QC Issue"
      size={900}
      centered
      keepMounted
    >
      <Tabs
        value={modal.activeTab}
        onChange={(activeTab) => setCreate((prev) => ({ ...prev, modal: { ...prev.modal, activeTab } }))}
        keepMounted={false}
      >
        <Tabs.List grow>
          <Tabs.Tab value="file">Select a File</Tabs.Tab>
          <Tabs.Tab value="checklist">Select a {singularCap}</Tabs.Tab>
          <Tabs.Tab value="relevant">Select Relevant Files</Tabs.Tab>
          <Tabs.Tab value="reviewers">Select Reviewer(s)</Tabs.Tab>
          {includeCollaborators && <Tabs.Tab value="collaborators">Select Collaborators</Tabs.Tab>}
        </Tabs.List>

        <Group align="flex-start" gap="md" wrap="nowrap" pt="md">
          <div style={{ flex: '1 1 0', minWidth: 0 }}>
            <Tabs.Panel value="file" keepMounted>
              <FileTreeBrowser
                selectedFile={modal.selectedFile}
                onSelect={handleSelectFile}
                claimedFiles={claimedFiles}
              />
            </Tabs.Panel>
            <Tabs.Panel value="checklist" keepMounted>
              <ChecklistTab
                key={modal.checklistKey}
                onChange={(checklistDraft) => setCreate((prev) => ({ ...prev, modal: { ...prev.modal, checklistDraft } }))}
                onSelect={() => setCreate((prev) => ({ ...prev, modal: { ...prev.modal, checklistSelected: true } }))}
                initialDraft={modal.checklistSelected ? modal.checklistDraft : null}
                persistedCustomTabs={modal.savedCustomTabs}
                onSaveCustom={(tab) => setCreate((prev) => {
                  const idx = prev.modal.savedCustomTabs.findIndex((entry) => entry.name === tab.name)
                  const savedCustomTabs = idx >= 0
                    ? prev.modal.savedCustomTabs.map((entry, i) => i === idx ? tab : entry)
                    : [...prev.modal.savedCustomTabs, tab]
                  return { ...prev, modal: { ...prev.modal, savedCustomTabs } }
                })}
              />
            </Tabs.Panel>
            <Tabs.Panel value="relevant">
              <RelevantFilesTab
                relevantFiles={modal.relevantFiles}
                onAdd={(draft) => setCreate((prev) => ({
                  ...prev,
                  modal: { ...prev.modal, relevantFiles: [...prev.modal.relevantFiles, draft] },
                }))}
                onRemove={(i) => setCreate((prev) => ({
                  ...prev,
                  modal: { ...prev.modal, relevantFiles: prev.modal.relevantFiles.filter((_, idx) => idx !== i) },
                }))}
                onUpdate={(i, draft) => setCreate((prev) => ({
                  ...prev,
                  modal: {
                    ...prev.modal,
                    relevantFiles: prev.modal.relevantFiles.map((rf, idx) => idx === i ? draft : rf),
                  },
                }))}
                queuedItems={queuedItems}
              />
            </Tabs.Panel>
            <Tabs.Panel value="reviewers">
              <ReviewersTab
                value={modal.assignees}
                onChange={(assignees) => setCreate((prev) => ({ ...prev, modal: { ...prev.modal, assignees } }))}
              />
            </Tabs.Panel>
            {includeCollaborators && (
              <Tabs.Panel value="collaborators">
                <CollaboratorsTab
                  author={modal.collaboratorAuthor}
                  collaborators={modal.collaborators}
                  loading={collaboratorsLoading}
                  onAdd={(value) => setCreate((prev) => ({
                    ...prev,
                    modal: {
                      ...prev.modal,
                      collaborators: prev.modal.collaborators.includes(value)
                        ? prev.modal.collaborators
                        : [...prev.modal.collaborators, value],
                    },
                  }))}
                  onRemove={(index) => setCreate((prev) => ({
                    ...prev,
                    modal: {
                      ...prev.modal,
                      collaborators: prev.modal.collaborators.filter((_, i) => i !== index),
                    },
                  }))}
                />
              </Tabs.Panel>
            )}
          </div>

          <div style={{ flex: '0 0 260px' }}>
            <IssuePreviewCard
              file={modal.selectedFile}
              branch={repoInfo?.branch ?? null}
              createdBy={repoInfo?.current_user ?? null}
              checklistName={modal.checklistSelected ? (modal.checklistDraft.name || null) : null}
              assignees={modal.assignees}
              collaborators={modal.collaborators}
              showCollaborators={includeCollaborators}
              relevantFiles={modal.relevantFiles}
            />
          </div>
        </Group>

        <Group justify="flex-end" pt="sm">
          <Button
            variant="default"
            disabled={!modal.selectedFile}
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
                file: modal.selectedFile!,
                checklistName: modal.checklistDraft.name,
                checklistContent: modal.checklistDraft.content,
                branch: repoInfo?.branch ?? null,
                createdBy: repoInfo?.current_user ?? null,
                milestoneTitle,
                assignees: modal.assignees,
                collaborators: includeCollaborators ? modal.collaborators : [],
                relevantFiles: modal.relevantFiles,
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
        opened={modal.filePreviewOpen}
        onClose={() => setCreate((prev) => ({ ...prev, modal: { ...prev.modal, filePreviewOpen: false } }))}
        title={modal.selectedFile ?? 'File Preview'}
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
            {modal.filePreviewContent ?? ''}
          </pre>
        </ScrollArea>
      </Modal>

      {/* Issue body HTML preview */}
      <Modal
        opened={modal.issuePreviewOpen}
        onClose={() => setCreate((prev) => ({ ...prev, modal: { ...prev.modal, issuePreviewOpen: false } }))}
        title="Issue Preview"
        size={800}
        centered
      >
        <iframe
          srcDoc={modal.issuePreviewHtml ? wrapInGithubStyles(modal.issuePreviewHtml) : ''}
          style={{ width: '100%', height: 500, border: '1px solid var(--mantine-color-gray-3)' , borderRadius: 6 }}
          title="Issue Preview"
        />
      </Modal>
    </Modal>
  )
}

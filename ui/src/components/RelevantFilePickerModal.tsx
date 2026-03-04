import { useEffect, useState } from 'react'
import { Box, Button, Group, Loader, Modal, Select, Text, TextInput, Tooltip } from '@mantine/core'
import { FileTreeBrowser } from './FileTreeBrowser'
import type { RelevantFileDraft } from './CreateIssueModal'
import type { RelevantFileKind } from '~/api/issues'

import type { IssueRef } from './RelevantFilesTab'
import type { QueuedItem } from './CreateIssueModal'

interface Props {
  opened: boolean
  onClose: () => void
  onAdd: (draft: RelevantFileDraft) => void
  fileToIssues: Map<string, IssueRef[]>
  queuedItems: QueuedItem[]
  alreadyAdded: Set<string>
  isLoading: boolean
  editDraft?: RelevantFileDraft | null
}

type Relation = 'gating' | 'relevant' | 'previous'

function toKind(relation: Relation): RelevantFileKind {
  return relation === 'relevant' ? 'relevant_qc' : 'blocking_qc'
}

const TYPE_TOOLTIPS: Record<string, string> = {
  gating: 'A QC which influences this QC and must be approved before the current QC.',
  previous: 'A prior QC of the same, or a similar, file. Must be approved before the current QC.',
  relevant: 'A related QC that provides useful context but is not blocking.',
  file: 'A related file with no associated QC issue.',
}

const TYPE_DATA_WITH_ISSUE = [
  { value: 'gating', label: 'Gating QC' },
  { value: 'previous', label: 'Previous QC' },
  { value: 'relevant', label: 'Relevant QC' },
  { value: 'file', label: 'File', disabled: true },
]

const TYPE_DATA_FILE_ONLY = [
  { value: 'file', label: 'File' },
]

export function RelevantFilePickerModal({ opened, onClose, onAdd, fileToIssues, queuedItems, alreadyAdded, isLoading, editDraft }: Props) {
  const [selectedFile, setSelectedFile] = useState<string | null>(null)
  const [selectedIssue, setSelectedIssue] = useState<string | null>(null)
  const [relation, setRelation] = useState<Relation>('gating')
  const [description, setDescription] = useState('')

  function reset() {
    setSelectedFile(null)
    setSelectedIssue(null)
    setRelation('gating')
    setDescription('')
  }

  useEffect(() => {
    if (!opened) return
    if (editDraft) {
      setSelectedFile(editDraft.file)
      setDescription(editDraft.description)
      if (editDraft.kind === 'file') {
        setSelectedIssue(null)
        setRelation('gating')
      } else {
        setRelation(editDraft.kind === 'relevant_qc' ? 'relevant' : 'gating')
        setSelectedIssue(editDraft.issueNumber !== null ? String(editDraft.issueNumber) : 'queued')
      }
    } else {
      reset()
    }
  }, [opened])

  function handleClose() {
    reset()
    onClose()
  }

  function handleSelectFile(file: string | null) {
    setSelectedFile(file)
    setSelectedIssue(null)
    setRelation('gating')
    setDescription('')
  }

  const queuedFileSet = new Set(queuedItems.map((item) => item.file))

  // Build file annotation map: issue numbers + queued badge
  const fileAnnotations = new Map<string, string[]>()
  for (const [file, refs] of fileToIssues) {
    const labels: string[] = []
    const shown = refs.slice(0, 3).map((r) => `#${r.number}`)
    labels.push(...shown)
    if (refs.length > 3) labels.push(`+${refs.length - 3} more`)
    if (queuedFileSet.has(file)) labels.push('queued')
    if (labels.length > 0) fileAnnotations.set(file, labels)
  }
  for (const file of queuedFileSet) {
    if (!fileAnnotations.has(file)) fileAnnotations.set(file, ['queued'])
  }

  const fileIssues = selectedFile ? (fileToIssues.get(selectedFile) ?? []) : []
  const queuedItem = queuedItems.find((item) => item.file === selectedFile) ?? null
  const hasIssues = fileIssues.length > 0 || queuedItem !== null
  const issueSelected = selectedIssue !== null && selectedIssue !== 'no_issue'
  const useFileType = !issueSelected

  // Build Issue dropdown: queued item first, then existing issues, then "No issue"
  const issueData = hasIssues ? [
    ...(queuedItem ? [{
      value: 'queued',
      label: queuedItem.milestoneTitle ? `Queued · ${queuedItem.milestoneTitle}` : 'Queued',
    }] : []),
    ...fileIssues.map((r) => ({
      value: String(r.number),
      label: r.milestone ? `#${r.number} · ${r.milestone}` : `#${r.number}`,
    })),
    { value: 'no_issue', label: 'No issue' },
  ] : []

  const canAdd =
    selectedFile !== null &&
    (useFileType ? description.trim().length > 0 : true)

  function handleAdd() {
    if (!selectedFile || !canAdd) return
    let draft: RelevantFileDraft
    if (useFileType) {
      draft = { file: selectedFile, kind: 'file', issueNumber: null, milestoneTitle: null, description: description.trim() }
    } else if (selectedIssue === 'queued') {
      draft = { file: selectedFile, kind: toKind(relation), issueNumber: null, milestoneTitle: queuedItem?.milestoneTitle ?? null, description: description.trim() }
    } else {
      const ref = fileIssues.find((r) => String(r.number) === selectedIssue)
      draft = { file: selectedFile, kind: toKind(relation), issueNumber: Number(selectedIssue), milestoneTitle: ref?.milestone ?? null, description: description.trim() }
    }
    onAdd(draft)
    reset()
    onClose()
  }

  return (
    <Modal opened={opened} onClose={handleClose} title={editDraft ? 'Edit Relevant File' : 'Add Relevant File'} size={860} centered>
      <Group align="stretch" gap="md" wrap="nowrap" style={{ height: 400 }}>
        {/* Left: file tree */}
        <div style={{ flex: 1, minWidth: 0, overflow: 'hidden' }}>
          <FileTreeBrowser
            selectedFile={selectedFile}
            onSelect={handleSelectFile}
            claimedFiles={alreadyAdded}
            fileAnnotations={fileAnnotations}
          />
        </div>

        {/* Right: form panel */}
        <div style={{ flex: '0 0 260px', height: '100%', display: 'flex', flexDirection: 'column', gap: 12 }}>
          {isLoading && (
            <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
              <Loader size={12} />
              <Text size="xs" c="dimmed">Loading issue data…</Text>
            </div>
          )}

          {selectedFile === null ? (
            <Text size="sm" c="dimmed">Select a file from the tree</Text>
          ) : (
            <>
              {/* Issue */}
              <Select
                label="Issue"
                size="xs"
                disabled={!hasIssues}
                placeholder={hasIssues ? 'Select an issue' : 'No issues found'}
                data={issueData}
                value={selectedIssue}
                onChange={setSelectedIssue}
                clearable={hasIssues}
              />

              {/* Type */}
              <Select
                label="Type"
                size="xs"
                disabled={!issueSelected}
                value={useFileType ? 'file' : relation}
                onChange={(v) => { if (v && v !== 'file') setRelation(v as Relation) }}
                data={issueSelected ? TYPE_DATA_WITH_ISSUE : TYPE_DATA_FILE_ONLY}
                renderOption={({ option }) => (
                  <Tooltip
                    label={TYPE_TOOLTIPS[option.value] ?? ''}
                    position="right"
                    withArrow
                    disabled={!TYPE_TOOLTIPS[option.value]}
                  >
                    <Box style={{ flex: 1 }}>{option.label}</Box>
                  </Tooltip>
                )}
              />

              {/* Description */}
              <TextInput
                label="Description"
                size="xs"
                withAsterisk={useFileType}
                placeholder={useFileType ? 'Required justification' : 'Optional note'}
                value={description}
                onChange={(e) => setDescription(e.currentTarget.value)}
              />

              <Button size="xs" disabled={!canAdd} onClick={handleAdd} mt="auto">
                {editDraft ? 'Update' : 'Add'}
              </Button>
            </>
          )}
        </div>
      </Group>
    </Modal>
  )
}

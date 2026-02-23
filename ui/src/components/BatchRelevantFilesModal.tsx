import { useEffect, useState } from 'react'
import { Badge, Box, Button, Checkbox, Group, Loader, Modal, Select, Stack, Text, TextInput, Tooltip } from '@mantine/core'
import { IconFile, IconLink, IconLock } from '@tabler/icons-react'
import { FileTreeBrowser } from './FileTreeBrowser'
import { useMilestones } from '~/api/milestones'
import { useAllMilestoneIssues } from '~/api/issues'
import type { RelevantFileDraft, QueuedItem } from './CreateIssueModal'
import type { IssueRef } from './RelevantFilesTab'
import type { RelevantFileKind } from '~/api/issues'

interface Props {
  opened: boolean
  onClose: () => void
  queuedItems: QueuedItem[]
  onApply: (indices: number[], draft: RelevantFileDraft) => void
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

interface SelectableIssueCardProps {
  item: QueuedItem
  checked: boolean
  onToggle: () => void
  alreadyHasFile: boolean
}

function SelectableIssueCard({ item, checked, onToggle, alreadyHasFile }: SelectableIssueCardProps) {
  const borderColor = checked ? '#2f9e44' : '#22b8cf'
  const bgColor = checked ? '#ebfbee' : '#f0fafb'
  const hoverBorder = checked ? '#2b8a3e' : '#0c8599'
  const hoverBg = checked ? '#d3f9d8' : '#e0f5f8'

  return (
    <div
      onClick={onToggle}
      style={{
        padding: '9px 11px',
        borderRadius: 6,
        border: `1px dashed ${borderColor}`,
        backgroundColor: bgColor,
        cursor: 'pointer',
        transition: 'background-color 0.12s, border-color 0.12s',
        boxSizing: 'border-box',
      }}
      onMouseEnter={(e) => {
        e.currentTarget.style.backgroundColor = hoverBg
        e.currentTarget.style.borderColor = hoverBorder
      }}
      onMouseLeave={(e) => {
        e.currentTarget.style.backgroundColor = bgColor
        e.currentTarget.style.borderColor = borderColor
      }}
    >
      {/* Title row */}
      <div style={{ display: 'flex', alignItems: 'flex-start', gap: 6, marginBottom: 4 }}>
        <Checkbox
          size="xs"
          checked={checked}
          onChange={onToggle}
          onClick={(e) => e.stopPropagation()}
          style={{ marginTop: 2, flexShrink: 0 }}
        />
        <Text size="sm" fw={700} style={{ wordBreak: 'break-all', flex: 1, lineHeight: 1.3 }}>
          {item.file}
        </Text>
        <div style={{ display: 'flex', gap: 4, flexShrink: 0 }}>
          {alreadyHasFile && (
            <Badge size="xs" color="gray" variant="light">Already added</Badge>
          )}
          <Badge size="xs" color="cyan" variant="light">Queued</Badge>
        </div>
      </div>

      {item.branch && (
        <Text size="xs" c="dimmed"><b>Branch:</b> {item.branch}</Text>
      )}
      {item.createdBy && (
        <Text size="xs" c="dimmed"><b>Created by:</b> {item.createdBy}</Text>
      )}
      {item.checklistName && (
        <Text size="xs" c="dimmed"><b>Checklist:</b> {item.checklistName}</Text>
      )}
      {item.assignees.length > 0 && (
        <Text size="xs" c="dimmed">
          <b>Reviewer{item.assignees.length > 1 ? 's' : ''}:</b> {item.assignees.join(', ')}
        </Text>
      )}

      {item.relevantFiles.length > 0 && (
        <>
          <Text size="xs" fw={600} c="dimmed" mt={4} mb={2}>Relevant Files</Text>
          {item.relevantFiles.map((rf, i) => {
            const icon =
              rf.kind === 'blocking_qc' ? <IconLock size={11} color="#c92a2a" /> :
              rf.kind === 'relevant_qc' ? <IconLink size={11} color="#666" /> :
              <IconFile size={11} color="#666" />
            return (
              <div key={i} style={{ display: 'flex', alignItems: 'center', gap: 4 }}>
                {icon}
                <Text size="xs" c="dimmed" style={{ wordBreak: 'break-all' }}>
                  {rf.file}{rf.issueNumber !== null ? ` #${rf.issueNumber}` : ''}
                </Text>
              </div>
            )
          })}
        </>
      )}
    </div>
  )
}

export function BatchRelevantFilesModal({ opened, onClose, queuedItems, onApply }: Props) {
  const [selectedFile, setSelectedFile] = useState<string | null>(null)
  const [selectedIssue, setSelectedIssue] = useState<string | null>(null)
  const [relation, setRelation] = useState<Relation>('gating')
  const [description, setDescription] = useState('')
  const [checkedIndices, setCheckedIndices] = useState<Set<number>>(new Set())

  const { data: allMilestones = [] } = useMilestones()
  const allMilestoneNumbers = allMilestones.map((m) => m.number)
  const { issues: allIssues, isLoading } = useAllMilestoneIssues(allMilestoneNumbers)

  function reset() {
    setSelectedFile(null)
    setSelectedIssue(null)
    setRelation('gating')
    setDescription('')
    setCheckedIndices(new Set())
  }

  useEffect(() => {
    if (!opened) return
    reset()
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

  // Build file → issue refs map
  const fileToIssues = new Map<string, IssueRef[]>()
  for (const issue of allIssues) {
    const file = issue.title
    if (!fileToIssues.has(file)) fileToIssues.set(file, [])
    fileToIssues.get(file)!.push({ number: issue.number, milestone: issue.milestone })
  }
  for (const refs of fileToIssues.values()) {
    refs.sort((a, b) => b.number - a.number)
  }

  // File annotations
  const queuedFileSet = new Set(queuedItems.map((item) => item.file))
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

  const allChecked = checkedIndices.size === queuedItems.length && queuedItems.length > 0
  const someChecked = checkedIndices.size > 0 && !allChecked

  function toggleSelectAll() {
    if (allChecked) {
      setCheckedIndices(new Set())
    } else {
      setCheckedIndices(new Set(queuedItems.map((_, i) => i)))
    }
  }

  function toggleIndex(i: number) {
    setCheckedIndices(prev => {
      const next = new Set(prev)
      if (next.has(i)) next.delete(i)
      else next.add(i)
      return next
    })
  }

  const canApply =
    selectedFile !== null &&
    checkedIndices.size > 0 &&
    (useFileType ? description.trim().length > 0 : true)

  function handleApply() {
    if (!selectedFile || !canApply) return
    let draft: RelevantFileDraft
    if (useFileType) {
      draft = { file: selectedFile, kind: 'file', issueNumber: null, milestoneTitle: null, description: description.trim() }
    } else if (selectedIssue === 'queued') {
      draft = { file: selectedFile, kind: toKind(relation), issueNumber: null, milestoneTitle: queuedItem?.milestoneTitle ?? null, description: description.trim() }
    } else {
      const ref = fileIssues.find((r) => String(r.number) === selectedIssue)
      draft = { file: selectedFile, kind: toKind(relation), issueNumber: Number(selectedIssue), milestoneTitle: ref?.milestone ?? null, description: description.trim() }
    }
    onApply([...checkedIndices], draft)
    reset()
    onClose()
  }

  return (
    <Modal opened={opened} onClose={handleClose} title="Batch Apply Relevant Files" size={1200} centered>
      <Group align="stretch" gap="md" wrap="nowrap" style={{ height: 520 }}>
        {/* Left: file tree */}
        <div style={{ flex: 1, minWidth: 0, overflow: 'hidden' }}>
          <FileTreeBrowser
            selectedFile={selectedFile}
            onSelect={handleSelectFile}
            claimedFiles={new Set()}
            fileAnnotations={fileAnnotations}
          />
        </div>

        {/* Middle: form panel */}
        <div style={{ flex: '0 0 220px', height: '100%', display: 'flex', flexDirection: 'column', gap: 12 }}>
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

              <TextInput
                label="Description"
                size="xs"
                withAsterisk={useFileType}
                placeholder={useFileType ? 'Required justification' : 'Optional note'}
                value={description}
                onChange={(e) => setDescription(e.currentTarget.value)}
              />
            </>
          )}
        </div>

        {/* Right: selectable issue preview cards */}
        <div style={{
          flex: '0 0 320px',
          height: '100%',
          display: 'flex',
          flexDirection: 'column',
          borderLeft: '1px solid var(--mantine-color-gray-3)',
          paddingLeft: 12,
        }}>
          {/* Header + Select All */}
          <div style={{
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'space-between',
            paddingBottom: 8,
            borderBottom: '1px solid var(--mantine-color-gray-3)',
            marginBottom: 8,
          }}>
            <Text size="xs" fw={600}>Apply to Issues</Text>
            <Checkbox
              size="xs"
              checked={allChecked}
              indeterminate={someChecked}
              onChange={toggleSelectAll}
              label={<Text size="xs" fw={500}>Select All</Text>}
            />
          </div>

          {/* Cards */}
          <div style={{ flex: 1, overflowY: 'auto' }}>
            {queuedItems.length === 0 ? (
              <Text size="xs" c="dimmed">No issues queued</Text>
            ) : (
              <Stack gap={8}>
                {queuedItems.map((item, i) => (
                  <SelectableIssueCard
                    key={i}
                    item={item}
                    checked={checkedIndices.has(i)}
                    onToggle={() => toggleIndex(i)}
                    alreadyHasFile={selectedFile !== null && item.relevantFiles.some(rf => rf.file === selectedFile)}
                  />
                ))}
              </Stack>
            )}
          </div>

          {/* Apply button */}
          <div style={{ paddingTop: 8 }}>
            <Button size="xs" fullWidth disabled={!canApply} onClick={handleApply}>
              Apply
            </Button>
          </div>
        </div>
      </Group>
    </Modal>
  )
}

import { useEffect, useState } from 'react'
import { useMilestones } from '~/api/milestones'
import { useAllMilestoneIssues } from '~/api/issues'
import type { RelevantFileDraft } from './CreateIssueModal'
import type { QueuedItem } from './CreateIssueModal'
import { RelevantFileCard } from './RelevantFileCard'
import { RelevantFilePickerModal } from './RelevantFilePickerModal'

export interface IssueRef {
  number: number
  milestone: string | null
}

interface Props {
  relevantFiles: RelevantFileDraft[]
  onAdd: (draft: RelevantFileDraft) => void
  onRemove: (index: number) => void
  onUpdate: (index: number, draft: RelevantFileDraft) => void
  queuedItems: QueuedItem[]
}

export function RelevantFilesTab({ relevantFiles, onAdd, onRemove, onUpdate, queuedItems }: Props) {
  const [pickerOpen, setPickerOpen] = useState(false)
  const [editingIndex, setEditingIndex] = useState<number | null>(null)
  const { data: allMilestones = [] } = useMilestones()
  const allMilestoneNumbers = allMilestones.map((m) => m.number)
  const { issues: allIssues, isLoading } = useAllMilestoneIssues(allMilestoneNumbers)

  useEffect(() => {
    console.log('[RelevantFilesTab] mounted — milestones:', allMilestoneNumbers.length, 'isLoading:', isLoading)
  }, [])

  useEffect(() => {
    console.log('[RelevantFilesTab] isLoading:', isLoading, '— issues loaded:', allIssues.length)
  }, [isLoading])

  // Build file → issue refs map (number + milestone name) from all milestone issues
  const fileToIssues = new Map<string, IssueRef[]>()
  for (const issue of allIssues) {
    const file = issue.title
    if (!fileToIssues.has(file)) fileToIssues.set(file, [])
    fileToIssues.get(file)!.push({ number: issue.number, milestone: issue.milestone })
  }
  // Highest issue number first
  for (const refs of fileToIssues.values()) {
    refs.sort((a, b) => b.number - a.number)
  }

  // Files already added as relevant (disable in picker); exclude the one being edited
  const alreadyAdded = new Set(relevantFiles.filter((_, i) => i !== editingIndex).map((rf) => rf.file))
  const editDraft = editingIndex !== null ? relevantFiles[editingIndex] : null

  function handlePickerSave(draft: RelevantFileDraft) {
    if (editingIndex !== null) {
      onUpdate(editingIndex, draft)
    } else {
      onAdd(draft)
    }
    setEditingIndex(null)
  }

  function handlePickerClose() {
    setPickerOpen(false)
    setEditingIndex(null)
  }

  return (
    <>
      <div
        style={{
          display: 'grid',
          gridTemplateColumns: 'repeat(auto-fill, minmax(160px, 1fr))',
          gridAutoRows: '96px',
          gap: 8,
        }}
      >
        {/* Add card */}
        <div
          onClick={() => setPickerOpen(true)}
          style={{
            height: '100%',
            borderRadius: 6,
            border: '2px dashed #74c69d',
            backgroundColor: '#f0faf4',
            display: 'flex',
            flexDirection: 'column',
            alignItems: 'center',
            justifyContent: 'center',
            gap: 4,
            cursor: 'pointer',
            boxSizing: 'border-box',
            color: '#2f7a3b',
            transition: 'background-color 0.15s, border-color 0.15s',
          }}
          onMouseEnter={(e) => {
            e.currentTarget.style.backgroundColor = '#d3f0df'
            e.currentTarget.style.borderColor = '#2f7a3b'
          }}
          onMouseLeave={(e) => {
            e.currentTarget.style.backgroundColor = '#f0faf4'
            e.currentTarget.style.borderColor = '#74c69d'
          }}
        >
          <span style={{ fontSize: 22, lineHeight: 1, fontWeight: 300 }}>+</span>
          <span style={{ fontSize: 11, fontWeight: 600, letterSpacing: 0.2 }}>Add Relevant File</span>
        </div>

        {/* Existing relevant file cards */}
        {[...relevantFiles].reverse().map((draft, i) => {
          const originalIndex = relevantFiles.length - 1 - i
          return (
            <RelevantFileCard
              key={originalIndex}
              draft={draft}
              onRemove={() => onRemove(originalIndex)}
              onEdit={() => { setEditingIndex(originalIndex); setPickerOpen(true) }}
            />
          )
        })}
      </div>

      <RelevantFilePickerModal
        opened={pickerOpen}
        onClose={handlePickerClose}
        onAdd={handlePickerSave}
        fileToIssues={fileToIssues}
        queuedItems={queuedItems}
        alreadyAdded={alreadyAdded}
        isLoading={isLoading}
        editDraft={editDraft}
      />
    </>
  )
}

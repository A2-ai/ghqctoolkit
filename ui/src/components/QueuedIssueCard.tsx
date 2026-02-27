import { Badge, Stack, Text, Tooltip } from '@mantine/core'
import { IconFile, IconLink, IconLock } from '@tabler/icons-react'
import type { QueuedItem, RelevantFileDraft } from './CreateIssueModal'

interface Props {
  item: QueuedItem
  onEdit?: () => void
  onRemove?: () => void
  conflictReason?: string
  dirty?: boolean
}

export function QueuedIssueCard({ item, onEdit, onRemove, conflictReason, dirty }: Props) {
  const conflict = !!conflictReason
  const baseBorder = conflict ? '#e03131' : dirty ? '#f59f00' : '#22b8cf'
  const baseBg    = conflict ? '#fff5f5' : dirty ? '#fff9db' : '#f0fafb'
  const hoverBorder = conflict ? '#c92a2a' : dirty ? '#e67700' : '#0c8599'
  const hoverBg     = conflict ? '#ffe3e3' : dirty ? '#fff3bf' : '#e0f5f8'

  const tooltipLabel = conflictReason ?? (dirty ? 'This file has uncommitted local changes' : '')
  const tooltipColor = conflict ? 'red' : 'yellow'

  return (
    <Tooltip label={tooltipLabel} withArrow color={tooltipColor} disabled={!conflict && !dirty} multiline maw={260}>
    <Stack
      gap={5}
      onClick={onEdit}
      style={{
        padding: '10px 12px',
        borderRadius: 6,
        border: `1px dashed ${baseBorder}`,
        backgroundColor: baseBg,
        minWidth: 0,
        height: '100%',
        overflowY: 'auto',
        boxSizing: 'border-box',
        cursor: onEdit ? 'pointer' : 'default',
        transition: 'background-color 0.15s, border-color 0.15s',
      }}
      onMouseEnter={(e) => { if (onEdit) { e.currentTarget.style.backgroundColor = hoverBg; e.currentTarget.style.borderColor = hoverBorder } }}
      onMouseLeave={(e) => { if (onEdit) { e.currentTarget.style.backgroundColor = baseBg; e.currentTarget.style.borderColor = baseBorder } }}
    >
      {/* Title row */}
      <div style={{ display: 'flex', alignItems: 'flex-start', gap: 6 }}>
        <Text size="sm" fw={700} style={{ wordBreak: 'break-all', flex: 1 }}>
          {item.file}
        </Text>
        <Badge size="xs" color="cyan" variant="light" style={{ flexShrink: 0 }}>
          Queued
        </Badge>
        {onRemove && (
          <button
            onClick={(e) => { e.stopPropagation(); onRemove() }}
            style={{
              background: 'none',
              border: 'none',
              cursor: 'pointer',
              padding: '0 2px',
              fontSize: 14,
              lineHeight: 1,
              color: '#868e96',
              flexShrink: 0,
            }}
            aria-label="Remove"
          >
            ×
          </button>
        )}
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
          <Text size="xs" fw={600} c="dimmed" mt={2}>Relevant Files</Text>
          {item.relevantFiles.map((rf, i) => (
            <QueuedRelevantFileLine key={i} draft={rf} />
          ))}
        </>
      )}
    </Stack>
    </Tooltip>
  )
}

function QueuedRelevantFileLine({ draft }: { draft: RelevantFileDraft }) {
  const icon =
    draft.kind === 'blocking_qc' ? <IconLock size={12} color="#c92a2a" /> :
    draft.kind === 'relevant_qc' ? <IconLink size={12} color="#666" /> :
    <IconFile size={12} color="#666" />

  return (
    <div style={{ display: 'flex', alignItems: 'center', gap: 5 }}>
      {icon}
      <Text size="xs" c="dimmed" style={{ wordBreak: 'break-all' }}>
        {draft.file}{draft.issueNumber !== null ? ` #${draft.issueNumber}` : ''}
      </Text>
    </div>
  )
}

import { Badge, Stack, Text } from '@mantine/core'
import type { RelevantFileDraft } from './CreateIssueModal'

interface Props {
  draft: RelevantFileDraft
  onRemove: () => void
  onEdit?: () => void
}

const KIND_LABELS: Record<RelevantFileDraft['kind'], string> = {
  blocking_qc: 'Gating QC',
  relevant_qc: 'Relevant QC',
  file: 'File',
}

const KIND_COLORS: Record<RelevantFileDraft['kind'], string> = {
  blocking_qc: 'red',
  relevant_qc: 'blue',
  file: 'gray',
}

export function RelevantFileCard({ draft, onRemove, onEdit }: Props) {
  return (
    <Stack
      gap={3}
      onClick={onEdit}
      style={{
        padding: '7px 10px',
        borderRadius: 6,
        border: '1px solid #dee2e6',
        backgroundColor: '#fff',
        minWidth: 0,
        height: '100%',
        overflowY: 'auto',
        boxSizing: 'border-box',
        cursor: onEdit ? 'pointer' : 'default',
        transition: 'background-color 0.15s, border-color 0.15s',
      }}
      onMouseEnter={(e) => { if (onEdit) { e.currentTarget.style.backgroundColor = '#f1f3f5'; e.currentTarget.style.borderColor = '#adb5bd' } }}
      onMouseLeave={(e) => { if (onEdit) { e.currentTarget.style.backgroundColor = '#fff'; e.currentTarget.style.borderColor = '#dee2e6' } }}
    >
      {/* Top row: file path + × button */}
      <div style={{ display: 'flex', alignItems: 'flex-start', gap: 4 }}>
        <Text size="xs" fw={700} style={{ wordBreak: 'break-all', flex: 1 }}>
          {draft.file}
        </Text>
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
      </div>

      {/* Kind badge + optional issue number on same row */}
      <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
        <Badge size="xs" color={KIND_COLORS[draft.kind]} variant="light">
          {KIND_LABELS[draft.kind]}
        </Badge>
        {draft.issueNumber !== null && (
          <Text size="xs" c="dimmed">#{draft.issueNumber}</Text>
        )}
      </div>

      {/* Milestone */}
      {draft.milestoneTitle && (
        <Text size="xs" c="dimmed">{draft.milestoneTitle}</Text>
      )}

      {/* Description */}
      {draft.description && (
        <Text size="xs" c="dimmed" fs="italic" style={{ wordBreak: 'break-all' }}>
          {draft.description}
        </Text>
      )}
    </Stack>
  )
}

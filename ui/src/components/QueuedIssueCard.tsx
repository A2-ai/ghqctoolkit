import { Badge, Stack, Text } from '@mantine/core'
import type { QueuedItem } from './CreateIssueModal'

interface Props {
  item: QueuedItem
}

export function QueuedIssueCard({ item }: Props) {
  return (
    <Stack
      gap={5}
      style={{
        padding: '10px 12px',
        borderRadius: 6,
        border: '1px dashed #22b8cf',
        backgroundColor: '#f0fafb',
        minWidth: 0,
        height: '100%',
        overflowY: 'auto',
        boxSizing: 'border-box',
      }}
    >
      {/* Title row */}
      <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
        <Text size="sm" fw={700} style={{ wordBreak: 'break-all', flex: 1 }}>
          {item.file}
        </Text>
        <Badge size="xs" color="cyan" variant="light" style={{ flexShrink: 0 }}>
          Queued
        </Badge>
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
    </Stack>
  )
}

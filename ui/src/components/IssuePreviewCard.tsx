import { Stack, Text } from '@mantine/core'

interface Props {
  file: string | null
  branch: string | null
  createdBy: string | null
  checklistName?: string | null
  assignees?: string[]
}

function PlaceholderRow({ label, value }: { label: string; value: string | null }) {
  return (
    <Text size="xs" c={value ? 'dimmed' : 'gray.4'}>
      <b>{label}:</b> {value ?? '—'}
    </Text>
  )
}

export function IssuePreviewCard({
  file,
  branch,
  createdBy,
  checklistName = null,
  assignees = [],
}: Props) {
  const title = file ? `${file}` : null

  return (
    <Stack
      gap={5}
      style={{
        padding: '10px 12px',
        borderRadius: 6,
        border: '1px solid var(--mantine-color-gray-3)',
        backgroundColor: 'white',
        height: '100%',
        overflowY: 'auto',
        boxSizing: 'border-box',
      }}
    >
      {/* Title */}
      <Text
        size="sm"
        fw={700}
        style={{
          wordBreak: 'break-all',
          color: title ? '#1c7ed6' : 'var(--mantine-color-gray-4)',
        }}
      >
        {title ?? '[file]'}
      </Text>

      <PlaceholderRow label="Branch" value={branch} />
      <PlaceholderRow label="Created by" value={createdBy} />
      <PlaceholderRow label="Checklist" value={checklistName} />

      {assignees.length > 0 ? (
        <Text size="xs" c="dimmed">
          <b>Reviewer{assignees.length > 1 ? 's' : ''}:</b> {assignees.join(', ')}
        </Text>
      ) : (
        <Text size="xs" c="gray.4">
          <b>Reviewers:</b> —
        </Text>
      )}
    </Stack>
  )
}

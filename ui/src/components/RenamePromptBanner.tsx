import { Alert, Button, Divider, Group, Stack, Text } from '@mantine/core'
import { IconArrowRight, IconAlertCircle } from '@tabler/icons-react'
import type { DetectedRenameWithMilestone } from '~/api/issues'

interface Props {
  renames: DetectedRenameWithMilestone[]
  onConfirm: (issueNumber: number, newPath: string) => void
  onDismiss: (issueNumber: number) => void
  confirming: Set<number>
  getMilestoneName: (milestoneNumber: number) => string
}

export function RenamePromptBanner({ renames, onConfirm, onDismiss, confirming, getMilestoneName }: Props) {
  if (renames.length === 0) return null

  const title = renames.length === 1 ? 'File rename detected' : `${renames.length} file renames detected`

  return (
    <Alert icon={<IconAlertCircle size={16} />} color="yellow" variant="light" title={title} mb="sm">
      <Stack gap={6}>
        {renames.map((r, i) => (
          <div key={r.issue_number}>
            {i > 0 && <Divider color="yellow.3" mb={6} />}
            <Group justify="space-between" align="center" wrap="nowrap">
              <Stack gap={2}>
                <Group gap={4} align="center" wrap="nowrap">
                  <Text span fw={600} ff="monospace" size="sm">{r.old_path}</Text>
                  <IconArrowRight size={12} />
                  <Text span fw={600} ff="monospace" size="sm">{r.new_path}</Text>
                </Group>
                <Text size="xs" c="dimmed">
                  Issue #{r.issue_number} · {getMilestoneName(r.milestone_number)}
                </Text>
              </Stack>
              <Group gap="xs" wrap="nowrap" style={{ flexShrink: 0 }}>
                <Button
                  size="xs"
                  variant="filled"
                  color="yellow"
                  loading={confirming.has(r.issue_number)}
                  onClick={() => onConfirm(r.issue_number, r.new_path)}
                >
                  Confirm
                </Button>
                <Button
                  size="xs"
                  variant="subtle"
                  color="gray"
                  disabled={confirming.has(r.issue_number)}
                  onClick={() => onDismiss(r.issue_number)}
                >
                  Dismiss
                </Button>
              </Group>
            </Group>
          </div>
        ))}
      </Stack>
    </Alert>
  )
}

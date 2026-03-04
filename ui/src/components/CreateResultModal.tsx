import { Anchor, Button, Group, Modal, Stack, Text } from '@mantine/core'
import { IconCheck, IconFlag, IconX } from '@tabler/icons-react'

export interface CreatedIssue {
  file: string
  issueUrl: string
  issueNumber: number
}

export type CreateOutcome =
  | { ok: true; milestoneNumber: number; milestoneTitle: string; milestoneCreated: boolean; created: CreatedIssue[] }
  | { ok: false; error: string }

interface Props {
  opened: boolean
  outcome: CreateOutcome | null
  onClose: () => void
  onDone: (milestoneNumber: number) => void   // called on success close — clears the queue
}

export function CreateResultModal({ opened, outcome, onClose, onDone }: Props) {
  if (!outcome) return null

  const title = outcome.ok
    ? `${outcome.created.length} QC Issue${outcome.created.length !== 1 ? 's' : ''} Created`
    : 'Failed to Create Issues'

  return (
    <Modal opened={opened} onClose={onClose} title={title} size={560} centered>
      <Stack gap="sm">
        {outcome.ok ? (
          <>
            {outcome.milestoneCreated && (
              <div
                style={{
                  display: 'flex',
                  alignItems: 'center',
                  gap: 8,
                  padding: '6px 10px',
                  borderRadius: 6,
                  border: '1px solid #bac8ff',
                  backgroundColor: '#edf2ff',
                }}
              >
                <IconFlag size={14} color="#4263eb" style={{ flexShrink: 0 }} />
                <Text size="sm">
                  New milestone created: <b>{outcome.milestoneTitle}</b>
                </Text>
              </div>
            )}
            {outcome.created.map((item) => (
              <div
                key={item.issueNumber}
                style={{
                  display: 'flex',
                  alignItems: 'center',
                  gap: 8,
                  padding: '6px 10px',
                  borderRadius: 6,
                  border: '1px solid #b2f2bb',
                  backgroundColor: '#f4fdf5',
                }}
              >
                <IconCheck size={14} color="#2f9e44" style={{ flexShrink: 0 }} />
                <Anchor href={item.issueUrl} target="_blank" size="sm" style={{ flex: 1, wordBreak: 'break-all' }}>
                  {item.file}
                </Anchor>
              </div>
            ))}
            <Group justify="flex-end" pt="xs">
              <Button onClick={() => onDone(outcome.milestoneNumber)}>Done</Button>
            </Group>
          </>
        ) : (
          <>
            <div
              style={{
                display: 'flex',
                alignItems: 'flex-start',
                gap: 8,
                padding: '10px 12px',
                borderRadius: 6,
                border: '1px solid #ffc9c9',
                backgroundColor: '#fff5f5',
              }}
            >
              <IconX size={14} color="#e03131" style={{ flexShrink: 0, marginTop: 2 }} />
              <Text size="sm" style={{ wordBreak: 'break-all' }}>{outcome.error}</Text>
            </div>
            <Group justify="flex-end" pt="xs">
              <Button variant="default" onClick={onClose}>Close</Button>
            </Group>
          </>
        )}
      </Stack>
    </Modal>
  )
}

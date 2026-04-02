import { ActionIcon, Button, Group, Stack, Text, TextInput } from '@mantine/core'
import { IconPlus, IconTrash } from '@tabler/icons-react'
import { useState } from 'react'

interface Props {
  author: string | null
  collaborators: string[]
  loading?: boolean
  onAdd: (value: string) => void
  onRemove: (index: number) => void
}

export function CollaboratorsTab({ author, collaborators, loading = false, onAdd, onRemove }: Props) {
  const [draft, setDraft] = useState('')
  const normalized = normalizeCollaborator(draft)
  const canAdd = normalized !== null && !collaborators.includes(normalized)

  return (
    <Stack gap="sm">
      <Text size="sm" c="dimmed">
        Collaborators default from git authors for the selected file. Add or remove `Name &lt;email&gt;` entries before queueing.
      </Text>

      <Text size="sm" c={author ? 'dimmed' : 'gray.5'}>
        <b>Author:</b> {author ?? '—'}
      </Text>

      <Group align="flex-end" wrap="nowrap">
        <TextInput
          label="Add collaborator"
          placeholder="Jane Doe <jane@example.com>"
          value={draft}
          onChange={(event) => setDraft(event.currentTarget.value)}
          error={draft.trim().length > 0 && normalized === null ? 'Use Name <email>' : undefined}
          style={{ flex: 1 }}
        />
        <Button
          leftSection={<IconPlus size={14} />}
          disabled={!canAdd}
          onClick={() => {
            if (!normalized) return
            onAdd(normalized)
            setDraft('')
          }}
        >
          Add
        </Button>
      </Group>

      {loading ? (
        <Text size="sm" c="dimmed">Loading detected collaborators…</Text>
      ) : collaborators.length === 0 ? (
        <Text size="sm" c="dimmed">No collaborators selected.</Text>
      ) : (
        <Stack gap={6}>
          {collaborators.map((collaborator, index) => (
            <Group
              key={`${collaborator}-${index}`}
              justify="space-between"
              wrap="nowrap"
              style={{
                border: '1px solid var(--mantine-color-gray-3)',
                borderRadius: 6,
                padding: '8px 10px',
              }}
            >
              <Text size="sm" style={{ wordBreak: 'break-word' }}>{collaborator}</Text>
              <ActionIcon
                color="red"
                variant="subtle"
                aria-label={`Remove ${collaborator}`}
                onClick={() => onRemove(index)}
              >
                <IconTrash size={14} />
              </ActionIcon>
            </Group>
          ))}
        </Stack>
      )}
    </Stack>
  )
}

function normalizeCollaborator(value: string): string | null {
  const match = value.trim().match(/^(.*)<(.*)>$/)
  if (!match) return null
  const name = match[1]?.trim()
  const email = match[2]?.trim()
  if (!name || !email) return null
  return `${name} <${email}>`
}

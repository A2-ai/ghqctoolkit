import { Loader, MultiSelect, Text } from '@mantine/core'
import { useAssignees } from '~/api/assignees'

interface Props {
  value: string[]
  onChange: (logins: string[]) => void
}

export function ReviewersTab({ value, onChange }: Props) {
  const { data: assignees = [], isLoading } = useAssignees()

  const data = assignees.map((a) => ({
    value: a.login,
    label: a.name ? `${a.login} (${a.name})` : a.login,
  }))

  if (isLoading) return <Loader size="sm" />

  return (
    <div>
      <MultiSelect
        label="Reviewers"
        placeholder="Search by login or name"
        data={data}
        value={value}
        onChange={onChange}
        searchable
        clearable
        nothingFoundMessage="No matching assignees"
        size="sm"
      />
      {value.length === 0 && (
        <Text size="xs" c="dimmed" mt={6}>
          Leave empty to create the issue without assignees.
        </Text>
      )}
    </div>
  )
}

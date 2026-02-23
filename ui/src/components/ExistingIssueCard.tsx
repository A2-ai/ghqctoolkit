import { Anchor, Text, Stack, Tooltip } from '@mantine/core'
import { IconLock, IconLink, IconFile } from '@tabler/icons-react'
import type { Issue, RelevantFileInfo } from '~/api/issues'

interface Props {
  issue: Issue
}

export function ExistingIssueCard({ issue }: Props) {
  const isClosed = issue.state === 'closed'

  return (
    <Stack
      gap={5}
      style={{
        opacity: 0.65,
        filter: 'grayscale(0.3)',
        padding: '10px 12px',
        borderRadius: 6,
        border: '1px solid var(--mantine-color-gray-3)',
        backgroundColor: 'white',
        minWidth: 0,
        height: '100%',
        overflowY: 'auto',
        boxSizing: 'border-box',
      }}
    >
      {/* Title row */}
      <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
        {isClosed && (
          <Tooltip label="Closed" withArrow>
            <IconLock size={14} color="#888" style={{ flexShrink: 0 }} />
          </Tooltip>
        )}
        <Anchor
          href={issue.html_url}
          target="_blank"
          size="sm"
          fw={700}
          style={{ wordBreak: 'break-all' }}
        >
          {issue.title}
        </Anchor>
      </div>

      {issue.branch && (
        <Text size="xs" c="dimmed"><b>Branch:</b> {issue.branch}</Text>
      )}
      {issue.created_by && (
        <Text size="xs" c="dimmed"><b>Created by:</b> {issue.created_by}</Text>
      )}
      {issue.checklist_name && (
        <Text size="xs" c="dimmed"><b>Checklist:</b> {issue.checklist_name}</Text>
      )}
      {issue.assignees.length > 0 && (
        <Text size="xs" c="dimmed">
          <b>Reviewer{issue.assignees.length > 1 ? 's' : ''}:</b>{' '}
          {issue.assignees.join(', ')}
        </Text>
      )}

      {issue.relevant_files.length > 0 && (
        <>
          <Text size="xs" fw={600} c="dimmed" mt={2}>Relevant Files</Text>
          {issue.relevant_files.map((rf, i) => (
            <RelevantFileLine key={i} file={rf} />
          ))}
        </>
      )}
    </Stack>
  )
}

function RelevantFileLine({ file }: { file: RelevantFileInfo }) {
  const icon =
    file.kind === 'blocking_qc' ? <IconLock size={12} color="#c92a2a" /> :
    file.kind === 'relevant_qc' ? <IconLink size={12} color="#666" /> :
    <IconFile size={12} color="#666" />

  return (
    <div style={{ display: 'flex', alignItems: 'center', gap: 5 }}>
      {icon}
      {file.issue_url ? (
        <Anchor href={file.issue_url} target="_blank" size="xs" style={{ wordBreak: 'break-all' }}>
          {file.file_name}
        </Anchor>
      ) : (
        <Text size="xs" c="dimmed">{file.file_name}</Text>
      )}
    </div>
  )
}

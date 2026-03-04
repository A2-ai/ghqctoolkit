import { Box, Text, Tooltip } from '@mantine/core'
import type { GitStatus, RepoInfo } from '~/api/repo'

const STATUS_COLOR: Record<GitStatus, string> = {
  clean:    '#2f9e44',
  ahead:    '#e67700',
  behind:   '#e67700',
  diverged: '#c92a2a',
}

const ACTION: Record<GitStatus, string | null> = {
  clean:    null,
  ahead:    'Push to synchronize with remote',
  behind:   'Pull to synchronize with remote',
  diverged: 'Resolve divergence (pull and rebase or merge)',
}

function StatusField({ label, value }: { label: string; value: string }) {
  return (
    <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'center' }}>
      <span style={{ fontSize: 10, opacity: 0.75, textTransform: 'uppercase', letterSpacing: '0.05em', lineHeight: 1 }}>
        {label}
      </span>
      <span style={{ fontSize: 14, fontWeight: 700, fontFamily: 'monospace', lineHeight: 1.4 }}>
        {value}
      </span>
    </div>
  )
}

function Divider() {
  return <div style={{ width: 1, alignSelf: 'stretch', backgroundColor: 'rgba(255,255,255,0.35)', margin: '0 6px' }} />
}

export function RepoStatus({ data }: { data: RepoInfo }) {
  const color = STATUS_COLOR[data.git_status]
  const action = ACTION[data.git_status]
  const shortSha = data.local_commit.slice(0, 7)
  const dirty = data.dirty_files.length
  const shownFiles = data.dirty_files.slice(0, 8)

  const tooltipContent = (
    <Box style={{ maxWidth: 340 }}>
      <Text size="sm" fw={600}>{data.git_status_detail}</Text>
      {action && (
        <Text size="sm" mt={4}>Action: {action}</Text>
      )}
      {data.git_status !== 'clean' && (
        <Text size="xs" c="dimmed" mt={4} style={{ fontFamily: 'monospace' }}>
          Remote commit: {data.remote_commit.slice(0, 7)}
        </Text>
      )}
      {shownFiles.length > 0 && (
        <>
          <Text size="xs" fw={500} mt={6}>
            Dirty files{dirty > 8 ? ` (showing 8 of ${dirty})` : ` (${dirty})`}:
          </Text>
          {shownFiles.map((f) => (
            <Text key={f} size="xs" style={{ fontFamily: 'monospace' }}>{f}</Text>
          ))}
          {dirty > 8 && (
            <Text size="xs" c="dimmed">…and {dirty - 8} more</Text>
          )}
        </>
      )}
    </Box>
  )

  return (
    <Tooltip label={tooltipContent} multiline withArrow position="bottom-end">
      <div style={{
        display: 'flex',
        alignItems: 'center',
        gap: 0,
        backgroundColor: color,
        color: 'white',
        borderRadius: 8,
        padding: '6px 14px',
        cursor: 'default',
        whiteSpace: 'nowrap',
      }}>
        <StatusField label="Branch" value={data.branch} />
        <Divider />
        <StatusField label="Commit" value={shortSha} />
        <Divider />
        <StatusField label="Dirty files" value={dirty === 0 ? 'none' : String(dirty)} />
      </div>
    </Tooltip>
  )
}

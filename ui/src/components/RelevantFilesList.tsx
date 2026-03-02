import { ActionIcon, Text, Tooltip } from '@mantine/core'
import { IconCopyPlus, IconSquarePlus } from '@tabler/icons-react'
import type { RelevantFileInfo } from '~/api/issues'

function extractIssueNumber(url: string): number | null {
  const match = url.match(/\/issues\/(\d+)(?:[^/]*)$/)
  return match ? parseInt(match[1], 10) : null
}

interface RelevantFilesListProps {
  relevantFiles: RelevantFileInfo[]
  claimedFiles: Set<string>
  onSelectFile: (rf: RelevantFileInfo) => void
  onSelectAll: (files: RelevantFileInfo[]) => void
}

export function RelevantFilesList({
  relevantFiles,
  claimedFiles,
  onSelectFile,
  onSelectAll,
}: RelevantFilesListProps) {
  if (relevantFiles.length === 0) return null

  const unclaimed = relevantFiles.filter(rf => !claimedFiles.has(rf.file_name))

  return (
    <div style={{ marginTop: 4 }}>
      <div
        style={{
          borderTop: '1px solid var(--mantine-color-gray-3)',
          paddingTop: 4,
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'space-between',
          gap: 4,
        }}
      >
        <Text size="xs" c="dimmed" fw={600}>Relevant files</Text>
        {unclaimed.length > 0 && (
          <Tooltip label="Add all" withArrow>
            <ActionIcon
              size="xs"
              variant="transparent"
              color="blue"
              onClick={e => { e.stopPropagation(); onSelectAll(unclaimed) }}
              aria-label="Add all relevant files"
            >
              <IconCopyPlus size={14} />
            </ActionIcon>
          </Tooltip>
        )}
      </div>
      {relevantFiles.map(rf => {
        const isClaimed = claimedFiles.has(rf.file_name)
        const isQc = rf.kind === 'blocking_qc' || rf.kind === 'relevant_qc'
        const issueNumber = rf.issue_url ? extractIssueNumber(rf.issue_url) : null
        return (
          <div
            key={rf.file_name}
            style={{
              display: 'flex',
              alignItems: 'center',
              gap: 4,
              padding: '1px 0',
              opacity: isClaimed ? 0.4 : 1,
            }}
          >
            {!isClaimed ? (
              <Tooltip label="Add to archive" withArrow>
                <ActionIcon
                  size="xs"
                  variant="transparent"
                  color="blue"
                  onClick={e => { e.stopPropagation(); onSelectFile(rf) }}
                  aria-label={`Add ${rf.file_name}`}
                  style={{ flexShrink: 0 }}
                >
                  <IconSquarePlus size={14} />
                </ActionIcon>
              </Tooltip>
            ) : (
              <IconSquarePlus size={14} style={{ flexShrink: 0, color: 'var(--mantine-color-gray-5)' }} />
            )}
            <Text
              size="xs"
              c="dimmed"
              style={{ wordBreak: 'break-all', flex: 1, minWidth: 0 }}
            >
              {rf.file_name}
              {isQc && issueNumber != null && (
                <span style={{ color: 'var(--mantine-color-gray-5)' }}> #{issueNumber}</span>
              )}
            </Text>
          </div>
        )
      })}
    </div>
  )
}

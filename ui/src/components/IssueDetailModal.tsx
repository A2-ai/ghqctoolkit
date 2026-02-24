import { useEffect, useState } from 'react'
import {
  ActionIcon,
  Anchor,
  Badge,
  Button,
  Card,
  Checkbox,
  Group,
  Modal,
  RangeSlider,
  Stack,
  Tabs,
  Text,
  Textarea,
  Tooltip,
} from '@mantine/core'
import { IconAsterisk, IconX } from '@tabler/icons-react'
import { useQueryClient } from '@tanstack/react-query'
import type { IssueStatusResponse, QCStatus } from '~/api/issues'
import { fetchSingleIssueStatus, postComment } from '~/api/issues'
import { fetchCommentPreview } from '~/api/preview'

function wrapInGithubStyles(body: string): string {
  return `<!DOCTYPE html>
<html>
<head>
<meta charset="utf-8">
<style>
  body {
    font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Helvetica, Arial, sans-serif;
    font-size: 14px; line-height: 1.6; color: #1f2328;
    padding: 16px 20px; margin: 0; word-wrap: break-word;
  }
  h1,h2,h3,h4,h5,h6 { margin-top: 20px; margin-bottom: 8px; font-weight: 600; line-height: 1.25; }
  h2 { padding-bottom: 6px; border-bottom: 1px solid #d0d7de; font-size: 1.3em; }
  h3 { font-size: 1.1em; }
  a { color: #0969da; text-decoration: none; }
  a:hover { text-decoration: underline; }
  p { margin-top: 0; margin-bottom: 12px; }
  ul,ol { padding-left: 2em; margin-top: 0; margin-bottom: 12px; }
  li { margin-bottom: 2px; }
  li:has(> input[type="checkbox"]) { list-style: none; }
  li:has(> input[type="checkbox"]) input[type="checkbox"] { margin: 0 0.3em 0.2em -1.4em; vertical-align: middle; }
  code { font-family: ui-monospace,SFMono-Regular,"SF Mono",Menlo,monospace; font-size: 85%; background: rgba(175,184,193,0.2); padding: 2px 5px; border-radius: 4px; }
  pre { background: #f6f8fa; border-radius: 6px; padding: 12px 16px; overflow: auto; font-size: 85%; line-height: 1.45; }
  pre code { background: none; padding: 0; }
  blockquote { margin: 0 0 12px; padding: 0 12px; color: #57606a; border-left: 4px solid #d0d7de; }
  hr { border: none; border-top: 1px solid #d0d7de; margin: 16px 0; }
  table { border-collapse: collapse; width: 100%; margin-bottom: 12px; }
  th,td { border: 1px solid #d0d7de; padding: 6px 12px; }
  th { background: #f6f8fa; font-weight: 600; }
</style>
</head>
<body>${body}</body>
</html>`
}

// Swimlane header colors keyed by QC status
const STATUS_LANE_COLOR: Record<QCStatus['status'], string> = {
  approved:               '#dcfce7',
  changes_after_approval: '#dcfce7',
  awaiting_review:        '#dbeafe',
  approval_required:      '#dbeafe',
  change_requested:       '#fee2e2',
  in_progress:            '#fef9c3',
  changes_to_comment:     '#fef9c3',
}

// Commit status dot colors (rendered oldest→newest, lowest→highest)
const STATUS_DOT_COLORS: Record<string, string> = {
  initial:      '#339af0', // blue
  notification: '#ffd43b', // yellow
  approved:     '#51cf66', // green
  reviewed:     '#ff922b', // orange
}
const STATUS_ORDER = ['initial', 'notification', 'approved', 'reviewed'] as const

interface Props {
  status: IssueStatusResponse | null
  onClose: () => void
  onStatusUpdate: (status: IssueStatusResponse) => void
}

export function IssueDetailModal({ status, onClose, onStatusUpdate }: Props) {
  if (!status) return null

  return (
    <Modal
      opened={!!status}
      onClose={onClose}
      size="xl"
      withCloseButton={false}
      styles={{ body: { padding: 0 } }}
    >
      <ModalContent status={status} onClose={onClose} onStatusUpdate={onStatusUpdate} />
    </Modal>
  )
}

function ModalContent({ status, onClose, onStatusUpdate }: { status: IssueStatusResponse; onClose: () => void; onStatusUpdate: (status: IssueStatusResponse) => void }) {
  return (
    <Tabs defaultValue="notify">
      <Group justify="space-between" align="center" px="md" pt="sm" style={{ borderBottom: '1px solid var(--mantine-color-gray-3)' }}>
        <Tabs.List style={{ borderBottom: 'none' }}>
          <Tabs.Tab value="notify">Notify</Tabs.Tab>
          <Tabs.Tab value="review">Review</Tabs.Tab>
          <Tabs.Tab value="approve">Approve</Tabs.Tab>
          <Tabs.Tab value="unapprove">Unapprove</Tabs.Tab>
        </Tabs.List>
        <ActionIcon variant="subtle" color="gray" onClick={onClose} aria-label="Close">
          <IconX size={16} />
        </ActionIcon>
      </Group>

      <Tabs.Panel value="notify" pt="md" px="md" pb={0}>
        <NotifyTab status={status} onStatusUpdate={onStatusUpdate} />
      </Tabs.Panel>
      <Tabs.Panel value="review" pt="md" px="md" pb={0}>
        <Text c="dimmed">Coming soon</Text>
      </Tabs.Panel>
      <Tabs.Panel value="approve" pt="md" px="md" pb={0}>
        <Text c="dimmed">Coming soon</Text>
      </Tabs.Panel>
      <Tabs.Panel value="unapprove" pt="md" px="md" pb={0}>
        <Text c="dimmed">Coming soon</Text>
      </Tabs.Panel>
    </Tabs>
  )
}

function NotifyTab({ status, onStatusUpdate }: { status: IssueStatusResponse; onStatusUpdate: (status: IssueStatusResponse) => void }) {
  const { issue, qc_status, branch, checklist_summary, blocking_qc_status } = status

  // Build oldest-first commit list
  const orderedCommits = [...status.commits].reverse()

  // Default FROM: last index with statuses.length > 0
  let fromDefault = 0
  for (let i = orderedCommits.length - 1; i >= 0; i--) {
    if (orderedCommits[i].statuses.length > 0) {
      fromDefault = i
      break
    }
  }

  // Default TO
  let toDefault: number
  if (fromDefault === orderedCommits.length - 1) {
    toDefault = fromDefault
  } else {
    toDefault = orderedCommits.length - 1
    for (let i = fromDefault + 1; i < orderedCommits.length; i++) {
      if (orderedCommits[i].file_changed) toDefault = i
    }
  }

  // Exception index: toDefault when latest commit and NOT file_changed
  const exceptionIdx =
    toDefault === orderedCommits.length - 1 && !orderedCommits[toDefault].file_changed
      ? toDefault
      : -1

  const [showAll, setShowAll] = useState(false)
  // Source of truth: origIdx values (positions in orderedCommits), not slider indices.
  // This means showAll toggling never loses track of which commits are selected.
  const [fromOrigIdx, setFromOrigIdx] = useState(fromDefault)
  const [toOrigIdx, setToOrigIdx] = useState(toDefault)
  const [includeDiff, setIncludeDiff] = useState(true)
  const [note, setNote] = useState('')
  const [previewLoading, setPreviewLoading] = useState(false)
  const [previewOpen, setPreviewOpen] = useState(false)
  const [previewHtml, setPreviewHtml] = useState<string | null>(null)
  const [postLoading, setPostLoading] = useState(false)
  const [postResultOpen, setPostResultOpen] = useState(false)
  const [postResultUrl, setPostResultUrl] = useState<string | null>(null)
  const [postError, setPostError] = useState<string | null>(null)
  const queryClient = useQueryClient()

  // Reset when the status prop changes (different issue opened)
  useEffect(() => {
    setFromOrigIdx(fromDefault)
    setToOrigIdx(toDefault)
    setShowAll(false)
    setIncludeDiff(true)
    setNote('')
    setPreviewOpen(false)
    setPostResultOpen(false)
    setPostResultUrl(null)
    setPostError(null)
  }, [status]) // eslint-disable-line react-hooks/exhaustive-deps

  const visibleCommits = orderedCommits
    .map((c, i) => ({ ...c, origIdx: i }))
    .filter(({ file_changed, statuses, origIdx }) =>
      showAll || file_changed || statuses.length > 0 || origIdx === exceptionIdx
    )

  // Derive slider indices from origIdx — snap to nearest if not currently visible
  const snapToVisible = (targetOrigIdx: number): number => {
    const exact = visibleCommits.findIndex((c) => c.origIdx === targetOrigIdx)
    if (exact >= 0) return exact
    let best = 0, bestDist = Infinity
    for (let i = 0; i < visibleCommits.length; i++) {
      const dist = Math.abs(visibleCommits[i].origIdx - targetOrigIdx)
      if (dist < bestDist) { bestDist = dist; best = i }
    }
    return best
  }

  const range: [number, number] = [snapToVisible(fromOrigIdx), snapToVisible(toOrigIdx)]

  const handleRangeChange = ([f, t]: [number, number]) => {
    setFromOrigIdx(visibleCommits[f]?.origIdx ?? fromOrigIdx)
    setToOrigIdx(visibleCommits[t]?.origIdx ?? toOrigIdx)
  }

  const fromCommit = visibleCommits[range[0]]
  const toCommit = visibleCommits[range[1]]

  // File changed: any commit strictly after from, up to and including to
  const fileChangedInRange =
    fromOrigIdx < toOrigIdx &&
    orderedCommits.slice(fromOrigIdx + 1, toOrigIdx + 1).some((c) => c.file_changed)

  const commentRequest = {
    current_commit: toCommit?.hash ?? '',
    previous_commit: fromOrigIdx !== toOrigIdx ? (fromCommit?.hash ?? null) : null,
    note: note.trim() || null,
    include_diff: fileChangedInRange ? includeDiff : false,
  }

  async function handlePreview() {
    setPreviewLoading(true)
    try {
      const html = await fetchCommentPreview(issue.number, commentRequest)
      setPreviewHtml(html)
      setPreviewOpen(true)
    } catch (err) {
      setPreviewHtml(`<pre>Error: ${(err as Error).message}</pre>`)
      setPreviewOpen(true)
    } finally {
      setPreviewLoading(false)
    }
  }

  async function handlePost() {
    setPostLoading(true)
    setPostError(null)
    setPostResultUrl(null)
    try {
      const result = await postComment(issue.number, commentRequest)
      setPostResultUrl(result.comment_url)
      void queryClient.invalidateQueries({ queryKey: ['issue', 'status', issue.number] })
      const fresh = await fetchSingleIssueStatus(issue.number)
      onStatusUpdate(fresh)
    } catch (err) {
      setPostError((err as Error).message)
    } finally {
      setPostLoading(false)
      setPostResultOpen(true)
    }
  }

  const laneColor = STATUS_LANE_COLOR[qc_status.status]
  const formattedStatus = qc_status.status.replace(/_/g, ' ')

  return (
    <>
    <Stack gap="md" pb="md">
      {/* Status section — centered card */}
      <Card
        withBorder
        p="md"
        style={{ maxWidth: 380, marginLeft: 'auto', marginRight: 'auto', width: '100%' }}
      >
        <Stack gap="xs">
          <div style={{ textAlign: 'center', display: 'flex', alignItems: 'center', justifyContent: 'center', gap: 4 }}>
            <Anchor href={issue.html_url} target="_blank" fw={700}>
              {issue.title}
            </Anchor>
            {status.dirty && (
              <Tooltip label="This file has uncommitted local changes" withArrow position="top">
                <span data-testid="dirty-indicator" style={{ color: '#c92a2a', display: 'flex', lineHeight: 1 }}>
                  <IconAsterisk size={14} stroke={3} />
                </span>
              </Tooltip>
            )}
          </div>
          <Text size="sm"><b>Branch:</b> {branch}</Text>
          <Text size="sm"><b>Reviewers:</b> {issue.assignees.join(', ') || 'None'}</Text>
          <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
            <Text size="sm" fw={700}>Status:</Text>
            <Badge
              style={{
                backgroundColor: laneColor,
                color: '#333',
                textTransform: 'capitalize',
                border: '1px solid rgba(0,0,0,0.12)',
              }}
            >
              {formattedStatus}
            </Badge>
          </div>

          {checklist_summary.total > 0 && (
            <InlineProgress
              label="Checklist"
              value={(checklist_summary.completed / checklist_summary.total) * 100}
              completed={checklist_summary.completed}
              total={checklist_summary.total}
              color="#5a9e6f"
            />
          )}

          {blocking_qc_status.total > 0 && (
            <Stack gap={4}>
              <Text size="sm" fw={700}>Blocking QC</Text>
              {blocking_qc_status.approved.map((item) => (
                <Text key={item.issue_number} size="sm" c="green">
                  ✓ {item.file_name} (#{item.issue_number})
                </Text>
              ))}
              {blocking_qc_status.not_approved.map((item) => (
                <Text key={item.issue_number} size="sm" c="orange">
                  ✗ {item.file_name} (#{item.issue_number}) — {item.status}
                </Text>
              ))}
              {blocking_qc_status.errors.map((item) => (
                <Text key={item.issue_number} size="sm" c="red">
                  ! #{item.issue_number}: {item.error}
                </Text>
              ))}
            </Stack>
          )}
        </Stack>
      </Card>

      {/* Commit range slider */}
      {visibleCommits.length > 0 && (
        <Stack gap="xs">
          {/* Header */}
          <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
            <Text size="sm" fw={700}>Commit Range</Text>
            <Checkbox
              label="Show all commits"
              checked={showAll}
              onChange={(e) => setShowAll(e.currentTarget.checked)}
            />
          </div>

          {/* Dots row + slider — single commit uses symmetric [-1,1] range so thumb centers */}
          <div style={{ display: 'flex', flexDirection: 'column', gap: 4, paddingLeft: 16, paddingRight: 16 }}>
            <div style={{ position: 'relative', height: 8 }}>
              {visibleCommits.map((c, i) => {
                const n = visibleCommits.length
                const pct = n > 1 ? i / (n - 1) : 0.5
                const left = `calc(10px + ${pct * 100}% - ${pct * 20}px)`
                return (
                  <div key={i} style={{ position: 'absolute', left, transform: 'translateX(-50%)', display: 'flex', gap: 2 }}>
                    {STATUS_ORDER.filter((s) => c.statuses.includes(s)).map((s) => (
                      <span key={s} title={s} style={{ display: 'inline-block', width: 7, height: 7, borderRadius: '50%', backgroundColor: STATUS_DOT_COLORS[s] }} />
                    ))}
                  </div>
                )
              })}
            </div>

            {visibleCommits.length === 1 ? (
              <RangeSlider
                min={-1} max={1} step={1} minRange={0} label={null}
                value={[0, 0]} onChange={() => {}}
                marks={[{ value: 0, label: (
                  <span style={{ fontFamily: 'monospace', fontSize: 10, color: visibleCommits[0].file_changed ? '#111' : '#999' }}>
                    {visibleCommits[0].hash.slice(0, 7)}
                  </span>
                )}]}
                mb={40}
              />
            ) : (
              <RangeSlider
                min={0} max={visibleCommits.length - 1} step={1} minRange={0} label={null}
                value={range} onChange={handleRangeChange}
                marks={visibleCommits.map((c, i) => ({
                  value: i,
                  label: (
                    <span style={{ fontFamily: 'monospace', fontSize: 10, color: c.file_changed ? '#111' : '#999' }}>
                      {c.hash.slice(0, 7)}
                    </span>
                  ),
                }))}
                mb={40}
              />
            )}
          </div>

          {/* Legend */}
          <div style={{ display: 'flex', gap: 14, flexWrap: 'wrap', marginTop: -20, justifyContent: 'center' }}>
            {STATUS_ORDER.map((s) => (
              <div key={s} style={{ display: 'flex', alignItems: 'center', gap: 4 }}>
                <span style={{ display: 'inline-block', width: 8, height: 8, borderRadius: '50%', backgroundColor: STATUS_DOT_COLORS[s] }} />
                <Text size="xs" c="dimmed" style={{ textTransform: 'capitalize' }}>{s}</Text>
              </div>
            ))}
          </div>

          {/* From / To / Include diff — left-aligned, matching status card width */}
          <Stack gap="xs" style={{ maxWidth: 380, marginLeft: 'auto', marginRight: 'auto', width: '100%' }}>
            {fromCommit && <CommitBlock label="From" commit={fromCommit} />}
            {toCommit && <CommitBlock label="To" commit={toCommit} />}
            <Tooltip
              label="No changes between selected commits"
              disabled={fileChangedInRange}
              withArrow
              position="right"
            >
              <span style={{ display: 'inline-flex' }}>
                <Checkbox
                  label="Include diff"
                  checked={fileChangedInRange ? includeDiff : false}
                  disabled={!fileChangedInRange}
                  onChange={(e) => setIncludeDiff(e.currentTarget.checked)}
                />
              </span>
            </Tooltip>
          </Stack>

          <Textarea
            label="Comment"
            placeholder="Optional"
            value={note}
            onChange={(e) => setNote(e.currentTarget.value)}
            resize="vertical"
            minRows={3}
          />

          <Group justify="flex-end">
            <Button
              variant="default"
              loading={previewLoading}
              disabled={!toCommit}
              onClick={handlePreview}
            >
              Preview
            </Button>
            <Button
              loading={postLoading}
              disabled={!toCommit}
              onClick={handlePost}
            >
              Post
            </Button>
          </Group>
        </Stack>
      )}

    </Stack>

      {/* Comment preview */}
      <Modal
        opened={previewOpen}
        onClose={() => setPreviewOpen(false)}
        title="Comment Preview"
        size={800}
        centered
        withinPortal={false}
      >
        <iframe
          srcDoc={previewHtml ? wrapInGithubStyles(previewHtml) : ''}
          style={{ width: '100%', height: 500, border: '1px solid var(--mantine-color-gray-3)', borderRadius: 6 }}
          title="Comment Preview"
        />
      </Modal>

      {/* Post result */}
      <Modal
        opened={postResultOpen}
        onClose={() => setPostResultOpen(false)}
        title={postError ? 'Post Failed' : 'Comment Posted'}
        size="sm"
        centered
        withinPortal={false}
      >
        {postError ? (
          <Text c="red" size="sm">{postError}</Text>
        ) : (
          <Text size="sm">
            Comment posted successfully.{' '}
            <Anchor href={postResultUrl ?? '#'} target="_blank">View on GitHub</Anchor>
          </Text>
        )}
      </Modal>
    </>
  )
}

function CommitBlock({
  label,
  commit,
}: {
  label: string
  commit: { hash: string; message: string; statuses: string[] }
}) {
  return (
    <div style={{ display: 'flex', alignItems: 'center', gap: 6, minWidth: 0, overflow: 'hidden' }}>
      <Text size="sm" fw={700} style={{ flexShrink: 0 }}>{label}:</Text>
      <Text size="sm" style={{ fontFamily: 'monospace', flexShrink: 0 }}>{commit.hash.slice(0, 7)}</Text>
      <Text size="sm" c="dimmed" style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', minWidth: 0, flexShrink: 1 }}>
        — {commit.message}
      </Text>
      {commit.statuses.map((s) => (
        <Badge
          key={s}
          size="xs"
          style={{
            backgroundColor: STATUS_DOT_COLORS[s],
            color: '#333',
            border: '1px solid rgba(0,0,0,0.10)',
            flexShrink: 0,
          }}
        >
          {s}
        </Badge>
      ))}
    </div>
  )
}

function InlineProgress({
  label,
  value,
  completed,
  total,
  color,
}: {
  label: string
  value: number
  completed: number
  total: number
  color: string
}) {
  const textOnFill = value >= 45
  return (
    <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
      <Text size="sm" c="black" fw={700} style={{ whiteSpace: 'nowrap', flexShrink: 0 }}>
        {label}
      </Text>
      <div
        style={{
          flex: 1,
          position: 'relative',
          height: 18,
          borderRadius: 4,
          backgroundColor: '#e9ecef',
          overflow: 'hidden',
        }}
      >
        <div
          style={{
            width: `${value}%`,
            height: '100%',
            backgroundColor: color,
            borderRadius: value >= 99 ? 4 : '4px 2px 2px 4px',
          }}
        />
        <span
          style={{
            position: 'absolute',
            inset: 0,
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
            fontSize: 11,
            fontWeight: 600,
            color: textOnFill ? 'white' : '#555',
            pointerEvents: 'none',
          }}
        >
          {completed}/{total}
        </span>
      </div>
    </div>
  )
}

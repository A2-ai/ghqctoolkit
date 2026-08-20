import { createContext, useContext, useEffect, useMemo, useState } from 'react'
import {
  ActionIcon,
  Alert,
  Anchor,
  Badge,
  Button,
  Card,
  Checkbox,
  Group,
  Modal,
  Stack,
  Tabs,
  Text,
  Tooltip,
} from '@mantine/core'
import { CommentEditor } from './CommentEditor'
import { IconAsterisk, IconX } from '@tabler/icons-react'
import { useQueryClient } from '@tanstack/react-query'
import type { ApproveRequest, Issue, IssueStatusResponse, QCStatus, ReviewRequest, ReviewStashResult } from '~/api/issues'
import { fetchSingleIssueStatus, postApprove, postComment, postReview, useInvalidateBlockingDependents } from '~/api/issues'
import { fetchApprovePreview, fetchCommentPreview, fetchReviewPreview } from '~/api/preview'
import {
  RoundCommitPickerTrack,
  STATUS_DOT_COLORS,
  useRoundPicker,
} from '~/components/RoundCommitPicker'
import { RoundRail } from '~/components/RoundRail'
import { UnapproveSwimLanes } from '~/components/UnapproveSwimLanes'
import { wrapInGithubStyles } from '~/utils/github'
import { STATUS_LANE_COLOR } from '~/utils/statusColors'
import { useChecklistDisplayName } from '~/api/configuration'
import { capitalize } from '~/utils/displayName'
import { StatusErrorDisplay } from './StatusErrorDisplay'
import {
  activeRound,
  activeRoundPos,
  findCommitIndex,
  flattenSegmentCommits,
  previousApprovalOf,
  previousRoundOf,
  shortHash,
} from '~/utils/rounds'

interface Props {
  status: IssueStatusResponse | null
  onClose: () => void
  onStatusUpdate: (status: IssueStatusResponse) => void
  /**
   * Opens the start-new-round modal for this issue. Threaded down to the round rail
   * in every tab through `StartRoundActionContext` rather than as a prop on each
   * tab: the rail is rendered in three places, and the modal it opens has exactly
   * one owner — the caller.
   */
  onStartRound?: () => void
}

/** The rail's "+ Start a new round" action, or undefined when none was supplied. */
const StartRoundActionContext = createContext<(() => void) | undefined>(undefined)

/** The round rail as every tab renders it: segments from the status, action from context. */
function DetailRoundRail({ segments }: { segments: IssueStatusResponse['segments'] }) {
  const onStartRound = useContext(StartRoundActionContext)
  return <RoundRail segments={segments} onStartRound={onStartRound} />
}

export function IssueDetailModal({ status, onClose, onStatusUpdate, onStartRound }: Props) {
  if (!status) return null

  return (
    <Modal
      opened={!!status}
      onClose={onClose}
      size="xl"
      withCloseButton={false}
      styles={{ body: { padding: 0, flex: 1, display: 'flex', flexDirection: 'column', overflow: 'hidden' }, content: { minHeight: 560, display: 'flex', flexDirection: 'column' } }}
    >
      <StartRoundActionContext.Provider value={onStartRound}>
        <ModalContent status={status} onClose={onClose} onStatusUpdate={onStatusUpdate} />
      </StartRoundActionContext.Provider>
    </Modal>
  )
}

function defaultTab(status: IssueStatusResponse): string {
  switch (status.qc_status.status) {
    case 'awaiting_review':
    case 'approval_required': {
      const checklist = status.checklist_summary
      const checklistComplete = checklist.total === 0 || checklist.completed === checklist.total
      const blocking = status.blocking_qc_status
      const blockingComplete = !blocking || blocking.total === 0 || blocking.approved_count === blocking.total
      return checklistComplete && blockingComplete ? 'approve' : 'review'
    }
    case 'change_requested':
    case 'in_progress':
    case 'changes_to_comment':
    // `unknown` has no correct default tab — an unplaceable segment supplies no
    // commit base for any action. This is where it landed before it had its own
    // status value, so the default is unchanged; the tab surfaces its own empty
    // state, and the card is already grayed with the reason.
    case 'unknown':
      return 'notify'
    case 'approved':
    case 'changes_after_approval':
      return 'unapprove'
  }
}

function ModalContent({ status, onClose, onStatusUpdate }: { status: IssueStatusResponse; onClose: () => void; onStatusUpdate: (status: IssueStatusResponse) => void }) {
  const [blockedUnavailable, setBlockedUnavailable] = useState(false)
  useEffect(() => { setBlockedUnavailable(false) }, [status.issue.number])

  const isApproved = status.qc_status.status === 'approved' || status.qc_status.status === 'changes_after_approval'
  const unapproveDisabled = blockedUnavailable && !isApproved

  return (
    <Tabs key={status.issue.number} defaultValue={defaultTab(status)} style={{ flex: 1, display: 'flex', flexDirection: 'column', overflow: 'hidden' }}>
      <Group justify="space-between" align="center" px="md" pt="sm" style={{ borderBottom: '1px solid var(--mantine-color-gray-3)' }}>
        <Tabs.List style={{ borderBottom: 'none' }}>
          <Tabs.Tab value="notify" color="yellow">Notify</Tabs.Tab>
          <Tabs.Tab value="review" color="orange">Review</Tabs.Tab>
          <Tabs.Tab value="approve" color="green" disabled={isApproved}>Approve</Tabs.Tab>
          <Tabs.Tab value="unapprove" color="red" disabled={unapproveDisabled}>Unapprove</Tabs.Tab>
        </Tabs.List>
        <ActionIcon variant="subtle" color="gray" onClick={onClose} aria-label="Close">
          <IconX size={16} />
        </ActionIcon>
      </Group>

      <Tabs.Panel value="notify" pt="md" px="md" pb="md" style={{ flex: 1, overflowY: 'auto' }}>
        <NotifyTab status={status} onStatusUpdate={onStatusUpdate} isApproved={isApproved} />
      </Tabs.Panel>
      <Tabs.Panel value="review" pt="md" px="md" pb="md" style={{ flex: 1, overflowY: 'auto' }}>
        <ReviewTab status={status} onStatusUpdate={onStatusUpdate} isApproved={isApproved} />
      </Tabs.Panel>
      <Tabs.Panel value="approve" pt="md" px="md" pb="md" style={{ flex: 1, overflowY: 'auto' }}>
        <ApproveTab status={status} onStatusUpdate={onStatusUpdate} />
      </Tabs.Panel>
      <Tabs.Panel value="unapprove" pt="md" px="md" pb={0} style={{ flex: 1, display: 'flex', flexDirection: 'column', overflow: 'hidden' }}>
        <UnapproveTab status={status} onStatusUpdate={onStatusUpdate} onBlockedUnavailable={() => setBlockedUnavailable(true)} />
      </Tabs.Panel>
    </Tabs>
  )
}

function NotifyTab({ status, onStatusUpdate, isApproved }: { status: IssueStatusResponse; onStatusUpdate: (status: IssueStatusResponse) => void; isApproved: boolean }) {
  const { issue } = status

  // Build oldest-first commit list from the segments (ordering only — the segment a
  // commit belongs to was decided server-side, per D7).
  const orderedCommits = useMemo(() => flattenSegmentCommits(status.segments), [status.segments])

  // A1: a round is open iff the last segment is a Round.
  const openRound = activeRound(status.segments)
  const openRoundPos = activeRoundPos(status.segments)
  // Q1: `previous_approval` is an accessor now — the closing commit of the Round two
  // positions before this one.
  const openRoundPreviousApproval =
    openRoundPos !== null ? previousApprovalOf(status.segments, openRoundPos) : null

  // S5: the default FROM comes from the API's `next_notification_from` — the
  // newest of {last standing approval, last notified, last reviewed, initial
  // commit}, computed server-side. Only fall back to the old client-side
  // heuristic (last commit carrying a status) when that hash is not in the list.
  const apiFromIdx = findCommitIndex(orderedCommits, status.next_notification_from)
  let fromDefault = 0
  if (apiFromIdx >= 0) {
    fromDefault = apiFromIdx
  } else {
    for (let i = orderedCommits.length - 1; i >= 0; i--) {
      if (orderedCommits[i].statuses.length > 0) {
        fromDefault = i
        break
      }
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
    toDefault === orderedCommits.length - 1 && !orderedCommits[toDefault]?.file_changed
      ? toDefault
      : -1

  const [showAll, setShowAll] = useState(false)
  // Two independent handle positions (origIdx in orderedCommits).
  // Either handle can be dragged past the other; from = min, to = max.
  const [sliderAOrigIdx, setSliderAOrigIdx] = useState(fromDefault)
  const [sliderBOrigIdx, setSliderBOrigIdx] = useState(toDefault)
  const [includeDiff, setIncludeDiff] = useState(true)
  const [note, setNote] = useState('')
  const [previewLoading, setPreviewLoading] = useState(false)
  const [previewOpen, setPreviewOpen] = useState(false)
  const [previewHtml, setPreviewHtml] = useState<string | null>(null)
  const [postLoading, setPostLoading] = useState(false)
  const [postResultOpen, setPostResultOpen] = useState(false)
  const [postResultUrl, setPostResultUrl] = useState<string | null>(null)
  const [postError, setPostError] = useState<string | null>(null)
  const [ackApproved, setAckApproved] = useState(false)
  const queryClient = useQueryClient()

  // Reset when the status prop changes (different issue opened)
  useEffect(() => {
    setSliderAOrigIdx(fromDefault)
    setSliderBOrigIdx(toDefault)
    setShowAll(false)
    setIncludeDiff(true)
    setNote('')
    setPreviewOpen(false)
    setPostResultOpen(false)
    setPostResultUrl(null)
    setPostError(null)
    setAckApproved(false)
  }, [status.issue.number]) // eslint-disable-line react-hooks/exhaustive-deps

  // S2/S3/S7: round-scoped picker. The window is the open round's membership
  // (plus its anchor, the commit the round is measured against); the defaults are
  // pinned visible so a from-commit that predates the round — e.g. the previous
  // round's approval — stays reachable without widening the whole track.
  const forcedIdxs = useMemo(
    () => [exceptionIdx, fromDefault, toDefault],
    [exceptionIdx, fromDefault, toDefault],
  )
  const picker = useRoundPicker({
    orderedCommits,
    segments: status.segments,
    mode: 'range',
    a: sliderAOrigIdx,
    setA: setSliderAOrigIdx,
    b: sliderBOrigIdx,
    setB: setSliderBOrigIdx,
    forcedIdxs,
    showAll,
    // U6: Notify asks someone to read a range, so `to` bounds the question — it means
    // "the newest state I claim to have addressed", which cannot sit in a round that
    // already closed. Keyed on position (U7), so trailing-gap drift still qualifies.
    endReach: 'scope-and-newer',
  })

  const { visibleCommits, fromCommit, toCommit } = picker
  const fromOrigIdx = fromCommit?.origIdx ?? 0
  const toOrigIdx   = toCommit?.origIdx ?? 0

  // S5: when the open round has no notification yet, `next_notification_from` is
  // the previous round's approval, so the default already spans the whole round.
  const prevRoundName =
    openRoundPos !== null ? (previousRoundOf(status.segments, openRoundPos)?.name ?? null) : null
  const spansWholeRound =
    !!openRound &&
    !!openRoundPreviousApproval &&
    apiFromIdx >= 0 &&
    apiFromIdx === findCommitIndex(orderedCommits, openRoundPreviousApproval) &&
    fromOrigIdx === apiFromIdx

  // S5: an empty diff. Happens after an unapproval, where the last notified commit
  // is the commit that was just approved. Only surfaced when there is a better
  // range to offer — the open round's previous approval — so single-round issues,
  // where from === to is an ordinary state, are untouched.
  const prevApprovalIdx = findCommitIndex(orderedCommits, openRoundPreviousApproval)
  const emptyDiff =
    fromOrigIdx === toOrigIdx && prevApprovalIdx >= 0 && prevApprovalIdx !== toOrigIdx

  function presentWholeRound() {
    setSliderAOrigIdx(prevApprovalIdx)
    setSliderBOrigIdx(toOrigIdx)
  }

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

  return (
    <>
    <Stack gap="md">
      <StatusCard status={status} />
      <DetailRoundRail segments={status.segments} />

      {isApproved && (
        <Alert color="orange">
          <Text size="sm" fw={600}>This issue is already approved</Text>
          <Text size="xs" mt={4}>For another QC pass, start a new round.</Text>
          <Checkbox
            mt="xs"
            label="Notify anyway"
            checked={ackApproved}
            onChange={(e) => setAckApproved(e.currentTarget.checked)}
          />
        </Alert>
      )}

      {/* Commit range slider */}
      {visibleCommits.length > 0 && (
        <Stack gap="xs">
          <RoundCommitPickerTrack
            title="Select Commits to Compare"
            picker={picker}
            showAll={showAll}
            onShowAllChange={setShowAll}
            testId="notify-picker"
            banner={
              <>
                {spansWholeRound && prevRoundName && (
                  <Text size="xs" c="dimmed" ta="center" data-testid="since-previous-approval">
                    Since {prevRoundName} approval — this shows the reviewer everything in{' '}
                    {openRound?.name}
                  </Text>
                )}
                {emptyDiff && (
                  <Alert color="yellow" p="xs" data-testid="empty-diff-alert">
                    <Text size="xs" fw={600}>
                      Nothing to compare — the from and to commits are the same (
                      {shortHash(toCommit?.hash)}).
                    </Text>
                    <Text size="xs" mt={2}>
                      This is usual right after unapproving: the last notification landed on the
                      commit that was just approved.
                    </Text>
                    <Button
                      mt="xs"
                      size="compact-xs"
                      variant="light"
                      data-testid="present-whole-round"
                      onClick={presentWholeRound}
                    >
                      Compare against {prevRoundName ?? 'the previous'} approval (
                      {shortHash(openRoundPreviousApproval)})
                    </Button>
                  </Alert>
                )}
              </>
            }
          >
            {/* From / To / Include diff */}
            <Stack gap="xs" style={{ maxWidth: 380, marginLeft: 'auto', marginRight: 'auto', width: '100%' }}>
              {/*
                U1: when a non-linear Gap sits between the two ends, the from-handle
                is rendered detached — cut off from the To block rather than reading
                as one continuous span, because the path between them does not exist.
              */}
              {fromCommit && (
                <CommitBlock label="From" commit={fromCommit} detached={picker.detached} />
              )}
              {picker.detached && (
                <Text size="xs" c="orange" data-testid="detached-previous-approval">
                  Not connected — the history between these two commits is not one path.
                </Text>
              )}
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
          </RoundCommitPickerTrack>

          <CommentEditor
            label="Comment"
            placeholder="Optional"
            value={note}
            onChange={setNote}
            showPreviewTabs
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
              disabled={!toCommit || (isApproved && !ackApproved)}
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
      styles={{ header: { paddingTop: 12, paddingBottom: 12 }, body: { paddingBottom: 20 } }}
    >
      <iframe
        srcDoc={previewHtml ? wrapInGithubStyles(previewHtml) : ''}
        style={{ width: '100%', height: 450, border: '1px solid var(--mantine-color-gray-3)', borderRadius: 6 }}
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

// ---------------------------------------------------------------------------
// Shared status card (used by both Notify and Review tabs)
// ---------------------------------------------------------------------------
const EMPTY_BLOCKING_QC_STATUS = { total: 0, approved_count: 0, summary: '-', approved: [], not_approved: [], errors: [] }

function StatusCard({ status }: { status: IssueStatusResponse }) {
  // A2: the branch the status was actually computed on — the active segment's — not
  // the issue body's. `status.issue.branch` still carries the body's answer; showing
  // that here is the bug the segment model deletes.
  const { issue, qc_status, active_branch, checklist_summary } = status
  const blocking_qc_status = status.blocking_qc_status ?? EMPTY_BLOCKING_QC_STATUS
  const laneColor = STATUS_LANE_COLOR[qc_status.status]
  const formattedStatus = qc_status.status.replace(/_/g, ' ')
  const { singular } = useChecklistDisplayName()
  const singularCap = capitalize(singular)

  return (
    <Card
      withBorder
      p="md"
      style={{ maxWidth: 380, marginLeft: 'auto', marginRight: 'auto', width: '100%' }}
    >
      <Stack gap="xs">
        <div
          style={{
            textAlign: 'center',
            display: 'flex',
            alignItems: 'baseline',
            justifyContent: 'center',
            gap: 4,
            // File-path titles have no spaces; without `anywhere` they
            // overflow the card. With it, the browser will break inside the
            // path (e.g. on `/`) only when needed to fit the available width.
            overflowWrap: 'anywhere',
            minWidth: 0,
          }}
        >
          <Anchor
            href={issue.html_url}
            target="_blank"
            fw={700}
            style={{ overflowWrap: 'anywhere', wordBreak: 'break-word' }}
          >
            {issue.title}
          </Anchor>
          {status.dirty && (
            <Tooltip label="This file has uncommitted local changes" withArrow position="top">
              <span
                data-testid="dirty-indicator"
                style={{ color: '#c92a2a', display: 'inline-flex', lineHeight: 1, flexShrink: 0 }}
              >
                <IconAsterisk size={14} stroke={3} />
              </span>
            </Tooltip>
          )}
        </div>
        <Text size="sm"><b>Branch:</b> {active_branch}</Text>
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
            label={singularCap}
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
            {blocking_qc_status.errors.length > 0 && (
              <StatusErrorDisplay
                errors={blocking_qc_status.errors}
                variant="inline-list"
              />
            )}
          </Stack>
        )}
      </Stack>
    </Card>
  )
}

// ---------------------------------------------------------------------------
// Review tab — single commit selector, diff against working directory
// ---------------------------------------------------------------------------
function ReviewTab({ status, onStatusUpdate, isApproved }: { status: IssueStatusResponse; onStatusUpdate: (status: IssueStatusResponse) => void; isApproved: boolean }) {
  const { issue } = status

  const orderedCommits = useMemo(() => flattenSegmentCommits(status.segments), [status.segments])

  // Default: newest commit (last in orderedCommits = latest)
  const defaultCommitOrigIdx = orderedCommits.length - 1

  // Exception: make the latest commit visible even when it has no statuses and didn't
  // change the file. If it already qualifies via those conditions, no exception needed.
  const latestCommit = orderedCommits[defaultCommitOrigIdx]
  const exceptionIdx =
    latestCommit && !latestCommit.file_changed && latestCommit.statuses.length === 0
      ? defaultCommitOrigIdx
      : -1

  const [showAll, setShowAll] = useState(false)
  const [commitOrigIdx, setCommitOrigIdx] = useState(defaultCommitOrigIdx)
  const [includeDiff, setIncludeDiff] = useState(true)
  const [autoStash, setAutoStash] = useState(status.dirty)
  const [note, setNote] = useState('')
  const [previewLoading, setPreviewLoading] = useState(false)
  const [previewOpen, setPreviewOpen] = useState(false)
  const [previewHtml, setPreviewHtml] = useState<string | null>(null)
  const [postLoading, setPostLoading] = useState(false)
  const [postResultOpen, setPostResultOpen] = useState(false)
  const [postResultUrl, setPostResultUrl] = useState<string | null>(null)
  const [postStashResult, setPostStashResult] = useState<ReviewStashResult | null>(null)
  const [postError, setPostError] = useState<string | null>(null)
  const [ackApproved, setAckApproved] = useState(false)
  const queryClient = useQueryClient()

  useEffect(() => {
    setCommitOrigIdx(defaultCommitOrigIdx)
    setShowAll(false)
    setIncludeDiff(true)
    setAutoStash(status.dirty)
    setNote('')
    setPreviewOpen(false)
    setPostResultOpen(false)
    setPostResultUrl(null)
    setPostStashResult(null)
    setPostError(null)
    setAckApproved(false)
  }, [status.issue.number]) // eslint-disable-line react-hooks/exhaustive-deps

  const forcedIdxs = useMemo(
    () => [exceptionIdx, defaultCommitOrigIdx],
    [exceptionIdx, defaultCommitOrigIdx],
  )
  const picker = useRoundPicker({
    orderedCommits,
    segments: status.segments,
    mode: 'single',
    a: commitOrigIdx,
    setA: setCommitOrigIdx,
    forcedIdxs,
    showAll,
    // U6: a review records what the reviewer *read*, and reading an older round's commit
    // is legitimate — so no constraint here.
    endReach: 'any',
  })
  const { visibleCommits, selectedCommit } = picker

  const reviewRequest: ReviewRequest = {
    commit: selectedCommit?.hash ?? '',
    note: note.trim() || null,
    include_diff: status.dirty ? includeDiff : false,
    auto_stash: status.dirty ? autoStash : false,
  }

  async function handlePreview() {
    setPreviewLoading(true)
    try {
      const html = await fetchReviewPreview(issue.number, reviewRequest)
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
    setPostStashResult(null)
    try {
      const result = await postReview(issue.number, reviewRequest)
      setPostResultUrl(result.comment_url)
      setPostStashResult(result.stash)
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

  return (
    <>
    <Stack gap="md">
      <StatusCard status={status} />
      <DetailRoundRail segments={status.segments} />

      {isApproved && (
        <Alert color="orange">
          <Text size="sm" fw={600}>This issue is already approved</Text>
          <Text size="xs" mt={4}>For another QC pass, start a new round.</Text>
          <Checkbox
            mt="xs"
            label="Review anyway"
            checked={ackApproved}
            onChange={(e) => setAckApproved(e.currentTarget.checked)}
          />
        </Alert>
      )}

      {visibleCommits.length > 0 && (
        <Stack gap="xs">
          <RoundCommitPickerTrack
            title="Select Commit"
            picker={picker}
            showAll={showAll}
            onShowAllChange={setShowAll}
            testId="review-picker"
          >
            <Stack gap="xs" style={{ maxWidth: 380, marginLeft: 'auto', marginRight: 'auto', width: '100%' }}>
              {selectedCommit && <CommitBlock label="Commit" commit={selectedCommit} />}
              <Tooltip
                label="No local changes for this file"
                disabled={status.dirty}
                withArrow
                position="right"
              >
                <span style={{ display: 'inline-flex' }}>
                  <Checkbox
                    label="Include diff"
                    checked={status.dirty ? includeDiff : false}
                    disabled={!status.dirty}
                    onChange={(e) => setIncludeDiff(e.currentTarget.checked)}
                  />
                </span>
              </Tooltip>
              <Tooltip
                label="No local changes to stash for this file"
                disabled={status.dirty}
                withArrow
                position="right"
              >
                <span style={{ display: 'inline-flex' }}>
                  <Checkbox
                    label="Stash file changes from review"
                    checked={status.dirty ? autoStash : false}
                    disabled={!status.dirty}
                    onChange={(e) => setAutoStash(e.currentTarget.checked)}
                  />
                </span>
              </Tooltip>
            </Stack>
          </RoundCommitPickerTrack>

          <CommentEditor
            label="Comment"
            placeholder="Optional"
            value={note}
            onChange={setNote}
            showPreviewTabs
          />
          <Group justify="flex-end">
            <Button
              variant="default"
              loading={previewLoading}
              disabled={!selectedCommit}
              onClick={handlePreview}
            >
              Preview
            </Button>
            <Button
              loading={postLoading}
              disabled={!selectedCommit || (isApproved && !ackApproved)}
              onClick={handlePost}
            >
              Post
            </Button>
          </Group>
        </Stack>
      )}
    </Stack>

    <Modal
      opened={previewOpen}
      onClose={() => setPreviewOpen(false)}
      title="Comment Preview"
      size={800}
      centered
      styles={{ header: { paddingTop: 12, paddingBottom: 12 }, body: { paddingBottom: 20 } }}
    >
      <iframe
        srcDoc={previewHtml ? wrapInGithubStyles(previewHtml) : ''}
        style={{ width: '100%', height: 450, border: '1px solid var(--mantine-color-gray-3)', borderRadius: 6 }}
        title="Comment Preview"
      />
    </Modal>

    <Modal
      opened={postResultOpen}
      onClose={() => setPostResultOpen(false)}
      title={postError ? 'Post Failed' : 'Comment Posted'}
      size="sm"
      centered
    >
      {postError ? (
        <Text c="red" size="sm">{postError}</Text>
      ) : (
        <Stack gap="xs">
          <Text size="sm">
            Comment posted successfully.{' '}
            <Anchor href={postResultUrl ?? '#'} target="_blank">View on GitHub</Anchor>
          </Text>
          {postStashResult?.message && (
            <Text size="sm" c={postStashResult.status === 'failed' ? 'orange' : 'dimmed'}>
              {postStashResult.message}
            </Text>
          )}
        </Stack>
      )}
    </Modal>
    </>
  )
}

// ---------------------------------------------------------------------------
// Approve tab — single commit selector, no include diff
// ---------------------------------------------------------------------------
function ApproveTab({ status, onStatusUpdate }: { status: IssueStatusResponse; onStatusUpdate: (status: IssueStatusResponse) => void }) {
  const { issue } = status

  const orderedCommits = useMemo(() => flattenSegmentCommits(status.segments), [status.segments])

  // Default: last commit with non-empty statuses; fall back to latest
  let defaultCommitOrigIdx = orderedCommits.length - 1
  for (let i = orderedCommits.length - 1; i >= 0; i--) {
    if (orderedCommits[i].statuses.length > 0) { defaultCommitOrigIdx = i; break }
  }

  // Exception: only needed when the default commit wouldn't otherwise be visible
  // (i.e., it has no statuses and didn't change the file)
  const defaultCommit = orderedCommits[defaultCommitOrigIdx]
  const exceptionIdx =
    defaultCommit && !defaultCommit.file_changed && defaultCommit.statuses.length === 0
      ? defaultCommitOrigIdx
      : -1

  const [showAll, setShowAll] = useState(false)
  const [commitOrigIdx, setCommitOrigIdx] = useState(defaultCommitOrigIdx)
  const [overrideBlocking, setOverrideBlocking] = useState(false)
  const [note, setNote] = useState('')
  const [previewLoading, setPreviewLoading] = useState(false)
  const [previewOpen, setPreviewOpen] = useState(false)
  const [previewHtml, setPreviewHtml] = useState<string | null>(null)
  const [postLoading, setPostLoading] = useState(false)
  const [postResultOpen, setPostResultOpen] = useState(false)
  const [postResultUrl, setPostResultUrl] = useState<string | null>(null)
  const [postError, setPostError] = useState<string | null>(null)
  const queryClient = useQueryClient()
  const invalidateBlockingDependents = useInvalidateBlockingDependents()

  useEffect(() => {
    setCommitOrigIdx(defaultCommitOrigIdx)
    setShowAll(false)
    setOverrideBlocking(false)
    setNote('')
    setPreviewOpen(false)
    setPostResultOpen(false)
    setPostResultUrl(null)
    setPostError(null)
  }, [status.issue.number]) // eslint-disable-line react-hooks/exhaustive-deps

  const bqs = status.blocking_qc_status ?? EMPTY_BLOCKING_QC_STATUS
  const hasBlockingIssues = bqs.total > 0 && (bqs.not_approved.length > 0 || bqs.errors.length > 0)

  const forcedIdxs = useMemo(
    () => [exceptionIdx, defaultCommitOrigIdx],
    [exceptionIdx, defaultCommitOrigIdx],
  )
  const picker = useRoundPicker({
    orderedCommits,
    segments: status.segments,
    mode: 'single',
    a: commitOrigIdx,
    setA: setCommitOrigIdx,
    forcedIdxs,
    showAll,
    // U6: approval closes a round **at** a commit, so the commit must belong to that
    // round. The trailing gap is excluded too: approving drift no round covers would
    // bypass the model (S5 resolves `changes_after_approval` by starting a new round).
    endReach: 'scope-round-only',
  })
  const { visibleCommits, selectedCommit } = picker

  const canApprove =
    !!selectedCommit &&
    (!hasBlockingIssues || overrideBlocking) &&
    (!overrideBlocking || note.trim() !== '')

  const approveRequest: ApproveRequest = {
    commit: selectedCommit?.hash ?? '',
    note: note.trim() || null,
  }

  async function handlePreview() {
    setPreviewLoading(true)
    try {
      const html = await fetchApprovePreview(issue.number, approveRequest)
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
      const result = await postApprove(issue.number, approveRequest, overrideBlocking)
      setPostResultUrl(result.approval_url)
      if (result.closed) {
        queryClient.setQueriesData<Issue[]>({ queryKey: ['milestones'] }, (old) =>
          old?.map((i) => i.number === issue.number ? { ...i, state: 'closed' } : i)
        )
      }
      void queryClient.invalidateQueries({ queryKey: ['issue', 'status', issue.number] })
      invalidateBlockingDependents(issue.number)
      const fresh = await fetchSingleIssueStatus(issue.number)
      onStatusUpdate(fresh)
    } catch (err) {
      setPostError((err as Error).message)
    } finally {
      setPostLoading(false)
      setPostResultOpen(true)
    }
  }

  return (
    <>
    <Stack gap="md">
      <StatusCard status={status} />
      <DetailRoundRail segments={status.segments} />

      {hasBlockingIssues && (
        <Alert color="orange">
          <Stack gap={4}>
            <Text size="sm" fw={600}>Blocking QCs are not fully approved</Text>
            {bqs.not_approved.map((item) => (
              <Text key={`${item.issue_number}-${item.file_name}`} size="xs">
                {item.file_name} (#{item.issue_number}) — {item.status}
              </Text>
            ))}
            {bqs.errors.length > 0 && (
              <StatusErrorDisplay errors={bqs.errors} variant="inline-list" />
            )}
          </Stack>
          <Checkbox
            mt="xs"
            label="Override and approve anyway"
            checked={overrideBlocking}
            onChange={(e) => setOverrideBlocking(e.currentTarget.checked)}
          />
        </Alert>
      )}

      {visibleCommits.length > 0 && (
        <Stack gap="xs">
          <RoundCommitPickerTrack
            title="Select Commit"
            picker={picker}
            showAll={showAll}
            onShowAllChange={setShowAll}
            testId="approve-picker"
          >
            <Stack gap="xs" style={{ maxWidth: 380, marginLeft: 'auto', marginRight: 'auto', width: '100%' }}>
              {selectedCommit && <CommitBlock label="Commit" commit={selectedCommit} />}
            </Stack>
          </RoundCommitPickerTrack>

          <CommentEditor
            label={overrideBlocking ? 'Note (required)' : 'Comment'}
            placeholder={overrideBlocking ? 'Required' : 'Optional'}
            required={overrideBlocking}
            error={overrideBlocking && note.trim() === '' ? 'A note is required when overriding blocking QCs' : undefined}
            value={note}
            onChange={setNote}
            showPreviewTabs
          />
          <Group justify="flex-end">
            <Button
              variant="default"
              loading={previewLoading}
              disabled={!selectedCommit}
              onClick={handlePreview}
            >
              Preview
            </Button>
            <Button
              color="green"
              loading={postLoading}
              disabled={!canApprove}
              onClick={handlePost}
            >
              Approve
            </Button>
          </Group>
        </Stack>
      )}
    </Stack>

    <Modal
      opened={previewOpen}
      onClose={() => setPreviewOpen(false)}
      title="Comment Preview"
      size={800}
      centered
      styles={{ header: { paddingTop: 12, paddingBottom: 12 }, body: { paddingBottom: 20 } }}
    >
      <iframe
        srcDoc={previewHtml ? wrapInGithubStyles(previewHtml) : ''}
        style={{ width: '100%', height: 450, border: '1px solid var(--mantine-color-gray-3)', borderRadius: 6 }}
        title="Comment Preview"
      />
    </Modal>

    <Modal
      opened={postResultOpen}
      onClose={() => setPostResultOpen(false)}
      title={postError ? 'Approve Failed' : 'Approved'}
      size="sm"
      centered
    >
      {postError ? (
        <Text c="red" size="sm">{postError}</Text>
      ) : (
        <Text size="sm">
          Issue approved and closed.{' '}
          <Anchor href={postResultUrl ?? '#'} target="_blank">View on GitHub</Anchor>
        </Text>
      )}
    </Modal>
    </>
  )
}

// ---------------------------------------------------------------------------
// Unapprove tab — swim lane layout with cascade impact
// ---------------------------------------------------------------------------
function UnapproveTab({ status, onStatusUpdate, onBlockedUnavailable }: { status: IssueStatusResponse; onStatusUpdate: (status: IssueStatusResponse) => void; onBlockedUnavailable: () => void }) {
  return (
    <>
      <div style={{ flexShrink: 0, paddingBottom: 12 }}>
        <StatusCard status={status} />
      </div>
      <UnapproveSwimLanes status={status} onStatusUpdate={onStatusUpdate} onBlockedUnavailable={onBlockedUnavailable} />
    </>
  )
}

function CommitBlock({
  label,
  commit,
  detached = false,
}: {
  label: string
  commit: { hash: string; message: string; statuses: string[] }
  /** U1: draw this handle as *not* connected to the other end of the comparison. */
  detached?: boolean
}) {
  return (
    <div
      data-detached={detached ? 'true' : undefined}
      style={{
        display: 'flex',
        alignItems: 'center',
        gap: 6,
        minWidth: 0,
        overflow: 'hidden',
        ...(detached
          ? {
              borderLeft: '2px dashed var(--mantine-color-orange-6)',
              paddingLeft: 6,
            }
          : undefined),
      }}
    >
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

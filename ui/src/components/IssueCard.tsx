import { Anchor, Badge, Button, Stack, Text, Tooltip } from '@mantine/core'
import { IconAsterisk } from '@tabler/icons-react'
import type { ReactNode } from 'react'
import type { IssueStatusResponse } from '~/api/issues'
import { useChecklistDisplayName } from '~/api/configuration'
import { capitalize } from '~/utils/displayName'
import { activeSegment, roundSegments, unplaceableReasonText } from '~/utils/rounds'

interface Props {
  status: IssueStatusResponse
  currentBranch: string
  remoteCommit: string
  postApprovalCommit?: string
  /** Opens the start-new-round modal for this issue. Omitted → no affordance. */
  onStartRound?: () => void
  /**
   * Opens the same modal to repair the issue's open round. Omitted → no affordance.
   * Only ever offered when `status.round_repair.needs_repair` says a follow-up step
   * of the open round is actually incomplete.
   */
  onRepairRound?: () => void
}

export function IssueCard({ status, currentBranch, remoteCommit, postApprovalCommit, onStartRound, onRepairRound }: Props) {
  const { issue, qc_status, dirty, active_branch, checklist_summary, blocking_qc_status } = status
  // U3/A2: compare the branch the status was *computed on* against the checkout.
  // `issue.branch` is still on the response and is still the issue body's branch — it
  // is what greyed out the very issue a user was QC'ing once rounds could move
  // branches, so it must not be read here.
  const isWrongBranch = active_branch !== currentBranch
  const { singular } = useChecklistDisplayName()
  const singularCap = capitalize(singular)

  const segments = status.segments ?? []
  const active = activeSegment(segments)

  // U3 (revised, D15): graying is not only a checkout mismatch. D4 grays whatever
  // "cannot be trusted at face value", and an active segment that could not be placed
  // is exactly that — its status is computed from a segment owning no commits.
  //
  // The reason is named rather than folded into one undifferentiated gray, because the
  // remedies differ: check out the other branch vs. fetch/restore the missing history.
  const unplaceableReason =
    active !== null && active.placement.kind === 'unplaceable' ? active.placement.reason : null
  // Third clause: an `unrelated` trailing Gap means the approval and the branch tip
  // share no ancestor, so `Approved` cannot be read at face value.
  //
  // DEFENSIVE ONLY — the current fold cannot produce this. A trailing Gap walks the
  // previous Round's branch, and a placed closed Round necessarily has its closing
  // commit on that walk, so the Gap's lower bound always resolves and its continuity
  // is always `linear`; an approval whose commit has vanished degrades through
  // `unplaceable` instead (D15 addendum). Kept because it is cheap and the API could
  // emit the shape after a future model change — do not read it as a reachable state.
  const unrelatedHistory = active !== null && active.kind === 'gap' && active.continuity.kind === 'unrelated'
  const grayed = isWrongBranch || unplaceableReason !== null || unrelatedHistory

  // Round indicator: only for multi-round issues, so the common single
  // `Initial QC` case stays exactly as quiet as it is today.
  const rounds = roundSegments(segments)
  const latestRound = rounds.length > 1 ? rounds[rounds.length - 1] : null
  // Any approved QC can start a new round, whether or not the file has moved since.
  // Gating on `changes_after_approval` made the affordance appear and disappear based
  // on unrelated commits, so a cleanly approved issue had no way in.
  const showStartRound =
    (qc_status.status === 'approved' || qc_status.status === 'changes_after_approval') &&
    onStartRound !== undefined
  // A round is open but one of its follow-up steps never landed. `needs_repair` is
  // the only flag to branch on: a round with no notification is not broken, just
  // quiet, so the common single-round case stays exactly as quiet as before.
  const repair = status.round_repair
  const showRepairRound = repair !== null && repair.needs_repair && onRepairRound !== undefined

  // Per-lane commit rows. Every sha on `qc_status` is nullable now (a commit-less
  // active segment is the normal approved state, not an edge case), so each row is
  // rendered only when its commit exists.
  let commitRows: ReactNode = null
  switch (qc_status.status) {
    case 'awaiting_review':
    case 'approval_required':
      // `latest_commit` is the newest commit of the active segment — the branch tip
      // of the round under review — which is exactly what "Latest" claims.
      commitRows = qc_status.latest_commit && (
        <CommitRow label="Latest" hash={qc_status.latest_commit} />
      )
      break
    case 'change_requested':
      // D12: this row used to read `latest_commit`, back when that meant "a commit a
      // comment named". M8 redefined it as the newest commit of the active segment
      // (≈ the branch tip), which would label unreviewed drift as reviewed — so the
      // row reads the active round's own newest review instead. Derived server-side;
      // the card does not walk `events` (D7).
      commitRows = (
        <>
          {qc_status.last_reviewed_commit && (
            <CommitRow label="Reviewed" hash={qc_status.last_reviewed_commit} />
          )}
          {remoteCommit && <CommitRow label="Remote" hash={remoteCommit} />}
        </>
      )
      break
    case 'in_progress':
    case 'changes_to_comment':
      // D12, same reasoning as "Reviewed": "Last Posted" asserts a commit was named in
      // a posted comment, which is `last_notified_commit`, not the branch tip.
      commitRows = (
        <>
          {qc_status.last_notified_commit && (
            <CommitRow label="Last Posted" hash={qc_status.last_notified_commit} />
          )}
          {remoteCommit && <CommitRow label="Remote" hash={remoteCommit} />}
        </>
      )
      break
    case 'approved':
    case 'changes_after_approval':
      commitRows = (
        <>
          {qc_status.last_approved_commit && (
            <CommitRow label="Approved" hash={qc_status.last_approved_commit} />
          )}
          {postApprovalCommit && <CommitRow label="Changed" hash={postApprovalCommit} />}
        </>
      )
      break
  }

  return (
    <Stack
      gap={6}
      data-testid={`issue-card-body-${issue.number}`}
      style={{
        opacity: grayed ? 0.45 : 1,
        filter: grayed ? 'grayscale(0.4)' : 'none',
        position: 'relative',
      }}
    >
      {dirty && (
        <Tooltip label="This file has uncommitted local changes" withArrow position="top">
          <span data-testid="dirty-indicator" style={{ position: 'absolute', top: 0, right: 0, color: '#c92a2a', display: 'flex', lineHeight: 1 }}>
            <IconAsterisk size={16} stroke={3} />
          </span>
        </Tooltip>
      )}

      {/* File link */}
      <div style={{ display: 'flex', alignItems: 'flex-start', justifyContent: 'center' }}>
        <Anchor
          href={issue.html_url}
          target="_blank"
          size="md"
          fw={700}
          onClick={(event) => event.stopPropagation()}
          style={{ lineHeight: 1.3, textAlign: 'center', wordBreak: 'break-all' }}
        >
          {issue.title}
        </Anchor>
      </div>

      {/* Round indicator — multi-round issues only */}
      {latestRound && (
        <div style={{ display: 'flex', justifyContent: 'center' }}>
          <Tooltip
            label={`${rounds.length} QC rounds; ${latestRound.name} is ${latestRound.state === 'open' ? 'open' : 'closed'}`}
            withArrow
            position="top"
          >
            <Badge
              size="sm"
              variant="light"
              color={latestRound.state === 'open' ? 'blue' : 'gray'}
              data-testid={`round-badge-${issue.number}`}
            >
              {latestRound.name}
            </Badge>
          </Tooltip>
        </div>
      )}

      {/* Milestone */}
      {issue.milestone && (
        <Text size="sm" c="black"><b>Milestone:</b> {issue.milestone}</Text>
      )}

      {/* Branch */}
      <Text size="sm" c={isWrongBranch ? 'red' : 'black'}>
        <b>Branch:</b> {active_branch}{isWrongBranch ? ' (different branch)' : ''}
      </Text>

      {/*
        D15: name the reason the card is grayed when it is not the checkout mismatch,
        which the branch line above already states. Two different problems, two
        different fixes.
      */}
      {unplaceableReason !== null && (
        <Text size="sm" c="orange" data-testid={`gray-reason-${issue.number}`}>
          Commits could not be placed — {unplaceableReasonText(unplaceableReason)}
        </Text>
      )}
      {unplaceableReason === null && unrelatedHistory && (
        <Text size="sm" c="orange" data-testid={`gray-reason-${issue.number}`}>
          History unrelated to the approval — the approved commit and this branch share
          no ancestor
        </Text>
      )}

      {/* Commit info */}
      {commitRows}

      {/* Checklist progress */}
      {checklist_summary.total > 0 && (
        <InlineProgress
          label={singularCap}
          value={(checklist_summary.completed / checklist_summary.total) * 100}
          completed={checklist_summary.completed}
          total={checklist_summary.total}
          color="#5a9e6f"
        />
      )}

      {/* Blocking QC progress */}
      {blocking_qc_status && blocking_qc_status.total > 0 && (
        <InlineProgress
          label="Blocking QCs"
          value={(blocking_qc_status.approved_count / blocking_qc_status.total) * 100}
          completed={blocking_qc_status.approved_count}
          total={blocking_qc_status.total}
          color="#3d7a57"
        />
      )}

      {/*
        Approved — offer another QC round, identically whether or not the file moved.
        Only the colour distinguishes the two: orange when the file has drifted since
        the approval, so the button reads as attention-wanting in step with the card's
        own orange tint (both derive from `postApprovalCommit`); green when the file is
        unchanged and a new round is a free choice rather than a response to drift.
      */}
      {showStartRound && (
        <Tooltip
          label={
            postApprovalCommit
              ? 'The file has changed since it was approved. Start another QC round to review those changes; the previous approval stays valid.'
              : 'Start another QC round on this file. The previous approval stays valid.'
          }
          withArrow
          position="top"
          multiline
          w={260}
        >
          <Button
            size="xs"
            variant="light"
            color={postApprovalCommit ? 'orange' : 'green'}
            data-testid={`start-round-action-${issue.number}`}
            onClick={(event) => {
              event.stopPropagation()
              onStartRound?.()
            }}
          >
            Start new round
          </Button>
        </Tooltip>
      )}

      {/* The open round exists but is incomplete — offer to finish it */}
      {showRepairRound && (
        <Tooltip
          label={`${repair.round_name} is open, but ${[
            repair.reopen && 'the issue was left closed',
            repair.body_marker && 'its QC Round body block is out of date',
          ]
            .filter(Boolean)
            .join(' and ')}`}
          withArrow
          position="top"
        >
          <Button
            size="xs"
            variant="light"
            color="yellow"
            data-testid={`repair-round-action-${issue.number}`}
            onClick={(event) => {
              event.stopPropagation()
              onRepairRound?.()
            }}
          >
            Repair {repair.round_name}
          </Button>
        </Tooltip>
      )}
    </Stack>
  )
}

function CommitRow({ label, hash }: { label: string; hash: string }) {
  return (
    <Text size="sm" c="black"><b>{label}:</b> <span style={{ fontFamily: 'monospace' }}>{hash.slice(0, 7)}</span></Text>
  )
}

function InlineProgress({ label, value, completed, total, color }: {
  label: string
  value: number
  completed: number
  total: number
  color: string
}) {
  const textOnFill = value >= 45

  return (
    <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
      <Text size="sm" c="black" fw={700} style={{ whiteSpace: 'nowrap', flexShrink: 0 }}>{label}</Text>
      <div style={{
        flex: 1,
        position: 'relative',
        height: 18,
        borderRadius: 4,
        backgroundColor: '#e9ecef',
        overflow: 'hidden',
      }}>
        <div style={{
          width: `${value}%`,
          height: '100%',
          backgroundColor: color,
          borderRadius: value >= 99 ? 4 : '4px 2px 2px 4px',
        }} />
        <span style={{
          position: 'absolute',
          inset: 0,
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'center',
          fontSize: 11,
          fontWeight: 600,
          color: textOnFill ? 'white' : '#555',
          pointerEvents: 'none',
        }}>
          {completed}/{total}
        </span>
      </div>
    </div>
  )
}

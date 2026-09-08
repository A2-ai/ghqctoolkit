import { Anchor, Button, Stack, Text, Tooltip } from '@mantine/core'
import { IconAsterisk } from '@tabler/icons-react'
import type { ReactNode } from 'react'
import type { IssueStatusResponse, RoundInfo } from '~/api/issues'
import { approvalCommentUrl, canStartRound, latestRound, roundApprovedCommit } from '~/api/issues'
import { useChecklistDisplayName } from '~/api/configuration'
import { capitalize } from '~/utils/displayName'
import { ApprovalNotInBranchBadge, FetchBranchBadge, RoundPill } from './RoundBadges'

interface Props {
  status: IssueStatusResponse
  currentBranch: string
  remoteCommit: string
  postApprovalCommit?: string
  /**
   * Opens the new-round modal (U1). The modal itself is owned by the parent: a Mantine
   * Modal is portaled in the DOM but still bubbles React events up its *element* tree,
   * so a modal rendered here would re-open the detail modal on every click inside it.
   */
  onNewRound?: () => void
}

export function IssueCard({ status, currentBranch, remoteCommit, postApprovalCommit, onNewRound }: Props) {
  const { issue, qc_status, dirty, drift, blocking_qc_status } = status
  // D24: the branch and the checklist are round-scoped — read the latest round, never
  // a top-level copy that could disagree with it.
  const round = latestRound(status)
  const branch = round.branch
  const checklist_summary = round.checklist_summary
  const isWrongBranch = branch !== currentBranch
  const { singular } = useChecklistDisplayName()
  const singularCap = capitalize(singular)
  const approvedCommit = roundApprovedCommit(round)
  const approvalUrl = approvalCommentUrl(issue, round)

  // Per-lane commit rows (commits array is newest-first)
  let commitRows: ReactNode = null
  switch (qc_status.status) {
    case 'awaiting_review':
    case 'approval_required':
      commitRows = <ArchiveCommitRow label="Latest" round={round} />
      break
    case 'change_requested':
      commitRows = (
        <>
          <ArchiveCommitRow label="Reviewed" round={round} />
          {remoteCommit && <CommitRow label="Remote" hash={remoteCommit} />}
        </>
      )
      break
    case 'in_progress':
    case 'changes_to_comment':
      commitRows = (
        <>
          <ArchiveCommitRow label="Last Posted" round={round} />
          {remoteCommit && <CommitRow label="Remote" hash={remoteCommit} />}
        </>
      )
      break
    case 'approved':
    case 'changes_after_approval':
      commitRows = (
        <>
          {/* U8: the approved-commit row deep-links the approval comment via state.comment_id. */}
          {approvedCommit && <CommitRow label="Approved" hash={approvedCommit} href={approvalUrl} />}
          {postApprovalCommit && <CommitRow label="Changed" hash={postApprovalCommit} />}
          {/* U6/D31: badge a divergent drift so the Changed hash is not read as meaningful. */}
          {drift.divergent && <ApprovalNotInBranchBadge />}
        </>
      )
      break
  }

  return (
    <Stack
      gap={6}
      style={{
        opacity: isWrongBranch ? 0.45 : 1,
        filter: isWrongBranch ? 'grayscale(0.4)' : 'none',
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

      {/* Which round the QC is on — the same pill the detail modal's card shows, so the
          two cannot drift. */}
      <div style={{ display: 'flex', justifyContent: 'center' }}>
        <RoundPill index={round.index} />
      </div>

      {/* Milestone */}
      {issue.milestone && (
        <Text size="sm" c="black"><b>Milestone:</b> {issue.milestone}</Text>
      )}

      {/* Branch */}
      <Text size="sm" c={isWrongBranch ? 'red' : 'black'}>
        <b>Branch:</b> {branch}{isWrongBranch ? ' (different branch)' : ''}
      </Text>

      {/* Commit info */}
      {commitRows}

      {/* U1: a new round may only be started from an approved QC (D12). */}
      {canStartRound(qc_status.status) && onNewRound && (
        <Button
          size="compact-xs"
          variant="light"
          color="blue"
          data-testid={`new-round-${issue.number}`}
          onClick={(event) => {
            event.stopPropagation()
            onNewRound()
          }}
        >
          New Round
        </Button>
      )}

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
    </Stack>
  )
}

/**
 * D54/D55: `archive_commit` is `null` when the round's representative commit does not
 * resolve. The remedy is named — never a substituted hash. In practice the status
 * endpoint returns `branch_not_local` for an issue whose *latest* round is in that
 * state, so this is the belt-and-braces half of the same refusal.
 */
function ArchiveCommitRow({ label, round }: { label: string; round: RoundInfo }) {
  if (round.archive_commit === null) return <FetchBranchBadge branch={round.branch} />
  return <CommitRow label={label} hash={round.archive_commit} />
}

function CommitRow({ label, hash, href }: { label: string; hash: string; href?: string | null }) {
  const short = hash.slice(0, 7)
  return (
    <Text size="sm" c="black">
      <b>{label}:</b>{' '}
      {href ? (
        <Anchor
          href={href}
          target="_blank"
          onClick={(event) => event.stopPropagation()}
          style={{ fontFamily: 'monospace' }}
        >
          {short}
        </Anchor>
      ) : (
        <span style={{ fontFamily: 'monospace' }}>{short}</span>
      )}
    </Text>
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

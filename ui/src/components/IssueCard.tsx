import { Anchor, Badge, Button, Stack, Text, Tooltip } from '@mantine/core'
import { IconAsterisk } from '@tabler/icons-react'
import type { ReactNode } from 'react'
import type { IssueStatusResponse } from '~/api/issues'
import { useChecklistDisplayName } from '~/api/configuration'
import { capitalize } from '~/utils/displayName'

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
  const { issue, qc_status, dirty, branch, checklist_summary, blocking_qc_status } = status
  const isWrongBranch = branch !== currentBranch
  const { singular } = useChecklistDisplayName()
  const singularCap = capitalize(singular)

  // Round indicator: only for multi-round issues, so the common single
  // `Initial QC` case stays exactly as quiet as it is today.
  const rounds = status.rounds ?? []
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

  // Per-lane commit rows (commits array is newest-first)
  let commitRows: ReactNode = null
  switch (qc_status.status) {
    case 'awaiting_review':
    case 'approval_required':
      commitRows = <CommitRow label="Latest" hash={qc_status.latest_commit} />
      break
    case 'change_requested':
      commitRows = (
        <>
          <CommitRow label="Reviewed" hash={qc_status.latest_commit} />
          {remoteCommit && <CommitRow label="Remote" hash={remoteCommit} />}
        </>
      )
      break
    case 'in_progress':
    case 'changes_to_comment':
      commitRows = (
        <>
          <CommitRow label="Last Posted" hash={qc_status.latest_commit} />
          {remoteCommit && <CommitRow label="Remote" hash={remoteCommit} />}
        </>
      )
      break
    case 'approved':
    case 'changes_after_approval':
      commitRows = (
        <>
          {qc_status.approved_commit && <CommitRow label="Approved" hash={qc_status.approved_commit} />}
          {postApprovalCommit && <CommitRow label="Changed" hash={postApprovalCommit} />}
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
        <b>Branch:</b> {branch}{isWrongBranch ? ' (different branch)' : ''}
      </Text>

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

      {/* Approved — offer another QC round, identically whether or not the file moved */}
      {showStartRound && (
        <Tooltip
          label="Start another QC round on this file. The previous approval stays valid."
          withArrow
          position="top"
          multiline
          w={260}
        >
          <Button
            size="xs"
            variant="light"
            color="orange"
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

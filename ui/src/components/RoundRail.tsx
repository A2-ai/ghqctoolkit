// U2: the round rail — the issue's thread as the API segmented it, oldest-first,
// newest round expanded.
//
// Single-round issues (exactly one `Initial QC`) are by far the common case, so they
// render as a single quiet line with no collapse chrome: the rail must not make them
// noisier than they were before rounds existed. Gaps are unnamed and positional
// (Q6/Q10) — an empty one renders nothing at all, since empty gaps are legal and
// expected (D6) and "0 commits between …" would be pure noise.

import { useState } from 'react'
import { Anchor, Badge, Collapse, Group, Stack, Text, UnstyledButton } from '@mantine/core'
import { IconChevronDown, IconChevronRight } from '@tabler/icons-react'
import type { GapSegment, RoundSegment, Segment } from '~/api/rounds'
import { previousApprovalOf, roundSegments, shortHash, unplaceableReasonText } from '~/utils/rounds'

interface Props {
  /** The thread's segments, oldest-first, exactly as the API returned them. */
  segments: Segment[]
  /**
   * Optional "start a new round" affordance. Wired by whoever owns the
   * start-round modal's state — `SwimLanes`, via `IssueDetailModal` — so the rail
   * itself never owns a dialog. Omitted → no affordance.
   */
  onStartRound?: () => void
}

export function RoundRail({ segments, onStartRound }: Props) {
  if (segments.length === 0) return null

  const rounds = roundSegments(segments)

  // A new round builds on an approval, so it is only offered once the current round
  // has one. By I3 the last segment is a Gap exactly when no round is open — which is
  // what replaced `open_round_index`. While a round *is* open the backend's
  // `can_start` is false anyway, and offering the link invites the very thing that
  // should not happen: a second round opened over an unfinished one, which the fold
  // would treat as an extension rather than a new round.
  const noRoundOpen = segments[segments.length - 1].kind === 'gap'

  return (
    <Stack
      gap={4}
      data-testid="round-rail"
      style={{ maxWidth: 380, marginLeft: 'auto', marginRight: 'auto', width: '100%' }}
    >
      {segments.map((segment, pos) =>
        segment.kind === 'round' ? (
          rounds.length === 1 ? (
            <SingleRoundLine key={pos} round={segment} segments={segments} pos={pos} />
          ) : (
            <RoundSection
              key={pos}
              round={segment}
              segments={segments}
              pos={pos}
              defaultOpen={pos === lastRoundPos(segments)}
            />
          )
        ) : (
          <GapLine key={pos} gap={segment} segments={segments} pos={pos} />
        ),
      )}
      {onStartRound && noRoundOpen && (
        <UnstyledButton onClick={onStartRound} data-testid="round-rail-start">
          <Text size="xs" c="blue">+ Start a new round</Text>
        </UnstyledButton>
      )}
    </Stack>
  )
}

/** Position of the newest Round segment. */
function lastRoundPos(segments: Segment[]): number {
  for (let i = segments.length - 1; i >= 0; i--) {
    if (segments[i].kind === 'round') return i
  }
  return 0
}

/** Compact one-liner for the single-round case. */
function SingleRoundLine({
  round,
  segments,
  pos,
}: {
  round: RoundSegment
  segments: Segment[]
  pos: number
}) {
  return (
    <Group gap={6} wrap="wrap" data-testid={`round-line-${round.index}`}>
      <Text size="xs" fw={700}>{round.name}</Text>
      {/*
        The anchor is only shown for a placed round. `opened_at` is non-nullable on the
        wire, so an anchor that resolved nowhere arrives as the all-zero sha — and an
        unplaceable round's anchor is not a commit anyone can look at.
      */}
      {round.placement.kind === 'placed' && (
        <Text size="xs" c="dimmed">
          from <span style={{ fontFamily: 'monospace' }}>{shortHash(round.opened_at)}</span>
        </Text>
      )}
      <StateBadge round={round} />
      <BranchLine segments={segments} pos={pos} />
      <UnplaceableNote segment={round} pos={pos} />
      <ChecklistLabel round={round} />
    </Group>
  )
}

function RoundSection({
  round,
  segments,
  pos,
  defaultOpen,
}: {
  round: RoundSegment
  segments: Segment[]
  pos: number
  defaultOpen: boolean
}) {
  const [open, setOpen] = useState(defaultOpen)
  const previousApproval = previousApprovalOf(segments, pos)

  return (
    <div data-testid={`round-section-${round.index}`}>
      <UnstyledButton
        onClick={() => setOpen((o) => !o)}
        aria-expanded={open}
        style={{ width: '100%' }}
        data-testid={`round-toggle-${round.index}`}
      >
        <Group gap={6} wrap="nowrap">
          {open ? <IconChevronDown size={12} /> : <IconChevronRight size={12} />}
          <Text size="xs" fw={700}>{round.name}</Text>
          <StateBadge round={round} />
        </Group>
      </UnstyledButton>

      <Collapse in={open}>
        <Stack gap={2} pl={18} pt={2} pb={4} data-testid={`round-detail-${round.index}`}>
          {/* See SingleRoundLine: no anchor line for an unplaceable round. */}
          {round.placement.kind === 'placed' && (
            <Text size="xs" c="dimmed">
              Anchored at <span style={{ fontFamily: 'monospace' }}>{shortHash(round.opened_at)}</span>
            </Text>
          )}
          <Text size="xs" c="dimmed">
            {previousApproval
              ? <>Compares against <span style={{ fontFamily: 'monospace' }}>{shortHash(previousApproval)}</span> (previous approval)</>
              : 'Compares against the initial commit'}
          </Text>
          <BranchLine segments={segments} pos={pos} />
          <UnplaceableNote segment={round} pos={pos} />
          {round.state === 'closed' && (
            <Text size="xs" c="dimmed" data-testid={`round-closed-${round.index}`}>
              Approved <span style={{ fontFamily: 'monospace' }}>{shortHash(round.closing_commit)}</span>
              {round.closed_by ? ` by ${round.closed_by}` : ''}
              {round.closed_at ? ` on ${formatDate(round.closed_at)}` : ''}
            </Text>
          )}
          <Counts round={round} />
          <ChecklistLabel round={round} />
        </Stack>
      </Collapse>
    </div>
  )
}

/**
 * Q10: gaps are unnamed. What matters about one is how many commits it holds and
 * which two rounds it sits between, so that is all it says.
 *
 * An empty gap renders nothing: it is the steady approved state (D6), not an event.
 */
function GapLine({
  gap,
  segments,
  pos,
}: {
  gap: GapSegment
  segments: Segment[]
  pos: number
}) {
  const unplaceable = gap.placement.kind === 'unplaceable'
  if (gap.commits.length === 0 && !unplaceable) return null

  // Positional: a Gap's bounding older round is at `pos - 1`, its newer one at
  // `pos + 1` — the newer one is absent for a trailing gap.
  const older = segments[pos - 1]
  const newer = segments[pos + 1]
  const olderName = older !== undefined && older.kind === 'round' ? older.name : null
  const newerName = newer !== undefined && newer.kind === 'round' ? newer.name : null

  const count = gap.commits.length
  const commits = `${count} commit${count === 1 ? '' : 's'}`
  const where =
    newerName !== null
      ? `between ${olderName} and ${newerName}`
      : `after ${olderName} approval`

  return (
    <Stack gap={2} data-testid={`gap-line-${pos}`}>
      <Text size="xs" c="dimmed" fs={unplaceable ? 'italic' : undefined}>
        {unplaceable ? `Commits ${where} could not be listed` : `${commits} ${where}`}
      </Text>
      {gap.continuity.kind !== 'linear' && (
        <Text size="xs" c="orange" data-testid={`gap-continuity-${pos}`}>
          {gap.continuity.kind === 'diverged'
            ? <>History diverges here — the two ends meet at <span style={{ fontFamily: 'monospace' }}>{shortHash(gap.continuity.merge_base)}</span></>
            : 'The two ends share no history'}
        </Text>
      )}
      <BranchLine segments={segments} pos={pos} />
      <UnplaceableNote segment={gap} pos={pos} />
    </Stack>
  )
}

/**
 * U2: a branch line, shown only when this segment's branch differs from the previous
 * segment's.
 *
 * Every segment declares a branch unconditionally (D5), so printing it on every one
 * would repeat the same name down the whole rail. The interesting fact is the
 * *switch* — a round QC'd somewhere other than where the last one was.
 */
function BranchLine({ segments, pos }: { segments: Segment[]; pos: number }) {
  const previous = segments[pos - 1]
  if (previous === undefined || previous.branch === segments[pos].branch) return null
  return (
    <Text size="xs" c="dimmed" data-testid={`segment-branch-${pos}`}>
      Moved to <span style={{ fontFamily: 'monospace' }}>{segments[pos].branch}</span>
    </Text>
  )
}

/** D4: an unresolvable segment is grayed with its reason, never rendered as an error. */
function UnplaceableNote({ segment, pos }: { segment: Segment; pos: number }) {
  if (segment.placement.kind !== 'unplaceable') return null
  return (
    <Text size="xs" c="dimmed" fs="italic" data-testid={`segment-unplaceable-${pos}`}>
      Commits could not be placed — {unplaceableReasonText(segment.placement.reason)}
    </Text>
  )
}

function StateBadge({ round }: { round: RoundSegment }) {
  return (
    <Badge size="xs" variant="light" color={round.state === 'open' ? 'yellow' : 'green'}>
      {round.state}
    </Badge>
  )
}

/** Counts are only rendered when non-zero, so a quiet round stays quiet. */
function Counts({ round }: { round: RoundSegment }) {
  const parts: string[] = []
  if (round.events.length > 0) parts.push(plural(round.events.length, 'event'))
  if (round.retractions.length > 0) parts.push(plural(round.retractions.length, 'unapproval'))
  if (round.extensions.length > 0) parts.push(plural(round.extensions.length, 'extension'))
  if (parts.length === 0) return null
  return (
    <Text size="xs" c="dimmed" data-testid={`round-counts-${round.index}`}>
      {parts.join(' · ')}
    </Text>
  )
}

/**
 * The round's checklist. Links to the sourcing comment when there is a URL —
 * `comment_url` is nullable even for `kind: 'comment'` (cache-loaded comments),
 * so the plain-text fallback is the normal path, not an error case.
 */
function ChecklistLabel({ round }: { round: RoundSegment }) {
  if (!round.checklist_name) return null
  const url = round.checklist_source.comment_url
  return (
    <Text size="xs" c="dimmed" data-testid={`round-checklist-${round.index}`}>
      Checklist:{' '}
      {url ? (
        <Anchor href={url} target="_blank" size="xs">{round.checklist_name}</Anchor>
      ) : (
        round.checklist_name
      )}
    </Text>
  )
}

function plural(n: number, word: string): string {
  return `${n} ${word}${n === 1 ? '' : 's'}`
}

function formatDate(iso: string): string {
  const d = new Date(iso)
  return Number.isNaN(d.getTime()) ? iso : d.toISOString().slice(0, 10)
}

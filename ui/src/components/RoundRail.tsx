// S1: the round rail — the issue's QC rounds, oldest-first, newest expanded.
//
// Legacy issues (exactly one `Initial QC` round) are by far the common case, so
// they render as a single quiet line with no collapse chrome: the rail must not
// make them noisier than they were before rounds existed.

import { useState } from 'react'
import { Anchor, Badge, Collapse, Group, Stack, Text, UnstyledButton } from '@mantine/core'
import { IconChevronDown, IconChevronRight } from '@tabler/icons-react'
import type { RoundInfo } from '~/api/rounds'
import { shortHash } from '~/utils/rounds'

interface Props {
  rounds: RoundInfo[]
  /**
   * Optional "start a new round" affordance. Wired by whoever owns the
   * start-round modal's state — `SwimLanes`, via `IssueDetailModal` — so the rail
   * itself never owns a dialog. Omitted → no affordance.
   */
  onStartRound?: () => void
}

export function RoundRail({ rounds, onStartRound }: Props) {
  if (rounds.length === 0) return null

  return (
    <Stack
      gap={4}
      data-testid="round-rail"
      style={{ maxWidth: 380, marginLeft: 'auto', marginRight: 'auto', width: '100%' }}
    >
      {rounds.length === 1 ? (
        <SingleRoundLine round={rounds[0]} />
      ) : (
        rounds.map((round, i) => (
          <RoundSection key={round.index} round={round} defaultOpen={i === rounds.length - 1} />
        ))
      )}
      {onStartRound && (
        <UnstyledButton onClick={onStartRound} data-testid="round-rail-start">
          <Text size="xs" c="blue">+ Start a new round</Text>
        </UnstyledButton>
      )}
    </Stack>
  )
}

/** Compact one-liner for the single-round (legacy) case. */
function SingleRoundLine({ round }: { round: RoundInfo }) {
  return (
    <Group gap={6} wrap="wrap" data-testid={`round-line-${round.index}`}>
      <Text size="xs" fw={700}>{round.name}</Text>
      <Text size="xs" c="dimmed">
        from <span style={{ fontFamily: 'monospace' }}>{shortHash(round.opened_at)}</span>
      </Text>
      <StateBadge round={round} />
      <ChecklistLabel round={round} />
    </Group>
  )
}

function RoundSection({ round, defaultOpen }: { round: RoundInfo; defaultOpen: boolean }) {
  const [open, setOpen] = useState(defaultOpen)

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
          <Text size="xs" c="dimmed">
            Anchored at <span style={{ fontFamily: 'monospace' }}>{shortHash(round.opened_at)}</span>
          </Text>
          <Text size="xs" c="dimmed">
            {round.previous_approval
              ? <>Compares against <span style={{ fontFamily: 'monospace' }}>{shortHash(round.previous_approval)}</span> (previous approval)</>
              : 'Compares against the initial commit'}
          </Text>
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

function StateBadge({ round }: { round: RoundInfo }) {
  return (
    <Badge size="xs" variant="light" color={round.state === 'open' ? 'yellow' : 'green'}>
      {round.state}
    </Badge>
  )
}

/** Counts are only rendered when non-zero, so a quiet round stays quiet. */
function Counts({ round }: { round: RoundInfo }) {
  const parts: string[] = []
  if (round.event_count > 0) parts.push(plural(round.event_count, 'event'))
  if (round.retraction_count > 0) parts.push(plural(round.retraction_count, 'unapproval'))
  if (round.extension_count > 0) parts.push(plural(round.extension_count, 'extension'))
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
function ChecklistLabel({ round }: { round: RoundInfo }) {
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

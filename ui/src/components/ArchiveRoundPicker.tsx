// U2: the per-file round picker on an archive card.
//
// Shown only when the thread has more than one round — for a single-round file there is
// nothing to choose, and D9's default is the only answer, so the card stays as quiet as it
// was before rounds existed.
//
// The default is the **latest** round (D9), never the newest approval, and an explicit
// choice is marked as an override so the card says which of the two it is. There is no
// gate here and no confirmation: a reopened file's default is unapproved content, which is
// labelled loudly (U1/U3) and archived anyway — a user cutting an archive for a previous
// QC round retargets with this control.
//
// A round is chosen; a commit never is. The bytes follow from the round (D2/S1), and the
// request carries `{issue_number, round}` and no commit, so this deliberately does *not*
// reuse `RoundCommitPicker`: putting a commit slider here would hand the client back the
// second source of truth the rework removed. The thread's structure comes from `RoundRail`
// instead — the same rendering the status surface uses, so the rounds a user chooses
// between look the way they look everywhere else.

import { Badge, Button, Group, Popover, Stack, Text, UnstyledButton } from '@mantine/core'
import { IconChevronDown } from '@tabler/icons-react'
import { useState } from 'react'
import type { Segment } from '~/api/rounds'
import { RoundRail } from './RoundRail'
import { type ArchiveSelection, describeRound } from '~/utils/archiveSelection'

interface Props {
  segments: Segment[]
  selection: ArchiveSelection
  /** Issue number, for stable test ids. */
  issueNumber: number
  /** `null` resets to the default — the latest round, sent as `round: null`. */
  onSelectRound: (round: number | null) => void
}

export function ArchiveRoundPicker({ segments, selection, issueNumber, onSelectRound }: Props) {
  const [open, setOpen] = useState(false)
  const { rounds, selected, isOverride } = selection

  // Single-round files get no control at all (U2).
  if (selected === null || rounds.length < 2) return null

  return (
    <Popover opened={open} onChange={setOpen} position="bottom-start" withArrow shadow="md" width={340}>
      <Popover.Target>
        <Button
          size="compact-xs"
          variant={isOverride ? 'light' : 'subtle'}
          color={isOverride ? 'violet' : 'gray'}
          rightSection={<IconChevronDown size={11} />}
          onClick={(e) => {
            e.stopPropagation()
            setOpen((o) => !o)
          }}
          data-testid={`archive-round-picker-${issueNumber}`}
          data-override={isOverride ? 'true' : undefined}
        >
          {selected.name}
          {isOverride ? ' · override' : ' · latest'}
        </Button>
      </Popover.Target>
      <Popover.Dropdown onClick={(e) => e.stopPropagation()}>
        <Stack gap={6}>
          <Text size="xs" fw={700}>Archive this file at</Text>
          {rounds.map((round) => {
            const isSelected = round.index === selected.index
            const isLatest = round.index === rounds[rounds.length - 1].index
            return (
              <UnstyledButton
                key={round.index}
                onClick={() => {
                  // The latest round is the default, so choosing it clears the override
                  // rather than pinning the same round by number: `round: null` is the
                  // one encoding of "latest" on the wire.
                  onSelectRound(isLatest ? null : round.index)
                  setOpen(false)
                }}
                data-testid={`archive-round-option-${issueNumber}-${round.index}`}
                data-selected={isSelected ? 'true' : undefined}
                style={{
                  padding: '4px 6px',
                  borderRadius: 4,
                  backgroundColor: isSelected ? 'var(--mantine-color-violet-0)' : undefined,
                  border: `1px solid ${isSelected ? 'var(--mantine-color-violet-3)' : 'transparent'}`,
                }}
              >
                <Group gap={6} wrap="nowrap">
                  <Text size="xs" style={{ flex: 1, minWidth: 0 }}>{describeRound(round)}</Text>
                  {isLatest && <Badge size="xs" variant="light" color="gray">default</Badge>}
                </Group>
              </UnstyledButton>
            )
          })}
          {isOverride && (
            <UnstyledButton
              onClick={() => {
                onSelectRound(null)
                setOpen(false)
              }}
              data-testid={`archive-round-reset-${issueNumber}`}
            >
              <Text size="xs" c="blue">Reset to the latest round</Text>
            </UnstyledButton>
          )}
          <RoundRail segments={segments} />
        </Stack>
      </Popover.Dropdown>
    </Popover>
  )
}

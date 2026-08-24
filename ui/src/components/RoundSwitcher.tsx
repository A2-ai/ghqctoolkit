import { Anchor, Badge, Group, SegmentedControl, Text } from '@mantine/core'
import type { Issue, IssueStatusResponse, RoundInfo } from '~/api/issues'
import { approvalCommentUrl } from '~/api/issues'
import { FetchBranchBadge, InheritedBranchBadge, NoCohesiveHistoryBadge } from './RoundBadges'

interface Props {
  status: IssueStatusResponse
  value: number
  onChange: (roundIndex: number) => void
}

/**
 * U4: the round switcher above `CommitSlider`. The slider renders the selected
 * round's commits **only** (W5) — before rounds it spanned the whole QC's life and
 * became unusable past ~20 commits (§0.5). A single-round QC still renders the
 * switcher's round label, so the scope of the slider is always stated.
 */
export function RoundSwitcher({ status, value, onChange }: Props) {
  const { rounds, issue } = status
  const selected = rounds.find((r) => r.index === value) ?? rounds[rounds.length - 1]

  return (
    <Group gap="xs" align="center" data-testid="round-switcher">
      <Text size="sm" fw={700}>Round:</Text>
      {rounds.length > 1 ? (
        <SegmentedControl
          size="xs"
          value={String(selected.index)}
          onChange={(next) => onChange(Number(next))}
          data={rounds.map((r) => ({ value: String(r.index), label: String(r.index) }))}
        />
      ) : (
        <Text size="sm" data-testid="round-switcher-single">1</Text>
      )}
      <RoundStateBadge issue={issue} round={selected} />
      {/* D53/D55: an unplaceable round renders its index and an explicit
          "fetch <branch>" state — never blank, and never a substituted hash. */}
      {selected.placement === 'unplaceable' && <FetchBranchBadge branch={selected.branch} />}
      {/* D56: a silent inherited branch can mis-scope two walks, so say so here —
          this is one of the surfaces where the round is the one being viewed. */}
      {selected.branch_inherited && <InheritedBranchBadge branch={selected.branch} />}
      {/* U6: the badge belongs on the switcher — a divergent preceding gap means the
          selected round's history does not continue the previous round's. */}
      {selected.preceding_gap.divergent && <NoCohesiveHistoryBadge />}
    </Group>
  )
}

/**
 * D36: `state` is the sole encoding of approvedness, so switch on `kind`. D21/D39.4:
 * "superseded" does **not** imply malformed — an unapproval before a later round
 * comment produces it legitimately — so the copy stays neutral.
 */
function RoundStateBadge({ issue, round }: { issue: Issue; round: RoundInfo }) {
  switch (round.state.kind) {
    case 'approved': {
      const url = approvalCommentUrl(issue, round)
      // D53/D55: an unplaceable round's commits are not local, so no hash here can be
      // checked against the branch. The verdict is still known; the hash is withheld
      // rather than shown beside a "fetch <branch>" state where it would read as placed.
      if (round.placement === 'unplaceable') {
        return <Badge color="green" variant="light" size="xs">approved</Badge>
      }
      return (
        <Badge color="green" variant="light" size="xs">
          {/* U8: deep-link the approval comment via state.comment_id. D44: the id is
              nullable, so with no id the hash renders plain rather than linking to
              comment 0. */}
          approved{' '}
          {url ? (
            <Anchor href={url} target="_blank" size="xs" style={{ fontFamily: 'monospace' }}>
              {round.state.commit.slice(0, 7)}
            </Anchor>
          ) : (
            <span style={{ fontFamily: 'monospace' }}>{round.state.commit.slice(0, 7)}</span>
          )}
        </Badge>
      )
    }
    case 'superseded':
      return <Badge color="gray" variant="light" size="xs">superseded</Badge>
    case 'open':
      return <Badge color="blue" variant="light" size="xs">open</Badge>
  }
}

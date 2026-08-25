import { Alert, Badge, Text, Tooltip } from '@mantine/core'
import type { RoundInfo } from '~/api/issues'

/**
 * Which round a QC is currently on — the latest round's **declared** index (D53.2), never
 * a position or a count, so a QC whose round 2 declaration was dropped still reads
 * "Round 3".
 *
 * Shown on both status cards, including single-round QCs: "Round 1" is information, and a
 * pill that appears only past round 1 makes its absence ambiguous — the reader cannot tell
 * a first round from a UI that forgot to say.
 */
export function RoundPill({ index }: { index: number }) {
  return (
    <Tooltip
      label={`This QC is on round ${index}`}
      withArrow
      position="top"
    >
      <Badge color="gray" variant="light" size="xs" data-testid="round-pill">
        Round {index}
      </Badge>
    </Tooltip>
  )
}

/**
 * U6/D22: a divergent `preceding_gap` — the round's start commit is not descended
 * from the previous round's approval, so the two rounds share no cohesive history.
 * Diffs still work, which is what matters; the badge only stops the sequence from
 * being read as a straight line.
 */
export function NoCohesiveHistoryBadge() {
  return (
    <Tooltip
      label="This round's start commit is not descended from the previous round's approval"
      withArrow
      position="top"
      multiline
      w={280}
    >
      <Badge color="orange" variant="light" size="xs" data-testid="no-cohesive-history-badge">
        no cohesive history
      </Badge>
    </Tooltip>
  )
}

/**
 * U6/D31: a divergent `drift` — a force-push or rebase removed the approval commit
 * from its branch, so the reported `ChangesAfterApproval` hash must not be read as
 * meaningful.
 */
export function ApprovalNotInBranchBadge() {
  return (
    <Tooltip
      label="A force-push or rebase removed the approval commit from this branch, so the reported changed commit is not meaningful"
      withArrow
      position="top"
      multiline
      w={300}
    >
      <Badge color="red" variant="light" size="xs" data-testid="drift-divergent-badge">
        approval commit not in branch history
      </Badge>
    </Tooltip>
  )
}

/**
 * D53/D55: the round is real — it is in the comment log — but its start commit could
 * not be resolved on `branch`, so it owns no commits here. The remedy is local: fetch
 * that branch. This says *which* branch, matching the `branch_not_local` idiom, and
 * deliberately shows no hash: a substituted commit is the audit lie D54 removed.
 */
export function FetchBranchBadge({ branch }: { branch: string }) {
  return (
    <Tooltip
      label={`This round's commits are not local — fetch ${branch} to place it`}
      withArrow
      position="top"
      multiline
      w={300}
    >
      <Badge color="orange" variant="light" size="xs" data-testid="fetch-branch-badge">
        fetch {branch}
      </Badge>
    </Tooltip>
  )
}

/**
 * D56: the round comment declared no `git branch:`, so this branch was inherited from
 * the previous round. Branch scopes both the round's and its gap's commit walk
 * (D7/D9), so the inheritance is surfaced wherever the round is viewed rather than
 * left to look like a declaration.
 */
export function InheritedBranchBadge({ branch }: { branch: string }) {
  return (
    <Tooltip
      label={`This round declared no branch — it inherited ${branch} from the previous round`}
      withArrow
      position="top"
      multiline
      w={300}
    >
      <Badge color="gray" variant="light" size="xs" data-testid="branch-inherited-badge">
        branch inherited
      </Badge>
    </Tooltip>
  )
}

/**
 * D55: the explicit state a round with no local commits renders in place of a commit
 * UI. The slider above it has nothing to draw (`commits` is empty for an unplaceable
 * round, D53.3) — without this the panel would simply go blank, which is the silent
 * degradation §18 exists to remove.
 */
export function UnplaceableRoundAlert({ round }: { round: RoundInfo }) {
  return (
    <Alert color="orange" p="xs" data-testid="unplaceable-round-notice">
      <Text size="xs">
        Round {round.index} is declared on <b>{round.branch}</b>, which is not fetched
        locally, so none of its commits can be shown. Fetch <b>{round.branch}</b> to
        place this round.
      </Text>
    </Alert>
  )
}
